//! Streaming AIFF writer (native, no ffmpeg).

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use redraw_shared::{PluginFactory, StudioProject};

use crate::{render_project, ExportError, ExportSummary, RenderOptions, Result};

/// PCM depth for AIFF export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiffBitDepth {
    Pcm16,
    Pcm24,
    Pcm32,
}

impl AiffBitDepth {
    const fn bits(self) -> u16 {
        match self {
            Self::Pcm16 => 16,
            Self::Pcm24 => 24,
            Self::Pcm32 => 32,
        }
    }

    const fn bytes(self) -> u64 {
        self.bits() as u64 / 8
    }
}

pub(crate) fn write(
    path: &Path,
    project: &StudioProject,
    factory: &dyn PluginFactory,
    options: &RenderOptions,
    depth: AiffBitDepth,
) -> Result<ExportSummary> {
    let mut file = File::create(path)?;
    write_header(&mut file, options.sample_rate, depth)?;
    let summary = render_project(project, factory, options, |left, right| {
        for (&l, &r) in left.iter().zip(right) {
            write_sample(&mut file, l, depth)?;
            write_sample(&mut file, r, depth)?;
        }
        Ok(())
    })?;

    let data_bytes = summary
        .frames
        .checked_mul(2)
        .and_then(|frames| frames.checked_mul(depth.bytes()))
        .ok_or_else(|| ExportError::Invalid("AIFF sample data is too large".into()))?;
    let form_size = 46u64
        .checked_add(data_bytes)
        .ok_or_else(|| ExportError::Invalid("AIFF form size overflow".into()))?;
    let ssnd_size = 8u64
        .checked_add(data_bytes)
        .ok_or_else(|| ExportError::Invalid("AIFF SSND size overflow".into()))?;
    let frames = u32::try_from(summary.frames)
        .map_err(|_| ExportError::Invalid("AIFF has more than 4 GiB of sample frames".into()))?;
    let form_size = u32::try_from(form_size)
        .map_err(|_| ExportError::Invalid("AIFF file exceeds 4 GiB".into()))?;
    let ssnd_size = u32::try_from(ssnd_size)
        .map_err(|_| ExportError::Invalid("AIFF SSND chunk exceeds 4 GiB".into()))?;

    file.seek(SeekFrom::Start(4))?;
    file.write_all(&form_size.to_be_bytes())?;
    file.seek(SeekFrom::Start(22))?;
    file.write_all(&frames.to_be_bytes())?;
    file.seek(SeekFrom::Start(42))?;
    file.write_all(&ssnd_size.to_be_bytes())?;
    file.flush()?;
    Ok(summary)
}

fn write_header(file: &mut File, sample_rate: u32, depth: AiffBitDepth) -> Result<()> {
    file.write_all(b"FORM")?;
    file.write_all(&0u32.to_be_bytes())?; // patched after rendering
    file.write_all(b"AIFF")?;

    file.write_all(b"COMM")?;
    file.write_all(&18u32.to_be_bytes())?;
    file.write_all(&2u16.to_be_bytes())?;
    file.write_all(&0u32.to_be_bytes())?; // patched after rendering
    file.write_all(&depth.bits().to_be_bytes())?;
    file.write_all(&extended80(sample_rate as f64))?;

    file.write_all(b"SSND")?;
    file.write_all(&0u32.to_be_bytes())?; // patched after rendering
    file.write_all(&0u32.to_be_bytes())?; // offset
    file.write_all(&0u32.to_be_bytes())?; // block size
    Ok(())
}

fn write_sample(file: &mut File, sample: f32, depth: AiffBitDepth) -> Result<()> {
    match depth {
        AiffBitDepth::Pcm16 => {
            let value = (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
            file.write_all(&value.to_be_bytes())?;
        }
        AiffBitDepth::Pcm24 => {
            let value = (sample.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
            let bytes = value.to_be_bytes();
            file.write_all(&bytes[1..])?;
        }
        AiffBitDepth::Pcm32 => {
            let value = (sample.clamp(-1.0, 1.0) * 2_147_483_647.0).round() as i32;
            file.write_all(&value.to_be_bytes())?;
        }
    }
    Ok(())
}

/// IEEE 754 80-bit extended precision, big-endian, as used by AIFF COMM.
fn extended80(value: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if !value.is_finite() || value <= 0.0 {
        return out;
    }
    let mut exponent = value.log2().floor() as i32;
    exponent = exponent.clamp(-16_382, 16_383);
    let mantissa = (value / 2f64.powi(exponent) * 2f64.powi(63)) as u64;
    let biased = (exponent + 16_383) as u16;
    out[..2].copy_from_slice(&biased.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_dir, test_project};
    use crate::{ExportScope, RenderOptions};
    use redraw_plugins::BuiltinFactory;

    fn read_u16(data: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([data[at], data[at + 1]])
    }

    fn read_u32(data: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
    }

    fn read_extended80(data: &[u8], at: usize) -> f64 {
        let exponent = read_u16(data, at) as i32 - 16_383;
        let mut mantissa = 0u64;
        for byte in &data[at + 2..at + 10] {
            mantissa = (mantissa << 8) | *byte as u64;
        }
        mantissa as f64 / 2f64.powi(63) * 2f64.powi(exponent)
    }

    #[test]
    fn aiff_depths_roundtrip_header_and_audio() {
        for depth in [
            AiffBitDepth::Pcm16,
            AiffBitDepth::Pcm24,
            AiffBitDepth::Pcm32,
        ] {
            let dir = test_dir("aiff");
            let path = dir.join(format!("{:?}.aiff", depth));
            let project = test_project();
            let options = RenderOptions {
                scope: ExportScope::Pattern(1),
                sample_rate: 8_000,
                tail_seconds: 0.0,
            };
            let summary = write(&path, &project, &BuiltinFactory::new(), &options, depth).unwrap();
            let data = std::fs::read(&path).unwrap();
            assert_eq!(&data[..4], b"FORM");
            assert_eq!(&data[8..12], b"AIFF");
            assert_eq!(&data[12..16], b"COMM");
            assert_eq!(read_u32(&data, 16), 18);
            assert_eq!(read_u16(&data, 20), 2);
            assert_eq!(read_u32(&data, 22) as u64, summary.frames);
            assert_eq!(read_u16(&data, 26), depth.bits());
            assert!((read_extended80(&data, 28) - 8_000.0).abs() < 0.01);
            assert_eq!(&data[38..42], b"SSND");
            assert_eq!(
                read_u32(&data, 42) as u64,
                8 + summary.frames * 2 * depth.bytes()
            );
            assert_eq!(
                read_u32(&data, 4) as u64,
                46 + summary.frames * 2 * depth.bytes()
            );
            let data_bytes = summary.frames as usize * 2 * depth.bytes() as usize;
            assert_eq!(data.len(), 54 + data_bytes);
            assert!(data[54..].iter().any(|byte| *byte != 0));
        }
    }

    #[test]
    fn aiff_export_overwrites_atomically() {
        let dir = test_dir("aiff-overwrite");
        let path = dir.join("song.aiff");
        let project = test_project();
        let factory = BuiltinFactory::new();
        let options = crate::ExportOptions {
            render: RenderOptions {
                scope: ExportScope::Pattern(1),
                sample_rate: 8_000,
                tail_seconds: 0.0,
            },
            ..Default::default()
        };
        for _ in 0..2 {
            crate::export_file(
                &path,
                &project,
                &factory,
                &options,
                crate::ExportFormat::Aiff(AiffBitDepth::Pcm16),
            )
            .unwrap();
        }
        assert!(path.is_file());
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("aiff"))
                .count(),
            1
        );
    }
}
