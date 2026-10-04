//! Loop-seam edge cases of the playing branch (code review RT-10, RT-12).
//!
//! - RT-10: the master-gain automation was evaluated at the pre-seam
//!   `playhead + frames` — a position past `loop_out` — on every seam
//!   callback, so a master move drawn after the loop pulled every pass
//!   toward it for one block. It is evaluated where the buffer ends on the
//!   post-wrap timeline now.
//! - RT-12: a loop shorter than the buffer escaped: the seam's tail
//!   sub-block wasn't reduced modulo the loop length, so the next buffer
//!   started past `loop_out` and never wrapped again.

use std::sync::atomic::Ordering;

use resonance_audio::test_support::{LoopRange, MixAudioHarness};
use resonance_audio::types::*;
use resonance_common::{real_to_lane_value, AutomationLane, AutomationTarget, Breakpoint, CurveKind};

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const LEVEL: f32 = 0.25;

fn dc_clip(frames: usize) -> AudioClip {
    AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::memory(vec![LEVEL; 2 * frames]),
        name: "dc".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

fn harness(loop_in: u64, loop_out: u64) -> MixAudioHarness {
    let mut track = Track::new(1, "dc".into());
    track.set_output(TrackOutput::Master);
    let h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        vec![dc_clip(64 * BLOCK)],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.shared()
        .master_volume_bits
        .store(1.0f32.to_bits(), Ordering::Relaxed);
    h.shared()
        .set_loop_range(LoopRange::new(true, loop_in, loop_out));
    h.shared().playhead.store(loop_in, Ordering::Release);
    h.shared().playing.store(true, Ordering::Relaxed);
    h
}

/// Master gain at 0 dB inside the loop, dropping to -40 dB right after
/// `loop_out`. Every pass stays inside the loop, so the master never
/// moves — not even on the seam callbacks.
#[test]
fn master_gain_automation_on_a_seam_block_reads_the_post_wrap_position() {
    const LOOP_IN: u64 = 2 * BLOCK as u64;
    // The seam lands 50 frames into a block.
    const LOOP_OUT: u64 = LOOP_IN + 4 * BLOCK as u64 + 50;
    let mut h = harness(LOOP_IN, LOOP_OUT);
    let target = AutomationTarget::MasterGain;
    let bp = |frame, db| Breakpoint::new(frame, real_to_lane_value(&target, db), CurveKind::Linear);
    let lane = AutomationLane::new(
        1,
        target.clone(),
        vec![bp(0, 0.0), bp(LOOP_OUT, 0.0), bp(LOOP_OUT + 1, -40.0)],
    );
    let mut snap = resonance_audio::test_support::AutomationSnapshot::default();
    snap.mix_lanes.insert(target, lane);
    h.set_automation(snap);

    let mut seams = 0;
    for block in 0..30 {
        let start = h.shared().playhead.load(Ordering::Acquire);
        let out = h.render().to_vec();
        if start + BLOCK as u64 >= LOOP_OUT {
            seams += 1;
        }
        // Skip the clip-head declick at the very top of the first pass.
        if block == 0 {
            continue;
        }
        for (f, s) in out.chunks(2).enumerate() {
            assert!(
                (s[0] - LEVEL).abs() < 1e-4,
                "block {block} frame {f}: {} — the master gain moved toward the \
                 automation past loop_out",
                s[0]
            );
        }
    }
    assert!(seams >= 4, "the run must cross the seam several times");
}

/// A loop shorter than one buffer keeps the playhead inside it on every
/// block, and never renders timeline audio from past `loop_out`.
#[test]
fn a_loop_shorter_than_a_block_never_escapes() {
    const LOOP_IN: u64 = 1_000;
    const LEN: u64 = 50;
    let mut h = harness(LOOP_IN, LOOP_IN + LEN);
    for block in 0..40 {
        h.render();
        let p = h.shared().playhead.load(Ordering::Acquire);
        assert!(
            (LOOP_IN..LOOP_IN + LEN).contains(&p),
            "block {block}: playhead {p} escaped the {LOOP_IN}..{} loop",
            LOOP_IN + LEN
        );
    }
}

/// The same shape at the edge: a loop exactly one block long wraps every
/// block back onto `loop_in`.
#[test]
fn a_loop_exactly_one_block_long_wraps_every_block() {
    const LOOP_IN: u64 = 640;
    let mut h = harness(LOOP_IN, LOOP_IN + BLOCK as u64);
    for _ in 0..10 {
        h.render();
        assert_eq!(h.shared().playhead.load(Ordering::Acquire), LOOP_IN);
    }
}

/// A normal loop: the next buffer starts `tail` frames past `loop_in`,
/// exactly as before the modulo.
#[test]
fn a_normal_loop_carries_the_tail_past_loop_in() {
    const LOOP_IN: u64 = 0;
    const LOOP_OUT: u64 = 3 * BLOCK as u64 + 17;
    let mut h = harness(LOOP_IN, LOOP_OUT);
    let mut expected = LOOP_IN;
    for _ in 0..12 {
        h.render();
        expected += BLOCK as u64;
        if expected >= LOOP_OUT {
            expected = LOOP_IN + (expected - LOOP_OUT);
        }
        assert_eq!(h.shared().playhead.load(Ordering::Acquire), expected);
    }
}
