//! A rendered vocal take stays its lane's take across a reload and an
//! undo/redo, with pre-roll and a lead-in (vocal-rendering.md, spec §7,
//! D22).
//!
//! Every reload and every undo/redo rebuilds the lane → take map from
//! the audio clips. It used to claim a vocal-track clip only when the
//! clip started on a placement's bar line — but a take sits at its first
//! note (a lead-in moves it off the bar) less its pre-roll (the onset
//! consonants or a written breath before the beat), so it never did: the
//! lane read `not_rendered`, its unit keys were dropped, and the next
//! render found no old take to replace — two takes, summed.
//!
//! A take is now saved with the placement it plays; one saved without
//! it is matched by where its first note sits. Its trims are mirrored at
//! install too: the engine's load echo carries none, so the take used to
//! be saved untrimmed and reopen playing its leading pad, late.

use resonance_app::project::{LoadedProject, ProjectFile};
use resonance_app::Resonance;
use resonance_audio::types::{ClipId, TICKS_PER_QUARTER_NOTE};
use resonance_control::ids::{ClipId as ProtoClipId, TrackId as ProtoTrackId};
use resonance_control::methods::mixer as mixer_proto;
use resonance_control::methods::notes::{NoteSpec, ReplaceAllParams};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::VocalRenderState;
use resonance_control::methods::vocal as proto;
use resonance_control::Request;
use serde_json::json;

use crate::control_vocal_render_state::{app_with_project, call, lane_state, vocal_view, TRACK};

const SAMPLE_RATE: u32 = 48_000;
/// 0.1 s of onset sung before the first note's beat (the pad is 0.3 s).
const PRE_ROLL: u64 = 4_800;

/// A lane on a section placed at `start_bar`, its melody moved
/// `shift_beats` later in the section, rendered as a real take would be:
/// at the first note, `PRE_ROLL` early.
fn rendered_take(app: &mut Resonance, start_bar: u32, shift_beats: f64) -> u64 {
    app.test_set_sample_rate(SAMPLE_RATE);
    // Rebuild the tempo map at that rate, as a project load does.
    app.test_set_flat_tempo(120.0);
    let section_id = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    let _ = call(
        app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: section_id,
            start_bar,
        },
    );
    let def = u64::from(section_id);
    app.test_install_vocal_lane(def, TRACK);
    let mut params =
        resonance_control::methods::harmony::ApplyProgressionParams::for_section(section_id);
    params.key = Some(resonance_control::KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
    call(app, "harmony.apply_progression", &params)
        .result::<resonance_control::methods::harmony::ApplyProgressionResult>()
        .expect("progression applies");
    call(
        app,
        "vocal.generate",
        &proto::GenerateParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(section_id),
            seed: Some(7),
            lyrics: true,
        },
    )
    .result::<proto::GenerateResult>()
    .expect("vocal.generate succeeds");
    assert!(app.test_finish_vocal_render_take(def, TRACK, PRE_ROLL));

    if shift_beats > 0.0 {
        let midi = app
            .test_midi_clips()
            .iter()
            .find(|c| c.track_id == TRACK)
            .expect("the lane's MIDI clip");
        let tpq = TICKS_PER_QUARTER_NOTE as f64;
        let notes: Vec<NoteSpec> = midi
            .notes
            .iter()
            .map(|n| NoteSpec {
                pitch: n.note,
                start_beat: n.start_tick as f64 / tpq + shift_beats,
                duration_beats: n.duration_ticks as f64 / tpq,
                velocity: n.velocity.round().clamp(1.0, 127.0) as u8,
            })
            .collect();
        let clip_id = ProtoClipId(midi.id);
        call(
            app,
            "notes.replace_all",
            &ReplaceAllParams {
                clip_id,
                notes,
                confirm: true,
            },
        )
        .result::<serde_json::Value>()
        .expect("notes.replace_all succeeds");
        call(app, "vocal.render", &json!({ "track_id": TRACK, "section_id": def }))
            .result::<serde_json::Value>()
            .expect("render starts");
        assert!(app.test_finish_vocal_render_take(def, TRACK, PRE_ROLL));
    }
    assert_eq!(lane_state(&vocal_view(app), def), VocalRenderState::Rendered);
    def
}

/// The lane's one take: its clip id, after checking it is the only audio
/// clip on the track (a second one is a summed duplicate).
fn the_take(app: &Resonance, def: u64) -> ClipId {
    let takes = app.test_vocal_audio_clips(TRACK);
    assert_eq!(takes.len(), 1, "one take for the lane: {takes:?}");
    assert_eq!(takes[0].0, def);
    let on_track: Vec<ClipId> =
        app.test_clips().iter().filter(|c| c.track_id == TRACK).map(|c| c.id).collect();
    assert_eq!(on_track, vec![takes[0].1], "no second take on the track");
    takes[0].1
}

fn reopen(app: &mut Resonance, file: ProjectFile) {
    let midi_notes = app.test_midi_clips().iter().map(|c| (c.id, c.notes.clone())).collect();
    app.test_replay_loaded_project_from(LoadedProject {
        file,
        project_dir: std::path::PathBuf::from("/tmp/control-vocal-take-reload.rprj"),
        midi_notes,
        plugin_states: std::collections::HashMap::new(),
    });
}

/// The take is still the lane's: it reads `rendered` (its unit keys came
/// back with it), and a re-render replaces it rather than adding one.
fn assert_lane_keeps_its_take(app: &mut Resonance, def: u64, take: ClipId) {
    assert_eq!(the_take(app, def), take, "the lane claims its take");
    assert_eq!(lane_state(&vocal_view(app), def), VocalRenderState::Rendered);
    call(app, "vocal.render", &json!({ "track_id": TRACK, "section_id": def }))
        .result::<serde_json::Value>()
        .expect("render starts");
    assert!(app.test_finish_vocal_render_take(def, TRACK, PRE_ROLL));
    assert_ne!(the_take(app, def), take, "the re-render replaced the take");
}

/// The precondition every test rests on: the take is off the bar line
/// and trimmed the way the install placed it, and saved that way.
fn assert_take_is_off_the_bar(app: &Resonance, def: u64, take: ClipId) -> ProjectFile {
    let placement = app.compose_state().placements.iter().find(|p| p.definition_id == def).unwrap();
    let bar = app.test_tempo_map().bar_to_sample(placement.start_bar);
    let file = app.test_build_project_file();
    let saved = file.clips.iter().find(|c| c.id == take).expect("the take is saved");
    assert_ne!(saved.start_sample, bar, "the take starts off the bar line");
    assert_eq!(
        saved.trim_end_frames,
        resonance_app::compose::vocal_svs::pad_frames(SAMPLE_RATE),
        "the take is saved trimmed"
    );
    file
}

#[test]
fn a_take_with_pre_roll_and_a_lead_in_is_still_the_lanes_after_a_reload() {
    let mut app = app_with_project();
    let def = rendered_take(&mut app, 3, 1.0);
    let take = the_take(&app, def);
    let file = assert_take_is_off_the_bar(&app, def, take);
    let saved = file.clips.iter().find(|c| c.id == take).unwrap();
    assert!(saved.vocal_take.is_some(), "the take is saved with its placement");

    reopen(&mut app, file);
    assert_lane_keeps_its_take(&mut app, def, take);
}

/// A take saved before its owner was: found by where its first note sits
/// (`start + pad − trim_start`), lead-in and pre-roll included.
#[test]
fn a_take_saved_without_its_placement_is_found_by_its_first_note() {
    let mut app = app_with_project();
    let def = rendered_take(&mut app, 3, 1.0);
    let take = the_take(&app, def);
    let mut file = assert_take_is_off_the_bar(&app, def, take);
    for clip in &mut file.clips {
        clip.vocal_take = None;
    }

    reopen(&mut app, file);
    assert_lane_keeps_its_take(&mut app, def, take);
}

/// A first note at sample 0 cannot start its pre-roll earlier: the take
/// sits at 0 with the pre-roll trimmed, and its first note is still found.
#[test]
fn a_take_clamped_at_sample_zero_is_found_by_its_first_note() {
    let mut app = app_with_project();
    let def = rendered_take(&mut app, 1, 0.0);
    let take = the_take(&app, def);
    let mut file = app.test_build_project_file();
    let saved = file.clips.iter().find(|c| c.id == take).unwrap();
    let lead = app
        .test_midi_clips()
        .iter()
        .find(|c| c.track_id == TRACK)
        .and_then(|c| c.notes.first())
        .map_or(0, |n| n.start_tick);
    assert_eq!(lead, 0, "precondition: the melody starts on the downbeat");
    assert_eq!(saved.start_sample, 0, "precondition: the take is clamped at 0");
    assert_eq!(
        saved.trim_start_frames,
        resonance_app::compose::vocal_svs::pad_frames(SAMPLE_RATE),
        "precondition: the pre-roll that did not fit is trimmed"
    );
    for clip in &mut file.clips {
        clip.vocal_take = None;
    }

    reopen(&mut app, file);
    assert_lane_keeps_its_take(&mut app, def, take);
}

/// An undo and a redo rebuild the map from the clips too.
#[test]
fn a_take_with_pre_roll_and_a_lead_in_is_still_the_lanes_after_undo_and_redo() {
    let mut app = app_with_project();
    let def = rendered_take(&mut app, 3, 1.0);
    let take = the_take(&app, def);
    assert_take_is_off_the_bar(&app, def, take);

    // An unrelated edit to take back.
    call(
        &mut app,
        "mixer.set_volume",
        &mixer_proto::SetVolumeParams {
            track_id: ProtoTrackId(TRACK),
            volume: 0.33,
        },
    )
    .result::<resonance_control::MutationAck>()
    .expect("mixer.set_volume succeeds");

    crate::common::roundtrip(&mut app, Request::without_params(98, "edit.undo"))
        .result::<serde_json::Value>()
        .expect("edit.undo succeeds");
    assert_eq!(the_take(&app, def), take, "after undo");
    assert_eq!(lane_state(&vocal_view(&mut app), def), VocalRenderState::Rendered);

    crate::common::roundtrip(&mut app, Request::without_params(99, "edit.redo"))
        .result::<serde_json::Value>()
        .expect("edit.redo succeeds");
    assert_lane_keeps_its_take(&mut app, def, take);
}
