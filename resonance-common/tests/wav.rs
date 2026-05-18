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

#[test]
fn resample_length_scales_with_ratio() {
    let input = vec![0.0_f32; 4_800]; // 100 ms at 48 kHz
    let down = linear_resample_mono(&input, 48_000.0, 24_000.0);
    assert!(down.len() >= 2_300 && down.len() <= 2_500);
    let up = linear_resample_mono(&input, 48_000.0, 96_000.0);
    assert!(up.len() >= 9_500 && up.len() <= 9_700);
}
