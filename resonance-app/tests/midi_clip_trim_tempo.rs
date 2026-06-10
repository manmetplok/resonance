//! Regression coverage for the MIDI clip trim reducer
//! (`update::clips::update_midi_clip_trim`) under tempo changes.
//!
//! Bug (Code review 2026-05-19, Medium): the trim handler computed a
//! single scalar `samples_per_tick` from `transport.bpm` and used it
//! both to project the right-edge sample position from
//! `duration_ticks` and to convert a snapped sample delta back into a
//! tick delta. When the project has tempo changes inside the trim
//! region, ticks at different positions correspond to different sample
//! offsets, so the scalar projection skews both directions. Audio
//! clips already routed the same calculation through
//! `TempoMap::tick_to_abs_sample`; this test pins the MIDI path to the
//! same tempo-map projection.
//!
//! The tests below build a tempo map with a ramp from 120 BPM at
//! bar 0 to 60 BPM at bar 1, place a MIDI clip across the ramp, then
//! drive a right-edge trim and a left-edge trim through the public
//! `MidiClipMessage` surface. Each test compares the post-trim clip
//! geometry against the tempo-aware projection (via `sample_to_abs_tick`
//! / `tick_to_abs_sample`) and asserts that the scalar projection — what
//! the buggy code produced — would have given a materially different
//! answer.

use resonance_app::message::{Message, MidiClipMessage};
use resonance_app::state::{ClipEdge, MidiClipState, TempoEvent, ViewMode};
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TICKS_PER_QUARTER_NOTE;

const SR: u32 = 48_000;
const ZOOM: f32 = 50.0; // pixels per second
const CLIP_ID: u64 = 4242;
const TRACK_ID: u64 = 1;
/// 4 quarter notes per bar at 4/4.
const TICKS_PER_BAR: u64 = 4 * TICKS_PER_QUARTER_NOTE;

/// Build a `Resonance` with a tempo ramp (120 BPM at bar 0, 60 BPM at
/// bar 1) and a single MIDI clip seeded at sample 0 with two bars'
/// worth of ticks. The clip spans the entire tempo ramp, so any
/// scalar `samples_per_tick` from `transport.bpm` (which stays at 120)
/// gives a wrong right-edge sample.
fn build_app_with_tempo_ramp_clip(duration_ticks: u64) -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);
    // Default `tempo_events` is just `{bar:0, bpm:120}`. Add a second
    // event at bar 1 to introduce a ramp during bar 0, with bar 1+
    // sitting at 60 BPM. The rebuilt tempo map's bar table then has
    // bar 0 wider than `48000 * 60 / 120 * 4 = 96_000` samples (the
    // flat-120 value) — exactly the skew the buggy scalar projection
    // ignored.
    app.test_push_tempo_event(TempoEvent { bar: 1, bpm: 60.0 });
    app.test_rebuild_tempo_map();
    app.test_push_midi_clip(MidiClipState {
        id: CLIP_ID,
        track_id: TRACK_ID,
        start_sample: 0,
        duration_ticks,
        name: "test".to_string(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app
}

fn snap_like_reducer(app: &Resonance, sample: u64) -> u64 {
    // Mirror `update_midi_clip_trim`'s snap closure exactly: it uses
    // the flat-BPM `snap_sample_to_grid` (which constructs an empty
    // `TempoMap`), driven from `transport.bpm` and the default
    // numerator.
    use resonance_app::view::timeline::snap_sample_to_grid;
    snap_sample_to_grid(
        sample,
        app.test_transport_bpm(),
        app.test_transport_time_sig().0,
        SR,
        ZOOM,
    )
}

/// Right-edge trim: drag the right edge inward by one slow bar's worth
/// of samples. With the bug, the reducer thought the right edge sat at
/// sample 192_000 (3840 ticks × scalar samples_per_tick at 120 BPM); in
/// reality the bar 0 ramp lets the clip occupy ~252_000 samples or
/// more, so the inward drag produced a trim_end roughly twice as large
/// as the tempo-aware projection.
#[test]
fn right_edge_trim_uses_tempo_map_for_projection() {
    let mut app = build_app_with_tempo_ramp_clip(2 * TICKS_PER_BAR);

    // Original right edge per the tempo map.
    let tempo_map_snapshot = app.test_tempo_map().clone();
    let original_edge = tempo_map_snapshot.tick_to_abs_sample(0, 2 * TICKS_PER_BAR, SR);
    // The scalar-BPM projection used by the buggy code (kept here so
    // the assertion below can prove the new code diverges from it).
    let scalar_samples_per_tick =
        (SR as f64 * 60.0 / app.test_transport_bpm() as f64) / TICKS_PER_QUARTER_NOTE as f64;
    let scalar_edge = ((2 * TICKS_PER_BAR) as f64 * scalar_samples_per_tick) as u64;
    assert!(
        original_edge.abs_diff(scalar_edge) > SR as u64,
        "test setup invalid: tempo-map and scalar projections differ by less than 1s; \
         got tempo-map={original_edge} scalar={scalar_edge}"
    );

    // Start a right-edge trim at the original edge pixel.
    let anchor_x = (original_edge as f64 / SR as f64) as f32 * ZOOM;
    let _ = app.update(Message::MidiClip(MidiClipMessage::StartMidiClipTrim {
        clip_id: CLIP_ID,
        edge: ClipEdge::Right,
        anchor_x,
    }));

    // Drag inward by 200 px (= 4 s = 192_000 samples at SR=48k).
    let drag_px = anchor_x - 200.0;
    let _ = app.update(Message::MidiClip(MidiClipMessage::UpdateMidiClipTrim(
        drag_px,
    )));

    // Expected: project the snapped new edge back through the tempo
    // map. `new_trim_end = original_trim_end + (orig_tick - new_tick)`
    // (orig_tick stays as 2 * TICKS_PER_BAR because the clip starts at
    // sample 0 where abs_tick is also 0, so the right edge's abs_tick
    // equals `2 * TICKS_PER_BAR`).
    let raw_target = original_edge as i64 - (4.0 * SR as f64) as i64;
    let raw_target = raw_target.max(0) as u64;
    let snapped_target = snap_like_reducer(&app, raw_target);
    let orig_edge_abs_tick = tempo_map_snapshot.sample_to_abs_tick(original_edge, SR);
    let snapped_abs_tick = tempo_map_snapshot.sample_to_abs_tick(snapped_target, SR);
    let expected_trim_end = orig_edge_abs_tick - snapped_abs_tick;

    let clip = app
        .test_midi_clips()
        .iter()
        .find(|c| c.id == CLIP_ID)
        .expect("clip survived the trim");

    // Allow a 1-tick tolerance for f64 round-trip noise inside
    // `tick_to_abs_sample` / `sample_to_abs_tick`.
    let diff = clip.trim_end_ticks as i64 - expected_trim_end as i64;
    assert!(
        diff.abs() <= 1,
        "trim_end_ticks {} should match tempo-map projection {} (diff {})",
        clip.trim_end_ticks,
        expected_trim_end,
        diff
    );

    // The scalar projection — what the buggy code computed — would
    // have produced a tick delta from a wrong reference edge
    // (`scalar_edge`) instead of `original_edge`, and would have used
    // the scalar `samples_per_tick` for the inverse conversion.
    // Compute that hypothetical value and confirm the live value
    // differs substantially, locking in the regression.
    let scalar_snapped_delta = snap_like_reducer(
        &app,
        (scalar_edge as i64 - (4.0 * SR as f64) as i64).max(0) as u64,
    ) as i64
        - scalar_edge as i64;
    let scalar_trim_end = (-scalar_snapped_delta as f64 / scalar_samples_per_tick) as u64;
    assert!(
        clip.trim_end_ticks.abs_diff(scalar_trim_end) > TICKS_PER_QUARTER_NOTE,
        "tempo-aware trim_end ({}) should differ from scalar trim_end ({}) by more than a beat \
         — otherwise the test isn't actually exercising the tempo-map fix",
        clip.trim_end_ticks,
        scalar_trim_end
    );

    let _ = app.update(Message::MidiClip(MidiClipMessage::EndMidiClipTrim));
}

/// Left-edge trim: drag the left edge inward (toward the right) by one
/// slow bar's worth of samples. The clip's `start_sample` must move to
/// the tempo-map-projected sample, not the scalar projection. With the
/// bug, `new_start_sample = original_start_sample + delta_ticks *
/// scalar_samples_per_tick`, which lands on the wrong sample once any
/// tempo change sits in the trim region.
#[test]
fn left_edge_trim_uses_tempo_map_for_projection() {
    let mut app = build_app_with_tempo_ramp_clip(2 * TICKS_PER_BAR);

    let tempo_map_snapshot = app.test_tempo_map().clone();
    let original_start = 0u64;

    // Anchor the trim at the left edge pixel.
    let anchor_x = (original_start as f64 / SR as f64) as f32 * ZOOM;
    let _ = app.update(Message::MidiClip(MidiClipMessage::StartMidiClipTrim {
        clip_id: CLIP_ID,
        edge: ClipEdge::Left,
        anchor_x,
    }));

    // Drag the left edge 200 px to the right (= 4 s = 192_000 samples).
    let drag_px = anchor_x + 200.0;
    let _ = app.update(Message::MidiClip(MidiClipMessage::UpdateMidiClipTrim(
        drag_px,
    )));

    let raw_target = (original_start as i64 + (4.0 * SR as f64) as i64).max(0) as u64;
    let snapped_target = snap_like_reducer(&app, raw_target);

    // The new `trim_start_ticks` should equal the absolute tick at the
    // snapped target (because the original `trim_start_ticks` is 0 and
    // the original start lives at abs_tick 0).
    let expected_trim_start = tempo_map_snapshot.sample_to_abs_tick(snapped_target, SR);

    let clip = app
        .test_midi_clips()
        .iter()
        .find(|c| c.id == CLIP_ID)
        .expect("clip survived the trim");

    let diff_trim = clip.trim_start_ticks as i64 - expected_trim_start as i64;
    assert!(
        diff_trim.abs() <= 1,
        "trim_start_ticks {} should match tempo-map projection {} (diff {})",
        clip.trim_start_ticks,
        expected_trim_start,
        diff_trim
    );

    // The new `start_sample` must round-trip back through the tempo
    // map to the same abs-tick. `tick_to_abs_sample(0, trim_start, SR)`
    // is how the engine projects the visible region's start during
    // playback (see `engine/midi/outbound.rs:180`).
    let expected_start_sample =
        tempo_map_snapshot.tick_to_abs_sample(0, clip.trim_start_ticks, SR);
    let diff_start = clip.start_sample as i64 - expected_start_sample as i64;
    assert!(
        diff_start.abs() <= 1,
        "start_sample {} should equal tempo-map projection {} (diff {})",
        clip.start_sample,
        expected_start_sample,
        diff_start
    );

    // Sanity: the scalar projection would have given a different start
    // sample (and wrong tick delta) because bar 0's ramp makes the
    // tempo-aware projection materially slower than 120 BPM scalar.
    let scalar_samples_per_tick =
        (SR as f64 * 60.0 / app.test_transport_bpm() as f64) / TICKS_PER_QUARTER_NOTE as f64;
    let scalar_start_sample =
        (clip.trim_start_ticks as f64 * scalar_samples_per_tick) as u64;
    assert!(
        clip.start_sample.abs_diff(scalar_start_sample) > SR as u64 / 2,
        "tempo-aware start_sample ({}) should differ from scalar start_sample ({}) by \
         more than 0.5s — otherwise the test isn't exercising the fix",
        clip.start_sample,
        scalar_start_sample
    );

    let _ = app.update(Message::MidiClip(MidiClipMessage::EndMidiClipTrim));
}

/// Sanity test: with a flat tempo (no extra events), the reducer's
/// behaviour must match the legacy scalar projection. Catches a
/// regression where the new tempo-map plumbing silently changes
/// behaviour under the original constant-tempo path.
#[test]
fn flat_tempo_right_edge_trim_matches_scalar_projection() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);
    // No additional tempo events: defaults to a single 120 BPM event,
    // so the tempo map's bar table is flat.
    app.test_rebuild_tempo_map();
    let duration_ticks = 2 * TICKS_PER_BAR;
    app.test_push_midi_clip(MidiClipState {
        id: CLIP_ID,
        track_id: TRACK_ID,
        start_sample: 0,
        duration_ticks,
        name: "test".to_string(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });

    let scalar_samples_per_tick =
        (SR as f64 * 60.0 / app.test_transport_bpm() as f64) / TICKS_PER_QUARTER_NOTE as f64;
    let original_edge = (duration_ticks as f64 * scalar_samples_per_tick) as u64;
    let anchor_x = (original_edge as f64 / SR as f64) as f32 * ZOOM;
    let _ = app.update(Message::MidiClip(MidiClipMessage::StartMidiClipTrim {
        clip_id: CLIP_ID,
        edge: ClipEdge::Right,
        anchor_x,
    }));

    // Drag inward by 100 px (= 2 s = 96_000 samples) — small enough to
    // stay well clear of the `max_trim` clamp (which kicks in once the
    // visible region shrinks below one quarter note).
    let drag_seconds: f64 = 2.0;
    let drag_px = (drag_seconds * ZOOM as f64) as f32;
    let _ = app.update(Message::MidiClip(MidiClipMessage::UpdateMidiClipTrim(
        anchor_x - drag_px,
    )));

    let raw_target =
        (original_edge as i64 - (drag_seconds * SR as f64) as i64).max(0) as u64;
    let snapped_target = snap_like_reducer(&app, raw_target);
    let snapped_delta = snapped_target as i64 - original_edge as i64;
    let expected_trim_end = ((-snapped_delta) as f64 / scalar_samples_per_tick) as u64;

    let clip = app
        .test_midi_clips()
        .iter()
        .find(|c| c.id == CLIP_ID)
        .expect("clip survived the trim");
    let diff = clip.trim_end_ticks as i64 - expected_trim_end as i64;
    assert!(
        diff.abs() <= 1,
        "flat-tempo trim_end_ticks {} should match scalar projection {} (diff {})",
        clip.trim_end_ticks,
        expected_trim_end,
        diff
    );

    let _ = app.update(Message::MidiClip(MidiClipMessage::EndMidiClipTrim));
}
