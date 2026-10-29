//! ffmpeg discovery, optional runtime download, and streaming encode.
//!
//! Compressed audio and video are external codecs by design (D-005). PCM is
//! always fed to ffmpeg as interleaved little-endian f32, so no temporary PCM
//! file is needed and rendering memory stays bounded.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use redraw_shared::{PluginFactory, StudioProject};

use crate::{
    render_project, ExportError, ExportFormat, ExportOptions, ExportSummary, Result,
};

/// FFmpeg executable selection and runtime-download policy.
#[derive(Clone, Debug)]
pub struct FfmpegOptions {
    /// Explicit executable. If set, only this path is accepted.
    pub executable: Option<PathBuf>,
    /// Application directory containing `vendor/ffmpeg` (usually cwd).
    pub app_root: Option<PathBuf>,
    /// Allow the first compressed/video export to download a build when absent.
    pub allow_download: bool,
}

impl Default for FfmpegOptions {
    fn default() -> Self {
        Self {
            executable: None,
            app_root: None,
            allow_download: true,
        }
    }
}

impl FfmpegOptions {
    pub fn resolve(&self) -> Result<PathBuf> {
        let (found, searched) = search_ffmpeg(self.executable.as_deref(), self.app_root.as_deref());
        if let Some(path) = found {
            return Ok(path);
        }
        if self.executable.is_some() {
            return Err(ExportError::FfmpegMissing { searched });
        }
        if self.allow_download {
            let root = self
                .app_root
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            return ensure_ffmpeg(&root);
        }
        Err(ExportError::FfmpegMissing { searched })
    }
}

/// Find ffmpeg without downloading it.
pub fn find_ffmpeg() -> Option<PathBuf> {
    search_ffmpeg(None, None).0
}

fn search_ffmpeg(explicit: Option<&Path>, app_root: Option<&Path>) -> (Option<PathBuf>, Vec<PathBuf>) {
    let filename = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = explicit {
        candidates.push(path.to_path_buf());
    } else {
        if let Some(path) = std::env::var_os("REDRAW_FFMPEG") {
            candidates.push(PathBuf::from(path));
        }
        if let Some(root) = app_root {
            candidates.push(root.join("vendor/ffmpeg").join(filename));
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("vendor/ffmpeg").join(filename));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("vendor/ffmpeg").join(filename));
            }
        }
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                candidates.push(dir.join(filename));
            }
        }
    }

    let mut searched = Vec::new();
    for candidate in candidates {
        if searched.contains(&candidate) {
            continue;
        }
        if candidate.is_file() {
            return (Some(candidate), searched);
        }
        searched.push(candidate);
    }
    (None, searched)
}

/// Return `root/vendor/ffmpeg/ffmpeg.exe`, downloading a static Windows build
/// when missing. Network access is explicit: callers that do not want it should
/// disable `FfmpegOptions::allow_download`.
pub fn ensure_ffmpeg(root: &Path) -> Result<PathBuf> {
    let filename = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let dest = root.join("vendor/ffmpeg").join(filename);
    if dest.is_file() {
        return Ok(dest);
    }
    if !cfg!(windows) {
        return Err(ExportError::Download(
            "automatic ffmpeg download is implemented for Windows builds only".into(),
        ));
    }
    download_windows_ffmpeg(root, &dest)?;
    Ok(dest)
}

#[cfg(target_os = "windows")]
const WINDOWS_FFMPEG_ZIP: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

#[cfg(target_os = "windows")]
fn download_windows_ffmpeg(root: &Path, dest: &Path) -> Result<()> {
    let dir = root.join("vendor/ffmpeg");
    fs::create_dir_all(&dir)?;
    let url = std::env::var("REDRAW_FFMPEG_URL").unwrap_or_else(|_| WINDOWS_FFMPEG_ZIP.to_string());
    let zip_path = dir.join("ffmpeg-download.zip");
    let temp_exe = dir.join("ffmpeg-download.tmp.exe");

    let result = (|| {
        let agent = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(120))
            .timeout_write(Duration::from_secs(120))
            .build();
        let response = agent
            .get(&url)
            .set("User-Agent", "REDRAW/0.1")
            .call()
            .map_err(|error| ExportError::Download(error.to_string()))?;
        let mut reader = response.into_reader();
        let mut file = File::create(&zip_path)?;
        io::copy(&mut reader, &mut file)
            .map_err(|error| ExportError::Download(error.to_string()))?;
        file.flush()?;
        drop(file);

        let file = File::open(&zip_path)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|error| ExportError::Download(error.to_string()))?;
        let index = (0..archive.len()).find(|index| {
            archive
                .by_index(*index)
                .ok()
                .map(|entry| entry.name().replace('\\', "/").ends_with("ffmpeg.exe"))
                .unwrap_or(false)
        });
        let index = index.ok_or_else(|| {
            ExportError::Download("the downloaded archive does not contain ffmpeg.exe".into())
        })?;
        let mut entry = archive
            .by_index(index)
            .map_err(|error| ExportError::Download(error.to_string()))?;
        if entry.size() > 512 * 1024 * 1024 {
            return Err(ExportError::Download(
                "the downloaded ffmpeg binary is unreasonably large".into(),
            ));
        }
        let mut output = File::create(&temp_exe)?;
        io::copy(&mut entry, &mut output)
            .map_err(|error| ExportError::Download(error.to_string()))?;
        output.flush()?;
        drop(output);
        fs::rename(&temp_exe, dest)?;
        Ok(())
    })();

    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_file(&temp_exe);
    result
}

#[cfg(not(target_os = "windows"))]
fn download_windows_ffmpeg(_root: &Path, _dest: &Path) -> Result<()> {
    Err(ExportError::Download(
        "automatic ffmpeg download is implemented for Windows builds only".into(),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FfmpegOutput {
    Audio(ExportFormat),
    Video(ExportFormat),
}

impl FfmpegOutput {
    fn args(self, sample_rate: u32) -> Vec<String> {
        let mut args = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-y".into(),
            "-f".into(),
            "f32le".into(),
            "-ar".into(),
            sample_rate.to_string(),
            "-ac".into(),
            "2".into(),
            "-i".into(),
            "pipe:0".into(),
        ];
        match self {
            Self::Audio(format) => {
                let codec: &[&str] = match format {
                    ExportFormat::Mp3 => &["-c:a", "libmp3lame", "-q:a", "2", "-f", "mp3"],
                    ExportFormat::Ogg => &["-c:a", "libvorbis", "-q:a", "5", "-f", "ogg"],
                    ExportFormat::Flac => &[
                        "-c:a",
                        "flac",
                        "-compression_level",
                        "8",
                        "-f",
                        "flac",
                    ],
                    _ => &[],
                };
                args.extend(codec.iter().map(|arg| (*arg).to_string()));
            }
            Self::Video(format) => {
                args.extend([
                    "-f".into(),
                    "lavfi".into(),
                    "-i".into(),
                    format!(
                        "color=c=0x0b0e14:s={}x{}:r={}",
                        crate::VIDEO_WIDTH,
                        crate::VIDEO_HEIGHT,
                        crate::VIDEO_FPS
                    ),
                    "-map".into(),
                    "0:a:0".into(),
                    "-map".into(),
                    "1:v:0".into(),
                    "-shortest".into(),
                ]);
                match format {
                    ExportFormat::Mp4 | ExportFormat::Mov => {
                        args.extend(
                            [
                                "-c:v",
                                "libx264",
                                "-preset",
                                "medium",
                                "-crf",
                                "20",
                                "-pix_fmt",
                                "yuv420p",
                                "-c:a",
                                "aac",
                                "-b:a",
                                "192k",
                                "-movflags",
                                "+faststart",
                                "-f",
                                if format == ExportFormat::Mp4 {
                                    "mp4"
                                } else {
                                    "mov"
                                },
                            ]
                            .iter()
                            .map(|arg| arg.to_string()),
                        );
                    }
                    ExportFormat::Avi => {
                        args.extend(
                            [
                                "-c:v", "mpeg4", "-q:v", "4", "-c:a", "libmp3lame", "-q:a", "2",
                                "-f", "avi",
                            ]
                            .iter()
                            .map(|arg| arg.to_string()),
                        );
                    }
                    _ => {}
                }
            }
        }
        args
    }
}

pub(crate) fn encode(
    executable: &Path,
    output_path: &Path,
    output: FfmpegOutput,
    options: &ExportOptions,
    project: &StudioProject,
    factory: &dyn PluginFactory,
) -> Result<ExportSummary> {
    let mut command = Command::new(executable);
    command
        .args(output.args(options.render.sample_rate))
        .arg(output_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    let log_path = log_path_for(output_path);
    let log = File::create(&log_path)?;
    command.stderr(Stdio::from(log));
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let _ = fs::remove_file(&log_path);
            return Err(ExportError::FfmpegMissing {
                searched: vec![executable.to_path_buf()],
            });
        }
        Err(error) => {
            let _ = fs::remove_file(&log_path);
            return Err(ExportError::Io(error));
        }
    };

    let Some(stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_file(&log_path);
        return Err(ExportError::Invalid(
            "ffmpeg stdin pipe was unavailable".into(),
        ));
    };
    let mut writer = BufWriter::with_capacity(256 * 1024, stdin);
    let mut pcm = Vec::with_capacity(redraw_shared::BLOCK_SIZE * 2 * 4);
    let rendered = render_project(project, factory, &options.render, |left, right| {
        pcm.clear();
        for (&l, &r) in left.iter().zip(right) {
            pcm.extend_from_slice(&l.to_le_bytes());
            pcm.extend_from_slice(&r.to_le_bytes());
        }
        writer.write_all(&pcm)?;
        Ok(())
    });
    let flush = writer.flush();
    drop(writer);

    let rendered = match rendered {
        Ok(summary) => summary,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(&log_path);
            return Err(error);
        }
    };
    if let Err(error) = flush {
        let _ = child.kill();
        let _ = child.wait();
        let detail = read_log(&log_path);
        let _ = fs::remove_file(&log_path);
        return Err(ExportError::FfmpegFailed {
            status: None,
            detail: format!("{error}; {detail}"),
        });
    }
    let status = child.wait()?;
    let detail = read_log(&log_path);
    let _ = fs::remove_file(&log_path);
    if !status.success() {
        return Err(ExportError::FfmpegFailed {
            status: status.code(),
            detail,
        });
    }
    Ok(rendered)
}

fn log_path_for(output_path: &Path) -> PathBuf {
    let mut value: OsString = output_path.as_os_str().to_os_string();
    value.push(".ffmpeg.log");
    PathBuf::from(value)
}

fn read_log(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_dir, test_project};
    use crate::{export_file, ExportScope, RenderOptions, WavBitDepth};

    #[test]
    fn audio_and_video_arguments_are_explicit() {
        let mp3 = FfmpegOutput::Audio(ExportFormat::Mp3).args(44_100);
        assert!(mp3.windows(2).any(|pair| pair == ["-f", "f32le"]));
        assert!(mp3.windows(2).any(|pair| pair == ["-ar", "44100"]));
        assert!(mp3.windows(2).any(|pair| pair == ["-c:a", "libmp3lame"]));

        let flac = FfmpegOutput::Audio(ExportFormat::Flac).args(48_000);
        assert!(flac.windows(2).any(|pair| pair == ["-c:a", "flac"]));

        let mp4 = FfmpegOutput::Video(ExportFormat::Mp4).args(48_000);
        assert!(mp4.iter().any(|arg| arg == "libx264"));
        assert!(mp4.windows(2).any(|pair| pair == ["-pix_fmt", "yuv420p"]));
        assert!(mp4.windows(2).any(|pair| pair == ["-map", "0:a:0"]));
    }

    #[test]
    fn missing_ffmpeg_is_reported_without_spawning() {
        let dir = test_dir("ffmpeg-missing");
        let executable = dir.join("does-not-exist.exe");
        let error = encode(
            &executable,
            &dir.join("out.mp3"),
            FfmpegOutput::Audio(ExportFormat::Mp3),
            &ExportOptions {
                render: RenderOptions {
                    scope: ExportScope::Pattern(1),
                    sample_rate: 8_000,
                    tail_seconds: 0.0,
                },
                ffmpeg: FfmpegOptions {
                    executable: Some(executable.clone()),
                    app_root: None,
                    allow_download: false,
                },
            },
            &test_project(),
            &redraw_plugins::BuiltinFactory::new(),
        );
        assert!(matches!(error, Err(ExportError::FfmpegMissing { .. })));
    }

    #[test]
    fn ffmpeg_options_only_honour_an_explicit_path() {
        let dir = test_dir("ffmpeg-explicit");
        let missing = dir.join("missing.exe");
        let options = FfmpegOptions {
            executable: Some(missing.clone()),
            app_root: Some(dir),
            allow_download: false,
        };
        let error = options.resolve().unwrap_err();
        match error {
            ExportError::FfmpegMissing { searched } => {
                assert_eq!(searched, vec![missing]);
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn ffmpeg_codecs_produce_expected_containers_when_available() {
        let Some(executable) = find_ffmpeg() else {
            eprintln!("ffmpeg not installed; skipping codec execution test");
            return;
        };
        let dir = test_dir("ffmpeg-codecs");
        let project = test_project();
        let factory = redraw_plugins::BuiltinFactory::new();
        let options = ExportOptions {
            render: RenderOptions {
                scope: ExportScope::Pattern(1),
                sample_rate: 48_000,
                tail_seconds: 0.0,
            },
            ffmpeg: FfmpegOptions {
                executable: Some(executable),
                app_root: None,
                allow_download: false,
            },
        };
        for format in [
            ExportFormat::Mp3,
            ExportFormat::Ogg,
            ExportFormat::Flac,
            ExportFormat::Mp4,
            ExportFormat::Mov,
            ExportFormat::Avi,
        ] {
            let path = dir.join(format!("out.{}", format.extension()));
            let summary = export_file(&path, &project, &factory, &options, format)
                .unwrap_or_else(|error| panic!("{format:?}: {error}"));
            assert!(summary.frames > 0, "{format:?}");
            let bytes = std::fs::read(&path).unwrap();
            assert!(bytes.len() > 128, "{format:?}");
            match format {
                ExportFormat::Mp3 => assert!(
                    bytes.starts_with(b"ID3")
                        || (bytes.len() > 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0),
                    "MP3 signature"
                ),
                ExportFormat::Ogg => assert!(bytes.starts_with(b"OggS"), "Ogg signature"),
                ExportFormat::Flac => assert!(bytes.starts_with(b"fLaC"), "FLAC signature"),
                ExportFormat::Mp4 | ExportFormat::Mov => {
                    assert_eq!(&bytes[4..8], b"ftyp", "{format:?} signature")
                }
                ExportFormat::Avi => {
                    assert_eq!(&bytes[..4], b"RIFF", "AVI RIFF");
                    assert_eq!(&bytes[8..12], b"AVI ", "AVI signature");
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn native_export_does_not_require_ffmpeg() {
        let dir = test_dir("native-no-ffmpeg");
        let path = dir.join("native.wav");
        export_file(
            &path,
            &test_project(),
            &redraw_plugins::BuiltinFactory::new(),
            &ExportOptions {
                render: RenderOptions {
                    scope: ExportScope::Pattern(1),
                    sample_rate: 8_000,
                    tail_seconds: 0.0,
                },
                ffmpeg: FfmpegOptions {
                    executable: None,
                    app_root: Some(dir.join("no-ffmpeg")),
                    allow_download: false,
                },
            },
            ExportFormat::Wav(WavBitDepth::Pcm16),
        )
        .unwrap();
        assert!(path.is_file());
    }
}
