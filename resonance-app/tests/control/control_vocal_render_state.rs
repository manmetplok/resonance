//! Per-lane vocal render state (vocal-mcp-control-and-quality.md §7,
//! D19–D22, slice 9).
//!
//! `song.vocal` used to report one `render_state` per track — `rendered`
//! as soon as ANY lane had audio — and never `stale` or `error`. Each lane
//! now compares the unit content keys of the take it plays with the keys
//! its current content plans to, keeps its last failure, and the track
//! reports the worst lane. Edits made over the control API leave a lane
//! `stale` and never render on their own; the same edits from the GUI arm
//! a debounced re-render. The old take plays until a new one lands.
//!
//! No inference runs here: `test_finish_vocal_render` lands an in-flight
//! render through the real `VocalAudioReady` path with the take the
//! render job would have produced.

use resonance_app::compose::messages::LaneInspectorMsg;
use resonance_app::compose::{ComposeMessage, VocalLaneErrorKind, VocalLaneRenderState};
use resonance_app::message::Message;
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::{VocalRenderErrorKind, VocalRenderState, VocalView};
use resonance_control::methods::vocal as proto;
use resonance_control::{Request, Response};
use serde_json::json;

use crate::common::roundtrip;

pub(crate) const TRACK: u64 = 70;

pub(crate) fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_install_shipped_voicebanks();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/control-vocal-render-state.rprj",
    ));
    app.test_add_track(TRACK, TrackType::Vocal);
    app
}

pub(crate) fn call<T: serde::Serialize>(
    app: &mut Resonance,
    method: &str,
    params: &T,
) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn apply_progression(app: &mut Resonance, def: u64, numerals: [&str; 4]) {
    apply_progression_with(app, def, numerals, None);
}

fn apply_progression_with(
    app: &mut Resonance,
    def: u64,
    numerals: [&str; 4],
    vocal_melody: Option<resonance_control::VocalMelodyChoice>,
) {
    let mut params = resonance_control::methods::harmony::ApplyProgressionParams::for_section(
        SectionDefinitionId(def),
    );
    params.key = Some(resonance_control::KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(numerals.into_iter().map(str::to_owned).collect());
    params.vocal_melody = vocal_melody;
    call(app, "harmony.apply_progression", &params)
        .result::<resonance_control::methods::harmony::ApplyProgressionResult>()
        .expect("progression applies");
}

/// A section placed at `start_bar` with a vocal lane on `TRACK`, chords,
/// a generated melody + lyrics, and that render landed: a `rendered`
/// lane.
pub(crate) fn rendered_lane(app: &mut Resonance, name: &str, start_bar: u32) -> u64 {
    let section_id = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: name.to_owned(),
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
    apply_progression(app, def, ["i", "iv", "v", "i"]);
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
    assert!(app.test_finish_vocal_render(def, TRACK), "generate queued a render");
    def
}

pub(crate) fn vocal_view(app: &mut Resonance) -> VocalView {
    call(app, "song.vocal", &json!({ "track_id": TRACK }))
        .result()
        .expect("song.vocal succeeds")
}

pub(crate) fn lane_state(view: &VocalView, def: u64) -> VocalRenderState {
    view.lanes
        .iter()
        .find(|l| u64::from(l.definition_id) == def)
        .expect("lane listed")
        .render_state
}

pub(crate) fn gui(app: &mut Resonance, def: u64, msg: LaneInspectorMsg) {
    let _ = app.update(Message::Compose(ComposeMessage::LaneInspector {
        definition_id: def,
        track_id: TRACK,
        msg,
    }));
}

/// The number (`LyricLine::n`) of the lane's first lyric line — what the
/// inspector's line fields address.
pub(crate) fn first_line_number(app: &Resonance, def: u64) -> u8 {
    let cfg = &app.compose_state().find_definition(def).unwrap().lane_generators[&TRACK];
    match &cfg.kind {
        resonance_app::compose::LaneGeneratorKind::Vocal(p) => p.draft[0].n,
        _ => panic!("a vocal lane"),
    }
}

#[test]
fn a_rendered_lane_reads_rendered_and_the_track_too() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let view = vocal_view(&mut app);
    assert_eq!(lane_state(&view, verse), VocalRenderState::Rendered);
    assert_eq!(view.lanes[0].stale_units, 0);
    assert_eq!(view.lanes[0].error, None);
    assert_eq!(view.render_state, VocalRenderState::Rendered);
}

/// The headline case: a lyric edit over MCP makes only that lane stale,
/// renders nothing, and the track reports the worst lane.
#[test]
fn a_control_lyric_edit_makes_only_that_lane_stale_and_does_not_render() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let chorus = rendered_lane(&mut app, "Chorus", 5);
    let audio_before = app.test_vocal_audio_clips(TRACK);
    let epoch = app.test_vocal_render_epoch(verse, TRACK);

    call(
        &mut app,
        "vocal.set_line",
        &proto::SetLineParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(verse)),
            line_index: 0,
            text: "something wholly different now".to_owned(),
            syllabify: true,
        },
    )
    .result::<serde_json::Value>()
    .expect("set_line succeeds");

    let view = vocal_view(&mut app);
    let verse_lane = view
        .lanes
        .iter()
        .find(|l| u64::from(l.definition_id) == verse)
        .unwrap();
    assert_eq!(verse_lane.render_state, VocalRenderState::Stale);
    assert!(verse_lane.stale_units > 0, "the changed phrase is counted");
    assert_eq!(lane_state(&view, chorus), VocalRenderState::Rendered);
    assert_eq!(view.render_state, VocalRenderState::Stale, "worst lane");

    assert_eq!(app.test_vocal_render_epoch(verse, TRACK), epoch, "nothing queued");
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), None, "nothing armed");
    assert_eq!(app.test_vocal_audio_clips(TRACK), audio_before, "the old take still plays");
    // The views read the same state, settled after the update.
    let settled = app.test_vocal_settled_render_status(verse, TRACK).unwrap();
    assert_eq!(settled.state, VocalLaneRenderState::Stale);
}

#[test]
fn set_params_makes_the_lane_stale() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    call(
        &mut app,
        "vocal.set_params",
        &json!({ "track_id": TRACK, "section_id": verse, "params": { "portamento_ms": 250.0 } }),
    )
    .result::<serde_json::Value>()
    .expect("set_params succeeds");
    let view = vocal_view(&mut app);
    assert_eq!(lane_state(&view, verse), VocalRenderState::Stale);
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), None);

    // `vocal.render` brings it back.
    call(&mut app, "vocal.render", &json!({ "track_id": TRACK }))
        .result::<serde_json::Value>()
        .expect("render starts");
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendering);
    assert!(app.test_finish_vocal_render(verse, TRACK));
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);
}

/// A chord edit that regenerates the lane's melody (`vocal_melody:
/// regenerate`; the default keeps it, BUG-70). It used to queue a render
/// and drop its task, leaving the lane audio-less and `rendering` forever;
/// over the control API it now only goes stale, old audio kept.
#[test]
fn a_control_chord_edit_goes_stale_and_keeps_the_old_audio() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let audio_before = app.test_vocal_audio_clips(TRACK);
    let epoch = app.test_vocal_render_epoch(verse, TRACK);

    apply_progression_with(
        &mut app,
        verse,
        ["VI", "III", "VII", "i"],
        Some(resonance_control::VocalMelodyChoice::Regenerate),
    );

    let view = vocal_view(&mut app);
    assert_eq!(lane_state(&view, verse), VocalRenderState::Stale);
    assert!(!app.test_vocal_render_in_flight(verse, TRACK), "not stuck rendering");
    assert_eq!(app.test_vocal_render_epoch(verse, TRACK), epoch, "nothing queued");
    assert_eq!(app.test_vocal_audio_clips(TRACK), audio_before, "old audio kept");
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), None);
}

/// The same edits from the GUI arm a re-render 750 ms after the last one;
/// the timer's message is what renders, and only the newest timer counts.
#[test]
fn a_gui_lyric_edit_arms_a_debounced_re_render() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let chorus = rendered_lane(&mut app, "Chorus", 5);
    let epoch = app.test_vocal_render_epoch(verse, TRACK);

    let n = first_line_number(&app, verse);
    gui(&mut app, verse, LaneInspectorMsg::SetVocalLineText(n, "a brand new line".into()));
    let first = app
        .test_vocal_auto_render_pending(verse, TRACK)
        .expect("a GUI edit arms the re-render");
    assert_eq!(app.test_vocal_auto_render_pending(chorus, TRACK), None, "only that lane");
    assert_eq!(app.test_vocal_render_epoch(verse, TRACK), epoch, "not rendered yet");

    // Another keystroke re-arms; the first timer then does nothing.
    gui(&mut app, verse, LaneInspectorMsg::SetVocalLineText(n, "a brand new line too".into()));
    let second = app.test_vocal_auto_render_pending(verse, TRACK).unwrap();
    assert_ne!(first, second);
    let due = |generation| {
        Message::Compose(ComposeMessage::VocalAutoRender {
            definition_id: verse,
            track_id: TRACK,
            generation,
        })
    };
    let _ = app.update(due(first));
    assert_eq!(app.test_vocal_render_epoch(verse, TRACK), epoch, "a superseded timer");

    let _ = app.update(due(second));
    assert!(app.test_vocal_render_epoch(verse, TRACK) > epoch, "the newest timer renders");
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), None);
    assert!(app.test_finish_vocal_render(verse, TRACK));
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);
}

/// A GUI chord edit takes the same path as any other GUI edit: stale,
/// then the debounced re-render — not the dropped task of before.
#[test]
fn a_gui_chord_edit_arms_the_re_render_instead_of_dropping_it() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let chord = app
        .compose_state()
        .find_definition(verse)
        .unwrap()
        .chords[0]
        .clone();
    let mut edited = chord.chord;
    edited.root = edited.root.transpose(5);
    let _ = app.update(Message::Compose(ComposeMessage::EditChord {
        definition_id: verse,
        chord_id: chord.id,
        chord: edited,
    }));
    // The edit asks what to do with the melody (BUG-70); a generated lane
    // defaults to Regenerate.
    let _ = app.update(Message::Compose(ComposeMessage::VocalMelody(
        resonance_app::compose::vocal_melody::VocalMelodyMsg::Apply,
    )));
    assert!(!app.test_vocal_render_in_flight(verse, TRACK), "nothing queued yet");
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Stale);
    assert!(app.test_vocal_auto_render_pending(verse, TRACK).is_some());
}

/// A lane whose GUI edit renders nothing new (a lyric-generator setting)
/// is not re-rendered: its audio is still current.
#[test]
fn a_gui_edit_that_changes_no_unit_arms_nothing() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    gui(&mut app, verse, LaneInspectorMsg::SetVocalTheme("the sea".into()));
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);
    assert_eq!(app.test_vocal_auto_render_pending(verse, TRACK), None);
}

#[test]
fn a_failed_render_reports_error_until_an_edit() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let chorus = rendered_lane(&mut app, "Chorus", 5);
    call(&mut app, "vocal.render", &json!({ "track_id": TRACK, "section_id": verse }))
        .result::<serde_json::Value>()
        .expect("render starts");
    assert!(app.test_fail_vocal_render(
        verse,
        TRACK,
        resonance_app::compose::vocal_svs::VocalRenderErrorKind::Failed,
        "the acoustic model fell over",
    ));

    let view = vocal_view(&mut app);
    let lane = view
        .lanes
        .iter()
        .find(|l| u64::from(l.definition_id) == verse)
        .unwrap();
    assert_eq!(lane.render_state, VocalRenderState::Error);
    let error = lane.error.as_ref().expect("the failure is reported");
    assert_eq!(error.kind, VocalRenderErrorKind::RenderFailed);
    assert!(error.message.contains("fell over"), "{}", error.message);
    assert_eq!(lane_state(&view, chorus), VocalRenderState::Rendered);
    assert_eq!(view.render_state, VocalRenderState::Error, "worst lane");

    // An edit to the lane retires the error; its audio is now stale.
    call(
        &mut app,
        "vocal.set_params",
        &json!({ "track_id": TRACK, "section_id": verse, "params": { "portamento_ms": 250.0 } }),
    )
    .result::<serde_json::Value>()
    .expect("set_params succeeds");
    let view = vocal_view(&mut app);
    assert_eq!(lane_state(&view, verse), VocalRenderState::Stale);
    assert!(view.lanes.iter().all(|l| l.error.is_none()));
}

#[test]
fn a_missing_bank_reports_error_with_its_kind() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    // The lane's bank goes away: the render refuses up front.
    std::fs::remove_dir_all(app.test_user_voicebank_dir("tiger")).expect("remove the bank");
    app.test_rescan_voicebanks();
    gui(&mut app, verse, LaneInspectorMsg::RerenderVocalAudio);

    let status = app.test_vocal_lane_render_status(verse, TRACK);
    assert_eq!(status.state, VocalLaneRenderState::Error);
    assert_eq!(
        status.error.as_ref().map(|(kind, _)| *kind),
        Some(VocalLaneErrorKind::VoicebankMissing)
    );
    let view = vocal_view(&mut app);
    let error = view.lanes[0].error.clone().expect("reported on the wire");
    assert_eq!(error.kind, VocalRenderErrorKind::VoicebankMissing);
}

/// The installed take's unit keys are saved with its clip, so the lane
/// reads `rendered` after the project is reopened — not `stale`.
#[test]
fn a_reopened_project_reads_rendered() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    // The engine's echo of the installed take (no engine runs here).
    let (_, clip_id) = app.test_vocal_audio_clips(TRACK)[0];
    let placement = app.compose_state().placements[0].start_bar;
    app.test_push_clip(ClipState {
        id: clip_id,
        track_id: TRACK,
        start_sample: app.test_tempo_map().bar_to_sample(placement),
        duration_samples: 48_000,
        name: "Verse · Vocal".into(),
        total_frames: 48_000,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: Default::default(),
        fade_out_frames: 0,
        fade_out_curve: Default::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    });

    let file = app.test_build_project_file();
    let saved = file.clips.iter().find(|c| c.id == clip_id).expect("the take is saved");
    assert_eq!(
        Some(saved.vocal_render_units.clone()),
        app.test_vocal_expected_unit_keys(verse, TRACK),
        "the take's unit keys are saved with it"
    );
    let midi_notes = app
        .test_midi_clips()
        .iter()
        .map(|c| (c.id, c.notes.clone()))
        .collect();
    app.test_replay_loaded_project_from(resonance_app::project::LoadedProject {
        file: file.clone(),
        project_dir: std::path::PathBuf::from("/tmp/control-vocal-render-state.rprj"),
        midi_notes,
        plugin_states: std::collections::HashMap::new(),
    });
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);

    // A take saved without keys (before render state existed) cannot be
    // vouched for: it reads stale until re-rendered.
    let mut legacy = file;
    for clip in &mut legacy.clips {
        clip.vocal_render_units.clear();
    }
    let midi_notes = app
        .test_midi_clips()
        .iter()
        .map(|c| (c.id, c.notes.clone()))
        .collect();
    app.test_replay_loaded_project_from(resonance_app::project::LoadedProject {
        file: legacy,
        project_dir: std::path::PathBuf::from("/tmp/control-vocal-render-state.rprj"),
        midi_notes,
        plugin_states: std::collections::HashMap::new(),
    });
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Stale);
}

fn add_tempo(app: &mut Resonance, bar: u32, bpm: f32) {
    call(app, "global.add_tempo_event", &json!({ "bar": bar, "bpm": bpm }))
        .result::<serde_json::Value>()
        .expect("tempo event adds");
}

fn place(app: &mut Resonance, def: u64, start_bar: u32) -> u64 {
    call(
        app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: SectionDefinitionId(def),
            start_bar,
        },
    )
    .result::<section_proto::PlaceResult>()
    .expect("section.place succeeds")
    .placement_id
    .into()
}

/// BUG-73 (status half): a placement the lane's take cannot be laid at —
/// here it plays in another tempo region than the take was sung at — gets
/// no take, so the lane is not `rendered`: it reads `stale` (the old take
/// plays where it is, nothing sings on the new placement) until a render
/// fans out to every placement.
#[test]
fn a_placement_without_a_take_keeps_the_lane_from_reading_rendered() {
    let mut app = app_with_project();
    // 120 BPM up to bar 8, 140 from bar 9 on.
    add_tempo(&mut app, 1, 120.0);
    add_tempo(&mut app, 8, 120.0);
    add_tempo(&mut app, 9, 140.0);
    let verse = rendered_lane(&mut app, "Verse", 1);
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);

    place(&mut app, verse, 9);
    assert_eq!(
        app.test_vocal_audio_clips(TRACK).len(),
        1,
        "a take sung at 120 BPM is not laid at a 140 BPM placement"
    );

    let view = vocal_view(&mut app);
    let lane = view
        .lanes
        .iter()
        .find(|l| u64::from(l.definition_id) == verse)
        .expect("lane listed");
    assert_eq!(lane.render_state, VocalRenderState::Stale);
    assert_eq!(lane.stale_units, 0, "no phrase changed; a placement has no take");
    assert_eq!(view.render_state, VocalRenderState::Stale);

    // A render covers both placements, and the lane reads rendered again.
    call(
        &mut app,
        "vocal.render",
        &json!({ "track_id": TRACK, "section_id": verse }),
    );
    assert!(app.test_finish_vocal_render(verse, TRACK), "vocal.render queued a render");
    assert_eq!(app.test_vocal_audio_clips(TRACK).len(), 2);
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);
}

/// The MIDI clips on `track`, `(start_sample, notes)`, in timeline order.
fn clips_on(app: &Resonance, track: u64) -> Vec<(u64, Vec<(u8, u64, u64)>)> {
    let mut clips: Vec<_> = app
        .test_midi_clips()
        .iter()
        .filter(|c| c.track_id == track)
        .map(|c| {
            let notes = c.notes.iter().map(|n| (n.note, n.start_tick, n.duration_ticks));
            (c.start_sample, notes.collect())
        })
        .collect();
    clips.sort();
    clips
}

/// Where each of the lane's installed takes starts, in timeline order.
fn take_starts(app: &Resonance) -> Vec<u64> {
    let ids: Vec<u64> = app.test_vocal_audio_clips(TRACK).into_iter().map(|(_, c)| c).collect();
    let mut starts: Vec<u64> = app
        .test_clips()
        .iter()
        .filter(|c| ids.contains(&c.id))
        .map(|c| c.start_sample)
        .collect();
    starts.sort_unstable();
    starts
}

/// BUG-73 (install half): placing a section that already has generated
/// parts and a rendered vocal gives the new placement the same parts and
/// lays the existing take there — no render needed, `song.vocal` reads
/// `rendered` — and one undo takes back the placement with its clips.
#[test]
fn placing_a_section_again_installs_its_parts_and_its_take() {
    const BASS: u64 = 71;
    let mut app = app_with_project();
    app.test_set_sample_rate(48_000);
    app.test_set_flat_tempo(120.0);
    // Off bar 1, so neither take's pre-roll is cut at sample 0.
    let verse = rendered_lane(&mut app, "Verse", 2);
    app.test_add_track(BASS, TrackType::Instrument);
    call(
        &mut app,
        "generate.part",
        &json!({ "section_id": verse, "track_id": BASS, "role": "bass", "seed": 3 }),
    )
    .result::<serde_json::Value>()
    .expect("generate.part succeeds");
    // A take shaped like a real one: a lead-in and onset pre-roll.
    call(
        &mut app,
        "vocal.render",
        &json!({ "track_id": TRACK, "section_id": verse }),
    );
    assert!(app.test_finish_vocal_render_take(verse, TRACK, 2_400));
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);
    let bass_before = clips_on(&app, BASS);
    let vocal_before = clips_on(&app, TRACK);
    let take_before = take_starts(&app);
    assert_eq!((bass_before.len(), vocal_before.len(), take_before.len()), (1, 1, 1));

    place(&mut app, verse, 10);

    let bass = clips_on(&app, BASS);
    assert_eq!(bass.len(), 2, "the new placement has its bass part");
    assert_eq!(bass[0], bass_before[0], "the first placement's part is untouched");
    assert_eq!(bass[1].1, bass_before[0].1, "same notes at the new placement");
    let vocal = clips_on(&app, TRACK);
    assert_eq!(vocal.len(), 2, "the new placement has its vocal line");
    assert_eq!(vocal[1].1, vocal_before[0].1);
    // Eight bars at 120 BPM in 4/4 = 16 s later.
    let eight_bars = 16 * 48_000;
    assert_eq!(vocal[1].0, vocal[0].0 + eight_bars);

    let takes = take_starts(&app);
    assert_eq!(takes.len(), 2, "the take is laid at the new placement too");
    assert_eq!(takes[1], takes[0] + eight_bars, "at the same offset from its section start");
    let view = vocal_view(&mut app);
    assert_eq!(lane_state(&view, verse), VocalRenderState::Rendered, "no render needed");
    assert_eq!(view.render_state, VocalRenderState::Rendered);

    // One undo step: the placement and everything it brought go together.
    crate::common::roundtrip(&mut app, Request::without_params(98, "edit.undo"))
        .result::<serde_json::Value>()
        .expect("edit.undo succeeds");
    assert_eq!(clips_on(&app, BASS), bass_before);
    assert_eq!(clips_on(&app, TRACK), vocal_before);
    assert_eq!(take_starts(&app), take_before);
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);

    // Reopened, the take's layout is read back off its clip.
    let file = app.test_build_project_file();
    let midi_notes = app
        .test_midi_clips()
        .iter()
        .map(|c| (c.id, c.notes.clone()))
        .collect();
    app.test_replay_loaded_project_from(resonance_app::project::LoadedProject {
        file,
        project_dir: std::path::PathBuf::from("/tmp/control-vocal-render-state.rprj"),
        midi_notes,
        plugin_states: std::collections::HashMap::new(),
    });
    assert_eq!(take_starts(&app), take_before);
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);
    place(&mut app, verse, 10);
    assert_eq!(take_starts(&app), [take_before[0], take_before[0] + eight_bars]);
    assert_eq!(lane_state(&vocal_view(&mut app), verse), VocalRenderState::Rendered);
}

#[test]
fn a_track_with_no_lanes_reads_not_rendered() {
    let mut app = app_with_project();
    let view = vocal_view(&mut app);
    assert!(view.lanes.is_empty());
    assert_eq!(view.render_state, VocalRenderState::NotRendered);
}
