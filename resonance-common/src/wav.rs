/// WAV file decoding and resampling utilities.

use std::io::Cursor;

/// Decoded WAV data split into separate channels.
pub struct WavChannels {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub stereo: bool,
}

/// Decode a WAV file from bytes into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav_stereo(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, String> {
    let cursor = Cursor::new(data);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV read error: {e}"))?;
    let spec = reader.spec();
    let source_rate = spec.sample_rate as f32;
    let channels = spec.channels as usize;

    let raw_samples = decode_samples(reader)?;

    let stereo = to_stereo_interleaved(&raw_samples, channels);

    if (source_rate - target_sample_rate).abs() > 1.0 {
        Ok(linear_resample_stereo(
            &stereo,
            source_rate,
            target_sample_rate,
        ))
    } else {
        Ok(stereo)
    }
}

/// Decode a WAV file from bytes into separate left/right channels,
/// resampled to the target sample rate if necessary.
pub fn decode_wav_channels(data: &[u8], target_sample_rate: f32) -> Result<WavChannels, String> {
    let cursor = Cursor::new(data);
    let reader = hound::WavReader::new(cursor).map_err(|e| format!("WAV read error: {e}"))?;
    let spec = reader.spec();
    let source_rate = spec.sample_rate as f32;
    let channels = spec.channels as usize;

    let raw_samples = decode_samples(reader)?;
    let frames = raw_samples.len() / channels;

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

    let needs_resample = (source_rate - target_sample_rate).abs() > 1.0;
    let (left, right) = if needs_resample {
        let l = linear_resample_mono(&left, source_rate, target_sample_rate);
        let r = if stereo {
            linear_resample_mono(&right, source_rate, target_sample_rate)
        } else {
            Vec::new()
        };
        (l, r)
    } else {
        (left, right)
    };

    Ok(WavChannels {
        left,
        right,
        stereo,
    })
}

fn decode_samples(reader: hound::WavReader<Cursor<&[u8]>>) -> Result<Vec<f32>, String> {
    let spec = reader.spec();
    match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = (1u32 << (spec.bits_per_sample - 1)) as f32;
            let samples: Vec<i32> = reader
                .into_samples::<i32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("WAV sample decode error: {e}"))?;
            Ok(samples.into_iter().map(|s| s as f32 / max_val).collect())
        }
        hound::SampleFormat::Float => {
            let samples = reader
                .into_samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("WAV sample decode error: {e}"))?;
            Ok(samples)
        }
    }
}

fn to_stereo_interleaved(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 2 {
        return samples.to_vec();
    }

    let frames = samples.len() / channels;
    let mut stereo = Vec::with_capacity(frames * 2);

    for frame in 0..frames {
        let sample = samples[frame];
        stereo.push(sample);
        stereo.push(sample);
    }

    stereo
}

/// Linear interpolation resampler for mono audio data.
pub fn linear_resample_mono(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
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

/// Linear interpolation resampler for stereo interleaved audio data.
pub fn linear_resample_stereo(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
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
