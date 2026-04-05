/// IR (Impulse Response) WAV file loading and resampling.

use std::io::Cursor;
use std::path::Path;

/// A loaded impulse response: one or two channels of f32 samples.
pub struct IrData {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub stereo: bool,
}

/// Load an IR from a WAV file, resampled to the target sample rate.
pub fn load_ir(path: &str, target_sample_rate: f32) -> Result<IrData, String> {
    let data = std::fs::read(Path::new(path)).map_err(|e| format!("Failed to read file: {e}"))?;
    load_ir_from_bytes(&data, target_sample_rate)
}

/// Load an IR from WAV bytes.
pub fn load_ir_from_bytes(data: &[u8], target_sample_rate: f32) -> Result<IrData, String> {
    let cursor = Cursor::new(data);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV read error: {e}"))?;
    let spec = reader.spec();
    let source_rate = spec.sample_rate as f32;
    let channels = spec.channels as usize;

    let raw_samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = (1u32 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max_val)
                .collect()
        }
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .filter_map(|s| s.ok())
            .collect(),
    };

    let frames = raw_samples.len() / channels;

    // Extract channels
    let (left, right, stereo) = if channels >= 2 {
        let mut l = Vec::with_capacity(frames);
        let mut r = Vec::with_capacity(frames);
        for frame in 0..frames {
            l.push(raw_samples[frame * channels]);
            r.push(raw_samples[frame * channels + 1]);
        }
        (l, r, true)
    } else {
        (raw_samples, Vec::new(), false)
    };

    // Resample if needed
    let needs_resample = (source_rate - target_sample_rate).abs() > 1.0;
    let (left, right) = if needs_resample {
        let l = linear_resample(&left, source_rate, target_sample_rate);
        let r = if stereo {
            linear_resample(&right, source_rate, target_sample_rate)
        } else {
            Vec::new()
        };
        (l, r)
    } else {
        (left, right)
    };

    Ok(IrData {
        left,
        right,
        stereo,
    })
}

fn linear_resample(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    let ratio = source_rate as f64 / target_rate as f64;
    let target_len = (input.len() as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(target_len);

    for i in 0..target_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;

        let s0 = input[idx.min(input.len() - 1)];
        let s1 = input[(idx + 1).min(input.len() - 1)];
        output.push(s0 + (s1 - s0) * frac);
    }

    output
}
