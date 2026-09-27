//! Regression test for the monitor-ring channel-alignment bug: partial
//! `push_slice` results and sub-frame `skip` counts used to permanently
//! rotate the interleave after an overflow. Producers and the mixer
//! consumer now round every push/skip/read down to whole frames via
//! the helpers exercised here.

use resonance_audio::test_support::{
    monitor_catchup_skip, monitor_read_len, whole_frame_push_len,
};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;

#[test]
fn push_len_rounds_down_to_whole_frames() {
    // Plenty of vacancy: push everything (already frame-aligned).
    assert_eq!(whole_frame_push_len(512 * 4, 4096, 4), 512 * 4);
    // Vacancy not frame-aligned: round down, never split a frame.
    assert_eq!(whole_frame_push_len(512 * 4, 1001, 4), 1000);
    assert_eq!(whole_frame_push_len(30, 10, 3), 9);
    // Mono is unaffected.
    assert_eq!(whole_frame_push_len(7, 5, 1), 5);
    // Full ring: push nothing rather than a torn frame.
    assert_eq!(whole_frame_push_len(512 * 4, 3, 4), 0);
}

#[test]
fn catchup_skip_is_frame_aligned_and_leaves_quantum_margin() {
    let stride = 4;
    let quantum = 256;
    let needed = 128 * stride;
    let target = needed + quantum * stride;

    // At or under target: no skip.
    assert_eq!(monitor_catchup_skip(target, needed, quantum, stride), 0);
    assert_eq!(monitor_catchup_skip(needed, needed, quantum, stride), 0);

    // Over target: skip down to the margin, in whole frames.
    for extra in [1, 3, 4, 7, 1000, 1003] {
        let available = target + extra;
        let skip = monitor_catchup_skip(available, needed, quantum, stride);
        assert_eq!(skip % stride, 0, "skip must be whole frames");
        let left = available - skip;
        assert!(left >= target, "must keep one quantum of jitter margin");
        assert!(left < target + stride, "must not leave a stale backlog");
    }

    // Even with a misaligned backlog (pre-fix ring state) the skip
    // count itself stays frame-aligned.
    let skip = monitor_catchup_skip(target + 9, needed, quantum, stride);
    assert_eq!(skip, 8);
}

#[test]
fn read_len_rounds_occupied_down_to_whole_frames() {
    assert_eq!(monitor_read_len(512, 1024, 4), 512);
    assert_eq!(monitor_read_len(512, 510, 4), 508);
    assert_eq!(monitor_read_len(512, 3, 4), 0);
}

/// End-to-end over a real ring: overflow the producer side, run the
/// consumer catch-up skip, and verify every frame popped afterwards
/// still has its channels in order.
#[test]
fn overflow_does_not_rotate_channel_alignment() {
    const STRIDE: usize = 4; // 4-channel interleaved input
    const QUANTUM: usize = 64;
    let ring = HeapRb::<f32>::new(QUANTUM * STRIDE * 4);
    let (mut prod, mut cons) = ring.split();

    // Encode channel index in the fractional part so alignment is
    // checkable after arbitrary frame drops: sample = frame + ch/8.
    let mut frame_no = 0usize;
    let mut push_block = |prod: &mut ringbuf::HeapProd<f32>, frames: usize| {
        let mut block = Vec::with_capacity(frames * STRIDE);
        for _ in 0..frames {
            for ch in 0..STRIDE {
                block.push(frame_no as f32 + ch as f32 / 8.0);
            }
            frame_no += 1;
        }
        let take = whole_frame_push_len(block.len(), prod.vacant_len(), STRIDE);
        let pushed = prod.push_slice(&block[..take]);
        assert_eq!(pushed, take);
    };

    // Overflow: offer far more than the ring holds, repeatedly.
    for _ in 0..8 {
        push_block(&mut prod, QUANTUM * 3);
    }
    assert_eq!(
        cons.occupied_len() % STRIDE,
        0,
        "ring contents must stay frame-aligned through overflow"
    );

    // Consumer catch-up, then a normal read.
    let needed = QUANTUM * STRIDE;
    let skip = monitor_catchup_skip(cons.occupied_len(), needed, QUANTUM, STRIDE);
    cons.skip(skip);
    let mut buf = vec![0.0f32; needed];
    let to_read = monitor_read_len(needed, cons.occupied_len(), STRIDE);
    let got = cons.pop_slice(&mut buf[..to_read]);
    assert_eq!(got, needed, "a full buffer must be readable after catch-up");

    // Every popped frame must be internally consistent: same integer
    // frame number, channels 0..STRIDE in order.
    for frame in buf.chunks_exact(STRIDE) {
        let base = frame[0];
        assert_eq!(base.fract(), 0.0, "frame must start at channel 0");
        for (ch, &s) in frame.iter().enumerate() {
            assert_eq!(s, base + ch as f32 / 8.0, "channel rotated within frame");
        }
    }
}

// -- Adaptive native-backend backlog drain (doc #260 finding #12) ------------

use resonance_audio::test_support::{MonitorDrain, MONITOR_DRAIN_STREAK};

#[test]
fn drain_fires_only_after_a_full_stable_high_streak() {
    let stride = 2;
    let needed = 128 * stride;
    let mut d = MonitorDrain::new(true);
    // One sticky extra quantum above `needed`, stable for the whole
    // streak: cycles before the threshold drain nothing…
    for _ in 0..MONITOR_DRAIN_STREAK - 1 {
        assert_eq!(d.excess_drain(needed + 256, needed, stride), 0);
    }
    // …the threshold cycle reclaims exactly the excess, whole frames.
    assert_eq!(d.excess_drain(needed + 256, needed, stride), 256);
    // And the streak restarts — no repeated draining right after.
    assert_eq!(d.excess_drain(needed + 2, needed, stride), 0);
}

#[test]
fn any_low_cycle_resets_the_streak() {
    let stride = 2;
    let needed = 128 * stride;
    let mut d = MonitorDrain::new(true);
    for _ in 0..MONITOR_DRAIN_STREAK - 1 {
        assert_eq!(d.excess_drain(needed + 64, needed, stride), 0);
    }
    // Jittery ordering: one cycle at/below `needed` resets everything —
    // the standing margin is kept when scheduling order isn't stable.
    assert_eq!(d.excess_drain(needed, needed, stride), 0);
    for _ in 0..MONITOR_DRAIN_STREAK - 1 {
        assert_eq!(d.excess_drain(needed + 64, needed, stride), 0);
    }
    assert_eq!(d.excess_drain(needed + 64, needed, stride), 64);
}

#[test]
fn post_drain_shortfall_locks_the_drain_out_for_the_session() {
    let stride = 2;
    let needed = 128 * stride;
    let mut d = MonitorDrain::new(true);
    // Converge to zero margin once…
    for _ in 0..MONITOR_DRAIN_STREAK - 1 {
        assert_eq!(d.excess_drain(needed + 256, needed, stride), 0);
    }
    assert_eq!(d.excess_drain(needed + 256, needed, stride), 256);
    // …then an ordering flip drops a quantum (the mixer reports it):
    d.note_shortfall();
    // The backlog rebuilds, stays stably high — but the drain never
    // fires again, so the standing margin absorbs further flips.
    for _ in 0..MONITOR_DRAIN_STREAK * 3 {
        assert_eq!(d.excess_drain(needed + 256, needed, stride), 0);
    }
}

#[test]
fn startup_shortfall_before_any_drain_does_not_lock_out() {
    let stride = 2;
    let needed = 128 * stride;
    let mut d = MonitorDrain::new(true);
    // Ring still filling at startup: shortfalls happen before any
    // drain and say nothing about ordering stability.
    d.note_shortfall();
    d.note_shortfall();
    for _ in 0..MONITOR_DRAIN_STREAK - 1 {
        assert_eq!(d.excess_drain(needed + 256, needed, stride), 0);
    }
    // The latency win is kept: the sticky startup quantum still drains.
    assert_eq!(d.excess_drain(needed + 256, needed, stride), 256);
}

#[test]
fn drain_rounds_down_to_whole_frames() {
    let stride = 4;
    let needed = 32 * stride;
    let mut d = MonitorDrain::new(true);
    for _ in 0..MONITOR_DRAIN_STREAK - 1 {
        assert_eq!(d.excess_drain(needed + 10, needed, stride), 0);
    }
    // 10 excess samples at stride 4 -> 8 (2 whole frames).
    assert_eq!(d.excess_drain(needed + 10, needed, stride), 8);
}

#[test]
fn cpal_fallback_never_drains() {
    let stride = 2;
    let needed = 128 * stride;
    let mut d = MonitorDrain::new(false);
    for _ in 0..MONITOR_DRAIN_STREAK * 3 {
        assert_eq!(
            d.excess_drain(needed + 512, needed, stride),
            0,
            "fallback clock drift needs its standing margin"
        );
    }
}

// -- Input wider than the monitor scratch (code review MIX-09) --------------

use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::{TempoMap, Track, TrackOutput};
use std::sync::atomic::Ordering;

const MIX09_BLOCK: usize = 64;

/// A stopped harness with one monitored track on input port 0, and the
/// ring filled as full as it goes with `in_ch`-channel frames whose
/// channel `c` carries `c + 1` — so a whole-frame read hands the track
/// exactly 1.0 and a torn one hands it something else.
fn wide_input_harness(in_ch: usize) -> MixAudioHarness {
    let mut t = Track::new(1, "mon".into());
    t.set_output(TrackOutput::Master);
    t.set_monitor_enabled(true);
    t.set_mono(true);
    let mut h = MixAudioHarness::new(
        vec![t],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        MIX09_BLOCK,
        2,
        48_000,
        false,
    );
    h.shared().monitoring.store(true, Ordering::Relaxed);
    h.shared().input_channels.store(in_ch as u16, Ordering::Relaxed);
    h.shared()
        .master_volume_bits
        .store(1.0f32.to_bits(), Ordering::Relaxed);
    fill_wide(&mut h, in_ch);
    h
}

fn fill_wide(h: &mut MixAudioHarness, in_ch: usize) {
    let frame: Vec<f32> = (0..in_ch).map(|c| (c + 1) as f32).collect();
    while h.push_monitor(&frame) == in_ch {}
}

/// More channels than `MAX_INPUT_CHANNELS` (a 64-channel MADI interface
/// opened through the cpal path): the read used to size itself from the
/// channel count alone and slice `monitor_temp` out of bounds — a panic
/// inside the output callback, which kills the stream.
#[test]
fn input_wider_than_the_supported_maximum_never_panics_the_callback() {
    let mut h = wide_input_harness(40);
    for _ in 0..4 {
        let out = h.render();
        assert!(out.iter().all(|s| s.is_finite()));
        fill_wide(&mut h, 40);
    }
}

/// Wider than the callback's scratch but within the supported maximum:
/// the read clamps to what the scratch holds, in whole frames, so the
/// interleave never rotates.
#[test]
fn input_wider_than_the_scratch_reads_whole_frames_only() {
    let mut h = wide_input_harness(16);
    let mut heard = false;
    for _ in 0..6 {
        let out = h.render().to_vec();
        // Port 0 carries 1.0 (the first block ramps up to it from the
        // fader's resting gain of 0); every other channel is > 1.
        for &s in &out {
            assert!(
                (0.0..=1.0 + 1e-6).contains(&s),
                "a torn frame fed the track channel data other than port 0: {s}"
            );
        }
        heard |= out.iter().any(|&s| s != 0.0);
        fill_wide(&mut h, 16);
    }
    assert!(heard, "the monitored input must still be heard");
}
