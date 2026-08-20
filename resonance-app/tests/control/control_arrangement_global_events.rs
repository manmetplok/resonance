//! Do structural bar shifts carry the tempo and signature tracks with
//! them? (ba todo #1386, design doc #286 §4.)
//!
//! `update/arrangement.rs` moves audio clips, MIDI clips, section
//! placements, markers and automation lanes. It never mentions
//! `tempo_events` or `signature_events`, so the suspicion was that a
//! meter change written for the bridge stays at its old bar while the
//! bridge itself moves past it.
//!
//! These tests EXIST TO PIN THE CURRENT, WRONG BEHAVIOUR so the finding
//! is reproducible from the repo, and so the fix has something to flip.
//! Every assertion that encodes the defect is marked `DEFECT:` and
//! points at the follow-up todo (#1388). When that todo lands, those
//! assertions change to the shifted bars — they are not describing
//! intended semantics. Everything not marked `DEFECT:` is correct
//! behaviour and stays as written.
//!
//! Two things worth knowing before reading the numbers:
//!
//! - `InsertBars` / `RemoveBars` have no GUI surface (`grep` finds them
//!   only under `update/control/`), so today only a control-API client
//!   can trigger this.
//! - `TempoMap` RAMPS between tempo points, so a tempo event at bar 33
//!   changes the length of every bar before it too. That is why the
//!   tempo cases assert bars rather than samples: with a ramp in the
//!   project an "8 bar" shift is not 8 × 2 seconds.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::{GlobalTrackMessage, Message};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::FadeCurve;
use resonance_control::methods::arrangement::{
    self as proto, InsertBarsParams, RemoveBarsParams, ShiftResult,
};
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

/// Write a 7/8 meter change at 0-based `bar`, through the same message
/// the global-tracks shelf sends.
fn add_meter_change(app: &mut Resonance, bar: u32) {
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar,
        numerator: 7,
        denominator: 8,
    }));
    assert_eq!(
        app.test_signature_events().len(),
        2,
        "the bar-0 default plus the bridge's own meter"
    );
}

/// Write a 140 BPM tempo change at 0-based `bar`.
fn add_tempo_change(app: &mut Resonance, bar: u32) {
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar,
        bpm: 140.0,
    }));
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

/// Insert 8 bars ahead of the bridge. The music moves; the meter written
/// for it does not.
#[test]
fn inserting_bars_strands_a_signature_event_at_its_old_bar() {
    let (mut app, placement) = app_with_bridge();
    add_meter_change(&mut app, BRIDGE);
    // A clip 8 bars ahead of the bridge — i.e. exactly where the inserted
    // bars will push it ONTO the stranded meter change.
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

    // DEFECT (ba todo #1388): the signature track stays put. The 7/8 that
    // was written for the bridge now applies to whatever the insertion
    // pushed onto bar 33, and the bridge — 8 bars later — plays in 4/4.
    assert_eq!(
        signature_at(&app, 1),
        (BRIDGE, 7, 8),
        "DEFECT: the signature event neither moves with the music nor \
         changes in any other way"
    );
}

/// Same shift, tempo track. Bars rather than samples, because the ramp
/// from 120 to 140 makes every bar between the two events shorter than
/// two seconds — which is itself part of the damage: the stranded event
/// keeps ramping over a span that is now eight bars longer.
#[test]
fn inserting_bars_strands_a_tempo_event_at_its_old_bar() {
    let (mut app, placement) = app_with_bridge();
    add_tempo_change(&mut app, BRIDGE);

    insert(&mut app, 20, 8);

    assert_eq!(
        placement_bar(&app, placement),
        Some(BRIDGE + 8),
        "the bridge moves 8 bars later"
    );
    // DEFECT (ba todo #1388).
    assert_eq!(
        tempo_at(&app, 1),
        (BRIDGE, 140.0),
        "DEFECT: the tempo event does not move with it"
    );
}

/// The control that shows the assertions above are about POSITION, not
/// about the events being immune to shifts in general: inserting bars
/// after both events leaves them alone for the right reason.
#[test]
fn inserting_bars_after_the_events_is_a_no_op_for_them() {
    let (mut app, _placement) = app_with_bridge();
    add_meter_change(&mut app, BRIDGE);
    add_tempo_change(&mut app, BRIDGE);

    insert(&mut app, BRIDGE + 4, 2);

    assert_eq!(signature_at(&app, 1), (BRIDGE, 7, 8));
    assert_eq!(tempo_at(&app, 1), (BRIDGE, 140.0));
}

// ---------------------------------------------------------------------------
// remove_bars
// ---------------------------------------------------------------------------

/// Removing bars before the bridge pulls it earlier and leaves the meter
/// change behind — the mirror image of the insert case.
#[test]
fn removing_bars_strands_a_signature_event_at_its_old_bar() {
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

    // DEFECT (ba todo #1388).
    assert_eq!(
        signature_at(&app, 1),
        (BRIDGE, 7, 8),
        "DEFECT: the signature event stays 8 bars later than the music it \
         belongs to"
    );
}

/// Removing bars, tempo track.
#[test]
fn removing_bars_strands_a_tempo_event_at_its_old_bar() {
    let (mut app, placement) = app_with_bridge();
    add_tempo_change(&mut app, BRIDGE);

    remove(&mut app, 20, 8);

    assert_eq!(placement_bar(&app, placement), Some(BRIDGE - 8));
    // DEFECT (ba todo #1388).
    assert_eq!(
        tempo_at(&app, 1),
        (BRIDGE, 140.0),
        "DEFECT: the tempo event does not move with it"
    );
}

/// The case `insert_bars` does not have: the removed span CONTAINS the
/// events. The bars they were written for no longer exist, and the
/// events survive them — neither deleted nor clamped to the cut, they
/// simply keep a bar number that now addresses completely different
/// music.
#[test]
fn removing_the_bars_that_hold_an_event_neither_deletes_nor_clamps_it() {
    // Events at 0-based bar 22 = 1-based bar 23, inside the 1-based bars
    // 20..27 the removal deletes.
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

    // DEFECT (ba todo #1388). Two answers were defensible — drop the
    // event (the previous meter/tempo runs on through the splice) or
    // clamp it to the cut point (the music after the splice keeps the
    // meter/tempo it had). This is neither: 0-based bar 22 is now the
    // music that used to be at bar 30.
    assert_eq!(
        app.test_signature_events().len(),
        2,
        "DEFECT: the event inside the deleted span is not removed"
    );
    assert_eq!(
        signature_at(&app, 1),
        (22, 7, 8),
        "DEFECT: nor clamped to the cut point — it is field-for-field unchanged"
    );
    assert_eq!(
        tempo_at(&app, 1),
        (22, 140.0),
        "DEFECT: the tempo event inside the span survives it too"
    );
}
