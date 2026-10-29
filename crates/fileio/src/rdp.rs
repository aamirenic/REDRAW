//! Native REDRAW project (`.rdp`) container.
//!
//! Layout (D-004 / D-022):
//!
//! ```text
//! ZIP:
//!   redraw.json   JSON RdpHeader
//!   project.bin   zlib(bincode(StudioProject))
//! ```
//!
//! Both entries are stored uncompressed inside the ZIP (method 0); the body
//! carries its own zlib stream. Each ZIP entry and the project body receive a
//! CRC32. The ZIP is deliberately minimal: no compression, no extras, no
//! comments, no Zip64. Saving is atomic through a same-directory temporary file.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::{DeflateDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use redraw_shared::{StudioProject, APP_NAME};
use serde::{Deserialize, Serialize};

use crate::{atomic_write, repair_project, FileIoError, Result, MAX_PROJECT_BYTES};

/// Current header/container version.
pub const RDP_VERSION: u16 = 1;
/// Value of [`RdpHeader::format`].
pub const RDP_FORMAT: &str = "redraw-project";

const HEADER_ENTRY: &str = "redraw.json";
const BODY_ENTRY: &str = "project.bin";
const MAX_ZIP_ENTRIES: usize = 64;
const MAX_ZIP_ENTRY_BYTES: usize = MAX_PROJECT_BYTES + 16 * 1024 * 1024;
const MAX_ZIP_TOTAL_BYTES: usize = MAX_PROJECT_BYTES * 2;
const MAX_CONTAINER_BYTES: u64 = 512 * 1024 * 1024;
const ZIP_LOCAL_SIG: u32 = 0x0403_4b50;
const ZIP_CENTRAL_SIG: u32 = 0x0201_4b50;
const ZIP_EOCD_SIG: u32 = 0x0605_4b50;

/// Human-readable and machine-checkable metadata stored in `redraw.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RdpHeader {
    pub app: String,
    pub format: String,
    pub version: u16,
    pub created: u64,
    pub modified: u64,
    pub title: String,
    pub author: String,
    pub bpm: f32,
    pub patterns: u32,
    pub channels: u32,
    pub automation: u32,
    pub samples: Vec<String>,
    /// CRC32 of the uncompressed bincode body.
    pub body_crc32: u32,
    /// Length of the zlib-compressed body entry.
    pub body_size: u64,
    /// Length of the uncompressed bincode body.
    pub body_uncompressed: u64,
}

/// Encode a project with the current timestamp.
pub fn encode_project(project: &StudioProject) -> Result<Vec<u8>> {
    let now = unix_now();
    encode_with_times(project, now, now)
}

/// Encode a project with an explicit timestamp (used by tests/autosave tools).
pub fn encode_project_at(project: &StudioProject, timestamp: u64) -> Result<Vec<u8>> {
    encode_with_times(project, timestamp, timestamp)
}

fn encode_with_times(project: &StudioProject, created: u64, modified: u64) -> Result<Vec<u8>> {
    let body = bincode::serialize(project)?;
    if body.len() > MAX_PROJECT_BYTES {
        return Err(FileIoError::Invalid(format!(
            "project body is {} bytes, limit is {}",
            body.len(),
            MAX_PROJECT_BYTES
        )));
    }
    let crc = crc32fast::hash(&body);
    let compressed = zlib_compress(&body)?;
    let mut samples: Vec<String> = project
        .playlist
        .clips
        .iter()
        .filter_map(|clip| clip.audio_path.clone())
        .collect();
    samples.sort();
    samples.dedup();

    let header = RdpHeader {
        app: APP_NAME.to_string(),
        format: RDP_FORMAT.to_string(),
        version: RDP_VERSION,
        created,
        modified,
        title: project.name.clone(),
        author: project.author.clone(),
        bpm: if project.bpm.is_finite() {
            project.bpm
        } else {
            0.0
        },
        patterns: project.patterns.len().min(u32::MAX as usize) as u32,
        channels: project.channels.len().min(u32::MAX as usize) as u32,
        automation: project.automation.len().min(u32::MAX as usize) as u32,
        samples,
        body_crc32: crc,
        body_size: compressed.len() as u64,
        body_uncompressed: body.len() as u64,
    };
    let header_json = serde_json::to_vec_pretty(&header)?;

    zip_write(&[
        (HEADER_ENTRY, header_json.as_slice()),
        (BODY_ENTRY, compressed.as_slice()),
    ])
}

/// Decode and structurally repair a project from `.rdp` bytes.
pub fn decode_project(bytes: &[u8]) -> Result<StudioProject> {
    let entries = zip_read(bytes)?;
    let header_entry = unique_entry(&entries, HEADER_ENTRY)?;
    let body_entry = unique_entry(&entries, BODY_ENTRY)?;
    let header: RdpHeader = serde_json::from_slice(header_entry)?;

    if header.app != APP_NAME {
        return Err(FileIoError::Invalid(format!(
            "header app is {:?}, expected {:?}",
            header.app, APP_NAME
        )));
    }
    if header.format != RDP_FORMAT {
        return Err(FileIoError::Invalid(format!(
            "header format is {:?}, expected {:?}",
            header.format, RDP_FORMAT
        )));
    }
    if header.version == 0 || header.version > RDP_VERSION {
        return Err(FileIoError::Invalid(format!(
            "unsupported RDP header version {} (current {})",
            header.version, RDP_VERSION
        )));
    }
    if body_entry.len() as u64 != header.body_size {
        return Err(FileIoError::Invalid(format!(
            "body size mismatch: header {}, container {}",
            header.body_size,
            body_entry.len()
        )));
    }
    if header.body_uncompressed > MAX_PROJECT_BYTES as u64 {
        return Err(FileIoError::Invalid(format!(
            "uncompressed body is {} bytes, limit is {}",
            header.body_uncompressed, MAX_PROJECT_BYTES
        )));
    }

    let body = zlib_decompress(body_entry, MAX_PROJECT_BYTES)?;
    if body.len() as u64 != header.body_uncompressed {
        return Err(FileIoError::Invalid(format!(
            "uncompressed body size mismatch: header {}, actual {}",
            header.body_uncompressed,
            body.len()
        )));
    }
    if crc32fast::hash(&body) != header.body_crc32 {
        return Err(FileIoError::Invalid(
            "uncompressed body CRC32 does not match header".into(),
        ));
    }

    let mut cursor = Cursor::new(body.as_slice());
    let mut project: StudioProject = bincode::deserialize_from(&mut cursor)?;
    if cursor.position() as usize != body.len() {
        return Err(FileIoError::Invalid(format!(
            "{} trailing bytes after project body",
            body.len() - cursor.position() as usize
        )));
    }
    repair_project(&mut project);
    Ok(project)
}

/// Save a project atomically. Existing creation time is preserved when the
/// destination is a readable RDP file.
pub fn save_project(path: impl AsRef<Path>, project: &StudioProject) -> Result<()> {
    let path = path.as_ref();
    let now = unix_now();
    let created = read_header(path)
        .ok()
        .map(|header| header.created)
        .filter(|created| *created <= now)
        .unwrap_or(now);
    let bytes = encode_with_times(project, created, now)?;
    atomic_write(path, &bytes)?;
    Ok(())
}

/// Load and repair a project from a `.rdp` file.
pub fn load_project(path: impl AsRef<Path>) -> Result<StudioProject> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_CONTAINER_BYTES {
        return Err(FileIoError::Invalid(format!(
            "container is {} bytes, limit is {}",
            metadata.len(),
            MAX_CONTAINER_BYTES
        )));
    }
    let bytes = fs::read(path)?;
    decode_project(&bytes)
}

/// Read only the header of a `.rdp` file.
pub fn read_header(path: impl AsRef<Path>) -> Result<RdpHeader> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_CONTAINER_BYTES {
        return Err(FileIoError::Invalid(format!(
            "container is {} bytes, limit is {}",
            metadata.len(),
            MAX_CONTAINER_BYTES
        )));
    }
    let bytes = fs::read(path)?;
    read_header_from_bytes(&bytes)
}

/// Read only the header from in-memory `.rdp` bytes.
pub fn read_header_from_bytes(bytes: &[u8]) -> Result<RdpHeader> {
    let entries = zip_read(bytes)?;
    let entry = unique_entry(&entries, HEADER_ENTRY)?;
    let header: RdpHeader = serde_json::from_slice(entry)?;
    if header.format != RDP_FORMAT {
        return Err(FileIoError::Invalid(format!(
            "header format is {:?}, expected {:?}",
            header.format, RDP_FORMAT
        )));
    }
    Ok(header)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unique_entry<'a>(entries: &'a [ZipEntry], name: &str) -> Result<&'a [u8]> {
    let mut found = None;
    for entry in entries {
        if entry.name == name {
            if found.is_some() {
                return Err(FileIoError::Zip(format!(
                    "duplicate required entry {name:?}"
                )));
            }
            found = Some(entry.data.as_slice());
        }
    }
    found.ok_or_else(|| FileIoError::Zip(format!("missing required entry {name:?}")))
}

fn zlib_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    let out = encoder.finish()?;
    Ok(out)
}

fn zlib_decompress(data: &[u8], max: usize) -> Result<Vec<u8>> {
    let decoder = ZlibDecoder::new(data);
    let mut limited = decoder.take(max as u64 + 1);
    let mut out = Vec::new();
    limited.read_to_end(&mut out)?;
    if out.len() > max {
        return Err(FileIoError::Invalid(format!(
            "decompressed body exceeds {max} bytes"
        )));
    }
    Ok(out)
}

// ------------------------------------------------------------------- ZIP writer

fn zip_write(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    if entries.is_empty() || entries.len() > u16::MAX as usize {
        return Err(FileIoError::Zip(format!(
            "invalid ZIP entry count {}",
            entries.len()
        )));
    }

    let mut out = Vec::new();
    let mut central = Vec::new();
    // 1980-01-01 00:00:00, the DOS epoch. The RDP header carries real times.
    const DOS_TIME: u16 = 0;
    const DOS_DATE: u16 = 0x0021;

    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        if name_bytes.is_empty() || name_bytes.len() > u16::MAX as usize {
            return Err(FileIoError::Zip(format!("invalid ZIP entry name {name:?}")));
        }
        let size = u32::try_from(data.len())
            .map_err(|_| FileIoError::Zip(format!("entry {name:?} is too large")))?;
        let offset = u32::try_from(out.len())
            .map_err(|_| FileIoError::Zip("archive exceeds 4 GiB".into()))?;
        let crc = crc32fast::hash(data);

        out.extend_from_slice(&ZIP_LOCAL_SIG.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 name
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&DOS_TIME.to_le_bytes());
        out.extend_from_slice(&DOS_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        central.extend_from_slice(&ZIP_CENTRAL_SIG.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // stored
        central.extend_from_slice(&DOS_TIME.to_le_bytes());
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }

    let central_offset = u32::try_from(out.len())
        .map_err(|_| FileIoError::Zip("archive exceeds 4 GiB".into()))?;
    let central_size = u32::try_from(central.len())
        .map_err(|_| FileIoError::Zip("central directory is too large".into()))?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&ZIP_EOCD_SIG.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // central-start disk
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment
    Ok(out)
}

// ------------------------------------------------------------------- ZIP reader

struct ZipEntry {
    name: String,
    data: Vec<u8>,
}

fn zip_read(bytes: &[u8]) -> Result<Vec<ZipEntry>> {
    if bytes.len() < 22 {
        return Err(FileIoError::Zip("file is shorter than an EOCD record".into()));
    }
    let eocd = find_eocd(bytes)?;
    let disk = zip_u16(bytes, eocd + 4)?;
    let central_disk = zip_u16(bytes, eocd + 6)?;
    let entries_on_disk = zip_u16(bytes, eocd + 8)?;
    let entry_count = zip_u16(bytes, eocd + 10)? as usize;
    let central_size = zip_u32(bytes, eocd + 12)? as usize;
    let central_offset = zip_u32(bytes, eocd + 16)? as usize;
    if disk != 0 || central_disk != 0 || entries_on_disk as usize != entry_count {
        return Err(FileIoError::Zip("multi-disk ZIP archives are not supported".into()));
    }
    if entry_count == 0 || entry_count > MAX_ZIP_ENTRIES {
        return Err(FileIoError::Zip(format!(
            "unsupported ZIP entry count {entry_count}"
        )));
    }
    let central_end = central_offset
        .checked_add(central_size)
        .ok_or_else(|| FileIoError::Zip("central directory range overflow".into()))?;
    if central_end > eocd || central_end > bytes.len() {
        return Err(FileIoError::Zip("central directory is out of range".into()));
    }

    let mut entries = Vec::with_capacity(entry_count);
    let mut cursor = central_offset;
    let mut total_uncompressed = 0usize;
    for _ in 0..entry_count {
        zip_need(bytes, cursor, 46)?;
        if zip_u32(bytes, cursor)? != ZIP_CENTRAL_SIG {
            return Err(FileIoError::Zip(format!(
                "missing central header at offset {cursor}"
            )));
        }
        let flags = zip_u16(bytes, cursor + 8)?;
        let method = zip_u16(bytes, cursor + 10)?;
        let crc = zip_u32(bytes, cursor + 16)?;
        let compressed_size = zip_u32(bytes, cursor + 20)? as usize;
        let uncompressed_size = zip_u32(bytes, cursor + 24)? as usize;
        let name_len = zip_u16(bytes, cursor + 28)? as usize;
        let extra_len = zip_u16(bytes, cursor + 30)? as usize;
        let comment_len = zip_u16(bytes, cursor + 32)? as usize;
        let local_offset = zip_u32(bytes, cursor + 42)? as usize;
        let name_start = cursor + 46;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or_else(|| FileIoError::Zip("ZIP name length overflow".into()))?;
        let next = name_end
            .checked_add(extra_len)
            .and_then(|v| v.checked_add(comment_len))
            .ok_or_else(|| FileIoError::Zip("ZIP central entry length overflow".into()))?;
        zip_need(bytes, name_end, extra_len + comment_len)?;
        if next > central_end {
            return Err(FileIoError::Zip("central entry exceeds directory".into()));
        }
        let name = std::str::from_utf8(&bytes[name_start..name_end])
            .map_err(|_| FileIoError::Zip("entry name is not valid UTF-8".into()))?
            .to_string();
        if name.is_empty() {
            return Err(FileIoError::Zip("empty entry name".into()));
        }
        if flags & 1 != 0 {
            return Err(FileIoError::Zip(format!("encrypted entry {name:?}")));
        }
        if compressed_size > MAX_ZIP_ENTRY_BYTES || uncompressed_size > MAX_ZIP_ENTRY_BYTES {
            return Err(FileIoError::Zip(format!("entry {name:?} exceeds size limit")));
        }

        zip_need(bytes, local_offset, 30)?;
        if zip_u32(bytes, local_offset)? != ZIP_LOCAL_SIG {
            return Err(FileIoError::Zip(format!(
                "missing local header for {name:?}"
            )));
        }
        let local_flags = zip_u16(bytes, local_offset + 6)?;
        let local_method = zip_u16(bytes, local_offset + 8)?;
        let local_name_len = zip_u16(bytes, local_offset + 26)? as usize;
        let local_extra_len = zip_u16(bytes, local_offset + 28)? as usize;
        if local_flags & 1 != 0 {
            return Err(FileIoError::Zip(format!("encrypted entry {name:?}")));
        }
        if local_method != method {
            return Err(FileIoError::Zip(format!(
                "compression method mismatch for {name:?}"
            )));
        }
        let local_name_start = local_offset + 30;
        let local_name_end = local_name_start
            .checked_add(local_name_len)
            .ok_or_else(|| FileIoError::Zip("local name length overflow".into()))?;
        zip_need(bytes, local_name_end, local_extra_len)?;
        if bytes.get(local_name_start..local_name_end) != Some(name.as_bytes()) {
            return Err(FileIoError::Zip(format!(
                "local/central name mismatch for {name:?}"
            )));
        }
        let data_start = local_name_end
            .checked_add(local_extra_len)
            .ok_or_else(|| FileIoError::Zip("local data offset overflow".into()))?;
        let data_end = data_start
            .checked_add(compressed_size)
            .ok_or_else(|| FileIoError::Zip("entry data range overflow".into()))?;
        if data_end > bytes.len() {
            return Err(FileIoError::Zip(format!("entry {name:?} is truncated")));
        }

        let data = match method {
            0 => {
                if compressed_size != uncompressed_size {
                    return Err(FileIoError::Zip(format!(
                        "stored entry {name:?} has unequal sizes"
                    )));
                }
                bytes[data_start..data_end].to_vec()
            }
            8 => {
                let decoder = DeflateDecoder::new(&bytes[data_start..data_end]);
                let mut limited = decoder.take(uncompressed_size as u64 + 1);
                let mut out = Vec::new();
                limited.read_to_end(&mut out)?;
                if out.len() != uncompressed_size {
                    return Err(FileIoError::Zip(format!(
                        "deflated entry {name:?} size mismatch"
                    )));
                }
                out
            }
            other => {
                return Err(FileIoError::Zip(format!(
                    "unsupported ZIP method {other} for {name:?}"
                )))
            }
        };
        if crc32fast::hash(&data) != crc {
            return Err(FileIoError::Zip(format!("CRC32 mismatch for {name:?}")));
        }
        total_uncompressed = total_uncompressed.saturating_add(data.len());
        if total_uncompressed > MAX_ZIP_TOTAL_BYTES {
            return Err(FileIoError::Zip("uncompressed archive exceeds size limit".into()));
        }
        entries.push(ZipEntry { name, data });
        cursor = next;
    }

    if cursor > central_end {
        return Err(FileIoError::Zip("central directory overrun".into()));
    }
    Ok(entries)
}

fn find_eocd(bytes: &[u8]) -> Result<usize> {
    if bytes.len() < 22 {
        return Err(FileIoError::Zip("file is too short".into()));
    }
    let last = bytes.len() - 22;
    let first = last.saturating_sub(u16::MAX as usize);
    for pos in (first..=last).rev() {
        if bytes.get(pos..pos + 4) == Some(&ZIP_EOCD_SIG.to_le_bytes()) {
            let comment_len = zip_u16(bytes, pos + 20)? as usize;
            if pos + 22 + comment_len == bytes.len() {
                return Ok(pos);
            }
        }
    }
    Err(FileIoError::Zip("EOCD record not found".into()))
}

fn zip_need(bytes: &[u8], pos: usize, len: usize) -> Result<()> {
    let end = pos
        .checked_add(len)
        .ok_or_else(|| FileIoError::Zip("ZIP offset overflow".into()))?;
    if end > bytes.len() {
        return Err(FileIoError::Zip(format!("truncated ZIP at offset {pos}")));
    }
    Ok(())
}

fn zip_u16(bytes: &[u8], pos: usize) -> Result<u16> {
    zip_need(bytes, pos, 2)?;
    Ok(u16::from_le_bytes([bytes[pos], bytes[pos + 1]]))
}

fn zip_u32(bytes: &[u8], pos: usize) -> Result<u32> {
    zip_need(bytes, pos, 4)?;
    Ok(u32::from_le_bytes([
        bytes[pos],
        bytes[pos + 1],
        bytes[pos + 2],
        bytes[pos + 3],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir;
    use redraw_shared::{
        AutoTarget, AutomationClipState, AutomationPoint, ClipKind, ClipState, FxSlotState, Interp,
    };

    fn rich_project() -> StudioProject {
        let mut project = StudioProject::default();
        project.name = "Round Trip".into();
        project.author = "REDRAW test".into();
        project.bpm = 123.5;
        project.time_sig = (7, 8);
        project.swing = 0.58;
        project.patterns[0].name = "Verse".into();
        project.patterns[0].channels[0].notes.push(redraw_shared::Note {
            id: 42,
            key: 64,
            start: 240,
            length: 480,
            velocity: 101,
            pan: -12,
            release: 73,
        });
        project.playlist.clips.push(ClipState {
            id: 1,
            track: 0,
            start: 0,
            length: 3840,
            kind: ClipKind::Audio,
            source_id: 1,
            name: "User loop".into(),
            color: [1, 2, 3],
            looping: true,
            gain_db: -3.0,
            transpose: 0,
            stretch: 1.0,
            audio_path: Some("C:/User Samples/loop.wav".into()),
        });
        project.automation.push(AutomationClipState {
            id: 1,
            name: "Master".into(),
            color: [10, 20, 30],
            target: AutoTarget::MasterVolume,
            points: vec![
                AutomationPoint {
                    time: 0.0,
                    value: 0.4,
                    tension: 0.0,
                },
                AutomationPoint {
                    time: 3840.0,
                    value: 1.0,
                    tension: 0.2,
                },
            ],
            interp: Interp::Smooth,
            enabled: true,
            length: 3840,
        });
        project.next_note_id = 43;
        project.next_clip_id = 2;
        project.next_automation_id = 2;
        project
    }

    #[test]
    fn rdp_roundtrip_preserves_project_and_header() {
        let project = rich_project();
        let bytes = encode_project_at(&project, 1_234).unwrap();
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        let header = read_header_from_bytes(&bytes).unwrap();
        assert_eq!(header.app, APP_NAME);
        assert_eq!(header.format, RDP_FORMAT);
        assert_eq!(header.version, RDP_VERSION);
        assert_eq!(header.created, 1_234);
        assert_eq!(header.modified, 1_234);
        assert_eq!(header.title, "Round Trip");
        assert_eq!(header.bpm, 123.5);
        assert_eq!(header.patterns, 1);
        assert_eq!(header.channels, 2);
        assert_eq!(header.automation, 1);
        assert_eq!(header.samples, vec!["C:/User Samples/loop.wav"]);
        assert!(header.body_size > 0);
        assert!(header.body_uncompressed > header.body_size);

        let decoded = decode_project(&bytes).unwrap();
        assert_eq!(
            bincode::serialize(&project).unwrap(),
            bincode::serialize(&decoded).unwrap()
        );
    }

    #[test]
    fn rdp_load_resolves_pre_phase_16_type_names() {
        let mut project = rich_project();
        project.channels[0].synth_type = "Toxic FM".into();
        project.mixer.strips[0].inserts[0] = Some(FxSlotState {
            effect_type: "Gross Beat".into(),
            params: vec![0.25],
            bypass: false,
        });
        project.mixer.master.inserts[0] = Some(FxSlotState {
            effect_type: "Fruity Compressor".into(),
            params: Vec::new(),
            bypass: true,
        });
        project.mixer.bus_inserts[1][2] = Some(FxSlotState {
            effect_type: "Edison".into(),
            params: Vec::new(),
            bypass: false,
        });

        let bytes = encode_project_at(&project, 7).unwrap();
        let loaded = decode_project(&bytes).unwrap();

        assert_eq!(loaded.channels[0].synth_type, "Ember FM");
        assert_eq!(loaded.channels[1].synth_type, "KickForge");
        assert_eq!(
            loaded.mixer.strips[0].inserts[0]
                .as_ref()
                .unwrap()
                .effect_type,
            "Time Loom"
        );
        assert_eq!(
            loaded.mixer.master.inserts[0]
                .as_ref()
                .unwrap()
                .effect_type,
            "Glue Comp"
        );
        assert_eq!(
            loaded.mixer.bus_inserts[1][2]
                .as_ref()
                .unwrap()
                .effect_type,
            "Wave Bench"
        );
        // Only the type name changes; parameters and bypass are preserved.
        assert_eq!(
            loaded.mixer.strips[0].inserts[0]
                .as_ref()
                .unwrap()
                .params,
            vec![0.25]
        );
        assert!(loaded.mixer.master.inserts[0].as_ref().unwrap().bypass);
    }

    #[test]
    fn rdp_save_and_load_from_disk() {
        let dir = test_dir("rdp");
        let path = dir.join("song.rdp");
        let project = rich_project();
        save_project(&path, &project).unwrap();
        assert!(path.exists());
        let loaded = load_project(&path).unwrap();
        assert_eq!(
            bincode::serialize(&project).unwrap(),
            bincode::serialize(&loaded).unwrap()
        );
        let header = read_header(&path).unwrap();
        assert_eq!(header.title, "Round Trip");

        // A second save replaces the file atomically and keeps the creation time.
        save_project(&path, &project).unwrap();
        assert_eq!(load_project(&path).unwrap().name, "Round Trip");
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
                .count(),
            0
        );
    }

    #[test]
    fn rdp_rejects_corrupted_body_crc() {
        let bytes = encode_project_at(&rich_project(), 7).unwrap();
        let name_pos = bytes
            .windows(BODY_ENTRY.len())
            .position(|w| w == BODY_ENTRY.as_bytes())
            .unwrap();
        let data_pos = name_pos + BODY_ENTRY.len();
        let mut corrupted = bytes.clone();
        corrupted[data_pos] ^= 0x40;
        let err = decode_project(&corrupted).unwrap_err();
        assert!(err.to_string().contains("CRC32"), "{err}");
    }

    #[test]
    fn rdp_rejects_truncated_and_bad_header() {
        let bytes = encode_project_at(&rich_project(), 7).unwrap();
        assert!(decode_project(&bytes[..bytes.len() / 2]).is_err());

        let bad_header = br#"{"format":"something-else"}"#;
        let body = zlib_compress(b"not a project").unwrap();
        let bad = zip_write(&[
            (HEADER_ENTRY, bad_header.as_slice()),
            (BODY_ENTRY, body.as_slice()),
        ])
        .unwrap();
        assert!(decode_project(&bad).is_err());
    }

    #[test]
    fn rdp_header_without_body_is_rejected() {
        let header = br#"{"app":"REDRAW","format":"redraw-project","version":1,"created":0,"modified":0,"title":"x","author":"","bpm":120.0,"patterns":0,"channels":0,"automation":0,"samples":[],"body_crc32":0,"body_size":0,"body_uncompressed":0}"#;
        let bytes = zip_write(&[(HEADER_ENTRY, header.as_slice())]).unwrap();
        let err = decode_project(&bytes).unwrap_err();
        assert!(err.to_string().contains("project.bin"), "{err}");
    }
}
