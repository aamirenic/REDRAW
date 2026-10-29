//! Rotating crash-recovery snapshots.
//!
//! Autosaves live under `user-data/autosave/` (the caller supplies the
//! user-data directory). A clean-session marker lets startup distinguish a
//! normal exit from a crash; snapshots are offered only after an unclean exit.
//! The manager keeps the newest 10 `.rdp` snapshots and saves at most once per
//! five minutes unless a caller explicitly forces a save.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use redraw_shared::StudioProject;

use crate::rdp::{self, RdpHeader};
use crate::{FileIoError, Result};

/// Default interval between non-forced snapshots.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(300);
/// Number of snapshots retained.
pub const AUTOSAVE_KEEP: usize = 10;
/// Marker file written on a clean shutdown.
pub const CLEAN_MARKER: &str = "session.clean";

/// One snapshot discovered on disk.
#[derive(Clone, Debug)]
pub struct AutosaveEntry {
    pub path: PathBuf,
    pub modified: SystemTime,
    pub size: u64,
    pub header: Option<RdpHeader>,
}

/// Startup recovery decision.
#[derive(Clone, Debug, Default)]
pub struct RecoveryOffer {
    /// True when the previous shutdown wrote the clean marker.
    pub was_clean_shutdown: bool,
    /// Newest snapshot, present only after an unclean shutdown.
    pub latest: Option<AutosaveEntry>,
}

impl RecoveryOffer {
    /// Whether there is a snapshot to offer to the user.
    pub fn has_recovery(&self) -> bool {
        self.latest.is_some()
    }
}

/// Autosave scheduler and snapshot store.
#[derive(Debug)]
pub struct AutosaveManager {
    dir: PathBuf,
    marker: PathBuf,
    interval: Duration,
    keep: usize,
    last_autosave: Option<Instant>,
}

impl AutosaveManager {
    /// Create a manager rooted at the application's user-data directory.
    pub fn new(user_data_dir: impl AsRef<Path>) -> Self {
        let user_data_dir = user_data_dir.as_ref();
        Self {
            dir: user_data_dir.join("autosave"),
            marker: user_data_dir.join(CLEAN_MARKER),
            interval: AUTOSAVE_INTERVAL,
            keep: AUTOSAVE_KEEP,
            last_autosave: Some(Instant::now()),
        }
    }

    /// Override the save interval (tests / preference wiring).
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Override the retention count (minimum 1).
    pub fn with_keep(mut self, keep: usize) -> Self {
        self.keep = keep.max(1);
        self
    }

    /// Directory containing snapshots.
    pub fn autosave_dir(&self) -> &Path {
        &self.dir
    }

    /// Path of the clean-session marker.
    pub fn clean_marker(&self) -> &Path {
        &self.marker
    }

    /// Called once at startup. If the old session ended cleanly, the marker is
    /// consumed and no recovery is offered. If it did not, the newest snapshot
    /// is returned as a recovery candidate.
    pub fn startup(&self) -> Result<RecoveryOffer> {
        fs::create_dir_all(&self.dir)?;
        let was_clean = self.marker.exists();
        if was_clean {
            fs::remove_file(&self.marker)?;
            return Ok(RecoveryOffer {
                was_clean_shutdown: true,
                latest: None,
            });
        }
        Ok(RecoveryOffer {
            was_clean_shutdown: false,
            latest: self.latest()?,
        })
    }

    /// Mark the session clean and discard completed snapshots so a later crash
    /// before the next autosave cannot resurrect stale data.
    pub fn end_session_clean(&self) -> Result<()> {
        if let Some(parent) = self.marker.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.marker, format!("clean {}\n", unix_seconds()))?;
        let entries = self.list()?;
        for entry in entries {
            match fs::remove_file(&entry.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(FileIoError::Io(e)),
            }
        }
        Ok(())
    }

    /// List snapshots newest-first. Unreadable headers are retained with
    /// `header = None`; the snapshot file itself is never hidden from the list.
    pub fn list(&self) -> Result<Vec<AutosaveEntry>> {
        let reader = match fs::read_dir(&self.dir) {
            Ok(reader) => reader,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(FileIoError::Io(e)),
        };
        let mut entries = Vec::new();
        for entry in reader {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rdp") {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let modified = metadata
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push(AutosaveEntry {
                header: rdp::read_header(&path).ok(),
                path,
                modified,
                size: metadata.len(),
            });
        }
        entries.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| b.path.cmp(&a.path))
        });
        Ok(entries)
    }

    /// Newest snapshot, if any.
    pub fn latest(&self) -> Result<Option<AutosaveEntry>> {
        Ok(self.list()?.into_iter().next())
    }

    /// Load and repair the newest snapshot.
    pub fn recover_latest(&self) -> Result<Option<(AutosaveEntry, StudioProject)>> {
        let Some(entry) = self.latest()? else {
            return Ok(None);
        };
        let project = rdp::load_project(&entry.path)?;
        Ok(Some((entry, project)))
    }

    /// Save a snapshot if the interval has elapsed, or when `force` is set.
    pub fn autosave(
        &mut self,
        project: &StudioProject,
        source_path: Option<&Path>,
        force: bool,
    ) -> Result<Option<PathBuf>> {
        if !force {
            if let Some(last) = self.last_autosave {
                if last.elapsed() < self.interval {
                    return Ok(None);
                }
            }
        }
        let path = self.save_snapshot_at(project, source_path, unix_millis())?;
        self.last_autosave = Some(Instant::now());
        Ok(Some(path))
    }

    /// Explicit snapshot at a caller-supplied timestamp. The timestamp is
    /// embedded in the filename for deterministic tests and manual recovery.
    pub fn save_snapshot_at(
        &mut self,
        project: &StudioProject,
        source_path: Option<&Path>,
        timestamp_ms: u128,
    ) -> Result<PathBuf> {
        fs::create_dir_all(&self.dir)?;
        let stem = source_path
            .and_then(|path| path.file_stem())
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.trim().is_empty())
            .map(sanitize)
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| sanitize(&project.name));
        let stem = if stem.is_empty() {
            "untitled".to_string()
        } else {
            stem
        };
        let mut path = self
            .dir
            .join(format!("{stem}-{timestamp_ms:013}.rdp"));
        let mut suffix = 1u32;
        while path.exists() {
            path = self
                .dir
                .join(format!("{stem}-{timestamp_ms:013}-{suffix}.rdp"));
            suffix += 1;
        }
        rdp::save_project(&path, project)?;
        self.rotate()?;
        Ok(path)
    }

    fn rotate(&self) -> Result<()> {
        // `list` is newest-first; drop the tail beyond the retention limit.
        for entry in self.list()?.into_iter().skip(self.keep) {
            match fs::remove_file(&entry.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(FileIoError::Io(e)),
            }
        }
        Ok(())
    }
}

fn sanitize(stem: &str) -> String {
    let mut out = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' ') {
            out.push(ch);
        } else if ch == '.' {
            out.push('_');
        } else {
            out.push('_');
        }
    }
    out.trim().trim_matches('_').to_string()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir;

    #[test]
    fn unclean_startup_offers_latest_snapshot() {
        let root = test_dir("autosave-crash");
        let mut manager = AutosaveManager::new(&root);
        let first = manager.startup().unwrap();
        assert!(!first.was_clean_shutdown);
        assert!(!first.has_recovery());

        let project = StudioProject::default();
        manager
            .save_snapshot_at(&project, Some(Path::new("My Song.rdp")), 1_000)
            .unwrap();
        manager
            .save_snapshot_at(&project, Some(Path::new("My Song.rdp")), 2_000)
            .unwrap();
        let latest = manager.latest().unwrap().unwrap();
        assert!(latest.path.to_string_lossy().contains("My Song-"));
        assert_eq!(latest.header.as_ref().unwrap().title, project.name);
        let (_, recovered) = manager.recover_latest().unwrap().unwrap();
        assert_eq!(recovered.name, project.name);

        let restarted = AutosaveManager::new(&root);
        let offer = restarted.startup().unwrap();
        assert!(!offer.was_clean_shutdown);
        assert!(offer.has_recovery());
    }

    #[test]
    fn clean_shutdown_suppresses_stale_recovery() {
        let root = test_dir("autosave-clean");
        let mut manager = AutosaveManager::new(&root);
        manager.startup().unwrap();
        manager
            .save_snapshot_at(&StudioProject::default(), None, 10)
            .unwrap();
        manager.end_session_clean().unwrap();
        assert!(manager.latest().unwrap().is_none());

        let restarted = AutosaveManager::new(&root);
        let offer = restarted.startup().unwrap();
        assert!(offer.was_clean_shutdown);
        assert!(!offer.has_recovery());
        assert!(!restarted.clean_marker().exists());
    }

    #[test]
    fn rotation_keeps_newest_ten() {
        let root = test_dir("autosave-rotate");
        let mut manager = AutosaveManager::new(&root);
        manager.startup().unwrap();
        let project = StudioProject::default();
        for i in 0..12u128 {
            manager
                .save_snapshot_at(&project, Some(Path::new("Rotation.rdp")), 1_000 + i)
                .unwrap();
        }
        let entries = manager.list().unwrap();
        assert_eq!(entries.len(), AUTOSAVE_KEEP);
        assert!(entries
            .iter()
            .all(|entry| entry.path.to_string_lossy().contains("Rotation-")));
    }

    #[test]
    fn interval_blocks_non_forced_saves() {
        let root = test_dir("autosave-interval");
        let mut manager = AutosaveManager::new(&root).with_interval(Duration::from_secs(300));
        manager.startup().unwrap();
        let project = StudioProject::default();
        assert!(manager.autosave(&project, None, false).unwrap().is_none());
        assert!(manager.autosave(&project, None, true).unwrap().is_some());
    }

    #[test]
    fn sanitize_keeps_safe_filename_characters() {
        assert_eq!(sanitize("My/Song: v1"), "My_Song_ v1");
        assert_eq!(sanitize(".."), "");
    }
}
