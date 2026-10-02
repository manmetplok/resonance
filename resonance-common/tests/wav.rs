use resonance_common::{
    decode_wav_channels, decode_wav_native, decode_wav_stereo, linear_resample_mono,
    linear_resample_stereo,
};

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

/// Regression: long WAVs decoded in multiple symphonia packets must
/// not lose all but the last packet's audio. `copy_to_vec_interleaved`
/// *resizes* its destination to the current packet's sample count
/// rather than appending, so the previous decoder implementation
/// silently kept only the trailing few hundred frames of any multi-
/// packet WAV. Surfaced in the drum-kit loader as "non-built-in kits
/// produce truncated clicks (or silence) on triggered pads".
#[test]
fn decode_long_wav_keeps_all_packets() {
    let sr = 48_000u32;
    let total_frames = sr as usize; // 1 second of audio
    // Build a deterministic sweep so we can spot-check the tail —
    // truncation would leave only zeros (the silent intro) or the
    // last packet's values, neither matching the sweep.
    let frames: Vec<(i16, i16)> = (0..total_frames)
        .map(|i| {
            let v = ((i as f32 / total_frames as f32) * 8_000.0) as i16;
            (v, -v)
        })
        .collect();
    let wav = build_wav_16_stereo(&frames, sr);
    let out = decode_wav_stereo(&wav, sr as f32).expect("decode");
    // Output must cover every input frame, not just the trailing
    // packet. Allow a few frame slack for decoder framing quirks.
    assert!(
        out.len() >= total_frames * 2 - 32,
        "expected ~{} samples, got {}",
        total_frames * 2,
        out.len()
    );
    // Spot-check the head, middle, and tail are non-trivially
    // populated — truncation would leave the head at zero.
    let head_energy: f32 = out[..1024].iter().map(|s| s.abs()).sum();
    let mid_idx = out.len() / 2;
    let mid_energy: f32 = out[mid_idx..mid_idx + 1024].iter().map(|s| s.abs()).sum();
    let tail_start = out.len().saturating_sub(1024);
    let tail_energy: f32 = out[tail_start..].iter().map(|s| s.abs()).sum();
    assert!(head_energy < mid_energy, "head should ramp up to mid");
    assert!(mid_energy > 0.0, "mid section should not be silent");
    assert!(tail_energy > mid_energy, "tail should be the loudest sweep peak");
}

#[test]
fn resample_length_scales_with_ratio() {
    let input = vec![0.0_f32; 4_800]; // 100 ms at 48 kHz
    let down = linear_resample_mono(&input, 48_000.0, 24_000.0);
    assert!(down.len() >= 2_300 && down.len() <= 2_500);
    let up = linear_resample_mono(&input, 48_000.0, 96_000.0);
    assert!(up.len() >= 9_500 && up.len() <= 9_700);
}

#[test]
fn tiny_input_resamples_to_at_least_one_sample() {
    // 2 samples downsampled 12:1 used to truncate to a 0-length output,
    // silently discarding non-empty audio. Non-empty in => non-empty out.
    // (The band-limited filter blends both input samples into it.)
    let mono = linear_resample_mono(&[0.5, 0.7], 96_000.0, 8_000.0);
    assert_eq!(mono.len(), 1);
    assert!(mono[0] > 0.5 && mono[0] < 0.7, "{mono:?}");

    // Same for stereo: one frame in, at least one frame out.
    let stereo = linear_resample_stereo(&[0.5, -0.5], 96_000.0, 8_000.0);
    assert_eq!(stereo.len(), 2);
    assert!((stereo[0] - 0.5).abs() < 1e-6 && (stereo[1] + 0.5).abs() < 1e-6);

    // Empty input still yields empty output.
    assert!(linear_resample_mono(&[], 96_000.0, 8_000.0).is_empty());
    assert!(linear_resample_stereo(&[], 96_000.0, 8_000.0).is_empty());
    // A lone sample is not a full stereo frame.
    assert!(linear_resample_stereo(&[0.5], 96_000.0, 8_000.0).is_empty());
}

/// Minimal 16-bit PCM mono WAV.
fn build_wav_16_mono(samples: &[i16], sr: u32) -> Vec<u8> {
    let data_bytes = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// `decode_wav_native` keeps a mono file mono, and its one channel is
/// bit-identical to either side of the duplicated-stereo decode — at the
/// file's own rate and through the resampler (44.1 → 48 kHz), which is
/// what lets the drum sampler hold mono takes at half the memory without
/// changing a rendered sample.
#[test]
fn native_decode_keeps_mono_and_matches_the_stereo_decode_bit_for_bit() {
    let samples: Vec<i16> = (0..4_410)
        .map(|i| ((i as f32 * 0.07).sin() * 12_000.0) as i16)
        .collect();
    let wav = build_wav_16_mono(&samples, 44_100);
    for rate in [44_100.0, 48_000.0] {
        let native = decode_wav_native(wav.clone(), rate).expect("native decode");
        assert_eq!(native.channels, 1);
        let stereo = decode_wav_stereo(&wav, rate).expect("stereo decode");
        assert_eq!(native.frames() * 2, stereo.len(), "at {rate} Hz");
        for (i, s) in native.samples.iter().enumerate() {
            assert_eq!(s.to_bits(), stereo[2 * i].to_bits(), "L frame {i} at {rate} Hz");
            assert_eq!(s.to_bits(), stereo[2 * i + 1].to_bits(), "R frame {i} at {rate} Hz");
        }
    }
}

/// A stereo file stays stereo, identical to `decode_wav_stereo`.
#[test]
fn native_decode_of_stereo_equals_the_stereo_decode() {
    let frames: Vec<(i16, i16)> = (0..2_000).map(|i| (i as i16 * 7, -(i as i16) * 5)).collect();
    let wav = build_wav_16_stereo(&frames, 44_100);
    let native = decode_wav_native(wav.clone(), 48_000.0).expect("native decode");
    assert_eq!(native.channels, 2);
    let stereo = decode_wav_stereo(&wav, 48_000.0).expect("stereo decode");
    let a: Vec<u32> = native.samples.iter().map(|s| s.to_bits()).collect();
    let b: Vec<u32> = stereo.iter().map(|s| s.to_bits()).collect();
    assert_eq!(a, b);
}
