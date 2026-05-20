use resonance_common::{decode_wav_channels, decode_wav_stereo, linear_resample_mono};

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
