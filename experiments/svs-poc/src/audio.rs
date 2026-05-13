use anyhow::{Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::path::Path;

pub fn write_mono_f32_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(path, spec)
        .with_context(|| format!("creating WAV writer for {}", path.display()))?;
    for &s in samples {
        writer.write_sample(s).context("writing WAV sample")?;
    }
    writer.finalize().context("finalising WAV file")?;
    Ok(())
}

pub fn mix_into_timeline(segments: &[(i64, Vec<f32>)]) -> Vec<f32> {
    let total = segments
        .iter()
        .map(|(off, w)| (*off).max(0) as usize + w.len())
        .max()
        .unwrap_or(0);
    let mut buf = vec![0.0f32; total];
    for (offset, waveform) in segments {
        let start = (*offset).max(0) as usize;
        let end = start + waveform.len();
        if end > buf.len() {
            continue;
        }
        for (dst, &src) in buf[start..end].iter_mut().zip(waveform.iter()) {
            *dst += src;
        }
    }
    buf
}
