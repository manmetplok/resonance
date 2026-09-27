//! FU-B6a: a new bus's last gains started at zero (`BusRuntime`,
//! `types/track.rs`), so re-routing an already-playing track onto it
//! during the bus's own first rendered block ramped the bus in from
//! silence — an audible one-block dip / fade-in even though the track
//! itself never stopped playing. Fixed by treating a bus that has never
//! completed a live block as having no real "previous gain" to ramp
//! from: its first live block renders flat at its target gain (fader x
//! pan law x mute) instead (`RenderStrategy::bus_disposition`).

use std::sync::atomic::Ordering;

use resonance_audio::test_support::{EngineHandlerHarness, MixAudioHarness};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 512;
const TRACK: TrackId = 1;
const BUS: BusId = 1;
const LEVEL: f32 = 0.2;

/// A constant `LEVEL` DC clip on `track_id`, several blocks long — no
/// edge declick matters here since every block under test starts well
/// past the clip's onset.
fn dc_clip(id: ClipId, track_id: TrackId) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(vec![LEVEL; BLOCK * 8 * 2]),
        name: "dc".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
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

/// Left-channel samples of an interleaved stereo block.
fn left(data: &[f32]) -> Vec<f32> {
    data.chunks(2).map(|fr| fr[0]).collect()
}

/// A playing track on [`TRACK`], routed to master, rendered past its own
/// (unrelated) fade-in so the caller's block-under-test starts from a
/// steady, already-audible signal — the "already playing" precondition
/// the bug needs.
fn playing_track_settled() -> (EngineHandlerHarness, MixAudioHarness) {
    let mut h = EngineHandlerHarness::new();
    let mut track = Track::new(TRACK, "t".into());
    track.set_output(TrackOutput::Master);
    h.push_track(track);
    h.push_clip(dc_clip(1, TRACK));
    let shared = h.shared_arc();
    shared.playing.store(true, Ordering::Relaxed);
    let mut cb = MixAudioHarness::on_shared(shared, BLOCK, 2, SR);
    // Block 1 ramps the track in from silence (its own unrelated
    // fade-in, since a fresh track's last gain also starts at zero);
    // block 2 is steady at LEVEL, with nothing left to settle.
    let _ = cb.render();
    let steady = left(cb.render());
    for &s in &steady {
        assert!(
            (s - LEVEL).abs() < 1e-6,
            "track should be steady at {LEVEL} before the reroute, got {s}"
        );
    }
    (h, cb)
}

/// The main repro: routing a playing track onto a brand-new default bus
/// must not dip below the level it was already playing at.
#[test]
fn default_bus_first_block_does_not_dip() {
    let (mut h, mut cb) = playing_track_settled();
    h.add_bus(BUS, None);
    h.set_track_output(TRACK, TrackOutput::Bus(BUS));

    let block = left(cb.render());
    let low = block.iter().copied().fold(f32::MAX, f32::min);
    assert!(
        low >= LEVEL - 1e-4,
        "bus's first rendered block dipped: min sample {low} < steady level {LEVEL} \
         (a track re-routed onto a brand-new bus must not ramp in from silence, FU-B6a)"
    );
    // With the fix the whole first block is flat at the target — no
    // ramp at all, so no sample exceeds it either.
    let high = block.iter().copied().fold(f32::MIN, f32::max);
    assert!(
        (high - LEVEL).abs() < 1e-4,
        "first block on a new bus should render flat at the target gain: max {high} != {LEVEL}"
    );
}

/// A new bus given a non-unity fader before it ever renders: the first
/// block must sit flat at the attenuated steady level, never dipping
/// below it.
#[test]
fn non_unity_fader_bus_first_block_does_not_dip() {
    let (mut h, mut cb) = playing_track_settled();
    h.add_bus(BUS, None);
    h.set_bus_volume(BUS, 0.5);
    h.set_track_output(TRACK, TrackOutput::Bus(BUS));

    let steady = LEVEL * 0.5;
    let block = left(cb.render());
    let low = block.iter().copied().fold(f32::MAX, f32::min);
    assert!(
        low >= steady - 1e-4,
        "attenuated new bus's first block dipped below its own steady level: {low} < {steady}"
    );
    let high = block.iter().copied().fold(f32::MIN, f32::max);
    assert!(
        (high - steady).abs() < 1e-4,
        "first block on a new bus should render flat at the target gain: max {high} != {steady}"
    );
}

/// A bus muted before it ever renders must stay silent on the block a
/// track is first routed onto it — not ramp from an unheard "1.0" down
/// to zero (a pop nobody should ever hear, since the bus was never
/// audible in the first place).
#[test]
fn muted_new_bus_stays_silent_no_pop() {
    let (mut h, mut cb) = playing_track_settled();
    h.add_bus(BUS, None);
    h.dispatch(AudioCommand::SetBusMute { bus_id: BUS, muted: true });
    h.set_track_output(TRACK, TrackOutput::Bus(BUS));

    let block = left(cb.render());
    assert!(
        block.iter().all(|&s| s == 0.0),
        "a bus muted before it ever rendered must stay silent, not pop through on its first \
         block: {block:?}"
    );
}
