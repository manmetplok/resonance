//! Render state for edits the unit cache key cannot see
//! (vocal-mcp-control-and-quality.md §7–§9, the phase-3 review).
//!
//! A unit's cache key is position-free — a phrase moved in time reuses
//! its audio — and leaves out the raw note velocity the post-vocoder gain
//! reads. The keys a take records, and the plan is compared with, are
//! not: they also hash where each unit starts and, with
//! `velocity_dynamics` on, its notes' velocities. Without that, shifting
//! the whole line or changing a velocity on a bank without energy or
//! tension (TIGER) read `rendered` while the take played the old timing
//! or dynamics.
//!
//! Also here: the lane's rest threshold in `song.vocal`'s articulation
//! report, and the debounce generation surviving a project load.

use resonance_app::compose::{ComposeMessage, VocalLaneRenderState};
use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::Resonance;
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_control::methods::song::VocalRenderState;
use serde_json::json;

use crate::control_vocal_render_state::{
    app_with_project, call, first_line_number, gui, lane_state, rendered_lane, vocal_view, TRACK,
};
use resonance_app::compose::messages::LaneInspectorMsg;

const TPQ: u64 = TICKS_PER_QUARTER_NOTE;

/// The lane's MIDI clip (one section placed once: one clip on `TRACK`).
fn lane_clip(app: &Resonance) -> (u64, Vec<MidiNote>) {
    let clip = app
        .test_midi_clips()
        .iter()
        .find(|c| c.track_id == TRACK)
        .expect("the lane has a clip");
    (clip.id, clip.notes.as_ref().clone())
}

/// Replace the clip's notes over the control API.
fn replace_notes(app: &mut Resonance, clip_id: u64, notes: &[MidiNote]) {
    let notes: Vec<serde_json::Value> = notes
        .iter()
        .map(|n| {
            json!({
                "pitch": n.note,
                "start_beat": n.start_tick as f64 / TPQ as f64,
                "duration_beats": n.duration_ticks as f64 / TPQ as f64,
                "velocity": (n.velocity * 127.0).round() as u8,
            })
        })
        .collect();
    call(
        app,
        "notes.replace_all",
        &json!({ "clip_id": clip_id, "notes": notes, "confirm": true }),
    )
    .result::<serde_json::Value>()
    .expect("notes.replace_all succeeds");
}

fn set_params(app: &mut Resonance, def: u64, params: serde_json::Value) {
    call(
        app,
        "vocal.set_params",
        &json!({ "track_id": TRACK, "section_id": def, "params": params }),
    )
    .result::<serde_json::Value>()
    .expect("set_params succeeds");
}

/// `vocal.render` the lane and land the take.
fn render(app: &mut Resonance, def: u64) {
    call(app, "vocal.render", &json!({ "track_id": TRACK, "section_id": def }))
        .result::<serde_json::Value>()
        .expect("render starts");
    assert!(app.test_finish_vocal_render(def, TRACK));
    assert_eq!(lane_state(&vocal_view(app), def), VocalRenderState::Rendered);
}

fn shifted_by_a_beat(notes: &[MidiNote]) -> Vec<MidiNote> {
    notes
        .iter()
        .map(|n| MidiNote {
            start_tick: n.start_tick + TPQ,
            ..n.clone()
        })
        .collect()
}

/// Moving every note by a beat changes no unit's content — same
/// phonemes, durations and curves, so the same cache keys — but the
/// installed take now sits a beat early. The lane must read `stale`.
#[test]
fn shifting_the_whole_line_over_control_goes_stale() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let (clip, notes) = lane_clip(&app);
    // Re-author the notes as written first, so the shift is the only
    // difference (a round trip through beats must not move a tick).
    replace_notes(&mut app, clip, &notes);
    render(&mut app, verse);
    let epoch = app.test_vocal_render_epoch(verse, TRACK);

    replace_notes(&mut app, clip, &shifted_by_a_beat(&notes));
    let view = vocal_view(&mut app);
    assert_eq!(lane_state(&view, verse), VocalRenderState::Stale);
    assert_eq!(app.test_vocal_render_epoch(verse, TRACK), epoch, "control renders nothing");
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), None);
}

/// The same shift from the GUI (a bulk edit the editor mirrors, then
/// hands the engine) arms the debounced re-render.
#[test]
fn shifting_the_whole_line_from_the_gui_arms_the_re_render() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let (clip, notes) = lane_clip(&app);
    let shifted = shifted_by_a_beat(&notes);
    let mirror = app
        .test_midi_clips_mut()
        .iter_mut()
        .find(|c| c.id == clip)
        .unwrap();
    mirror.notes = std::sync::Arc::new(shifted.clone());
    let _ = app.update(Message::MidiEditor(MidiEditorMessage::SetClipNotes {
        clip_id: clip,
        notes: shifted,
    }));
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Stale);
    assert!(app.test_vocal_auto_render_pending(verse, TRACK).is_some());
}

/// A lane on TIGER (no energy, no tension input): velocity reaches the
/// take only through the post gain, after the cache. With
/// `velocity_dynamics` on a velocity edit makes the lane stale; with it
/// at 0 the take cannot change, and the lane stays rendered.
#[test]
fn a_velocity_edit_on_tiger_is_stale_only_with_velocity_dynamics() {
    for (amount, want) in [(0.5, VocalRenderState::Stale), (0.0, VocalRenderState::Rendered)] {
        let mut app = app_with_project();
        let verse = rendered_lane(&mut app, "Verse", 1);
        set_params(
            &mut app,
            verse,
            json!({ "voicebank": "tiger", "velocity_dynamics": amount }),
        );
        render(&mut app, verse);
        let (clip, notes) = lane_clip(&app);

        let mut louder = notes.clone();
        louder[0].velocity = if louder[0].velocity > 0.9 { 0.6 } else { 1.0 };
        call(
            &mut app,
            "notes.edit",
            &json!({
                "clip_id": clip,
                "index": 0,
                "velocity": (louder[0].velocity * 127.0).round() as u8,
            }),
        )
        .result::<serde_json::Value>()
        .expect("notes.edit succeeds");
        assert_ne!(lane_clip(&app).1[0].velocity, notes[0].velocity, "the edit landed");

        let view = vocal_view(&mut app);
        assert_eq!(lane_state(&view, verse), want, "velocity_dynamics {amount}");
    }
}

/// `song.vocal`'s articulation report reads the lane's own rest
/// threshold: a ¾-beat gap is a rest at ½ beat (the note sings its own
/// length) and sung legato at 1 beat (the note holds to the next onset).
#[test]
fn song_vocal_articulation_follows_the_lane_rest_threshold() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let (clip, notes) = lane_clip(&app);
    // Two notes, half a beat each, a ¾-beat gap between them.
    let two = vec![
        MidiNote {
            start_tick: 0,
            duration_ticks: TPQ / 2,
            ..notes[0].clone()
        },
        MidiNote {
            start_tick: TPQ * 5 / 4,
            duration_ticks: TPQ / 2,
            ..notes[1].clone()
        },
    ];
    replace_notes(&mut app, clip, &two);

    let first_note_ms = |app: &mut Resonance| {
        let view = vocal_view(app);
        let lane = view
            .lanes
            .iter()
            .find(|l| u64::from(l.definition_id) == verse)
            .unwrap();
        lane.notes[0].duration_ms
    };
    let beat_ms = 60_000.0 / app.test_transport_bpm() as f64;

    set_params(&mut app, verse, json!({ "rest_threshold_beats": 0.5 }));
    let rest = first_note_ms(&mut app);
    assert!((rest - 0.5 * beat_ms).abs() < 1.0, "a rest: sings its half beat ({rest} ms)");

    set_params(&mut app, verse, json!({ "rest_threshold_beats": 1.0 }));
    let legato = first_note_ms(&mut app);
    assert!(
        (legato - 1.25 * beat_ms).abs() < 1.0,
        "legato: holds to the next onset ({legato} ms)"
    );
}

/// A disk load resets the render-state tables but not the debounce
/// generation: a timer armed before the load must not match one armed
/// after it, or the stale one would fire the new lane's re-render early.
#[test]
fn a_load_keeps_the_debounce_generation_counting() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let n = first_line_number(&app, verse);
    gui(&mut app, verse, LaneInspectorMsg::SetVocalLineText(n, "before the load".into()));
    let before = app
        .test_vocal_auto_render_pending(verse, TRACK)
        .expect("armed before the load");

    let file = app.test_build_project_file();
    let midi_notes = app
        .test_midi_clips()
        .iter()
        .map(|c| (c.id, c.notes.clone()))
        .collect();
    app.test_replay_loaded_project_from(resonance_app::project::LoadedProject {
        file,
        project_dir: std::path::PathBuf::from("/tmp/control-vocal-render-state-edits.rprj"),
        midi_notes,
        plugin_states: std::collections::HashMap::new(),
    });
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), None, "the load disarms");

    // A failed render gives the lane something to bring up to date, so a
    // GUI edit arms again after the load.
    call(&mut app, "vocal.render", &json!({ "track_id": TRACK, "section_id": verse }))
        .result::<serde_json::Value>()
        .expect("render starts");
    assert!(app.test_fail_vocal_render(
        verse,
        TRACK,
        resonance_app::compose::vocal_svs::VocalRenderErrorKind::Failed,
        "boom",
    ));
    gui(&mut app, verse, LaneInspectorMsg::SetVocalLineText(n, "after the load".into()));
    let after = app
        .test_vocal_auto_render_pending(verse, TRACK)
        .expect("armed after the load");
    assert_ne!(before, after, "generations keep counting across a load");

    // The pre-load timer fires late: it renders nothing.
    let epoch = app.test_vocal_render_epoch(verse, TRACK);
    let _ = app.update(Message::Compose(ComposeMessage::VocalAutoRender {
        definition_id: verse,
        track_id: TRACK,
        generation: before,
    }));
    assert_eq!(app.test_vocal_render_epoch(verse, TRACK), epoch);
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), Some(after));
}

/// A tick that only moved the playhead edits nothing, so it skips the
/// render-state settle (it runs at frame rate during playback); any other
/// update still settles.
#[test]
fn a_quiet_tick_skips_the_settle() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let (clip, notes) = lane_clip(&app);
    // A change no update made (nothing in the app does this; it shows
    // which updates look).
    let mirror = app
        .test_midi_clips_mut()
        .iter_mut()
        .find(|c| c.id == clip)
        .unwrap();
    mirror.notes = std::sync::Arc::new(shifted_by_a_beat(&notes));

    let _ = app.update(Message::Tick);
    let settled = app.test_vocal_settled_render_status(verse, TRACK).unwrap();
    assert_eq!(settled.state, VocalLaneRenderState::Rendered, "a quiet tick skipped it");

    let _ = app.update(Message::MidiEditor(MidiEditorMessage::ScrollY(0.0)));
    let settled = app.test_vocal_settled_render_status(verse, TRACK).unwrap();
    assert_eq!(settled.state, VocalLaneRenderState::Stale, "the next update settles");
}
