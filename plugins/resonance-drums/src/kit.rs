/// Drum kit definition and WAV sample loading.

use std::io::Cursor;

/// A loaded pad with its decoded audio data.
pub struct LoadedPad {
    pub note: u8,
    pub name: String,
    /// Stereo interleaved f32 sample data.
    pub sample_data: Vec<f32>,
    /// Number of stereo frames.
    pub sample_frames: usize,
    pub choke_group: Option<u8>,
}

/// Decode a WAV file from a byte slice into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, String> {
    let cursor = Cursor::new(data);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV read error: {e}"))?;
    let spec = reader.spec();
    let source_rate = spec.sample_rate as f32;
    let channels = spec.channels as usize;

    let raw_samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            let max_val = (1u32 << (bits - 1)) as f32;
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

    // Convert to stereo interleaved
    let stereo = to_stereo_interleaved(&raw_samples, channels);

    // Resample if needed
    if (source_rate - target_sample_rate).abs() > 1.0 {
        Ok(linear_resample(&stereo, source_rate, target_sample_rate))
    } else {
        Ok(stereo)
    }
}

fn to_stereo_interleaved(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 2 {
        return samples.to_vec();
    }

    let frames = samples.len() / channels;
    let mut stereo = Vec::with_capacity(frames * 2);

    for frame in 0..frames {
        let base = frame * channels;
        let left = samples[base];
        let right = if channels > 1 {
            samples[base + 1]
        } else {
            left
        };
        stereo.push(left);
        stereo.push(right);
    }

    stereo
}

fn linear_resample(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    let source_frames = input.len() / 2;
    let ratio = source_rate as f64 / target_rate as f64;
    let target_frames = (source_frames as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(target_frames * 2);

    for i in 0..target_frames {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = (src_pos - src_idx as f64) as f32;

        let idx0 = src_idx.min(source_frames.saturating_sub(1));
        let idx1 = (src_idx + 1).min(source_frames.saturating_sub(1));

        let l0 = input[idx0 * 2];
        let r0 = input[idx0 * 2 + 1];
        let l1 = input[idx1 * 2];
        let r1 = input[idx1 * 2 + 1];

        output.push(l0 + (l1 - l0) * frac);
        output.push(r0 + (r1 - r0) * frac);
    }

    output
}
