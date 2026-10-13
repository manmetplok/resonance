//! `vocal.get_params` / `vocal.set_params`, the `section.set_lane_generator`
//! vocal merge, `song.vocal`'s per-lane `params` and `lines`, and the
//! render-time voicebank rule (vocal-mcp-control-and-quality.md §3,
//! slices 1 and 3; BUG-52).

use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::job::JobStarted;
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::VocalView;
use resonance_control::methods::vocal as proto;
use resonance_control::{ErrorKind, KeyScale, Request, Response};
use resonance_music_theory::VoicebankId;
use serde_json::{json, Value};

use crate::common::roundtrip;

const TRACK: u64 = 60;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_install_shipped_voicebanks();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-vocal-params.rprj"));
    app.test_add_track(TRACK, TrackType::Vocal);
    app
}

fn call(app: &mut Resonance, method: &str, params: Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn expect_invalid(response: Response) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(
        error.kind(),
        ErrorKind::InvalidParams,
        "unexpected error kind: {}",
        error.message
    );
    error.message
}

/// A placed section at `start_bar` with an A-minor progression, returning
/// its definition id. No lane yet.
fn section(app: &mut Resonance, name: &str, start_bar: u32) -> u64 {
    let section_id = call(
        app,
        "section.create",
        json!({ "name": name, "length_bars": 4, "place": false }),
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    call(
        app,
        "section.place",
        json!({ "definition_id": section_id, "start_bar": start_bar }),
    )
    .result::<Value>()
    .expect("section.place succeeds");
    let mut progression = harmony_proto::ApplyProgressionParams::for_section(section_id);
    progression.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    progression.numerals = Some(["i", "iv", "v", "i"].map(str::to_owned).to_vec());
    roundtrip(
        app,
        Request::new(1, "harmony.apply_progression", &progression).unwrap(),
    )
    .result::<Value>()
    .expect("progression applies");
    u64::from(section_id)
}

/// Install a vocal lane over the wire with optional `options`.
fn set_lane(app: &mut Resonance, def: u64, options: Option<Value>, seed: Option<u64>) -> Response {
    let mut params = json!({ "section_id": def, "track_id": TRACK, "kind": "vocal" });
    if let Some(options) = options {
        params["options"] = options;
    }
    if let Some(seed) = seed {
        params["seed"] = json!(seed);
    }
    call(app, "section.set_lane_generator", params)
}

fn get_params(app: &mut Resonance, def: u64) -> proto::ParamsResult {
    call(
        app,
        "vocal.get_params",
        json!({ "track_id": TRACK, "section_id": def }),
    )
    .result()
    .expect("vocal.get_params succeeds")
}

fn set_params(app: &mut Resonance, def: u64, patch: Value) -> Response {
    call(
        app,
        "vocal.set_params",
        json!({ "track_id": TRACK, "section_id": def, "params": patch }),
    )
}

fn vocal_view(app: &mut Resonance) -> VocalView {
    call(app, "song.vocal", json!({ "track_id": TRACK }))
        .result()
        .expect("song.vocal succeeds")
}

fn one_lane_app() -> (Resonance, u64) {
    let mut app = app_with_project();
    let def = section(&mut app, "Verse", 1);
    set_lane(&mut app, def, None, None)
        .result::<Value>()
        .expect("vocal lane installs");
    (app, def)
}

#[test]
fn a_new_lane_starts_on_tiger_mystic() {
    let (mut app, def) = one_lane_app();
    let got = get_params(&mut app, def);
    assert_eq!(got.params.voicebank, "tiger");
    assert_eq!(
        got.params.speaker.as_deref(),
        Some("mystic"),
        "mystic whatever the voice type (an alto here)"
    );
    assert_eq!(u64::from(got.section_id), def);
}

#[test]
fn set_params_with_one_field_keeps_the_rest_and_song_vocal_echoes_it() {
    let (mut app, def) = one_lane_app();
    let before = get_params(&mut app, def);

    let reply: proto::ParamsResult = set_params(&mut app, def, json!({ "vibrato": 0.8 }))
        .result()
        .expect("vocal.set_params succeeds");
    assert_eq!(reply.params.vibrato, 0.8);
    assert!(reply.revision > before.revision, "a write bumps the revision");
    assert_eq!(reply.seed, before.seed, "the seed is untouched");

    // Every other field is exactly as it was.
    let mut expected = before.params.clone();
    expected.vibrato = 0.8;
    assert_eq!(reply.params, expected);
    assert_eq!(get_params(&mut app, def).params, expected);

    // song.vocal carries the same view per lane.
    let view = vocal_view(&mut app);
    assert_eq!(view.lanes.len(), 1);
    assert_eq!(view.lanes[0].params, expected);
    assert_eq!(view.lanes[0].voicebank.as_deref(), Some("tiger"));

    // One call is one undo entry.
    call(&mut app, "edit.undo", json!({}))
        .result::<Value>()
        .expect("edit.undo succeeds");
    assert_eq!(get_params(&mut app, def).params, before.params);
}

#[test]
fn set_params_rejects_invalid_values_at_set_time() {
    let (mut app, def) = one_lane_app();
    let before = get_params(&mut app, def);

    let message = expect_invalid(set_params(&mut app, def, json!({ "vibrato": 1.5 })));
    assert!(message.contains("vibrato"), "names the field: {message}");

    // Checked on the MERGED result: min above the lane's current max.
    let message = expect_invalid(set_params(&mut app, def, json!({ "syllables_min": 30 })));
    assert!(message.contains("syllables_min"), "names the field: {message}");

    let message = expect_invalid(set_params(
        &mut app,
        def,
        json!({ "range": { "low": 70, "high": 60 } }),
    ));
    assert!(message.contains("range"), "names the field: {message}");

    let message = expect_invalid(set_params(&mut app, def, json!({ "voicebank": "nightingale" })));
    assert!(message.contains("unknown voicebank"), "{message}");

    let message = expect_invalid(set_params(&mut app, def, json!({ "speaker": "phantom" })));
    assert!(message.contains("phantom") && message.contains("mystic"), "not tiger's: {message}");

    let message = expect_invalid(set_params(
        &mut app,
        def,
        json!({ "voicebank": "lilia", "speaker": "glam" }),
    ));
    assert!(message.contains("single voice"), "lilia refuses a speaker: {message}");

    // Wire-level refusals: unknown fields, lyrics, the old enum spelling.
    expect_invalid(set_params(&mut app, def, json!({ "draft": [] })));
    expect_invalid(set_params(&mut app, def, json!({ "style": "PopBallad" })));

    // A bad patch leaves the lane untouched, even its valid fields.
    expect_invalid(set_params(&mut app, def, json!({ "vibrato": 0.1, "breath": 2.0 })));
    let after = get_params(&mut app, def);
    assert_eq!(after.params, before.params);
    assert_eq!(after.revision, before.revision, "a refused call bumps nothing");
}

#[test]
fn a_bank_switch_resets_the_speaker_unless_the_patch_sets_one() {
    let (mut app, def) = one_lane_app();

    let reply: proto::ParamsResult = set_params(
        &mut app,
        def,
        json!({ "voicebank": "tiger", "speaker": "vinyl" }),
    )
    .result()
    .expect("tiger + vinyl");
    assert_eq!(reply.params.voicebank, "tiger");
    assert_eq!(reply.params.speaker.as_deref(), Some("vinyl"));

    // Speaker alone applies to the current bank.
    let reply: proto::ParamsResult = set_params(&mut app, def, json!({ "speaker": "royal" }))
        .result()
        .expect("speaker on tiger");
    assert_eq!(reply.params.speaker.as_deref(), Some("royal"));

    // Re-stating the same bank does not reset.
    let reply: proto::ParamsResult = set_params(&mut app, def, json!({ "voicebank": "tiger" }))
        .result()
        .expect("same bank");
    assert_eq!(reply.params.speaker.as_deref(), Some("royal"));

    // Switching does, to the new bank's default.
    let reply: proto::ParamsResult = set_params(&mut app, def, json!({ "voicebank": "meiji" }))
        .result()
        .expect("meiji");
    assert_eq!(reply.params.speaker.as_deref(), Some("standard"));
    let message = expect_invalid(set_params(&mut app, def, json!({ "speaker": "glam" })));
    assert!(message.contains("phantom"), "lists meiji's speakers: {message}");

    // Back to tiger: the default singer of the (patched) voice type.
    let reply: proto::ParamsResult = set_params(
        &mut app,
        def,
        json!({ "voicebank": "tiger", "voice": "soprano" }),
    )
    .result()
    .expect("tiger soprano");
    assert_eq!(reply.params.speaker.as_deref(), Some("glam"));
}

#[test]
fn snake_case_round_trips_through_get_and_set() {
    let (mut app, def) = one_lane_app();
    let raw: Value = call(
        &mut app,
        "vocal.get_params",
        json!({ "track_id": TRACK, "section_id": def }),
    )
    .result()
    .expect("vocal.get_params succeeds");
    let params = &raw["params"];
    assert_eq!(params["style"], json!("pop_ballad"));
    assert_eq!(params["pov"], json!("first_singular"));
    assert_eq!(params["voice"], json!("alto"));
    assert_eq!(params["rhyme"], json!("abab"));
    assert_eq!(params["range"]["low"], json!(55));
    assert!(params.get("draft").is_none(), "lyrics are not params");

    // The view is a valid patch: writing it back changes nothing.
    let before = get_params(&mut app, def);
    let reply: proto::ParamsResult = set_params(&mut app, def, params.clone())
        .result()
        .expect("the view is accepted as a patch");
    assert_eq!(reply.params, before.params);

    // And a non-default value of every enum survives the trip.
    let reply: proto::ParamsResult = set_params(
        &mut app,
        def,
        json!({
            "voice": "mezzo_soprano", "style": "anthemic", "contour": "wave",
            "syllable_mode": "melismatic", "mood": "melancholy",
            "pov": "third_person", "rhyme": "free", "timbre": "edged"
        }),
    )
    .result()
    .expect("enum patch");
    let wire = serde_json::to_value(&reply.params).unwrap();
    for (field, value) in [
        ("voice", "mezzo_soprano"),
        ("style", "anthemic"),
        ("contour", "wave"),
        ("syllable_mode", "melismatic"),
        ("mood", "melancholy"),
        ("pov", "third_person"),
        ("rhyme", "free"),
        ("timbre", "edged"),
    ] {
        assert_eq!(wire[field], json!(value), "{field}");
    }
}

#[test]
fn set_lane_generator_re_issue_merges_and_keeps_lyrics_and_seed() {
    let mut app = app_with_project();
    let def = section(&mut app, "Verse", 1);

    // A new lane: a partial patch, the rest default.
    set_lane(&mut app, def, Some(json!({ "style": "folk" })), Some(77))
        .result::<Value>()
        .expect("partial vocal options are accepted");
    let first = get_params(&mut app, def);
    assert_eq!(first.params.style, proto::VocalStyle::Folk);
    assert_eq!(first.params.voicebank, "tiger");
    assert_eq!(first.seed, 77);

    call(
        &mut app,
        "vocal.set_lyrics",
        json!({ "track_id": TRACK, "section_id": def, "text": "keep these words\nand these" }),
    )
    .result::<Value>()
    .expect("lyrics set");

    // Re-issue with another partial patch and no seed.
    set_lane(&mut app, def, Some(json!({ "vibrato": 0.9 })), None)
        .result::<Value>()
        .expect("re-issue succeeds");
    let second = get_params(&mut app, def);
    assert_eq!(second.seed, 77, "the seed is kept");
    assert_eq!(second.params.style, proto::VocalStyle::Folk, "earlier fields kept");
    assert_eq!(second.params.vibrato, 0.9);
    let view = vocal_view(&mut app);
    let texts: Vec<&str> = view.lanes[0].lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, vec!["keep these words", "and these"], "lyrics are kept");

    // No options at all leaves an existing lane alone.
    set_lane(&mut app, def, None, None)
        .result::<Value>()
        .expect("bare re-issue succeeds");
    assert_eq!(get_params(&mut app, def).params, second.params);

    // Validation applies here too.
    let message = expect_invalid(set_lane(&mut app, def, Some(json!({ "tension": 3.0 })), None));
    assert!(message.contains("tension"), "{message}");
}

/// BUG-52: `vocal.render` with no `voicebank` remapped a lane holding the
/// code default (TIGER) to Lilia, so explicitly choosing TIGER was
/// impossible to keep. The lane's bank is now always honoured.
#[test]
fn an_explicitly_chosen_tiger_survives_a_render_without_voicebank() {
    let (mut app, def) = one_lane_app();
    set_params(&mut app, def, json!({ "voicebank": "tiger" }))
        .result::<proto::ParamsResult>()
        .expect("choose tiger");
    call(
        &mut app,
        "vocal.generate",
        json!({ "track_id": TRACK, "section_id": def, "seed": 1, "lyrics": false }),
    )
    .result::<proto::GenerateResult>()
    .expect("vocal.generate succeeds");

    call(
        &mut app,
        "vocal.render",
        json!({ "track_id": TRACK, "section_id": def }),
    )
    .result::<JobStarted>()
    .expect("render returns a job");
    assert_eq!(app.test_vocal_voicebank(def, TRACK), Some(VoicebankId::new("tiger")));

    // The render's own voicebank takes the snake_case id too.
    call(
        &mut app,
        "vocal.render",
        json!({ "track_id": TRACK, "section_id": def, "voicebank": "meiji" }),
    )
    .result::<JobStarted>()
    .expect("render returns a job");
    assert_eq!(app.test_vocal_voicebank(def, TRACK), Some(VoicebankId::new("meiji")));
}

#[test]
fn song_vocal_lines_are_indexed_per_lane_like_set_line() {
    let mut app = app_with_project();
    let verse = section(&mut app, "Verse", 1);
    let chorus = section(&mut app, "Chorus", 5);
    for def in [verse, chorus] {
        set_lane(&mut app, def, None, None)
            .result::<Value>()
            .expect("vocal lane installs");
    }
    for (def, text) in [(verse, "verse one\nverse two"), (chorus, "hook one\nhook two")] {
        call(
            &mut app,
            "vocal.set_lyrics",
            json!({ "track_id": TRACK, "section_id": def, "text": text }),
        )
        .result::<Value>()
        .expect("lyrics set");
    }

    let view = vocal_view(&mut app);
    assert_eq!(view.lanes.len(), 2);
    for lane in &view.lanes {
        let indices: Vec<usize> = lane.lines.iter().map(|l| l.index).collect();
        assert_eq!(indices, vec![0, 1], "lane {:?} indexes from 0", lane.name);
    }
    let chorus_lane = view
        .lanes
        .iter()
        .find(|l| l.definition_id == SectionDefinitionId(chorus))
        .expect("chorus lane");

    // The index song.vocal reports is the one set_line takes for that lane.
    let target = &chorus_lane.lines[1];
    roundtrip(
        &mut app,
        Request::new(
            1,
            "vocal.set_line",
            &proto::SetLineParams {
                track_id: ProtoTrackId(TRACK),
                section_id: Some(SectionDefinitionId(chorus)),
                line_index: target.index,
                text: "hook new".to_owned(),
                syllabify: false,
            },
        )
        .unwrap(),
    )
    .result::<Value>()
    .expect("set_line succeeds");

    let view = vocal_view(&mut app);
    let texts = |def: u64| -> Vec<String> {
        view.lanes
            .iter()
            .find(|l| l.definition_id == SectionDefinitionId(def))
            .unwrap()
            .lines
            .iter()
            .map(|l| l.text.clone())
            .collect()
    };
    assert_eq!(texts(chorus), vec!["hook one", "hook new"]);
    assert_eq!(texts(verse), vec!["verse one", "verse two"]);
}

#[test]
fn get_params_on_a_track_without_a_vocal_lane_is_refused() {
    let mut app = app_with_project();
    let message = expect_invalid(call(
        &mut app,
        "vocal.get_params",
        json!({ "track_id": TRACK }),
    ));
    assert!(message.contains("no vocal lane"), "{message}");
}

/// `song.vocal`'s pre-flight counts notes the draft fills, not sung
/// syllables: `love _ _` is one syllable over three notes, so a three-note
/// lane is not a mismatch — and the `_` marks survive `vocal.generate`.
#[test]
fn a_held_syllable_fills_its_notes_without_a_counts_mismatch() {
    let (mut app, def) = one_lane_app();
    // On Lilia, whose dictionary respells English: the report still shows
    // the ARPAbet.
    set_params(&mut app, def, json!({ "voicebank": "lilia" }))
        .result::<proto::ParamsResult>()
        .expect("choose lilia");
    call(
        &mut app,
        "vocal.set_lyrics",
        json!({ "track_id": TRACK, "section_id": def, "text": "love _ _" }),
    )
    .result::<Value>()
    .expect("lyrics set");
    call(
        &mut app,
        "vocal.generate",
        json!({ "track_id": TRACK, "section_id": def, "seed": 3, "lyrics": false }),
    )
    .result::<proto::GenerateResult>()
    .expect("vocal.generate succeeds");

    let view = vocal_view(&mut app);
    let lane = &view.lanes[0];
    assert_eq!(lane.lines.len(), 1);
    assert_eq!(lane.lines[0].text, "love _ _", "the holds survive generate");
    assert_eq!(lane.syllable_count, 3, "one syllable plus two holds");
    assert_eq!(lane.note_count, 3, "the melody fills one note per unit");
    assert!(!lane.counts_mismatch, "{lane:?}");
    // The held word closes on the hold's last note (Lilia spells `v` as
    // her own `w`; the report shows the ARPAbet).
    let phonemes: Vec<Vec<&str>> = lane
        .notes
        .iter()
        .map(|n| n.phonemes.iter().map(String::as_str).collect())
        .collect();
    assert_eq!(phonemes, vec![vec!["l", "ah"], vec!["ah"], vec!["ah", "v"]]);
}

/// `vocal.get_params` only reads the open project, so like the other
/// reads it stays answerable while a GUI drag is open; `set_params` is an
/// edit and is refused `busy` (code review STATE2-05).
#[test]
fn get_params_answers_during_a_gui_drag() {
    use resonance_app::message::{Message, TransportMessage};
    use resonance_app::state::LoopDragTarget;

    let (mut app, def) = one_lane_app();
    let _ = app.update(Message::Transport(TransportMessage::StartLoopDrag(
        LoopDragTarget::Out,
    )));
    let got = get_params(&mut app, def);
    assert_eq!(got.params.voicebank, "tiger");

    let refused = set_params(&mut app, def, json!({ "vibrato": 0.2 }));
    let error = refused.error.expect("an edit mid-drag is refused");
    assert_eq!(error.kind(), ErrorKind::Busy, "{}", error.message);
    let _ = app.update(Message::Transport(TransportMessage::EndLoopDrag));
}

/// The GUI caps the theme at 240 characters; over the wire a longer one
/// is refused, naming the field, rather than silently truncated.
#[test]
fn set_params_refuses_a_theme_over_the_gui_limit() {
    let (mut app, def) = one_lane_app();
    let reply: proto::ParamsResult = set_params(&mut app, def, json!({ "theme": "a".repeat(240) }))
        .result()
        .expect("240 characters is accepted");
    assert_eq!(reply.params.theme.chars().count(), 240);

    // Characters, not bytes: 240 two-byte characters still fit.
    set_params(&mut app, def, json!({ "theme": "é".repeat(240) }))
        .result::<proto::ParamsResult>()
        .expect("240 two-byte characters are accepted");

    let message = expect_invalid(set_params(&mut app, def, json!({ "theme": "a".repeat(241) })));
    assert!(message.contains("theme"), "names the field: {message}");
    assert_eq!(get_params(&mut app, def).params.theme, "é".repeat(240), "lane untouched");
}

/// `vocal.render {voicebank}` switching the bank resets the speaker the
/// same way `vocal.set_params` does.
#[test]
fn a_render_that_switches_the_bank_resets_the_speaker() {
    let (mut app, def) = one_lane_app();
    set_params(&mut app, def, json!({ "voicebank": "meiji", "speaker": "phantom" }))
        .result::<proto::ParamsResult>()
        .expect("meiji phantom");
    call(
        &mut app,
        "vocal.generate",
        json!({ "track_id": TRACK, "section_id": def, "seed": 1, "lyrics": false }),
    )
    .result::<proto::GenerateResult>()
    .expect("vocal.generate succeeds");
    let render = |app: &mut Resonance, bank: &str| {
        call(
            app,
            "vocal.render",
            json!({ "track_id": TRACK, "section_id": def, "voicebank": bank }),
        )
        .result::<JobStarted>()
        .expect("render returns a job");
    };

    // Re-rendering on the same bank keeps the chosen speaker.
    render(&mut app, "meiji");
    assert_eq!(get_params(&mut app, def).params.speaker.as_deref(), Some("phantom"));

    // Switching banks resets to the new bank's default — the same value a
    // `set_params` bank switch lands on.
    render(&mut app, "tiger");
    let after_render = get_params(&mut app, def).params.speaker;
    set_params(&mut app, def, json!({ "voicebank": "meiji" }))
        .result::<proto::ParamsResult>()
        .expect("back to meiji");
    let via_set_params: proto::ParamsResult = set_params(&mut app, def, json!({ "voicebank": "tiger" }))
        .result()
        .expect("back to tiger");
    assert_eq!(after_render, via_set_params.params.speaker);

    set_params(&mut app, def, json!({ "speaker": "royal" }))
        .result::<proto::ParamsResult>()
        .expect("royal");
    render(&mut app, "meiji");
    assert_eq!(get_params(&mut app, def).params.speaker.as_deref(), Some("standard"));
}

/// `rest_threshold_beats` and `velocity_dynamics` (spec §8, §9): default,
/// set, echoed by `get_params` and `song.vocal`, range-checked.
#[test]
fn rest_threshold_and_velocity_dynamics_round_trip_and_are_range_checked() {
    let (mut app, def) = one_lane_app();
    let before = get_params(&mut app, def);
    assert_eq!(before.params.rest_threshold_beats, 0.5);
    assert_eq!(before.params.velocity_dynamics, 0.5);

    let reply: proto::ParamsResult = set_params(
        &mut app,
        def,
        json!({ "rest_threshold_beats": 1.5, "velocity_dynamics": 0.8 }),
    )
    .result()
    .expect("vocal.set_params succeeds");
    assert_eq!(reply.params.rest_threshold_beats, 1.5);
    assert_eq!(reply.params.velocity_dynamics, 0.8);
    let mut expected = before.params.clone();
    expected.rest_threshold_beats = 1.5;
    expected.velocity_dynamics = 0.8;
    assert_eq!(reply.params, expected, "nothing else moved");
    assert_eq!(get_params(&mut app, def).params, expected);
    assert_eq!(vocal_view(&mut app).lanes[0].params, expected);

    // The ends of the ranges are accepted.
    for (field, value) in [
        ("rest_threshold_beats", 0.125),
        ("rest_threshold_beats", 4.0),
        ("velocity_dynamics", 0.0),
        ("velocity_dynamics", 1.0),
    ] {
        set_params(&mut app, def, json!({ field: value }))
            .result::<proto::ParamsResult>()
            .unwrap_or_else(|e| panic!("{field} = {value} is in range: {e:?}"));
    }
    for (field, value) in [
        ("rest_threshold_beats", 0.1),
        ("rest_threshold_beats", 4.5),
        ("velocity_dynamics", -0.1),
        ("velocity_dynamics", 1.1),
    ] {
        let message = expect_invalid(set_params(&mut app, def, json!({ field: value })));
        assert!(message.contains(field), "{field} = {value} names the field: {message}");
    }
}

/// `bank_timing` / `bank_pitch`: on by default, set by a patch, echoed by
/// `get_params` and `song.vocal`, nothing else moved.
#[test]
fn bank_timing_and_bank_pitch_round_trip() {
    let (mut app, def) = one_lane_app();
    let before = get_params(&mut app, def);
    assert!(before.params.bank_timing && before.params.bank_pitch);

    let reply: proto::ParamsResult = set_params(
        &mut app,
        def,
        json!({ "bank_timing": false, "bank_pitch": false }),
    )
    .result()
    .expect("vocal.set_params succeeds");
    let mut expected = before.params.clone();
    expected.bank_timing = false;
    expected.bank_pitch = false;
    assert_eq!(reply.params, expected, "nothing else moved");
    assert_eq!(get_params(&mut app, def).params, expected);
    assert_eq!(vocal_view(&mut app).lanes[0].params, expected);

    let reply: proto::ParamsResult = set_params(&mut app, def, json!({ "bank_pitch": true }))
        .result()
        .expect("vocal.set_params succeeds");
    assert!(!reply.params.bank_timing && reply.params.bank_pitch);
}
