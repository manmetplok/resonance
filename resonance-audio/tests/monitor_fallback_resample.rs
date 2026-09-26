//! Rate conversion on the cpal-fallback monitor path (doc #260 finding
//! #21, ba todo #1124).
//!
//! When the fallback input device runs at a different rate than the
//! engine, the monitor ring's consumer (which replays at the engine
//! rate) would pitch-shift and glitch the monitor signal. The
//! `MonitorResampler` wraps the existing streaming (band-limited,
//! LIB-01) resampler over interleaved N-channel frames; these tests pin the frame-rate
//! ratio, channel-layout preservation, and streaming continuity across
//! chunk boundaries.

use resonance_audio::__test_support::MonitorResampler;

/// Interleaved frames where channel `c` carries the constant `c + 1`.
fn constant_frames(channels: usize, frames: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(channels * frames);
    for _ in 0..frames {
        for c in 0..channels {
            v.push((c + 1) as f32);
        }
    }
    v
}

#[test]
fn converts_frame_rate_and_preserves_channel_layout() {
    // 44.1k device -> 48k engine, stereo. Feed one second of input; the
    // output must be ~48000 frames (streaming holds back ~1 ms of filter
    // lookahead) with each channel's constant intact (the filter has
    // unit DC gain).
    let mut rs = MonitorResampler::new(44_100, 48_000, 2);
    let out = rs.process(&constant_frames(2, 44_100)).to_vec();
    let out_frames = out.len() / 2;
    assert!(
        (47_900..=48_000).contains(&out_frames),
        "expected ~48000 output frames, got {out_frames}"
    );
    for f in out.chunks(2) {
        assert!((f[0] - 1.0).abs() < 1e-5, "{f:?}");
        assert!((f[1] - 2.0).abs() < 1e-5, "{f:?}");
    }
}

#[test]
fn odd_channel_counts_keep_identity() {
    // 3 channels: the pair wrapper must not swap or smear channels.
    let mut rs = MonitorResampler::new(44_100, 48_000, 3);
    let out = rs.process(&constant_frames(3, 4_410)).to_vec();
    assert!(!out.is_empty());
    assert_eq!(out.len() % 3, 0);
    for f in out.chunks(3) {
        for (c, v) in f.iter().enumerate() {
            assert!((v - (c + 1) as f32).abs() < 1e-5, "{f:?}");
        }
    }
}

#[test]
fn chunked_processing_is_continuous() {
    // A ramp fed in two chunks must come out identical to the same ramp
    // fed at once — the streaming state carries the phase across the
    // seam (no glitch at chunk boundaries, the audible symptom of the
    // old path).
    let ramp: Vec<f32> = (0..2000)
        .flat_map(|f| {
            let v = f as f32 / 10.0;
            [v, -v]
        })
        .collect();

    let mut whole = MonitorResampler::new(44_100, 48_000, 2);
    let expected = whole.process(&ramp).to_vec();

    let mut chunked = MonitorResampler::new(44_100, 48_000, 2);
    let mut got = chunked.process(&ramp[..700 * 2]).to_vec();
    got.extend_from_slice(chunked.process(&ramp[700 * 2..]));

    assert_eq!(got, expected);
}

#[test]
fn matching_rates_pass_frames_through_unchanged_count() {
    // Same-rate construction isn't used by the callback (it skips the
    // resampler entirely), but the wrapper must still behave sanely: an
    // identity rate is a straight pass-through.
    let mut rs = MonitorResampler::new(48_000, 48_000, 2);
    let input = constant_frames(2, 128);
    let out = rs.process(&input).to_vec();
    assert_eq!(out, input);
}
