//! Whole-frame recording-ring pushes (doc #260 finding #17).
//!
//! The capture callbacks (native PipeWire and cpal) push interleaved
//! frames into the recording ring through `push_recording_frames`. On
//! overflow the old `push_slice` could push a *partial* frame, after
//! which every subsequent frame in the ring is rotated by the leftover
//! channels — permanently corrupting the take. The helper must only
//! ever push whole frames (and report the overflow), so drained audio
//! stays channel-aligned no matter how often the ring overflows.

use resonance_audio::__test_support::push_recording_frames;
use ringbuf::traits::{Consumer, Observer, Split};

/// Interleaved test signal: sample value encodes `frame * 10 + channel`
/// so alignment is directly assertable after any sequence of pushes.
fn frames(stride: usize, start_frame: usize, count: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(count * stride);
    for f in 0..count {
        for c in 0..stride {
            v.push(((start_frame + f) * 10 + c) as f32);
        }
    }
    v
}

#[test]
fn overflow_never_rotates_channels() {
    const STRIDE: usize = 4;
    // Capacity deliberately NOT a multiple of the stride (10 samples =
    // 2.5 frames) so a naive push_slice would leave a partial frame on
    // overflow.
    let ring = ringbuf::HeapRb::<f32>::new(10);
    let (mut prod, mut cons) = ring.split();

    // Push 4 frames into space for 2.5: only 2 whole frames may land.
    let dropped = push_recording_frames(&mut prod, &frames(STRIDE, 0, 4), STRIDE);
    assert_eq!(dropped, 2, "both frames that could not land are counted");
    assert_eq!(cons.occupied_len(), 2 * STRIDE, "whole frames only");

    // Drain one frame, push again, drain everything: every drained
    // frame must start on a frame boundary with channel 0 first.
    let mut out = [0.0f32; 4];
    assert_eq!(cons.pop_slice(&mut out), STRIDE);
    assert_eq!(out, [0.0, 1.0, 2.0, 3.0], "frame 0 intact");

    let dropped = push_recording_frames(&mut prod, &frames(STRIDE, 4, 3), STRIDE);
    assert_eq!(dropped, 2, "still only room for one of the three frames");

    let mut drained = Vec::new();
    let mut buf = [0.0f32; 4];
    while cons.pop_slice(&mut buf) == STRIDE {
        drained.extend_from_slice(&buf);
    }
    // Whatever landed, every frame is whole and channel-aligned.
    assert_eq!(drained.len() % STRIDE, 0);
    for frame in drained.chunks(STRIDE) {
        let base = frame[0];
        assert_eq!(base % 10.0, 0.0, "frame must start at channel 0");
        for (c, &s) in frame.iter().enumerate() {
            assert_eq!(s, base + c as f32, "channels in order within the frame");
        }
    }
}

#[test]
fn fitting_pushes_are_lossless_and_unreported() {
    const STRIDE: usize = 2;
    let ring = ringbuf::HeapRb::<f32>::new(64);
    let (mut prod, mut cons) = ring.split();
    let data = frames(STRIDE, 0, 8);
    assert_eq!(push_recording_frames(&mut prod, &data, STRIDE), 0);
    let mut out = vec![0.0f32; data.len()];
    assert_eq!(cons.pop_slice(&mut out), data.len());
    assert_eq!(out, data);
}
