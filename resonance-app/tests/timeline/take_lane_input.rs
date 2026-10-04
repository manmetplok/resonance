//! Take-lane comping gestures: select, split at playhead, promote a
//! segment (epic #15, doc #165, todo #414).
//!
//! Todo #413 built the render surface; this suite pins the verbs that sit
//! on top of it, end to end through the *real* canvas input path
//! (`canvas::Program::update`) and the *real* `update::takes` reducers —
//! asserting only the hit-test would pass for a canvas that resolved the
//! right take and then published the wrong message.
//!
//! The traps it is built around, each with its own case:
//!
//! * **Promote targets the SLOT, not the card.** A take card is drawn over
//!   the take's audible extent, but the comp addresses the whole slot, so
//!   a press over the "no audio here" lead-in of a punched-in take is a
//!   hit and the comp region it names is the slot's.
//! * **The promote range is a request.** The view emits the *raw* drag
//!   range; `update::takes` owns the clamp. Asserting on the message
//!   proves the view is not clamping too, and asserting on the resulting
//!   comp proves the reducer still is.
//! * **A refused edit is silent.** The gate drops an impossible edit
//!   before dispatch so it spends no undo entry and bumps no revision — so
//!   the affordance has to say "not here" *before* the press. The cursor
//!   is that channel and it is asserted here.
//! * **A comp edit ends the take solo.** Deliberate, and nothing in the
//!   undo label says so, so the caption does. It no longer *also* warns
//!   that the audible take may change: since todo #1395 a split
//!   materializes the soloed take's cover, so it does not.
//!
//! It also pins what the take lane does **not** take. This todo moved the
//! take-lane hit test ahead of the automation breakpoint dots and the
//! clips, to match the draw order. Every other clip and automation input
//! test in the suite runs with an empty `take_groups` and so never reaches
//! those branches at all — the reorder would be invisible to them. See
//! "The press order" below, which asserts the clip and the dots still
//! behave, and pins the one band the ribbon claims from the clip body so
//! that widening it later is a visible decision.
//!
//! Six goldens, each pinning a state no other golden in the epic reaches:
//! the promote drag mid-flight, the promoted comp it lands, a promote that
//! runs across a stretch the take never recorded, the split affordance
//! while a take is soloed, the refused split, and a MIDI take soloed over
//! audio takes.

use crate::common;

use iced::{Point, Size};
use iced_test::simulator::Simulator;
use resonance_app::message::{
    AutomationMessage, ClipMessage, Message, TakeMessage, TransportMessage, UiMessage,
    ViewportMessage,
};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::view::timeline::automation::{
    automation_band, value_to_y, BREAKPOINT_HIT_RADIUS,
};
use resonance_app::view::timeline::takes::effective_cover;
use resonance_app::view::timeline::TimelineState;
use resonance_app::{demo, theme, Resonance};
use resonance_audio::types::{AudioEvent, FadeCurve};
use resonance_common::{AutomationTarget, CurveKind, TakeContent, TakeGroup, TakeNote, TimelineRange};
use tempfile::TempDir;

const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The demo audio track ("Drums Bounce") the take groups hang off.
const AUDIO_TRACK: u64 = 5;
/// The demo instrument track ("Synth Bass") the MIDI group hangs off.
const MIDI_TRACK: u64 = 2;
const GROUP: u64 = 77;
const TAKE_CLIP_BASE: u64 = 5_000;
/// Project-directory name, and therefore the title the transport bar
/// draws into every golden here. Fixed so the goldens are reproducible.
const PROJECT_NAME: &str = "resonance-take-input-test";

/// Window-space origin of the timeline canvas inside the Arrange page, at
/// [`WINDOW`]: the track-header column to its left, the transport bar and
/// tab strip above it.
///
/// Golden snapshots have to drive the *widget tree*, which is laid out in
/// window space, while every hit test in this file works in canvas space.
/// The offset is asserted by
/// [`the_canvas_origin_constants_still_hold`] rather than trusted, so a
/// layout change fails loudly here instead of silently re-aiming every
/// simulated gesture at the wrong pixel.
const CANVAS_ORIGIN: (f32, f32) = (theme::TRACK_HEADER_WIDTH, 136.0);

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// Demo session in the Arrange view, anchored at a temporary project
/// directory: a take's waveform is read out of
/// `<project>/audio/clip_{clip_ref}.wav` (todo #1400), so the fixtures
/// need somewhere real to write recordings. The returned [`TempDir`] must
/// outlive the app.
///
/// The directory is a fixed-name child of the temp dir. The transport bar
/// titles the session from `project_path.file_stem()`, so a random
/// `tempfile` name would put a different string in every golden;
/// [`PROJECT_NAME`] is the one these were blessed against, back when the
/// fixture pointed at a hard-coded path under `/tmp` (which was also not
/// hermetic — two runs of this binary shared it).
fn build_app() -> (Resonance, TempDir) {
    let dir = tempfile::tempdir().expect("temp project dir");
    let project = dir.path().join(PROJECT_NAME);
    std::fs::create_dir_all(&project).expect("create project dir");
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    app.test_set_active_project(true);
    app.test_set_project_path(project);
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportWidth(
        WINDOW.0 - theme::TRACK_HEADER_WIDTH,
    )));
    let _ = app.update(Message::Viewport(ViewportMessage::ViewportHeight(WINDOW.1)));
    let _ = app.update(Message::Viewport(ViewportMessage::TimelineContentSize(
        2000.0,
        WINDOW.1 * 4.0,
    )));
    (app, dir)
}

/// The slot every fixture records over: 2 s in, 5 s long — canvas
/// x 200..700 at the default 100 px/s zoom.
fn slot(app: &Resonance) -> TimelineRange {
    let sr = app.sample_rate as u64;
    TimelineRange::new(2 * sr, 5 * sr)
}

/// Seconds from the slot's start, as an absolute sample position.
fn at(app: &Resonance, seconds: u64) -> u64 {
    slot(app).start + seconds * app.sample_rate as u64
}

/// Canvas x of an absolute sample position, at the fixture's zoom.
fn x_of(app: &Resonance, sample: u64) -> f32 {
    (sample as f64 / app.sample_rate as f64) as f32 * app.test_arrange_zoom()
}

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

/// Write the recording a take's `clip_ref` names, `extent` long — the
/// file `recording.rs` streams a pass into and the only place the lane's
/// waveform can come from (todo #1400). The same envelope
/// `take_lane_render` authors, so the two suites' goldens agree.
fn write_take_recording(
    app: &Resonance,
    dir: &TempDir,
    pass_index: u32,
    extent: TimelineRange,
) {
    let peak_count =
        (extent.length as usize).div_ceil(resonance_audio::types::WAVEFORM_PEAK_FRAMES);
    common::write_take_wav(
        &dir.path().join(PROJECT_NAME),
        TAKE_CLIP_BASE + u64::from(pass_index),
        app.sample_rate,
        extent.length,
        |i| {
            let t = i as f32 / peak_count.max(1) as f32;
            let cycles = (pass_index + 2) as f32;
            let phase = (t * cycles).fract();
            0.25 + 0.65 * (1.0 - (phase - 0.5).abs() * 2.0)
        },
    );
}

fn capture_audio_pass(app: &mut Resonance, pass_index: u32) {
    capture_audio_pass_over(app, pass_index, slot(app));
}

/// One captured pass that recorded only `extent` of its slot. The event is
/// the app's sole account of that (todo #1396) — a take clip never reaches
/// `Resonance::clips`, so the recording `write_take_recording` puts on
/// disk supplies the waveform and nothing else.
fn capture_audio_pass_over(app: &mut Resonance, pass_index: u32, extent: TimelineRange) {
    let slot = slot(app);
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        take_id: u64::from(pass_index),
        track_id: AUDIO_TRACK,
        slot,
        pass_index,
        extent,
        content: TakeContent::Audio {
            clip_ref: TAKE_CLIP_BASE + u64::from(pass_index),
        },
    });
}

/// `passes` audio takes, each with a recording filling the whole slot.
fn capture_audio_passes(app: &mut Resonance, dir: &TempDir, passes: u32) {
    for pass_index in 0..passes {
        let slot = slot(app);
        write_take_recording(app, dir, pass_index, slot);
        capture_audio_pass(app, pass_index);
    }
}

fn capture_midi_pass(app: &mut Resonance, pass_index: u32) {
    let slot = slot(app);
    let notes: Vec<TakeNote> = (0..8)
        .map(|i| TakeNote {
            note: 52 + (pass_index * 5) as u8 + ((i * 3) % 12) as u8,
            velocity: 0.8,
            start_tick: i * 240,
            duration_ticks: 200,
        })
        .collect();
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        take_id: u64::from(pass_index),
        track_id: MIDI_TRACK,
        slot,
        pass_index,
        extent: slot,
        content: TakeContent::Midi { notes },
    });
}

fn toggle_lane(app: &mut Resonance, track_id: u64) {
    let _ = app.update(Message::Ui(UiMessage::ToggleTakeLane(track_id)));
}

fn seek(app: &mut Resonance, pos: u64) {
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(pos)));
}

fn set_active(app: &mut Resonance, take_id: Option<u64>) {
    app.test_apply_engine_event(AudioEvent::ActiveTakeChanged {
        group_id: GROUP,
        take_id,
    });
}

fn group_of(app: &Resonance) -> &TakeGroup {
    app.test_take_groups()
        .iter()
        .find(|g| g.id == GROUP)
        .expect("take group")
}

fn comp(app: &Resonance) -> Vec<(u64, u64, u64)> {
    group_of(app)
        .comp
        .segments
        .iter()
        .map(|s| (s.range.start, s.range.end(), s.take_id))
        .collect()
}

// ---------------------------------------------------------------------
// Canvas-space geometry of the fixture
// ---------------------------------------------------------------------

/// Canvas y at the vertical centre of `take`'s card in the expanded stack.
fn take_card_y(app: &Resonance, take_id: u64) -> f32 {
    let layout = app.test_arrange_row_layout();
    let track = group_of(app).track_id;
    let (top, h) = layout
        .take_row_rect(track, GROUP, take_id)
        .expect("take sub-row — is the lane expanded?");
    app.test_arrange_header_offset() + top + h / 2.0
}

/// Canvas y inside the comp ribbon on `track`'s own lane.
fn ribbon_y(app: &Resonance, track: u64) -> f32 {
    let layout = app.test_arrange_row_layout();
    let (top, h) = layout.track_row_rect(track).expect("track row");
    let row_y = app.test_arrange_header_offset() + top;
    let (band_top, band_height) =
        resonance_app::view::timeline::takes::comp_ribbon_band(row_y, h);
    band_top + band_height / 2.0
}

// ---------------------------------------------------------------------
// Gesture driving
// ---------------------------------------------------------------------

fn left_press() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
}
fn right_press() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
        iced::mouse::Button::Right,
    ))
}
fn left_release() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ))
}
fn cursor_moved() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: Point::ORIGIN,
    })
}

/// Press at `(x, y)` and return the message the canvas published, if any.
fn press(app: &Resonance, state: &mut TimelineState, x: f32, y: f32) -> Option<Message> {
    app.test_timeline_canvas_event(state, &left_press(), x, y)
}

/// Press → move → release across a take card, returning the message the
/// *release* published. A press alone publishes nothing: click and drag
/// are the same gesture until the pointer stops.
fn sweep(
    app: &Resonance,
    state: &mut TimelineState,
    y: f32,
    from_x: f32,
    to_x: f32,
) -> Option<Message> {
    assert!(
        press(app, state, from_x, y).is_none(),
        "a take-card press must publish nothing — the release decides the verb"
    );
    let _ = app.test_timeline_canvas_event(state, &cursor_moved(), to_x, y);
    app.test_timeline_canvas_event(state, &left_release(), to_x, y)
}

/// A press and release at the same point: a click.
fn click(app: &Resonance, state: &mut TimelineState, x: f32, y: f32) -> Option<Message> {
    sweep(app, state, y, x, x)
}

// ---------------------------------------------------------------------
// Hit-testing: the slot, not the card
// ---------------------------------------------------------------------

/// The inherited trap. Pass 0 punched in a second late, so its **card** is
/// drawn over 3 s..7 s while its lane still spans the 2 s..7 s slot. The
/// hit region has to be the slot: a press over the flat "no audio here"
/// lead-in must still resolve to that take, or the lead-in of every
/// punched-in take would be un-comp-able.
#[test]
fn a_press_on_a_punched_in_takes_silent_lead_in_still_hits_the_take() {
    let (mut app, dir) = build_app();
    let sr = app.sample_rate as u64;
    let slot = slot(&app);
    write_take_recording(
        &app,
        &dir,
        0,
        TimelineRange::from_bounds(slot.start + sr, slot.end()),
    );
    capture_audio_pass(&mut app, 0);
    toggle_lane(&mut app, AUDIO_TRACK);

    let y = take_card_y(&app, 0);
    // Canvas x 250 = 2.5 s: inside the slot, a full half-second before the
    // take's own audio starts.
    let hit = app
        .test_take_card_at(x_of(&app, at(&app, 0) + sr / 2), y)
        .expect("the silent lead-in is part of the take's lane");
    assert_eq!(
        hit,
        (GROUP, 0, slot.start, slot.end()),
        "the hit carries the SLOT, which is what a comp edit addresses"
    );
}

/// The card band, not the whole 38 px row: the sliver of row chrome above
/// and below stays inert so a press there keeps todo #413's behaviour of
/// selecting the owning track.
#[test]
fn the_take_rows_chrome_is_not_part_of_the_card() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    toggle_lane(&mut app, AUDIO_TRACK);

    let layout = app.test_arrange_row_layout();
    let (top, h) = layout.take_row_rect(AUDIO_TRACK, GROUP, 1).expect("row");
    let row_y = app.test_arrange_header_offset() + top;
    let x = x_of(&app, at(&app, 2));

    assert!(app.test_take_card_at(x, row_y + 1.0).is_none());
    assert!(app.test_take_card_at(x, row_y + h - 1.0).is_none());
    assert!(
        app.test_take_card_at(x, row_y + h / 2.0).is_some(),
        "the card itself is a hit"
    );

    let mut state = TimelineState::default();
    assert!(
        matches!(
            press(&app, &mut state, x, row_y + 1.0),
            Some(Message::Ui(UiMessage::SelectTrack(Some(AUDIO_TRACK))))
        ),
        "chrome keeps the #413 fallback"
    );
}

/// Outside the slot horizontally there is no take to hit — the take
/// sub-row extends across the whole canvas, but the take does not.
#[test]
fn a_press_beyond_the_slot_is_not_a_take_hit() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    toggle_lane(&mut app, AUDIO_TRACK);
    let y = take_card_y(&app, 0);

    assert!(app.test_take_card_at(x_of(&app, slot(&app).start) - 20.0, y).is_none());
    assert!(app.test_take_card_at(x_of(&app, slot(&app).end()) + 20.0, y).is_none());
    assert!(app.test_take_card_at(x_of(&app, at(&app, 2)), y).is_some());
}

/// The comp ribbon rides the *track's own* lane and is present whether the
/// stack is folded or open — a closed take folder still answers "split
/// here". It resolves only inside its own group's slot.
#[test]
fn the_comp_ribbon_is_hittable_folded_and_expanded() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    let y = ribbon_y(&app, AUDIO_TRACK);
    let inside = x_of(&app, at(&app, 2));

    assert!(!app.test_take_lane_expanded(AUDIO_TRACK));
    assert_eq!(app.test_comp_ribbon_at(inside, y), Some(GROUP));
    assert_eq!(
        app.test_comp_ribbon_at(x_of(&app, slot(&app).end()) + 40.0, y),
        None,
        "past the slot the lane carries no comp"
    );

    toggle_lane(&mut app, AUDIO_TRACK);
    assert_eq!(
        app.test_comp_ribbon_at(inside, ribbon_y(&app, AUDIO_TRACK)),
        Some(GROUP),
        "unfolding the stack does not move the ribbon off its track's lane"
    );
}

// ---------------------------------------------------------------------
// Select the active take
// ---------------------------------------------------------------------

/// A click on a take card solos it; a second click on the same card
/// releases the solo. Both go through the real reducers, so the mirror and
/// the engine command are what is asserted, not just the message.
#[test]
fn clicking_a_take_card_solos_it_and_clicking_again_releases_it() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);
    let rx = app.test_capture_engine();
    let mut state = TimelineState::default();

    let y = take_card_y(&app, 1);
    let x = x_of(&app, at(&app, 2));
    let msg = click(&app, &mut state, x, y).expect("the release publishes");
    assert!(
        matches!(
            msg,
            Message::Take(TakeMessage::SetActiveTake {
                group_id: GROUP,
                take_id: Some(1)
            })
        ),
        "got {msg:?}"
    );
    app.test_dispatch(msg);
    assert_eq!(group_of(&app).active_take, Some(1));
    assert!(
        drain(&rx).iter().any(|c| matches!(
            c,
            resonance_audio::types::AudioCommand::SetActiveTake {
                take_id: Some(1),
                ..
            }
        )),
        "the engine is told, or nothing is actually soloed"
    );

    let again = click(&app, &mut state, x, take_card_y(&app, 1)).expect("release");
    assert!(
        matches!(
            again,
            Message::Take(TakeMessage::SetActiveTake {
                group_id: GROUP,
                take_id: None
            })
        ),
        "a second click on the soloed take releases it, got {again:?}"
    );
    app.test_dispatch(again);
    assert_eq!(group_of(&app).active_take, None);
}

/// Clicking a *different* card moves the solo rather than clearing it —
/// the toggle is read off the mirror, not remembered in canvas state, so
/// it cannot drift out of step with an engine echo.
#[test]
fn clicking_another_card_moves_the_solo() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);
    set_active(&mut app, Some(0));
    let mut state = TimelineState::default();

    let msg = click(&app, &mut state, x_of(&app, at(&app, 2)), take_card_y(&app, 2))
        .expect("release");
    assert!(
        matches!(
            msg,
            Message::Take(TakeMessage::SetActiveTake {
                take_id: Some(2),
                ..
            })
        ),
        "got {msg:?}"
    );
    app.test_dispatch(msg);
    assert_eq!(group_of(&app).active_take, Some(2));
}

// ---------------------------------------------------------------------
// Promote a segment
// ---------------------------------------------------------------------

/// The core comping gesture: sweep across a take, and exactly that stretch
/// of the comp becomes that take.
///
/// The message must carry the **raw** drag range — `update::takes` owns
/// the clamp, and a view that pre-clamped would give two clamps free to
/// disagree — while the *resulting comp* must still be a gap-free cover of
/// the slot, because #411 materializes the implicit cover before editing
/// it.
#[test]
fn dragging_across_a_take_promotes_exactly_that_span() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);
    let rx = app.test_capture_engine();
    let mut state = TimelineState::default();

    let (from, to) = (at(&app, 1), at(&app, 3));
    let msg = sweep(
        &app,
        &mut state,
        take_card_y(&app, 0),
        x_of(&app, from),
        x_of(&app, to),
    )
    .expect("the release publishes a promote");
    let Message::Take(TakeMessage::PromoteTakeSegment {
        group_id,
        take_id,
        range,
    }) = &msg
    else {
        panic!("expected PromoteTakeSegment, got {msg:?}");
    };
    assert_eq!((*group_id, *take_id), (GROUP, 0));
    // Pixel → frame rounding is one frame at 48 kHz / 100 px per second.
    assert!(
        range.start.abs_diff(from) < 500 && range.end().abs_diff(to) < 500,
        "the raw drag range, got {range:?} for {from}..{to}"
    );

    app.test_dispatch(msg);
    assert_eq!(
        comp(&app)
            .into_iter()
            .map(|(_, _, take)| take)
            .collect::<Vec<_>>(),
        vec![2, 0, 2],
        "take 0 in the middle, the latest pass either side"
    );
    let segments = comp(&app);
    assert_eq!(segments[0].0, slot(&app).start, "the cover starts at the slot");
    assert_eq!(segments[2].1, slot(&app).end(), "...and reaches its end");
    assert!(
        drain(&rx).iter().any(|c| matches!(
            c,
            resonance_audio::types::AudioCommand::SetTakeComp { .. }
        )),
        "a comp nobody told the engine about is a comp nobody hears"
    );
}

/// A backwards sweep names the same span: the range is normalized, so
/// comping right-to-left works exactly like left-to-right.
#[test]
fn a_backwards_drag_promotes_the_same_span() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    toggle_lane(&mut app, AUDIO_TRACK);
    let mut state = TimelineState::default();

    let (a, b) = (at(&app, 1), at(&app, 3));
    let msg = sweep(
        &app,
        &mut state,
        take_card_y(&app, 0),
        x_of(&app, b),
        x_of(&app, a),
    )
    .expect("release");
    let Message::Take(TakeMessage::PromoteTakeSegment { range, .. }) = &msg else {
        panic!("expected a promote, got {msg:?}");
    };
    assert!(range.start < range.end(), "normalized, got {range:?}");
    assert!(range.start.abs_diff(a) < 500 && range.end().abs_diff(b) < 500);
}

/// A drag that runs off the end of the lane still emits the raw range —
/// the view does not clamp — and the reducer's clamp is what keeps the
/// resulting comp inside the slot.
#[test]
fn a_drag_past_the_slot_emits_the_raw_range_and_the_reducer_clamps_it() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    toggle_lane(&mut app, AUDIO_TRACK);
    let mut state = TimelineState::default();

    let slot = slot(&app);
    let past = x_of(&app, slot.end()) + 300.0;
    let msg = sweep(
        &app,
        &mut state,
        take_card_y(&app, 0),
        x_of(&app, at(&app, 3)),
        past,
    )
    .expect("release");
    let Message::Take(TakeMessage::PromoteTakeSegment { range, .. }) = &msg else {
        panic!("expected a promote, got {msg:?}");
    };
    assert!(
        range.end() > slot.end(),
        "the VIEW emits the raw drag: {range:?} must overhang the slot end {}",
        slot.end()
    );

    app.test_dispatch(msg);
    let last = *comp(&app).last().expect("a segment");
    assert_eq!(
        (last.1, last.2),
        (slot.end(), 0),
        "the REDUCER's clamp is what bounds the comp"
    );
}

/// A sweep shorter than the slop threshold is a click, not a promote: it
/// solos. Otherwise every slightly-shaky click on a 29 px card would
/// promote a few-millisecond sliver into the comp.
#[test]
fn a_sweep_inside_the_slop_threshold_is_still_a_click() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    toggle_lane(&mut app, AUDIO_TRACK);
    let mut state = TimelineState::default();

    let x = x_of(&app, at(&app, 2));
    let msg = sweep(&app, &mut state, take_card_y(&app, 0), x, x + 3.0).expect("release");
    assert!(
        matches!(
            msg,
            Message::Take(TakeMessage::SetActiveTake {
                take_id: Some(0),
                ..
            })
        ),
        "3 px is a shaky click, got {msg:?}"
    );
}

/// Promoting a take over a stretch it never recorded is legal — the
/// message is emitted raw and the reducer's clamp is what decides. Here
/// the clamp bites: pass 0 punched in a second late, so a drag over its
/// silent lead-in promotes only the part it can actually fill.
#[test]
fn promoting_a_punched_in_takes_lead_in_is_clamped_to_its_audio() {
    let (mut app, dir) = build_app();
    let sr = app.sample_rate as u64;
    let slot = slot(&app);
    let punch_in = slot.start + sr;
    let punched = TimelineRange::from_bounds(punch_in, slot.end());
    write_take_recording(&app, &dir, 0, punched);
    capture_audio_pass_over(&mut app, 0, punched);
    write_take_recording(&app, &dir, 1, slot);
    capture_audio_pass(&mut app, 1);
    toggle_lane(&mut app, AUDIO_TRACK);
    let mut state = TimelineState::default();

    // Sweep the whole lane, silent lead-in included.
    let msg = sweep(
        &app,
        &mut state,
        take_card_y(&app, 0),
        x_of(&app, slot.start),
        x_of(&app, slot.end()),
    )
    .expect("release");
    let Message::Take(TakeMessage::PromoteTakeSegment { range, .. }) = &msg else {
        panic!("expected a promote, got {msg:?}");
    };
    assert!(
        range.start < punch_in,
        "the view emits the whole sweep, lead-in included"
    );

    app.test_dispatch(msg);
    let segments = comp(&app);
    assert_eq!(
        segments.iter().map(|s| s.2).collect::<Vec<_>>(),
        vec![1, 0],
        "the lead-in stays with the take that recorded it"
    );
    assert_eq!(
        segments[1].0, punch_in,
        "take 0 takes over exactly where its audio starts"
    );
}

// ---------------------------------------------------------------------
// Split at the playhead
// ---------------------------------------------------------------------

/// Clicking the comp ribbon splits the comp where the playhead is,
/// creating the boundary a promote is made against.
#[test]
fn clicking_the_comp_ribbon_splits_at_the_playhead() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let cut = at(&app, 2);
    seek(&mut app, cut);
    let mut state = TimelineState::default();

    let msg = press(
        &app,
        &mut state,
        x_of(&app, at(&app, 4)),
        ribbon_y(&app, AUDIO_TRACK),
    )
    .expect("the ribbon publishes on press — a split names no range");
    assert!(
        matches!(
            msg,
            Message::Take(TakeMessage::SplitCompAtPlayhead { group_id: GROUP })
        ),
        "got {msg:?}"
    );

    app.test_dispatch(msg);
    assert_eq!(
        comp(&app),
        vec![
            (slot(&app).start, cut, 2),
            (cut, slot(&app).end(), 2),
        ],
        "one cover, cut at the playhead, both halves still the latest take"
    );
}

/// The refusal that has to be visible *before* the press. With the
/// playhead outside the slot there is no cut point; the gate drops the
/// edit silently and spends no undo entry, so the cursor is what tells the
/// user — `NotAllowed` over the ribbon rather than a click into the void.
#[test]
fn a_split_with_the_playhead_outside_the_slot_is_flagged_by_the_cursor() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let state = TimelineState::default();
    let (x, y) = (x_of(&app, at(&app, 2)), ribbon_y(&app, AUDIO_TRACK));

    let past_end = slot(&app).end() + app.sample_rate as u64;
    seek(&mut app, past_end);
    assert_eq!(
        app.test_timeline_cursor(&state, x, y),
        iced::mouse::Interaction::NotAllowed,
        "no cut point: say so, because the refusal itself is silent"
    );

    let cut = at(&app, 2);
    seek(&mut app, cut);
    assert_eq!(
        app.test_timeline_cursor(&state, x, y),
        iced::mouse::Interaction::ResizingHorizontally,
        "with a cut point the ribbon offers the split"
    );
}

/// ...and pressing anyway really is a no-op: the comp is untouched and no
/// undo entry is spent. The gate refuses it before dispatch (ba doc #292),
/// which is exactly why the UI cannot use the undo history as a receipt.
#[test]
fn a_refused_split_changes_nothing_and_spends_no_undo_entry() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let past_end = slot(&app).end() + app.sample_rate as u64;
    seek(&mut app, past_end);
    let could_undo = app.test_can_undo();
    let mut state = TimelineState::default();

    let msg = press(
        &app,
        &mut state,
        x_of(&app, at(&app, 2)),
        ribbon_y(&app, AUDIO_TRACK),
    )
    .expect("the gesture still publishes — refusal is the reducer's call");
    app.test_dispatch(msg);

    assert!(comp(&app).is_empty(), "no comp was materialized");
    assert_eq!(
        app.test_can_undo(),
        could_undo,
        "a refused edit spends no undo entry"
    );
}

/// A comp edit ends the take solo — #411's deliberate rule, matching Logic
/// and Pro Tools. The gesture that triggers it is the one this todo wires,
/// so the round-trip is pinned here.
#[test]
fn a_split_under_a_solo_ends_the_solo() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    set_active(&mut app, Some(0));
    let cut = at(&app, 2);
    seek(&mut app, cut);
    let mut state = TimelineState::default();

    let msg = press(
        &app,
        &mut state,
        x_of(&app, at(&app, 1)),
        ribbon_y(&app, AUDIO_TRACK),
    )
    .expect("publishes");
    app.test_dispatch(msg);

    assert_eq!(
        group_of(&app).active_take,
        None,
        "editing the comp means 'play the comp'"
    );
    assert_eq!(comp(&app).len(), 2, "and the cut landed");
}

// ---------------------------------------------------------------------
// Delete a take
// ---------------------------------------------------------------------

/// Right-click deletes a take — the codebase's delete convention — and the
/// slot is re-covered from what remains, because a hole in the comp is
/// silence in the middle of the part.
#[test]
fn right_clicking_a_take_card_deletes_it_and_re_covers_the_slot() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);
    let mut state = TimelineState::default();

    let msg = app
        .test_timeline_canvas_event(
            &mut state,
            &right_press(),
            x_of(&app, at(&app, 2)),
            take_card_y(&app, 2),
        )
        .expect("right-press publishes");
    assert!(
        matches!(
            msg,
            Message::Take(TakeMessage::DeleteTake {
                group_id: GROUP,
                take_id: 2
            })
        ),
        "got {msg:?}"
    );

    app.test_dispatch(msg);
    assert_eq!(
        group_of(&app).takes.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![0, 1]
    );
    let cover = effective_cover(group_of(&app));
    assert!(
        cover.iter().all(|s| s.take_id != 2),
        "nothing still points at the deleted take"
    );
    assert_eq!(
        cover.last().map(|s| s.range.end()),
        Some(slot(&app).end()),
        "the cover still reaches the end of the slot"
    );
}

// ---------------------------------------------------------------------
// Affordances for the states that are otherwise invisible
// ---------------------------------------------------------------------

/// A MIDI take soloed on a group that also holds audio silences the audio
/// path — by design, and indistinguishable from a bug unless the UI says
/// so. #413 could not use this predicate (it branched before #411); the
/// affordance pass does.
#[test]
fn a_midi_take_soloed_over_audio_reports_that_it_silences_the_audio() {
    let (mut app, dir) = build_app();
    let slot = slot(&app);
    write_take_recording(&app, &dir, 0, slot);
    capture_audio_pass(&mut app, 0);
    capture_midi_pass(&mut app, 1);
    // Both takes landed in one group even though the second is MIDI:
    // an armed instrument track records both paths.
    assert_eq!(group_of(&app).takes.len(), 2);

    assert!(!app.test_active_take_silences_audio(GROUP));
    set_active(&mut app, Some(1));
    assert!(
        app.test_active_take_silences_audio(GROUP),
        "the MIDI solo mutes the group's audio takes"
    );
    set_active(&mut app, Some(0));
    assert!(
        !app.test_active_take_silences_audio(GROUP),
        "soloing the audio take back restores it"
    );
}

/// The take lane's cursor vocabulary, all three shapes. Every gesture
/// this todo adds has to announce itself before the press — a comp edit
/// that turns out to be impossible is dropped without a trace, so the
/// pointer is the only channel left for "this will do nothing".
#[test]
fn the_take_lane_announces_its_gestures_through_the_cursor() {
    use iced::mouse::Interaction;
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    toggle_lane(&mut app, AUDIO_TRACK);
    let cut = at(&app, 2);
    seek(&mut app, cut);
    let state = TimelineState::default();
    let x = x_of(&app, at(&app, 2));

    assert_eq!(
        app.test_timeline_cursor(&state, x, take_card_y(&app, 0)),
        Interaction::Grab,
        "a take card is grabbable: click solos, drag promotes"
    );
    assert_eq!(
        app.test_timeline_cursor(&state, x, ribbon_y(&app, AUDIO_TRACK)),
        Interaction::ResizingHorizontally,
        "the ribbon offers the cut"
    );

    // The inert row chrome above a card keeps the default cursor, so the
    // affordance is exactly as wide as the hit region.
    let layout = app.test_arrange_row_layout();
    let (top, _) = layout.take_row_rect(AUDIO_TRACK, GROUP, 0).expect("row");
    let chrome_y = app.test_arrange_header_offset() + top + 1.0;
    assert_eq!(
        app.test_timeline_cursor(&state, x, chrome_y),
        Interaction::default(),
    );
}

// ---------------------------------------------------------------------
// The press order — what the take lane does NOT take
// ---------------------------------------------------------------------
//
// This todo moved the take-lane hit test ahead of the automation
// breakpoint dots *and* the clips in both `handle_press` and
// `hover_interaction`, because that is the draw order: the comp ribbon is
// painted after both, overlaps the bottom of the clip body
// (`CLIP_LANE_INSET` 10 px vs the ribbon starting `TAKE_COMP_RIBBON_HEIGHT
// + 3` px above the row's bottom edge) and reaches into the pick radius of
// a breakpoint pinned at value 0.
//
// Every *other* clip and automation input test in the suite runs with an
// empty `take_groups`, so none of them reach the new branches at all — the
// reorder is invisible to them. These cases exist so that a change to what
// the ribbon claims is a visible decision rather than an accident.

/// A track carrying all three surfaces at once over the same stretch of
/// timeline: an audio clip, an automation lane whose first point sits at
/// value **0.0** (the lowest a dot can go, i.e. the closest one can get to
/// the ribbon), and a take group. Only here do the three compete.
fn build_crowded_lane() -> (Resonance, TempDir) {
    let (mut app, dir) = build_app();
    let slot = slot(&app);
    // A clip spanning exactly the slot, so clip and ribbon share every x
    // under test and only the y can decide between them.
    app.test_push_clip(ClipState {
        id: CROWDED_CLIP,
        track_id: AUDIO_TRACK,
        start_sample: slot.start,
        duration_samples: slot.length,
        name: "crowded".to_string(),
        total_frames: slot.length,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: vec![(-0.5, 0.5); 64],
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    });
    // Two gain breakpoints at value 0: one inside the slot (where the take
    // lane now competes) and one well past its end (where it must not).
    for frame in [at(&app, 2), slot.end() + 3 * app.sample_rate as u64] {
        let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
            target: AutomationTarget::TrackGain(AUDIO_TRACK),
            time_frames: frame,
            value: 0.0,
            curve: CurveKind::Linear,
        }));
    }
    capture_audio_passes(&mut app, &dir, 2);
    toggle_lane(&mut app, AUDIO_TRACK);
    let cut = at(&app, 2);
    seek(&mut app, cut);
    (app, dir)
}

const CROWDED_CLIP: u64 = 9_100;

/// Canvas-space `(top, height)` of the comp ribbon band on `AUDIO_TRACK`.
fn ribbon_band(app: &Resonance) -> (f32, f32) {
    let layout = app.test_arrange_row_layout();
    let (top, h) = layout.track_row_rect(AUDIO_TRACK).expect("track row");
    resonance_app::view::timeline::takes::comp_ribbon_band(
        app.test_arrange_header_offset() + top,
        h,
    )
}

/// Canvas-space y of a breakpoint dot at `value` in `AUDIO_TRACK`'s
/// in-track overlay band.
fn overlay_dot_y(app: &Resonance, value: f32) -> f32 {
    let layout = app.test_arrange_row_layout();
    let (top, h) = layout.track_row_rect(AUDIO_TRACK).expect("track row");
    let (band_top, band_height) =
        automation_band(app.test_arrange_header_offset() + top, h);
    value_to_y(value, band_top, band_height)
}

/// The clip body above the ribbon is untouched: same x, a few pixels
/// higher, and the press is a clip drag exactly as it was before the
/// reorder.
#[test]
fn a_clip_press_above_the_ribbon_still_starts_a_clip_drag() {
    let (app, _dir) = build_crowded_lane();
    let mut state = TimelineState::default();
    let x = x_of(&app, at(&app, 3));
    let (band_top, _) = ribbon_band(&app);

    let msg = press(&app, &mut state, x, band_top - 2.0).expect("publishes");
    assert!(
        matches!(
            msg,
            Message::Clip(ClipMessage::StartClipDrag {
                clip_id: CROWDED_CLIP,
                ..
            })
        ),
        "two pixels above the ribbon is still the clip, got {msg:?}"
    );
    // ...and two pixels lower is the ribbon.
    let mut state = TimelineState::default();
    let msg = press(&app, &mut state, x, band_top + 2.0).expect("publishes");
    assert!(
        matches!(
            msg,
            Message::Take(TakeMessage::SplitCompAtPlayhead { group_id: GROUP })
        ),
        "got {msg:?}"
    );
}

/// The exact band the ribbon claims from the clip, pinned.
///
/// The clip body runs `CLIP_LANE_INSET` in from both row edges; the ribbon
/// starts `TAKE_COMP_RIBBON_HEIGHT + 3` above the bottom one. Their overlap
/// is the whole cost of the reorder, and it is asserted to be exactly the
/// band the ribbon is *drawn* over — no pick-radius slack, no rounding.
/// Widening it later will fail here, which is the point.
#[test]
fn the_ribbon_claims_exactly_the_band_it_draws() {
    let (app, _dir) = build_crowded_lane();
    let x = x_of(&app, at(&app, 3));
    let (band_top, band_height) = ribbon_band(&app);
    let layout = app.test_arrange_row_layout();
    let (row_top, row_h) = layout.track_row_rect(AUDIO_TRACK).expect("track row");
    let row_y = app.test_arrange_header_offset() + row_top;
    let clip_top = row_y + theme::CLIP_LANE_INSET;
    let clip_bottom = row_y + row_h - theme::CLIP_LANE_INSET;

    // Walk the clip's whole vertical extent and record where the verb
    // flips. Half-pixel steps so an off-by-one boundary cannot hide.
    let mut first_take_y: Option<f32> = None;
    let mut y = clip_top;
    while y <= clip_bottom {
        let mut state = TimelineState::default();
        match press(&app, &mut state, x, y) {
            Some(Message::Take(_)) => {
                first_take_y.get_or_insert(y);
            }
            Some(Message::Clip(_)) => assert!(
                first_take_y.is_none(),
                "the clip must not come back below the ribbon's top edge (y {y})"
            ),
            other => panic!("unexpected verb at y {y}: {other:?}"),
        }
        y += 0.5;
    }

    let boundary = first_take_y.expect("the ribbon claims some of the clip body");
    assert!(
        (boundary - band_top).abs() <= 0.5,
        "the ribbon claims from its own drawn top edge {band_top}, not {boundary}"
    );
    assert!(
        band_top + band_height > clip_bottom,
        "the claimed band runs to the bottom of the clip body and past it"
    );
}

/// A breakpoint pinned at value 0 — the lowest a dot can sit — stays
/// reachable over the top of its pick column, and the part of that column
/// the ribbon claims is claimed *only* inside the slot, and *only* when the
/// track has take groups at all.
#[test]
fn a_value_zero_breakpoint_survives_the_reorder() {
    let (app, _dir) = build_crowded_lane();
    let dot_y = overlay_dot_y(&app, 0.0);
    let (band_top, _) = ribbon_band(&app);
    let dot_x = x_of(&app, at(&app, 2));
    let gain = AutomationTarget::TrackGain(AUDIO_TRACK);

    // The dot itself sits above the ribbon and is hit as it always was.
    assert!(dot_y < band_top, "the dot is drawn above the ribbon band");
    let mut state = TimelineState::default();
    let msg = press(&app, &mut state, dot_x, dot_y).expect("publishes");
    assert!(
        matches!(
            &msg,
            Message::Automation(AutomationMessage::StartBreakpointDrag { target, .. })
                if *target == gain
        ),
        "got {msg:?}"
    );

    // The lower reach of its pick radius is inside the ribbon and now goes
    // to the split. Asserting the *same* press on an otherwise identical
    // app with no take group proves the region was genuinely taken, rather
    // than never having belonged to the dot.
    let stolen_y = band_top + 2.0;
    assert!(
        stolen_y < dot_y + BREAKPOINT_HIT_RADIUS,
        "the point under test is inside the dot's pick radius"
    );
    let mut state = TimelineState::default();
    let msg = press(&app, &mut state, dot_x, stolen_y).expect("publishes");
    assert!(
        matches!(msg, Message::Take(TakeMessage::SplitCompAtPlayhead { .. })),
        "inside the slot the ribbon takes it, got {msg:?}"
    );

    let (no_takes, _no_takes_dir) = build_app_with_only_the_breakpoints();
    let mut state = TimelineState::default();
    let msg = press(&no_takes, &mut state, dot_x, stolen_y).expect("publishes");
    assert!(
        matches!(
            &msg,
            Message::Automation(AutomationMessage::StartBreakpointDrag { target, .. })
                if *target == gain
        ),
        "with no take group the dot keeps its whole pick radius, got {msg:?}"
    );
}

/// ...and the claim is x-scoped to the slot: the second dot, three seconds
/// past the slot's end, keeps its full pick radius on a track that *does*
/// carry a take lane.
#[test]
fn a_breakpoint_outside_the_slot_keeps_its_whole_pick_radius() {
    let (app, _dir) = build_crowded_lane();
    let (band_top, _) = ribbon_band(&app);
    let outside_x = x_of(&app, slot(&app).end() + 3 * app.sample_rate as u64);
    let mut state = TimelineState::default();

    assert_eq!(
        app.test_comp_ribbon_at(outside_x, band_top + 2.0),
        None,
        "the lane carries no comp out here"
    );
    let msg = press(&app, &mut state, outside_x, band_top + 2.0).expect("publishes");
    assert!(
        matches!(
            &msg,
            Message::Automation(AutomationMessage::StartBreakpointDrag { target, .. })
                if *target == AutomationTarget::TrackGain(AUDIO_TRACK)
        ),
        "got {msg:?}"
    );
}

/// The same crowded lane through `hover_interaction`, which was re-ordered
/// to match: the cursor must promise exactly the verb the press performs,
/// or the affordance lies.
#[test]
fn the_cursor_agrees_with_the_reordered_press() {
    use iced::mouse::Interaction;
    let (app, _dir) = build_crowded_lane();
    let state = TimelineState::default();
    let x = x_of(&app, at(&app, 3));
    let (band_top, _) = ribbon_band(&app);

    assert_eq!(
        app.test_timeline_cursor(&state, x, band_top - 2.0),
        Interaction::Grab,
        "clip body: grab, as before the reorder"
    );
    assert_eq!(
        app.test_timeline_cursor(&state, x, band_top + 2.0),
        Interaction::ResizingHorizontally,
        "ribbon: the split, matching the press"
    );
    assert_eq!(
        app.test_timeline_cursor(&state, x_of(&app, at(&app, 2)), overlay_dot_y(&app, 0.0)),
        Interaction::Grab,
        "the value-0 dot still hovers as grabbable"
    );

    // The contended point, and the only one here that orders take-lane
    // hover against `breakpoint_hit`: inside the value-0 dot's pick
    // radius *and* inside the ribbon band. None of the three above
    // contend — the dot probe sits 2 px above the band, and both ribbon
    // probes are ~100 px along the x axis from the nearest dot — so
    // moving take-lane hover below the dots used to leave the whole
    // suite green. `a_value_zero_breakpoint_survives_the_reorder` pins
    // the *press* at this exact point as the split; if the cursor
    // disagreed it would show `Grab` over a click that splits the comp,
    // which is the affordance lie this test exists to prevent.
    let dot_x = x_of(&app, at(&app, 2));
    let stolen_y = band_top + 2.0;
    assert!(
        stolen_y < overlay_dot_y(&app, 0.0) + BREAKPOINT_HIT_RADIUS,
        "precondition: the point is inside the dot's pick radius"
    );
    assert_eq!(
        app.test_timeline_cursor(&state, dot_x, stolen_y),
        Interaction::ResizingHorizontally,
        "where the press splits the comp, the cursor must promise the split"
    );
}

/// The crowded fixture minus the takes: same clip, same two breakpoints,
/// no take group. Stands in for "the app before this todo" so the stolen
/// band can be shown to be a *change* rather than a coincidence.
fn build_app_with_only_the_breakpoints() -> (Resonance, TempDir) {
    let (mut app, dir) = build_app();
    let slot = slot(&app);
    app.test_push_clip(ClipState {
        id: CROWDED_CLIP,
        track_id: AUDIO_TRACK,
        start_sample: slot.start,
        duration_samples: slot.length,
        name: "crowded".to_string(),
        total_frames: slot.length,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: vec![(-0.5, 0.5); 64],
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    });
    for frame in [at(&app, 2), slot.end() + 3 * app.sample_rate as u64] {
        let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
            target: AutomationTarget::TrackGain(AUDIO_TRACK),
            time_frames: frame,
            value: 0.0,
            curve: CurveKind::Linear,
        }));
    }
    assert!(app.test_take_groups().is_empty());
    (app, dir)
}

// ---------------------------------------------------------------------
// Golden snapshots
// ---------------------------------------------------------------------

/// The window-space offset the goldens aim through. Asserted rather than
/// assumed: this maps a canvas-space point onto the laid-out widget tree,
/// and a shift in the page chrome would otherwise silently re-aim every
/// simulated gesture at the wrong pixel while the goldens still "passed"
/// by rendering nothing.
#[test]
fn the_canvas_origin_constants_still_hold() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 2);
    let y = ribbon_y(&app, AUDIO_TRACK);
    let x = x_of(&app, at(&app, 2));

    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.point_at(Point::new(x + CANVAS_ORIGIN.0, y + CANVAS_ORIGIN.1));
    let _ = ui.simulate(iced_test::simulator::click());
    let msgs: Vec<Message> = ui.into_messages().collect();
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            Message::Take(TakeMessage::SplitCompAtPlayhead { group_id: GROUP })
        )),
        "a window-space click at the comp ribbon must reach the canvas; got {msgs:?}"
    );
}

/// Snapshot the app as-is, with the cursor parked at a canvas-space point
/// so the hover affordances render.
fn snapshot_hovering(app: &Resonance, at: (f32, f32), path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.point_at(Point::new(at.0 + CANVAS_ORIGIN.0, at.1 + CANVAS_ORIGIN.1));
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

fn snapshot(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// **Promote drag, mid-flight.** Three takes over 2 s..7 s; T1's card is
/// being swept from 3 s to 5.5 s. The golden pins the preview band on T1's
/// row — accent wash, 2 px top rule, both edges — and the caption naming
/// the take and the range, which is the only thing that tells the user
/// what the release will do (the edit itself reports nothing).
#[test]
fn promote_drag_snapshot() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);

    let y = take_card_y(&app, 0);
    let (from, to) = (x_of(&app, at(&app, 1)), x_of(&app, at(&app, 3)) + 50.0);
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.point_at(Point::new(from + CANVAS_ORIGIN.0, y + CANVAS_ORIGIN.1));
    let _ = ui.simulate([left_press()]);
    ui.point_at(Point::new(to + CANVAS_ORIGIN.0, y + CANVAS_ORIGIN.1));
    let _ = ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: Point::new(to + CANVAS_ORIGIN.0, y + CANVAS_ORIGIN.1),
    })]);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/take_lane_promote_drag.png");
}

/// **The comp that drag lands.** The same three takes after promoting T1
/// over 1 s..3 s of the slot: the ribbon reads T3 / T1 / T3, T1's card is
/// lit only over the middle, and the two fallback spans wear the quieter
/// hairline treatment. Distinct from #413's `expanded_stack` golden, whose
/// comp was injected as an engine echo — this one is what a gesture built.
#[test]
fn promoted_comp_snapshot() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    toggle_lane(&mut app, AUDIO_TRACK);
    let mut state = TimelineState::default();
    let msg = sweep(
        &app,
        &mut state,
        take_card_y(&app, 0),
        x_of(&app, at(&app, 1)),
        x_of(&app, at(&app, 3)),
    )
    .expect("release");
    app.test_dispatch(msg);
    assert_eq!(comp(&app).len(), 3, "T3 · T1 · T3");
    snapshot(&app, "tests/snapshots/take_lane_promoted_comp.png");
}

/// **Split under a solo.** T1 is soloed and the playhead sits at 4 s,
/// inside the slot. Hovering the comp ribbon draws the warm cut tick at
/// the playhead and the caption that carries the one thing nothing else in
/// the UI says: the split ends the solo.
///
/// It used to carry a second, `BAD` line — "the take you hear may change
/// with it" — because the app materialized the *fallback* take's cover
/// rather than the soloed one's. Todo #1395 removed that divergence
/// (`update::takes` now starts from `TakeGroup::effective_comp`, which
/// honours the active take), so the warning would now be false and the
/// golden is re-blessed one line shorter.
#[test]
fn split_under_a_solo_snapshot() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    set_active(&mut app, Some(0));
    let cut = at(&app, 2);
    seek(&mut app, cut);
    toggle_lane(&mut app, AUDIO_TRACK);

    let y = ribbon_y(&app, AUDIO_TRACK);
    let x = x_of(&app, at(&app, 1));
    assert_eq!(app.test_comp_ribbon_at(x, y), Some(GROUP));
    snapshot_hovering(&app, (x, y), "tests/snapshots/take_lane_split_under_solo.png");
}

/// **The refused split.** Same lane, playhead parked past the end of the
/// slot. There is no cut point, the edit would be dropped without a trace,
/// so the ribbon says so before the click rather than after it. Pins the
/// `BAD` caption — the one affordance standing in for a refusal that is
/// silent by design.
#[test]
fn refused_split_snapshot() {
    let (mut app, dir) = build_app();
    capture_audio_passes(&mut app, &dir, 3);
    let past_end = slot(&app).end() + 2 * app.sample_rate as u64;
    seek(&mut app, past_end);
    toggle_lane(&mut app, AUDIO_TRACK);

    let y = ribbon_y(&app, AUDIO_TRACK);
    let x = x_of(&app, at(&app, 3));
    assert!(!app.test_take_lane_expanded(MIDI_TRACK));
    snapshot_hovering(&app, (x, y), "tests/snapshots/take_lane_refused_split.png");
}

/// **A MIDI take soloed over audio takes.** An armed instrument track
/// records both paths, so one group can hold an audio pass and a MIDI
/// pass. Soloing the MIDI one resolves to zero audio spans while the
/// group's audio takes stay governed — the lane goes silent on the audio
/// path, by design (ba doc #292) and indistinguishable from a bug unless
/// something says so.
///
/// Hovering the soloed card is the only place `midi_solo_note` and
/// `draw_take_card_hint` are depicted: the caption's first line is the
/// card's verb list in `TEXT_2`, the second is the `BAD` consequence.
/// Without this golden the wording and its contrast are unreviewable —
/// which is exactly how a caption ends up unreadable over the lane.
#[test]
fn midi_solo_silences_audio_snapshot() {
    let (mut app, dir) = build_app();
    let slot = slot(&app);
    write_take_recording(&app, &dir, 0, slot);
    capture_audio_pass(&mut app, 0);
    capture_midi_pass(&mut app, 1);
    set_active(&mut app, Some(1));
    toggle_lane(&mut app, AUDIO_TRACK);
    assert!(
        app.test_active_take_silences_audio(GROUP),
        "the state under test — a MIDI solo muting the group's audio"
    );

    // Hover T2's own card: `soloed` is true there, which is what reaches
    // the MIDI note.
    let y = take_card_y(&app, 1);
    let x = x_of(&app, at(&app, 1));
    assert_eq!(
        app.test_take_card_at(x, y).map(|h| h.1),
        Some(1),
        "the cursor really is over the soloed MIDI take's card"
    );
    snapshot_hovering(&app, (x, y), "tests/snapshots/take_lane_midi_solo_hint.png");
}

/// **Promoting across silence.** Pass 0 punched in a second into the slot,
/// so its card starts at 3 s; the drag runs from the slot start at 2 s to
/// 6 s. The 2 s..3 s lead-in is inside the promoted range and outside the
/// take's audio, which is legal — `update::takes` keeps only what the take
/// can fill — but a segment over a stretch its take never recorded is a
/// real state the engine renders as silence.
///
/// The golden pins the disclosure: `BAD` centre-line rules over the silent
/// part of the preview band, and the `BAD` caption line under the promote
/// range. The preview band itself still spans the whole drag, because the
/// gesture really does address the whole slot — the silence marking is a
/// warning, not a clamp, and the two must not be conflated.
#[test]
fn promote_across_silence_snapshot() {
    let (mut app, dir) = build_app();
    let sr = app.sample_rate as u64;
    let slot = slot(&app);
    let punched = TimelineRange::from_bounds(slot.start + sr, slot.end());
    write_take_recording(&app, &dir, 0, punched);
    capture_audio_pass_over(&mut app, 0, punched);
    write_take_recording(&app, &dir, 1, slot);
    capture_audio_pass(&mut app, 1);
    toggle_lane(&mut app, AUDIO_TRACK);

    let y = take_card_y(&app, 0);
    let (from, to) = (x_of(&app, slot.start), x_of(&app, at(&app, 4)));
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.point_at(Point::new(from + CANVAS_ORIGIN.0, y + CANVAS_ORIGIN.1));
    let _ = ui.simulate([left_press()]);
    ui.point_at(Point::new(to + CANVAS_ORIGIN.0, y + CANVAS_ORIGIN.1));
    let _ = ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: Point::new(to + CANVAS_ORIGIN.0, y + CANVAS_ORIGIN.1),
    })]);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/take_lane_promote_across_silence.png");
}

fn drain(
    rx: &resonance_audio::test_support::Receiver<resonance_audio::types::AudioCommand>,
) -> Vec<resonance_audio::types::AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}
