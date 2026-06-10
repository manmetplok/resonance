//! Regression test for the monitor-ring channel-alignment bug: partial
//! `push_slice` results and sub-frame `skip` counts used to permanently
//! rotate the interleave after an overflow. Producers and the mixer
//! consumer now round every push/skip/read down to whole frames via
//! the helpers exercised here.

use resonance_audio::__test_support::{
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
