//! Clip warp ("follow tempo") in the GUI: the clip inspector's WARP
//! section, the warp-marker gestures on the timeline canvas, the engine
//! echo / tempo-detection round-trip, and undo.
//!
//! Every edit is driven through the real reducers against a capturing
//! engine, so each test asserts both the `ClipState::warp` mirror and the
//! exact `SetClipWarp` / `SetClipWarpMarkers` / `DetectClipTempo` the app
//! emits. Canvas gestures go through `canvas::Program::update`
//! (`test_timeline_canvas_event`), the code the live pointer runs.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ClipMessage, ClipWarpMessage, Message};
use resonance_app::state::{ClipState, ClipWarpState, TempoDetectStatus, ViewMode};
use resonance_app::view::timeline::TimelineState;
use resonance_app::{theme, Resonance};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    AudioCommand, AudioEvent, FadeCurve, TrackType, WarpAlgorithm, WarpMarker,
};

const SR: u32 = 48_000;
/// px per second: at the default 120 BPM one beat is 0.5 s = 50 px.
const ZOOM: f32 = 100.0;
const TRACK: u64 = 1;
const CLIP: u64 = 7;
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// A 4-second (8-beat, 400 px) audio clip at the timeline origin.
fn clip(warp: ClipWarpState) -> ClipState {
    ClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 4 * SR as u64,
        name: "loop".to_string(),
        total_frames: 4 * SR as u64,
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
        warp,
    }
}

fn marker(source_frame: u64, timeline_beat: f64) -> WarpMarker {
    WarpMarker {
        source_frame,
        timeline_beat,
    }
}

/// Warp on, 100 BPM source, markers at beats 2 and 4.
fn warped() -> ClipWarpState {
    ClipWarpState {
        enabled: true,
        original_bpm: Some(100.0),
        markers: vec![marker(57_600, 2.0), marker(115_200, 4.0)],
        ..ClipWarpState::default()
    }
}

/// Arrange-view app with an undoable project, one audio track holding the
/// clip, and a capturing engine.
fn app_with(warp: ClipWarpState) -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/clip-warp-ui.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_clip(clip(warp));
    let _ = drain(&rx);
    (app, rx)
}

/// The commands sent since the last drain, minus the `PersistClipWavs`
/// every recorded undo snapshot sends first.
fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if !matches!(cmd, AudioCommand::PersistClipWavs) {
            cmds.push(cmd);
        }
    }
    cmds
}

fn warp_of(app: &Resonance) -> ClipWarpState {
    app.test_clips()
        .iter()
        .find(|c| c.id == CLIP)
        .expect("clip")
        .warp
        .clone()
}

fn entries(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

fn warp_msg(m: ClipWarpMessage) -> Message {
    Message::Clip(ClipMessage::Warp(m))
}

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

/// The messages one click on `label` in the rendered app produces.
fn click_messages(app: &Resonance, label: &str) -> Vec<Message> {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.click(label)
        .unwrap_or_else(|_| panic!("clicking {label:?} should hit a control"));
    ui.into_messages().collect()
}

/// Canvas y of the middle of the clip's marker strip (its bottom edge).
fn strip_y(app: &Resonance) -> f32 {
    let layout = app.test_arrange_row_layout();
    let (y_top, height) = layout.track_row_rect(TRACK).expect("track row");
    let body_bottom = app.test_arrange_header_offset() + y_top + height - theme::CLIP_LANE_INSET;
    body_bottom - theme::WARP_MARKER_STRIP_HEIGHT / 2.0
}

fn left_press() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
}

fn left_release() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))
}

fn right_press() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Right))
}

fn cursor_moved() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: iced::Point::ORIGIN,
    })
}

// ---------------------------------------------------------------------
// Inspector
// ---------------------------------------------------------------------

#[test]
fn inspector_shows_the_warp_section_for_a_selected_clip() {
    let (mut app, _rx) = app_with(ClipWarpState::default());
    app.test_set_selected_clip(Some(CLIP));
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    for label in ["WARP", "Warp on", "Tempo", "Detect", "Transient", "Tonal", "Transpose", "Markers"] {
        ui.find(label)
            .unwrap_or_else(|_| panic!("the warp section shows {label:?}"));
    }
}

#[test]
fn warp_toggle_emits_set_clip_warp_and_records_one_entry() {
    let (mut app, rx) = app_with(ClipWarpState::default());
    app.test_set_selected_clip(Some(CLIP));

    let msgs = click_messages(&app, "Warp on");
    assert_eq!(msgs.len(), 1, "one click, one edit: {msgs:?}");
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(warp_of(&app).enabled);
    assert_eq!(entries(&app), 1, "the toggle is one undo entry");
    match drain(&rx).as_slice() {
        [AudioCommand::SetClipWarp {
            clip_id: CLIP,
            warp_enabled: true,
            original_bpm: None,
            transpose_semitones,
            warp_algorithm: WarpAlgorithm::Transient,
        }] => assert_eq!(*transpose_semitones, 0.0),
        other => panic!("expected one SetClipWarp, got {other:?}"),
    }
}

#[test]
fn algorithm_and_transpose_controls_keep_the_other_fields() {
    let (mut app, rx) = app_with(warped());
    app.test_set_selected_clip(Some(CLIP));

    for m in click_messages(&app, "Tonal") {
        let _ = app.update(m);
    }
    for m in click_messages(&app, "+1") {
        let _ = app.update(m);
    }
    let w = warp_of(&app);
    assert_eq!(w.algorithm, WarpAlgorithm::Tonal);
    assert_eq!(w.transpose_semitones, 1.0);
    assert!(w.enabled, "warp stays on");
    assert_eq!(w.original_bpm, Some(100.0), "the tempo is untouched");
    assert_eq!(w.markers.len(), 2, "the markers are untouched");
    let sent = drain(&rx);
    assert!(
        matches!(
            sent.last(),
            Some(AudioCommand::SetClipWarp {
                warp_algorithm: WarpAlgorithm::Tonal,
                transpose_semitones,
                original_bpm: Some(_),
                ..
            }) if *transpose_semitones == 1.0
        ),
        "{sent:?}"
    );
    assert_eq!(entries(&app), 2);
}

#[test]
fn tempo_field_commits_on_enter_and_ignores_junk() {
    let (mut app, rx) = app_with(ClipWarpState::default());

    // Typing records nothing and sends nothing.
    let _ = app.update(warp_msg(ClipWarpMessage::BpmDraftChanged {
        clip_id: CLIP,
        text: "123.5".into(),
    }));
    assert!(drain(&rx).is_empty());
    assert_eq!(entries(&app), 0);
    assert_eq!(warp_of(&app).original_bpm, None, "a draft is not an edit");

    let _ = app.update(warp_msg(ClipWarpMessage::CommitBpmDraft { clip_id: CLIP }));
    assert_eq!(warp_of(&app).original_bpm, Some(123.5));
    assert_eq!(entries(&app), 1, "Enter is one undo entry");
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::SetClipWarp {
            original_bpm: Some(b),
            ..
        }] if *b == 123.5
    ));

    // Junk is dropped; Enter on an unchanged value is no edit.
    for text in ["abc", "123.5"] {
        let _ = app.update(warp_msg(ClipWarpMessage::BpmDraftChanged {
            clip_id: CLIP,
            text: text.into(),
        }));
        let _ = app.update(warp_msg(ClipWarpMessage::CommitBpmDraft { clip_id: CLIP }));
    }
    assert_eq!(warp_of(&app).original_bpm, Some(123.5));
    assert!(drain(&rx).is_empty());
    assert_eq!(entries(&app), 1);

    // An empty field clears the tempo; an out-of-range one is clamped.
    let _ = app.update(warp_msg(ClipWarpMessage::BpmDraftChanged {
        clip_id: CLIP,
        text: "  ".into(),
    }));
    let _ = app.update(warp_msg(ClipWarpMessage::CommitBpmDraft { clip_id: CLIP }));
    assert_eq!(warp_of(&app).original_bpm, None);
    let _ = app.update(warp_msg(ClipWarpMessage::BpmDraftChanged {
        clip_id: CLIP,
        text: "5".into(),
    }));
    let _ = app.update(warp_msg(ClipWarpMessage::CommitBpmDraft { clip_id: CLIP }));
    assert_eq!(warp_of(&app).original_bpm, Some(resonance_app::state::MIN_WARP_BPM));
}

#[test]
fn detect_tempo_round_trip_offers_the_result() {
    let (mut app, rx) = app_with(ClipWarpState::default());
    app.test_set_selected_clip(Some(CLIP));

    let msgs = click_messages(&app, "Detect");
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::DetectClipTempo { clip_id: CLIP }]
    ));
    assert_eq!(entries(&app), 0, "asking for a detection is not an edit");
    assert_eq!(
        app.test_tempo_detect_status(CLIP),
        Some(TempoDetectStatus::Running)
    );
    {
        let mut ui =
            Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
        ui.find("Detecting…").expect("the button says it is busy");
    }

    app.test_apply_engine_event(AudioEvent::ClipTempoDetected {
        clip_id: CLIP,
        bpm: 97.5,
        confidence: 0.82,
    });
    assert_eq!(
        app.test_tempo_detect_status(CLIP),
        Some(TempoDetectStatus::Detected {
            bpm: 97.5,
            confidence: 0.82
        })
    );
    assert_eq!(warp_of(&app).original_bpm, None, "a result is not applied by itself");

    // "Use" applies it as one edit.
    let msgs = click_messages(&app, "Use");
    for m in msgs {
        let _ = app.update(m);
    }
    assert_eq!(warp_of(&app).original_bpm, Some(97.5));
    assert_eq!(entries(&app), 1);
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::SetClipWarp { original_bpm: Some(b), .. }] if *b == 97.5
    ));
}

#[test]
fn detect_tempo_reports_no_tempo() {
    let (mut app, _rx) = app_with(ClipWarpState::default());
    app.test_set_selected_clip(Some(CLIP));
    let _ = app.update(warp_msg(ClipWarpMessage::DetectTempo { clip_id: CLIP }));
    app.test_apply_engine_event(AudioEvent::ClipTempoDetected {
        clip_id: CLIP,
        bpm: 0.0,
        confidence: 0.0,
    });
    assert_eq!(
        app.test_tempo_detect_status(CLIP),
        Some(TempoDetectStatus::NotFound)
    );
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.find("No steady tempo found").expect("the failure is shown");
    assert!(ui.find("Use").is_err(), "nothing to apply");
}

#[test]
fn clear_markers_button_empties_the_set() {
    let (mut app, rx) = app_with(warped());
    app.test_set_selected_clip(Some(CLIP));
    for m in click_messages(&app, "Clear") {
        let _ = app.update(m);
    }
    assert!(warp_of(&app).markers.is_empty());
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::SetClipWarpMarkers { clip_id: CLIP, markers }] if markers.is_empty()
    ));
}

// ---------------------------------------------------------------------
// Engine echoes
// ---------------------------------------------------------------------

#[test]
fn warp_echoes_update_the_mirror() {
    let (mut app, _rx) = app_with(ClipWarpState::default());
    app.test_apply_engine_event(AudioEvent::ClipWarpChanged {
        clip_id: CLIP,
        warp_enabled: true,
        original_bpm: Some(90.0),
        transpose_semitones: -2.0,
        warp_algorithm: WarpAlgorithm::Tonal,
    });
    app.test_apply_engine_event(AudioEvent::ClipWarpMarkersChanged {
        clip_id: CLIP,
        markers: vec![marker(0, 0.0), marker(48_000, 1.5)],
    });
    let w = warp_of(&app);
    assert!(w.enabled);
    assert_eq!(w.original_bpm, Some(90.0));
    assert_eq!(w.transpose_semitones, -2.0);
    assert_eq!(w.algorithm, WarpAlgorithm::Tonal);
    assert_eq!(w.markers, vec![marker(0, 0.0), marker(48_000, 1.5)]);
}

// ---------------------------------------------------------------------
// Marker gestures
// ---------------------------------------------------------------------

#[test]
fn marker_drag_is_one_undo_entry_and_sends_once_on_release() {
    let (mut app, rx) = app_with(warped());

    let _ = app.update(warp_msg(ClipWarpMessage::StartMarkerDrag {
        clip_id: CLIP,
        index: 0,
    }));
    // Beat 3 = 1.5 s = 150 px.
    for x in [110.0, 130.0, 150.0] {
        let _ = app.update(warp_msg(ClipWarpMessage::UpdateMarkerDrag(x)));
    }
    assert!(drain(&rx).is_empty(), "nothing is sent mid-drag");
    let m = &warp_of(&app).markers[0];
    assert!((m.timeline_beat - 3.0).abs() < 1e-9, "beat follows the pointer: {m:?}");
    assert_eq!(m.source_frame, 57_600, "a dragged marker keeps its source frame");

    let _ = app.update(warp_msg(ClipWarpMessage::EndMarkerDrag));
    assert_eq!(entries(&app), 1, "the whole drag is one undo entry");
    match drain(&rx).as_slice() {
        [AudioCommand::SetClipWarpMarkers { clip_id: CLIP, markers }] => {
            assert!((markers[0].timeline_beat - 3.0).abs() < 1e-9)
        }
        other => panic!("expected one SetClipWarpMarkers, got {other:?}"),
    }
}

#[test]
fn marker_drag_is_clamped_between_its_neighbours() {
    let (mut app, _rx) = app_with(warped());
    let _ = app.update(warp_msg(ClipWarpMessage::StartMarkerDrag {
        clip_id: CLIP,
        index: 0,
    }));
    // Far past marker 1 (beat 4 = 200 px).
    let _ = app.update(warp_msg(ClipWarpMessage::UpdateMarkerDrag(380.0)));
    let w = warp_of(&app);
    assert!(w.markers[0].timeline_beat < w.markers[1].timeline_beat);
    assert!(
        (w.markers[0].timeline_beat - (4.0 - resonance_app::state::MIN_WARP_MARKER_GAP_BEATS))
            .abs()
            < 1e-9
    );
    // And not before the clip's start.
    let _ = app.update(warp_msg(ClipWarpMessage::UpdateMarkerDrag(-50.0)));
    assert_eq!(warp_of(&app).markers[0].timeline_beat, 0.0);
    let _ = app.update(warp_msg(ClipWarpMessage::EndMarkerDrag));
}

#[test]
fn a_marker_drag_that_ends_where_it_began_records_nothing() {
    let (mut app, _rx) = app_with(warped());
    let _ = app.update(warp_msg(ClipWarpMessage::StartMarkerDrag {
        clip_id: CLIP,
        index: 1,
    }));
    let _ = app.update(warp_msg(ClipWarpMessage::UpdateMarkerDrag(200.0)));
    let _ = app.update(warp_msg(ClipWarpMessage::EndMarkerDrag));
    assert_eq!(entries(&app), 0);
}

#[test]
fn canvas_press_on_a_marker_starts_its_drag() {
    let (mut app, _rx) = app_with(warped());
    let y = strip_y(&app);
    let mut state = TimelineState::default();
    // Marker 1 sits at beat 4 = 200 px.
    let msg = app
        .test_timeline_canvas_event(&mut state, &left_press(), 201.0, y)
        .expect("the press publishes");
    assert!(
        matches!(
            msg,
            Message::Clip(ClipMessage::Warp(ClipWarpMessage::StartMarkerDrag {
                clip_id: CLIP,
                index: 1
            }))
        ),
        "{msg:?}"
    );
    let _ = app.update(msg);
    let msg = app
        .test_timeline_canvas_event(&mut state, &cursor_moved(), 250.0, y)
        .expect("the move publishes");
    assert!(matches!(
        msg,
        Message::Clip(ClipMessage::Warp(ClipWarpMessage::UpdateMarkerDrag(x))) if x == 250.0
    ));
    let _ = app.update(msg);
    let msg = app
        .test_timeline_canvas_event(&mut state, &left_release(), 250.0, y)
        .expect("the release publishes");
    assert!(matches!(
        msg,
        Message::Clip(ClipMessage::Warp(ClipWarpMessage::EndMarkerDrag))
    ));
    let _ = app.update(msg);
    assert!((warp_of(&app).markers[1].timeline_beat - 5.0).abs() < 1e-9);
    assert_eq!(entries(&app), 1);
}

#[test]
fn double_click_on_the_strip_adds_a_marker_that_changes_nothing_audible() {
    let (mut app, _rx) = app_with(warped());
    let y = strip_y(&app);
    let mut state = TimelineState::default();
    // Beat 6 = 300 px, past the last marker: the new one extends the
    // last segment's rate, so it pins the frame that already plays there.
    let x = 300.0;
    let first = app.test_timeline_canvas_event(&mut state, &left_press(), x, y);
    assert!(
        !matches!(
            first,
            Some(Message::Clip(ClipMessage::Warp(ClipWarpMessage::SetWarpMarkers { .. })))
        ),
        "a single press does not add a marker: {first:?}"
    );
    if let Some(m) = first {
        let _ = app.update(m);
    }
    if let Some(m) = app.test_timeline_canvas_event(&mut state, &left_release(), x, y) {
        let _ = app.update(m);
    }
    let msg = app
        .test_timeline_canvas_event(&mut state, &left_press(), x, y)
        .expect("the double-click publishes");
    let Message::Clip(ClipMessage::Warp(ClipWarpMessage::SetWarpMarkers { clip_id, markers })) =
        &msg
    else {
        panic!("expected SetWarpMarkers, got {msg:?}");
    };
    assert_eq!(*clip_id, CLIP);
    assert_eq!(markers.len(), 3);
    assert_eq!(markers[2], marker(172_800, 6.0), "beat 6 plays frame 172 800");
    let _ = app.update(msg);
    assert_eq!(warp_of(&app).markers.len(), 3);
}

#[test]
fn right_click_on_a_marker_removes_it() {
    let (mut app, rx) = app_with(warped());
    let y = strip_y(&app);
    let mut state = TimelineState::default();
    let msg = app
        .test_timeline_canvas_event(&mut state, &right_press(), 100.0, y)
        .expect("the right-click publishes");
    let _ = app.update(msg);
    assert_eq!(warp_of(&app).markers, vec![marker(115_200, 4.0)]);
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::SetClipWarpMarkers { markers, .. }] if markers.len() == 1
    ));
    assert_eq!(entries(&app), 1);
}

#[test]
fn an_unwarped_clip_has_no_marker_strip() {
    let (app, _rx) = app_with(ClipWarpState {
        markers: vec![marker(0, 2.0)],
        ..ClipWarpState::default()
    });
    let y = strip_y(&app);
    let mut state = TimelineState::default();
    let msg = app.test_timeline_canvas_event(&mut state, &left_press(), 100.0, y);
    assert!(
        matches!(msg, Some(Message::Clip(ClipMessage::StartClipDrag { .. }))),
        "with warp off the bottom edge is clip body: {msg:?}"
    );
}

#[test]
fn the_cache_fingerprint_follows_warp_edits() {
    let (mut app, _rx) = app_with(ClipWarpState::default());
    let before = app.test_timeline_fingerprint();
    let _ = app.update(warp_msg(ClipWarpMessage::SetWarp {
        clip_id: CLIP,
        enabled: true,
        original_bpm: None,
        transpose_semitones: 0.0,
        algorithm: WarpAlgorithm::Transient,
    }));
    let on = app.test_timeline_fingerprint();
    assert_ne!(before, on, "the badge appears");
    let _ = app.update(warp_msg(ClipWarpMessage::SetWarpMarkers {
        clip_id: CLIP,
        markers: vec![marker(0, 1.0)],
    }));
    assert_ne!(on, app.test_timeline_fingerprint(), "a marker appears");
}

// ---------------------------------------------------------------------
// Undo and split
// ---------------------------------------------------------------------

#[test]
fn undo_restores_the_warp_state_and_resyncs_the_engine() {
    let (mut app, rx) = app_with(warped());
    let before = app.test_snapshot_for_undo();
    let _ = app.update(warp_msg(ClipWarpMessage::SetWarp {
        clip_id: CLIP,
        enabled: false,
        original_bpm: None,
        transpose_semitones: 3.0,
        algorithm: WarpAlgorithm::Tonal,
    }));
    let _ = app.update(warp_msg(ClipWarpMessage::SetWarpMarkers {
        clip_id: CLIP,
        markers: Vec::new(),
    }));
    let _ = drain(&rx);

    app.test_begin_restore_from_snapshot(before);
    assert_eq!(warp_of(&app), warped(), "the mirror is back");
    let sent = drain(&rx);
    assert!(
        sent.iter().any(|c| matches!(
            c,
            AudioCommand::SetClipWarp {
                clip_id: CLIP,
                warp_enabled: true,
                original_bpm: Some(b),
                warp_algorithm: WarpAlgorithm::Transient,
                ..
            } if *b == 100.0
        )),
        "the engine gets the scalars back: {sent:?}"
    );
    assert!(
        sent.iter().any(|c| matches!(
            c,
            AudioCommand::SetClipWarpMarkers { clip_id: CLIP, markers } if *markers == warped().markers
        )),
        "and the markers: {sent:?}"
    );
}

#[test]
fn split_keeps_the_settings_but_drops_the_tails_markers() {
    let (mut app, _rx) = app_with(warped());
    let _ = app.update(Message::Clip(ClipMessage::SplitClipAt {
        clip_id: CLIP,
        new_clip_id: 99,
        at_sample: 2 * SR as u64,
    }));
    let tail = app
        .test_clips()
        .iter()
        .find(|c| c.id == 99)
        .expect("tail")
        .warp
        .clone();
    assert!(tail.enabled);
    assert_eq!(tail.original_bpm, Some(100.0));
    assert!(tail.markers.is_empty(), "markers stay with the head, as in the engine");
    assert_eq!(warp_of(&app).markers.len(), 2);
}
