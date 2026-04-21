//! WAV (and PCM) decoding + resampling utilities used by IR / drum /
//! sample-loading plugin code.
//!
//! Implemented on top of `symphonia`: one reader covers all the raw
//! WAV bit depths the project cares about (8/16/24/32-bit integer,
//! 32/64-bit float) without per-format branching here. The public API
//! — `decode_wav_stereo`, `decode_wav_channels`, and the linear
//! resamplers — is unchanged from the earlier `hound`-based version,
//! so every downstream crate compiles without modification.

use std::io::Cursor;

use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decoded WAV data split into separate channels.
pub struct WavChannels {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub stereo: bool,
}

/// Decode a WAV file from bytes into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav_stereo(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, String> {
    let decoded = decode_to_interleaved(data)?;
    let source_rate = decoded.sample_rate;
    let stereo = to_stereo_interleaved(&decoded.samples, decoded.channels);

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
    let decoded = decode_to_interleaved(data)?;
    let source_rate = decoded.sample_rate;
    let channels = decoded.channels;
    let raw_samples = decoded.samples;
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

struct Decoded {
    samples: Vec<f32>,
    sample_rate: f32,
    channels: usize,
}

/// Run the input bytes through symphonia's default decoder registry
/// and return the full interleaved `f32` sample stream plus the
/// source rate and channel count.
fn decode_to_interleaved(data: &[u8]) -> Result<Decoded, String> {
    let cursor = Cursor::new(data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("wav");

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("WAV probe error: {e}"))?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "WAV has no decodable track".to_string())?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let sample_rate = codec_params
        .sample_rate
        .map(|sr| sr as f32)
        .ok_or_else(|| "WAV missing sample rate".to_string())?;
    let channels = codec_params.channels.map(|c| c.count()).unwrap_or(1).max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("WAV decoder error: {e}"))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(format!("WAV read packet: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(format!("WAV decode: {e}")),
        };
        if sample_buf.is_none() {
            let duration = decoded.capacity() as u64;
            let spec = *decoded.spec();
            sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
        }
        let buf = sample_buf.as_mut().unwrap();
        append_interleaved(&decoded, buf, &mut samples);
    }

    if samples.is_empty() {
        return Err("WAV decoded 0 samples".to_string());
    }

    Ok(Decoded {
        samples,
        sample_rate,
        channels,
    })
}

fn append_interleaved(
    decoded: &AudioBufferRef,
    sample_buf: &mut SampleBuffer<f32>,
    out: &mut Vec<f32>,
) {
    sample_buf.copy_interleaved_ref(decoded.clone());
    out.extend_from_slice(sample_buf.samples());
}

fn to_stereo_interleaved(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 2 {
        return samples.to_vec();
    }
    if channels > 2 {
        // Take the first two channels only, ignore the rest.
        let frames = samples.len() / channels;
        let mut stereo = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            stereo.push(samples[frame * channels]);
            stereo.push(samples[frame * channels + 1]);
        }
        return stereo;
    }
    // Mono → duplicate.
    let mut stereo = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        stereo.push(s);
        stereo.push(s);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal 16-bit PCM WAV file in memory from f32 samples.
    fn build_wav_16_stereo(samples: &[(i16, i16)], sr: u32) -> Vec<u8> {
        let num_samples = samples.len() as u32;
        let byte_rate = sr * 2 * 2;
        let block_align: u16 = 4;
        let bits: u16 = 16;
        let data_bytes = num_samples * 4;
        let riff_size = 36 + data_bytes;

        let mut out = Vec::with_capacity(44 + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&riff_size.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&2u16.to_le_bytes()); // channels
        out.extend_from_slice(&sr.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        for (l, r) in samples {
            out.extend_from_slice(&l.to_le_bytes());
            out.extend_from_slice(&r.to_le_bytes());
        }
        out
    }

    #[test]
    fn decode_16_bit_stereo_round_trip() {
        // Four interleaved stereo frames at 48 kHz.
        let frames = [
            (0_i16, 0_i16),
            (16_384, -16_384),
            (32_767, -32_768),
            (-8_192, 8_192),
        ];
        let wav = build_wav_16_stereo(&frames, 48_000);
        let out = decode_wav_stereo(&wav, 48_000.0).expect("decode");
        assert_eq!(out.len(), frames.len() * 2);
        // 16-bit → f32 has ~4.6e-5 quantization; allow a generous
        // tolerance of 1e-4.
        let expected: Vec<f32> = frames
            .iter()
            .flat_map(|(l, r)| vec![*l as f32 / 32768.0, *r as f32 / 32768.0])
            .collect();
        for (a, b) in out.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-4, "got {a}, expected {b}");
        }
    }

    #[test]
    fn decode_channels_splits_lr() {
        let frames = [(1_000_i16, -1_000_i16), (2_000, -2_000)];
        let wav = build_wav_16_stereo(&frames, 48_000);
        let wc = decode_wav_channels(&wav, 48_000.0).expect("decode");
        assert!(wc.stereo);
        assert_eq!(wc.left.len(), 2);
        assert_eq!(wc.right.len(), 2);
        assert!((wc.left[0] - (1_000.0 / 32_768.0)).abs() < 1e-4);
        assert!((wc.right[0] - (-1_000.0 / 32_768.0)).abs() < 1e-4);
    }

    #[test]
    fn resample_length_scales_with_ratio() {
        let input = vec![0.0_f32; 4_800]; // 100 ms at 48 kHz
        let down = linear_resample_mono(&input, 48_000.0, 24_000.0);
        assert!(down.len() >= 2_300 && down.len() <= 2_500);
        let up = linear_resample_mono(&input, 48_000.0, 96_000.0);
        assert!(up.len() >= 9_500 && up.len() <= 9_700);
    }
}
