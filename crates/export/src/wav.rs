//! Streaming WAV writer (native, no ffmpeg).

use std::path::Path;

use redraw_shared::{PluginFactory, StudioProject};
use hound::{SampleFormat, WavSpec, WavWriter};

use crate::{render_project, ExportSummary, RenderOptions, Result};

/// PCM/float depth for WAV export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavBitDepth {
    Pcm16,
    Pcm24,
    Pcm32,
    Float32,
}

impl WavBitDepth {
    const fn bits(self) -> u16 {
        match self {
            Self::Pcm16 => 16,
            Self::Pcm24 => 24,
            Self::Pcm32 | Self::Float32 => 32,
        }
    }

    const fn sample_format(self) -> SampleFormat {
        match self {
            Self::Float32 => SampleFormat::Float,
            _ => SampleFormat::Int,
        }
    }
}

pub(crate) fn write(
    path: &Path,
    project: &StudioProject,
    factory: &dyn PluginFactory,
    options: &RenderOptions,
    depth: WavBitDepth,
) -> Result<ExportSummary> {
    let spec = WavSpec {
        channels: 2,
        sample_rate: options.sample_rate,
        bits_per_sample: depth.bits(),
        sample_format: depth.sample_format(),
    };
    let mut writer = WavWriter::create(path, spec)?;
    let summary = render_project(project, factory, options, |left, right| {
        for (&l, &r) in left.iter().zip(right) {
            match depth {
                WavBitDepth::Pcm16 => {
                    writer.write_sample((l.clamp(-1.0, 1.0) * 32_767.0).round() as i16)?;
                    writer.write_sample((r.clamp(-1.0, 1.0) * 32_767.0).round() as i16)?;
                }
                WavBitDepth::Pcm24 => {
                    writer.write_sample((l.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32)?;
                    writer.write_sample((r.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32)?;
                }
                WavBitDepth::Pcm32 => {
                    writer.write_sample((l.clamp(-1.0, 1.0) * 2_147_483_647.0).round() as i32)?;
                    writer.write_sample((r.clamp(-1.0, 1.0) * 2_147_483_647.0).round() as i32)?;
                }
                WavBitDepth::Float32 => {
                    writer.write_sample(l)?;
                    writer.write_sample(r)?;
                }
            }
        }
        Ok(())
    })?;
    writer.finalize()?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_dir, test_project};
    use crate::{ExportOptions, RenderOptions, WavBitDepth};
    use redraw_plugins::BuiltinFactory;

    fn write_test(depth: WavBitDepth) -> (ExportSummary, hound::WavSpec, Vec<f32>) {
        let dir = test_dir("wav");
        let path = dir.join(format!("{:?}.wav", depth));
        let project = test_project();
        let options = RenderOptions {
            scope: crate::ExportScope::Pattern(1),
            sample_rate: 8_000,
            tail_seconds: 0.0,
        };
        let summary = write(&path, &project, &BuiltinFactory::new(), &options, depth).unwrap();
        let mut reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        let samples = match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Int, 16) => reader
                .samples::<i16>()
                .map(|sample| sample.unwrap() as f32 / 32_768.0)
                .collect(),
            (hound::SampleFormat::Int, 24) => reader
                .samples::<i32>()
                .map(|sample| sample.unwrap() as f32 / 8_388_608.0)
                .collect(),
            (hound::SampleFormat::Int, 32) => reader
                .samples::<i32>()
                .map(|sample| sample.unwrap() as f32 / 2_147_483_648.0)
                .collect(),
            (hound::SampleFormat::Float, 32) => {
                reader.samples::<f32>().map(|sample| sample.unwrap()).collect()
            }
            other => panic!("unexpected WAV format {other:?}"),
        };
        (summary, spec, samples)
    }

    #[test]
    fn wav_depths_roundtrip_headers_and_audio() {
        for depth in [
            WavBitDepth::Pcm16,
            WavBitDepth::Pcm24,
            WavBitDepth::Pcm32,
            WavBitDepth::Float32,
        ] {
            let (summary, spec, samples) = write_test(depth);
            assert_eq!(spec.channels, 2);
            assert_eq!(spec.sample_rate, 8_000);
            assert_eq!(spec.bits_per_sample, depth.bits());
            assert_eq!(samples.len() as u64, summary.frames * 2);
            assert!(samples.iter().all(|sample| sample.is_finite()));
        }
    }

    #[test]
    fn wav_export_is_atomic_and_overwrites() {
        let dir = test_dir("wav-overwrite");
        let path = dir.join("song.wav");
        let project = test_project();
        let factory = BuiltinFactory::new();
        let options = ExportOptions {
            render: RenderOptions {
                scope: crate::ExportScope::Pattern(1),
                sample_rate: 8_000,
                tail_seconds: 0.0,
            },
            ..Default::default()
        };
        let first = crate::export_file(
            &path,
            &project,
            &factory,
            &options,
            crate::ExportFormat::Wav(WavBitDepth::Pcm16),
        )
        .unwrap();
        let second = crate::export_file(
            &path,
            &project,
            &factory,
            &options,
            crate::ExportFormat::Wav(WavBitDepth::Pcm16),
        )
        .unwrap();
        assert_eq!(first.frames, second.frames);
        assert!(path.is_file());
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("wav"))
                .count(),
            1
        );
    }
}
