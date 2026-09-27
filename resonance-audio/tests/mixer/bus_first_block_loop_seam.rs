//! FU-B6b: the A2-9 hammer (`tests/retire_queue/render_graph_hammer.rs`)
//! occasionally saw the block after an aligned loop seam dip a hair below
//! its base level once its routing edits were allowed onto a brand-new
//! bus. These pin down what that was, deterministically.
//!
//! It was not the bus. Routing a playing track onto a bus created in the
//! seam block, one block before it, or across an unaligned seam, renders
//! sample-for-sample what the track renders straight to the master (the
//! first three tests). What dips is the clip: the hammer's base clip
//! starts exactly on `loop_in`, so every pass re-enters it at its head,
//! and a clip head always gets the `CLIP_DECLICK_FRAMES` anti-click ramp
//! (last test) — the first frame after the wrap carries none of it. The
//! hammer's floor held there only because the other sources' pile-up
//! (sends accumulating over the run) happened to sum above the base
//! level; when that pile was a little lighter, the frame read as a dip.

use std::sync::atomic::Ordering;

use resonance_audio::test_support::{EngineHandlerHarness, MixAudioHarness, CLIP_DECLICK_FRAMES};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const TRACK: TrackId = 1;
const BUS: BusId = 1;
const LEVEL: f32 = 0.1;
/// Ten blocks: block 9 ends exactly on `loop_out`, so it is the aligned
/// seam block — full-length head, zero-frame tail sub-render.
const ALIGNED_LOOP: u64 = 10 * BLOCK as u64;
/// Block 9 crosses `loop_out` 91 frames in: a 91 + 37 frame split.
const UNALIGNED_LOOP: u64 = ALIGNED_LOOP - 37;
const SEAM_BLOCK: usize = 9;
/// Two full passes and then some: every seam case is crossed twice.
const BLOCKS: usize = 25;

/// A constant `LEVEL` clip on [`TRACK`] from the top, longer than any
/// loop here, so it never ends inside one.
fn dc_clip() -> AudioClip {
    AudioClip {
        id: 1,
        track_id: TRACK,
        start_sample: 0,
        source: ClipSource::memory(vec![LEVEL; 2 * 4 * ALIGNED_LOOP as usize]),
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

/// When the bus appears and when the playing track is routed onto it,
/// as block indices (the edit lands before that block renders).
#[derive(Clone, Copy)]
struct Reroute {
    create_at: usize,
    route_at: usize,
}

/// Render [`BLOCKS`] blocks of a track playing [`dc_clip`] through a
/// `loop_in = 0 .. loop_out` cycle, optionally re-routing it onto a
/// brand-new unity bus mid-run, and return the left channel.
fn render(loop_out: u64, reroute: Option<Reroute>) -> Vec<f32> {
    let mut h = EngineHandlerHarness::new();
    let mut track = Track::new(TRACK, "t".into());
    track.set_output(TrackOutput::Master);
    h.push_track(track);
    h.push_clip(dc_clip());
    let shared = h.shared_arc();
    shared.loop_in.store(0, Ordering::Relaxed);
    shared.loop_out.store(loop_out, Ordering::Relaxed);
    shared.loop_enabled.store(true, Ordering::Relaxed);
    shared.playing.store(true, Ordering::Relaxed);
    let mut cb = MixAudioHarness::on_shared(shared, BLOCK, 2, SR);
    let mut left = Vec::with_capacity(BLOCKS * BLOCK);
    for b in 0..BLOCKS {
        if let Some(r) = reroute {
            if b == r.create_at {
                h.add_bus(BUS, None);
            }
            if b == r.route_at {
                h.set_track_output(TRACK, TrackOutput::Bus(BUS));
            }
        }
        left.extend(cb.render().chunks(2).map(|fr| fr[0]));
    }
    left
}

/// The re-routed render must match the master-only render frame for
/// frame: a unity bus with an empty chain adds nothing, on its first
/// block or across the seam.
fn assert_matches_master(loop_out: u64, reroute: Reroute, what: &str) {
    let reference = render(loop_out, None);
    let routed = render(loop_out, Some(reroute));
    for (i, (&r, &x)) in reference.iter().zip(&routed).enumerate() {
        assert!(
            (r - x).abs() <= 1e-6,
            "{what}: frame {i} (block {}, offset {}) is {x} through the new bus but {r} straight \
             to the master",
            i / BLOCK,
            i % BLOCK,
        );
    }
}

/// The bus is born, and the track routed onto it, in the aligned seam
/// block — the bus's first rendered block is the one whose tail
/// sub-render is zero frames long.
#[test]
fn new_bus_first_rendered_on_aligned_seam_block_adds_no_dip() {
    assert_matches_master(
        ALIGNED_LOOP,
        Reroute {
            create_at: SEAM_BLOCK,
            route_at: SEAM_BLOCK,
        },
        "bus born on the aligned seam block",
    );
}

/// Born one block before the seam (its first block is an ordinary one),
/// with the track routed onto it either then or on the seam block itself.
#[test]
fn new_bus_born_one_block_before_seam_adds_no_dip() {
    for route_at in [SEAM_BLOCK - 1, SEAM_BLOCK] {
        assert_matches_master(
            ALIGNED_LOOP,
            Reroute {
                create_at: SEAM_BLOCK - 1,
                route_at,
            },
            &format!("bus born the block before the seam, routed on block {route_at}"),
        );
    }
}

/// An unaligned seam: the bus's first block is split into a 91-frame head
/// and a 37-frame tail, so both sub-renders carry audio.
#[test]
fn new_bus_first_rendered_on_unaligned_seam_block_adds_no_dip() {
    assert_matches_master(
        UNALIGNED_LOOP,
        Reroute {
            create_at: SEAM_BLOCK,
            route_at: SEAM_BLOCK,
        },
        "bus born on the unaligned seam block",
    );
}

/// What the hammer actually saw: a clip that starts on `loop_in` is
/// re-entered at its head on every pass, and its head always ramps in
/// over [`CLIP_DECLICK_FRAMES`] — on the master exactly as through a bus.
/// The first frame after each wrap carries nothing of it. By design (a
/// wrap is a splice); the hammer now loops a window its base clip has
/// already started before.
#[test]
fn clip_starting_on_loop_in_declicks_on_every_pass() {
    let declick = CLIP_DECLICK_FRAMES as usize;
    for routed in [false, true] {
        let reroute = routed.then_some(Reroute {
            create_at: SEAM_BLOCK,
            route_at: SEAM_BLOCK,
        });
        let left = render(ALIGNED_LOOP, reroute);
        for pass in 1..=2 {
            let wrap = pass * ALIGNED_LOOP as usize;
            assert_eq!(left[wrap], 0.0, "pass {pass} (routed: {routed}) starts from silence");
            assert!(
                left[wrap + declick / 2] > 0.0 && left[wrap + declick / 2] < LEVEL,
                "pass {pass} (routed: {routed}) is mid-ramp half a declick in"
            );
            assert!(
                left[wrap + declick..wrap + BLOCK].iter().all(|&s| (s - LEVEL).abs() < 1e-6),
                "pass {pass} (routed: {routed}) is back at {LEVEL} once the declick is over"
            );
            assert!(
                left[wrap - BLOCK..wrap].iter().all(|&s| (s - LEVEL).abs() < 1e-6),
                "pass {pass} (routed: {routed}): the seam block itself is flat at {LEVEL}"
            );
        }
    }
}
