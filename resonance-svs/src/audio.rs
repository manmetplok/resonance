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

/// Sum rendered segments into a single timeline buffer.
///
/// `offset` is in samples and may be negative: segment offsets come from
/// `.ds` note onsets, and a leading consonant can start before the
/// timeline origin (offset 0). A negative offset trims that many leading
/// samples from the segment so the remainder still lands at the origin
/// with correct timing — clamping the offset instead would shift the
/// whole segment late by `|offset|` samples. Segments lying entirely
/// before the origin are dropped.
pub fn mix_into_timeline(segments: &[(i64, Vec<f32>)]) -> Vec<f32> {
    // Where a segment lands: `(start, skip)` = timeline start sample and
    // number of leading segment samples trimmed. `None` if the segment
    // ends at or before the origin.
    fn place(offset: i64, len: usize) -> Option<(usize, usize)> {
        if offset >= 0 {
            Some((offset as usize, 0))
        } else {
            let trim = offset.unsigned_abs() as usize;
            (trim < len).then_some((0, trim))
        }
    }

    let total = segments
        .iter()
        .filter_map(|(off, w)| place(*off, w.len()).map(|(start, skip)| start + w.len() - skip))
        .max()
        .unwrap_or(0);
    let mut buf = vec![0.0f32; total];
    for (offset, waveform) in segments {
        let Some((start, skip)) = place(*offset, waveform.len()) else {
            continue;
        };
        let end = start + waveform.len() - skip;
        for (dst, &src) in buf[start..end].iter_mut().zip(waveform[skip..].iter()) {
            *dst += src;
        }
    }
    buf
}
