//! Do structural bar shifts carry the tempo and signature tracks with
//! them? (ba todo #1386 found that they did not; #1388 is the fix.
//! Design doc #286 §4, finding doc #287.)
//!
//! `update/arrangement.rs` moves audio clips, MIDI clips, section
//! placements, markers and automation lanes. It used to never mention
//! `tempo_events` or `signature_events`, so a meter change written for
//! the bridge stayed at its old bar while the bridge itself moved past
//! it. These tests were written against that behaviour and have been
//! flipped to the fixed one; the shape of each case is unchanged so the
//! finding stays readable next to its fix.
//!
//! The semantics they pin, decided in #1388:
//!
//! - Events SHIFT with the music they describe, by exact `± count` bar
//!   arithmetic, like section placements.
//! - The opening event at bar 0 is pinned: every bar needs a tempo and a
//!   meter in force, so inserting at bar 1 leaves it alone and the new
//!   bars take it.
//! - `remove_bars` CLAMPS an event inside the removed span onto the cut
//!   rather than dropping it, so the surviving music keeps the tempo and
//!   meter it was written in. When that lands two events of one kind on
//!   the cut bar, the LATER one wins — the state in force when the
//!   surviving music starts, and the only answer that keeps both lists
//!   one-event-per-bar (ba todo #1382).
//!
//! Two things worth knowing before reading the numbers:
//!
//! - `InsertBars` / `RemoveBars` have no GUI surface (`grep` finds them
//!   only under `update/control/`), so today only a control-API client
//!   can trigger this.
//! - `TempoMap` RAMPS between tempo points, so moving a tempo event at
//!   bar 33 changes the length of every bar before it too. That is why
//!   the tempo cases assert bars rather than samples, and why
//!   `shift_samples` is measured rather than assumed — see
//!   `a_moved_tempo_event_stretches_its_ramp_and_shift_samples_says_so`.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::{GlobalTrackMessage, Message};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::FadeCurve;
use resonance_control::methods::arrangement::{
    self as proto, InsertBarsParams, RemoveBarsParams, ShiftResult,
};
use resonance_control::methods::edit::{EditStatus, UndoResult};
use resonance_control::{Request, Response};

const AUDIO: u64 = 40;
const SR: u32 = 48_000;
/// 4/4 at 120 BPM: one bar is two seconds.
const BAR: u64 = 2 * SR as u64;

/// The bar the "bridge" starts on, 0-based as app state stores it
/// (1-based bar 33 on the wire).
const BRIDGE: u32 = 32;

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let request = Request::new(1, method, params).expect("params serialize");
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("every request gets exactly one reply")
}

fn call_without_params(app: &mut Resonance, method: &str) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: Request::without_params(1, method),
        reply,
    })));
    rx.try_recv().expect("every request gets exactly one reply")
}

fn section_definition(id: u64, length_bars: u32) -> resonance_app::compose::SectionDefinitionState {
    use resonance_app::compose::{GenerateParams, SectionDefinitionState};
    use resonance_music_theory::MotifSource;
    SectionDefinitionState {
        id,
        name: format!("S{id}"),
        color: [0, 0, 0],
        length_bars,
        chords: Vec::new(),
        scale: None,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: std::collections::HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    }
}

fn push_audio_clip(app: &mut Resonance, id: u64, start_sample: u64) {
    app.test_push_clip(ClipState {
        id,
        track_id: AUDIO,
        start_sample,
        duration_samples: BAR,
        name: format!("take {id}"),
        total_frames: BAR,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    });
}

fn clip_start(app: &Resonance, id: u64) -> Option<u64> {
    app.test_clips()
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.start_sample)
}

fn placement_bar(app: &Resonance, id: u64) -> Option<u32> {
    app.test_placements()
        .iter()
        .find(|(pid, _, _)| *pid == id)
        .map(|(_, _, bar)| *bar)
}

/// The bridge's own meter/tempo, as `(bar, numerator, denominator)` and
/// `(bar, bpm)` — the pair every assertion below compares.
fn signature_at(app: &Resonance, index: usize) -> (u32, u8, u8) {
    let e = &app.test_signature_events()[index];
    (e.bar, e.numerator, e.denominator)
}

fn tempo_at(app: &Resonance, index: usize) -> (u32, f32) {
    let e = &app.test_tempo_events()[index];
    (e.bar, e.bpm)
}

/// The 0-based bar of every event on a track, which is what "one event
/// per bar" is asserted against.
fn signature_bars(app: &Resonance) -> Vec<u32> {
    app.test_signature_events().iter().map(|e| e.bar).collect()
}

fn tempo_bars(app: &Resonance) -> Vec<u32> {
    app.test_tempo_events().iter().map(|e| e.bar).collect()
}

/// A 4/4, 120 BPM song with a bridge (a section placement) at [`BRIDGE`]
/// and an audio track to hang clips on. The bridge is a section
/// PLACEMENT because placements are stored in bars: they move by exact
/// bar arithmetic, so "where did the music end up" needs no sample
/// rounding argument.
fn app_with_bridge() -> (Resonance, u64) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-arrangement-global.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(AUDIO, resonance_audio::types::TrackType::Audio);

    app.test_push_section_definition(section_definition(1, 8));
    let placement = app.test_place_section(1, BRIDGE);
    (app, placement)
}

/// Write a meter change at 0-based `bar`, through the same message the
/// global-tracks shelf sends.
fn add_meter(app: &mut Resonance, bar: u32, numerator: u8, denominator: u8) {
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar,
        numerator,
        denominator,
    }));
}

/// Write a 140 BPM tempo change at 0-based `bar`.
fn add_tempo(app: &mut Resonance, bar: u32, bpm: f32) {
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar,
        bpm,
    }));
}

/// The bridge's 7/8.
fn add_meter_change(app: &mut Resonance, bar: u32) {
    add_meter(app, bar, 7, 8);
    assert_eq!(
        app.test_signature_events().len(),
        2,
        "the bar-0 default plus the bridge's own meter"
    );
}

/// The bridge's 140 BPM.
fn add_tempo_change(app: &mut Resonance, bar: u32) {
    add_tempo(app, bar, 140.0);
    assert_eq!(app.test_tempo_events().len(), 2);
}

fn insert(app: &mut Resonance, at_bar: u32, count: u32) -> ShiftResult {
    call(app, proto::INSERT_BARS, &InsertBarsParams { at_bar, count })
        .result::<ShiftResult>()
        .expect("arrangement.insert_bars succeeds")
}

fn remove(app: &mut Resonance, at_bar: u32, count: u32) -> ShiftResult {
    call(
        app,
        proto::REMOVE_BARS,
        &RemoveBarsParams {
            at_bar,
            count,
            confirm: true,
        },
    )
    .result::<ShiftResult>()
    .expect("arrangement.remove_bars succeeds")
}

// ---------------------------------------------------------------------------
// insert_bars
// ---------------------------------------------------------------------------

/// Insert 8 bars ahead of the bridge. The music moves, and the meter
/// written for it moves with it.
#[test]
fn inserting_bars_carries_a_signature_event_with_the_music() {
    let (mut app, placement) = app_with_bridge();
    add_meter_change(&mut app, BRIDGE);
    // A clip 8 bars ahead of the bridge — i.e. exactly where the inserted
    // bars push it onto the bridge's old bar.
    push_audio_clip(&mut app, 1, u64::from(BRIDGE - 8) * BAR);

    // 8 bars at 1-based bar 20: before the bridge, before the event, and
    // inside the flat 4/4 120 region, so the shift is 8 × 2 seconds.
    let result = insert(&mut app, 20, 8);
    assert_eq!(result.shift_samples, 8 * BAR as i64);

    assert_eq!(
        placement_bar(&app, placement),
        Some(BRIDGE + 8),
        "the bridge itself moves 8 bars later, as everything else does"
    );
    assert_eq!(
        clip_start(&app, 1),
        Some(u64::from(BRIDGE) * BAR),
        "and the clip that was 8 bars before the bridge lands exactly on \
         the bridge's OLD bar"
    );

    assert_eq!(
        signature_at(&app, 1),
        (BRIDGE + 8, 7, 8),
        "the 7/8 written for the bridge travels with it, so the bridge is \
         still the bar that changes meter"
    );
    assert_eq!(result.signature_events_moved, 1);
    assert_eq!(result.signature_events_removed, 0, "nothing collides");
}

/// Same shift, tempo track. Bars rather than samples, because the ramp
/// from 120 to 140 makes every bar between the two events shorter than
/// two seconds.
#[test]
fn inserting_bars_carries_a_tempo_event_with_the_music() {
    let (mut app, placement) = app_with_bridge();
    add_tempo_change(&mut app, BRIDGE);

    let result = insert(&mut app, 20, 8);

    assert_eq!(
        placement_bar(&app, placement),
        Some(BRIDGE + 8),
        "the bridge moves 8 bars later"
    );
    assert_eq!(
        tempo_at(&app, 1),
        (BRIDGE + 8, 140.0),
        "and the 140 it arrives at moves with it"
    );
    assert_eq!(result.tempo_events_moved, 1);
}

/// The control that shows the assertions above are about POSITION, not
/// about the events moving on every shift: inserting bars after both
/// events leaves them alone.
#[test]
fn inserting_bars_after_the_events_is_a_no_op_for_them() {
    let (mut app, _placement) = app_with_bridge();
    add_meter_change(&mut app, BRIDGE);
    add_tempo_change(&mut app, BRIDGE);

    let result = insert(&mut app, BRIDGE + 4, 2);

    assert_eq!(signature_at(&app, 1), (BRIDGE, 7, 8));
    assert_eq!(tempo_at(&app, 1), (BRIDGE, 140.0));
    assert_eq!(result.signature_events_moved, 0);
    assert_eq!(result.tempo_events_moved, 0);
}

/// The song's opening tempo and meter are not content: they are the
/// state bar 0 is played in, and bar 0 always exists. Inserting at bar 1
/// therefore leaves them where they are — the new bars simply take them,
/// which is also what the material they push later already had.
#[test]
fn the_opening_events_at_bar_0_are_pinned() {
    let (mut app, _placement) = app_with_bridge();
    add_meter_change(&mut app, 8);
    add_tempo_change(&mut app, 8);

    let result = insert(&mut app, 1, 4);

    assert_eq!(tempo_bars(&app), vec![0, 12], "bar 0 stays, bar 8 -> 12");
    assert_eq!(signature_bars(&app), vec![0, 12]);
    assert_eq!(
        tempo_at(&app, 0),
        (0, 120.0),
        "and it is the same event, not a replacement"
    );
    assert_eq!(result.tempo_events_moved, 1, "only the bar-8 one moved");
    assert_eq!(result.signature_events_moved, 1);
}

/// The inserted bars straddle a meter change. Because that change moves
/// with its music, the new bars are in the meter in force BEFORE the cut
/// — so the gap is `count` bars of that meter, not the tick span of the
/// bars that currently occupy those numbers. Getting this wrong lands
/// everything after the cut off the grid by the difference (a 7/8 bar is
/// 1680 ticks against 4/4's 1920 — ba doc #288).
#[test]
fn inserting_bars_over_a_meter_change_opens_them_in_the_meter_before_it() {
    let (mut app, _placement) = app_with_bridge();
    add_meter_change(&mut app, 22);

    // A clip on the downbeat of 0-based bar 24: two 7/8 bars past the
    // meter change.
    let before = app.test_tempo_map().bar_to_sample(24);
    push_audio_clip(&mut app, 1, before);

    // 8 bars at 1-based 20 = 0-based 19..26, which is where the 7/8 sits.
    insert(&mut app, 20, 8);

    assert_eq!(signature_at(&app, 1), (30, 7, 8), "the 7/8 moved 8 bars");
    assert_eq!(
        clip_start(&app, 1),
        Some(app.test_tempo_map().bar_to_sample(32)),
        "and the clip is still exactly two 7/8 bars past it — the eight new \
         bars are 4/4, the meter the cut point was in"
    );
}

// ---------------------------------------------------------------------------
// remove_bars
// ---------------------------------------------------------------------------

/// Removing bars before the bridge pulls it earlier and takes the meter
/// change with it — the mirror image of the insert case.
#[test]
fn removing_bars_carries_a_signature_event_with_the_music() {
    let (mut app, placement) = app_with_bridge();
    add_meter_change(&mut app, BRIDGE);
    push_audio_clip(&mut app, 1, u64::from(BRIDGE) * BAR);

    let result = remove(&mut app, 20, 8);
    assert_eq!(result.shift_samples, -(8 * BAR as i64));
    assert!(
        result.clips_deleted.is_empty(),
        "1-based bars 20..27 are empty, so nothing is destroyed"
    );

    assert_eq!(
        placement_bar(&app, placement),
        Some(BRIDGE - 8),
        "the bridge moves 8 bars earlier"
    );
    assert_eq!(
        clip_start(&app, 1),
        Some(u64::from(BRIDGE - 8) * BAR),
        "and so does its audio"
    );

    assert_eq!(
        signature_at(&app, 1),
        (BRIDGE - 8, 7, 8),
        "the 7/8 stays on the bar of the music it belongs to"
    );
    assert_eq!(result.signature_events_moved, 1);
}

/// Removing bars, tempo track.
#[test]
fn removing_bars_carries_a_tempo_event_with_the_music() {
    let (mut app, placement) = app_with_bridge();
    add_tempo_change(&mut app, BRIDGE);

    let result = remove(&mut app, 20, 8);

    assert_eq!(placement_bar(&app, placement), Some(BRIDGE - 8));
    assert_eq!(tempo_at(&app, 1), (BRIDGE - 8, 140.0));
    assert_eq!(result.tempo_events_moved, 1);
}

/// The case `insert_bars` does not have: the removed span CONTAINS the
/// events. The bars they were written for are gone, but the music that
/// survives the splice was written under them, so they are clamped onto
/// the cut rather than dropped — dropping a tempo event silently
/// reshapes the ramp for everything after it, and dropping a signature
/// event renumbers every later bar.
#[test]
fn removing_the_bars_that_hold_an_event_clamps_it_to_the_cut() {
    // Events at 0-based bar 22 = 1-based bar 23, inside the 1-based bars
    // 20..27 the removal deletes. The cut is 0-based bar 19.
    let (mut app, placement) = app_with_bridge();
    add_meter_change(&mut app, 22);
    add_tempo_change(&mut app, 22);

    let result = remove(&mut app, 20, 8);
    assert!(
        result.clips_deleted.is_empty() && result.placements_deleted.is_empty(),
        "only the bars go here; the bridge sits after them"
    );
    assert_eq!(
        placement_bar(&app, placement),
        Some(BRIDGE - 8),
        "the bridge still moves earlier"
    );

    assert_eq!(
        app.test_signature_events().len(),
        2,
        "the event inside the deleted span survives it"
    );
    assert_eq!(
        signature_at(&app, 1),
        (19, 7, 8),
        "clamped onto the cut, so the music after the splice is still in 7/8"
    );
    assert_eq!(
        tempo_at(&app, 1),
        (19, 140.0),
        "and so is the tempo it was written at"
    );
    assert_eq!(result.signature_events_moved, 1);
    assert_eq!(result.tempo_events_moved, 1);
    assert_eq!(
        (result.signature_events_removed, result.tempo_events_removed),
        (0, 0),
        "one event each, so nothing is superseded"
    );
}

/// Several events can land on the cut at once — two inside the span, or
/// one inside plus the one that was already on the first surviving bar.
/// The bar can only carry one (ba todo #1382: two events on a bar make it
/// unaddressable), and the one that survives is the LAST, because that is
/// what is in force when the surviving music starts.
#[test]
fn events_clamped_onto_an_occupied_cut_bar_resolve_to_the_later_one() {
    // Two inside the removed 1-based bars 20..27 (0-based 19..26).
    let (mut app, _placement) = app_with_bridge();
    add_meter(&mut app, 19, 5, 4);
    add_meter(&mut app, 22, 7, 8);
    add_tempo(&mut app, 19, 132.0);
    add_tempo(&mut app, 22, 140.0);

    let result = remove(&mut app, 20, 8);

    assert_eq!(signature_bars(&app), vec![0, 19], "one event on the cut bar");
    assert_eq!(
        signature_at(&app, 1),
        (19, 7, 8),
        "the 7/8 from bar 22 wins: it is the meter the surviving music is in"
    );
    assert_eq!(tempo_bars(&app), vec![0, 19]);
    assert_eq!(tempo_at(&app, 1), (19, 140.0));
    assert_eq!(
        (result.signature_events_removed, result.tempo_events_removed),
        (1, 1),
        "the superseded bar-19 events are reported, not silently dropped"
    );

    // And the same when the collision is with the event on the first
    // surviving bar (0-based 27), which outranks everything inside.
    let (mut app, _placement) = app_with_bridge();
    add_meter(&mut app, 22, 7, 8);
    add_meter(&mut app, 27, 3, 4);

    remove(&mut app, 20, 8);

    assert_eq!(signature_bars(&app), vec![0, 19]);
    assert_eq!(
        signature_at(&app, 1),
        (19, 3, 4),
        "the 3/4 that opened the surviving music keeps opening it"
    );
}

// ---------------------------------------------------------------------------
// The tempo map the shift is computed against is the one the shift changes
// ---------------------------------------------------------------------------

/// `TempoMap` ramps linearly between tempo points, so moving a tempo
/// event does not only re-time its own bar: its whole approach ramp is
/// now longer, which changes the length of every bar between it and the
/// previous event — including bars before the cut.
///
/// That makes "8 bars" an ambiguous number of samples, so `shift_samples`
/// is measured after the fact: it is what the cut point actually moved
/// by, on the rebuilt map. This test pins that against the clip sitting
/// on the cut, which must still be on a downbeat afterwards.
#[test]
fn a_moved_tempo_event_stretches_its_ramp_and_shift_samples_says_so() {
    let (mut app, _placement) = app_with_bridge();
    // 120 at bar 0 ramping to 140 at the bridge: every bar before the
    // bridge is shorter than two seconds, and gets longer when the
    // bridge — and its 140 — move 8 bars later.
    add_tempo_change(&mut app, BRIDGE);

    let cut_sample = app.test_tempo_map().bar_to_sample(19);
    push_audio_clip(&mut app, 1, cut_sample);

    let result = insert(&mut app, 20, 8);

    assert_eq!(tempo_at(&app, 1), (BRIDGE + 8, 140.0));

    let landed = clip_start(&app, 1).expect("the clip is still there");
    assert_eq!(
        landed,
        app.test_tempo_map().bar_to_sample(27),
        "the clip that sat on the cut is still on a downbeat, 8 bars later, \
         measured against the map the shift left behind"
    );
    assert_eq!(
        result.shift_samples,
        landed as i64 - cut_sample as i64,
        "and shift_samples is exactly that displacement — not the pre-shift \
         bar table's idea of 8 bars"
    );
    assert_ne!(
        result.shift_samples,
        8 * BAR as i64,
        "which is not 8 flat bars: the ramp is 8 bars longer than it was"
    );
}

/// The same for the removal direction, where the ramp gets shorter.
#[test]
fn removing_bars_across_a_ramp_reports_the_displacement_it_produced() {
    let (mut app, _placement) = app_with_bridge();
    add_tempo_change(&mut app, BRIDGE);

    let cut_sample = app.test_tempo_map().bar_to_sample(27);
    push_audio_clip(&mut app, 1, cut_sample);

    let result = remove(&mut app, 20, 8);

    assert_eq!(tempo_at(&app, 1), (BRIDGE - 8, 140.0));
    let landed = clip_start(&app, 1).expect("the clip starts after the cut");
    assert_eq!(
        landed,
        app.test_tempo_map().bar_to_sample(19),
        "the clip that opened the surviving music is on the cut's downbeat"
    );
    assert_eq!(result.shift_samples, landed as i64 - cut_sample as i64);
    assert!(result.shift_samples < 0);
}

/// The engine keeps its own copy of the tempo map, and `arrangement.rs`
/// used to send no tempo command at all — so a shift left the engine
/// playing the old map while the GUI drew the new one.
#[test]
fn the_engine_is_told_about_the_rebuilt_map() {
    use resonance_audio::types::AudioCommand;

    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-arrangement-engine.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    add_tempo_change(&mut app, BRIDGE);
    while cmd_rx.try_recv().is_ok() {}

    insert(&mut app, 20, 8);

    let mut tempo: Vec<Vec<resonance_audio::types::TempoPoint>> = Vec::new();
    while let Ok(command) = cmd_rx.try_recv() {
        if let AudioCommand::SetTempoEvents { tempo: events, .. } = command {
            tempo.push(events);
        }
    }
    assert_eq!(
        tempo.len(),
        1,
        "one shift sends the rebuilt map once, not never and not per event"
    );
    assert_eq!(
        tempo[0].iter().map(|e| e.bar).collect::<Vec<_>>(),
        vec![0, BRIDGE + 8],
        "and it is the moved map, not the one the shift started from"
    );
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

fn undo(app: &mut Resonance) -> UndoResult {
    call_without_params(app, resonance_control::methods::edit::UNDO)
        .result()
        .expect("edit.undo succeeds")
}

fn status(app: &mut Resonance) -> EditStatus {
    call_without_params(app, resonance_control::methods::edit::STATUS)
        .result()
        .expect("edit.status succeeds")
}

/// One structural shift is one undo entry, and the events are inside it.
/// If they moved in a second recorded edit, a single undo would put the
/// clips and the placement back while leaving the tempo and meter where
/// the shift left them.
#[test]
fn one_undo_puts_the_events_back_with_everything_else() {
    let (mut app, placement) = app_with_bridge();
    add_meter_change(&mut app, BRIDGE);
    add_tempo_change(&mut app, BRIDGE);
    push_audio_clip(&mut app, 1, u64::from(BRIDGE) * BAR);
    let clip_before = clip_start(&app, 1);

    insert(&mut app, 20, 8);
    assert_eq!(signature_at(&app, 1), (BRIDGE + 8, 7, 8));
    assert_eq!(
        status(&mut app).undo_label.as_deref(),
        Some("insert bars"),
        "the shift is on top of the history"
    );

    let undone = undo(&mut app);
    assert_eq!(undone.undone.as_deref(), Some("insert bars"));

    assert_eq!(
        signature_at(&app, 1),
        (BRIDGE, 7, 8),
        "the meter change is back on the bridge's original bar"
    );
    assert_eq!(tempo_at(&app, 1), (BRIDGE, 140.0));
    assert_eq!(placement_bar(&app, placement), Some(BRIDGE));
    assert_eq!(clip_start(&app, 1), clip_before);
    assert_ne!(
        undone.status.undo_label.as_deref(),
        Some("insert bars"),
        "and the shift left exactly one entry behind, not one per collection"
    );
}
