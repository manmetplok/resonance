//! The voicebank registry over the wire (vocal-mcp-control-and-quality.md
//! §4, slice 4): `vocal.voicebanks` lists what is installed and follows
//! the folder; `vocal.set_params` accepts installed (or known) English
//! banks only; `vocal.render` and `vocal.generate` report a missing bank
//! as `voicebank_missing`, naming it.

use resonance_app::state::ViewMode;
use resonance_app::{Resonance, VoicebankFixture};
use resonance_audio::types::TrackType;
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::methods::vocal as proto;
use resonance_control::{ErrorKind, KeyScale, Request, Response};
use serde_json::{json, Value};

use crate::common::roundtrip;

const TRACK: u64 = 61;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-voicebanks.rprj"));
    app.test_add_track(TRACK, TrackType::Vocal);
    app
}

fn call(app: &mut Resonance, method: &str, params: Value) -> Response {
    roundtrip(
        app,
        Request::new(1, method, &params).expect("params serialize"),
    )
}

fn voicebanks(app: &mut Resonance) -> proto::VoicebankList {
    call(app, proto::VOICEBANKS, Value::Null)
        .result()
        .expect("vocal.voicebanks succeeds")
}

/// A placed, chorded section with a vocal lane on `TRACK`.
fn vocal_lane(app: &mut Resonance) -> u64 {
    let section_id = call(
        app,
        "section.create",
        json!({ "name": "Verse", "length_bars": 4, "place": false }),
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create")
    .section_id;
    call(
        app,
        "section.place",
        json!({ "definition_id": section_id, "start_bar": 1 }),
    )
    .result::<Value>()
    .expect("section.place");
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
    let def = u64::from(section_id);
    call(
        app,
        "section.set_lane_generator",
        json!({ "section_id": def, "track_id": TRACK, "kind": "vocal" }),
    )
    .result::<Value>()
    .expect("vocal lane installs");
    def
}

fn set_params(app: &mut Resonance, def: u64, patch: Value) -> Response {
    call(
        app,
        "vocal.set_params",
        json!({ "track_id": TRACK, "section_id": def, "params": patch }),
    )
}

fn error_of(response: Response) -> (ErrorKind, String) {
    let error = response.error.expect("expected an error reply");
    (error.kind(), error.message)
}

#[test]
fn a_test_app_sees_only_its_own_voicebank_folder() {
    let (app, _task) = Resonance::new_for_test();
    let roots = app.test_voicebank_roots();
    let hermetic = resonance_app::user_dirs::hermetic_root().expect("test app is hermetic");
    let banks = roots.banks.expect("a user folder");
    assert!(
        banks.starts_with(hermetic),
        "{banks:?} is under {hermetic:?}"
    );
    assert!(roots.vocoders.unwrap().starts_with(hermetic));
    // No development root, whatever the environment: neither the source
    // tree's `resonance-svs/models/` nor `$RESONANCE_SVS_MODELS_DIR`.
    assert!(roots.dev_models.is_empty(), "{:?}", roots.dev_models);
}

/// A test opts in to the developer's real banks explicitly; without the
/// variable the opt-in changes nothing.
#[test]
fn a_test_opts_in_to_the_dev_banks_explicitly() {
    let (mut app, _task) = Resonance::new_for_test();
    let opted = app.test_install_dev_voicebanks_from_env();
    let env = resonance_app::compose::vocal_svs::VoicebankRoots::dev_models_from_env();
    assert_eq!(opted, env.is_some());
    assert_eq!(app.test_voicebank_roots().dev_models, env.into_iter().collect::<Vec<_>>());
}

/// `$RESONANCE_VOICEBANK_DIR` alone moves the vocoders with it, into the
/// `vocoders/` folder beside it (§4.2); `$RESONANCE_VOCODER_DIR` still
/// wins, and with neither both sit in the data dir.
#[test]
fn the_vocoder_folder_follows_the_voicebank_override() {
    use resonance_app::compose::vocal_svs::VoicebankRoots;
    use std::path::{Path, PathBuf};
    let data = Path::new("/data/resonance");
    let banks = PathBuf::from("/elsewhere/voicebanks");

    let roots = VoicebankRoots::from_dirs(Some(banks.clone()), None, Some(data), Vec::new());
    assert_eq!(roots.banks.as_deref(), Some(banks.as_path()));
    assert_eq!(roots.vocoders.as_deref(), Some(Path::new("/elsewhere/vocoders")));

    let own = PathBuf::from("/vocoders/here");
    let roots = VoicebankRoots::from_dirs(Some(banks), Some(own.clone()), Some(data), Vec::new());
    assert_eq!(roots.vocoders, Some(own));

    let roots = VoicebankRoots::from_dirs(None, None, Some(data), Vec::new());
    assert_eq!(roots.banks.as_deref(), Some(Path::new("/data/resonance/voicebanks")));
    assert_eq!(roots.vocoders.as_deref(), Some(Path::new("/data/resonance/vocoders")));
}

/// A folder whose name slugs to nothing takes its id from the character
/// file; two folders with one id keep the first (sorted); a user bank
/// beats a development root's bank of the same id.
#[test]
fn bank_ids_fall_back_to_the_character_name_and_duplicates_keep_the_first() {
    let mut app = app_with_project();
    let user = app.test_voicebank_roots().banks.expect("a user folder");
    let named = |name: &str| VoicebankFixture {
        name: Some(name.to_owned()),
        ..VoicebankFixture::default()
    };
    resonance_app::write_voicebank_fixture(&user.join("日本語"), &named("Kiyo Voice"));
    resonance_app::write_voicebank_fixture(&user.join("___"), &VoicebankFixture::default());
    resonance_app::write_voicebank_fixture(&user.join("Duo!"), &named("First"));
    resonance_app::write_voicebank_fixture(&user.join("duo"), &named("Second"));

    let list = voicebanks(&mut app);
    let ids: Vec<&str> = list.installed.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["duo", "kiyo-voice"], "the nameless folder is skipped");
    let duo = list.installed.iter().find(|b| b.id == "duo").unwrap();
    assert_eq!(duo.name, "First", "`Duo!` sorts before `duo`");

    // A development root's Lilia is shadowed by the user's.
    let dev = std::env::temp_dir().join(format!("resonance-dev-models-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dev);
    resonance_app::write_voicebank_fixture(&dev.join("voicebanks/lilia"), &named("Dev Lilia"));
    app.test_install_voicebank("lilia", &VoicebankFixture::shipped("lilia"));
    let roots = resonance_app::compose::vocal_svs::VoicebankRoots {
        dev_models: vec![dev.clone()],
        ..app.test_voicebank_roots()
    };
    app.test_set_voicebank_roots(roots);
    let lilia = app.test_voicebank_profile("lilia");
    assert!(
        lilia.installed().unwrap().root.starts_with(&user),
        "the user's copy wins"
    );
    std::fs::remove_dir_all(&dev).unwrap();
}

/// A rescan keeps an unchanged bank's `Arc` (and so its parsed
/// dictionary) and re-reads one whose files changed.
#[test]
fn a_rescan_reuses_an_unchanged_bank() {
    let mut app = app_with_project();
    let dir = app.test_install_voicebank("keiro", &VoicebankFixture::default());
    let first = app.test_voicebank_profile("keiro");
    app.test_rescan_voicebanks();
    let again = app.test_voicebank_profile("keiro");
    assert!(
        std::ptr::eq(first.installed().unwrap(), again.installed().unwrap()),
        "unchanged: the same bank"
    );

    // A dictionary dropped in later changes the bank.
    resonance_app::write_voicebank_fixture(
        &dir,
        &VoicebankFixture {
            english_dict: vec![("keiro".to_owned(), vec!["k".to_owned(), "ey".to_owned()])],
            ..VoicebankFixture::default()
        },
    );
    app.test_rescan_voicebanks();
    let changed = app.test_voicebank_profile("keiro");
    assert!(!std::ptr::eq(first.installed().unwrap(), changed.installed().unwrap()));
    assert!(changed.dictionary().is_some(), "the new dictionary is read");
}

#[test]
fn voicebanks_lists_a_dropped_in_bank_and_drops_it_when_deleted() {
    let mut app = app_with_project();
    assert!(
        voicebanks(&mut app).installed.is_empty(),
        "a fresh test app has none"
    );

    let dir = app.test_user_voicebank_dir("peiton");
    resonance_app::write_voicebank_fixture(
        &dir,
        &VoicebankFixture {
            speakers: vec!["embeds/soft".into(), "embeds/power".into()],
            use_tension_embed: true,
            name: Some("Peiton DS".into()),
            ..VoicebankFixture::default().without(&["zh"])
        },
    );

    // The call itself rescans: no restart, no explicit rescan.
    let list = voicebanks(&mut app);
    assert!(list.available.is_empty());
    let [bank] = list.installed.as_slice() else {
        panic!("one bank: {:?}", list.installed);
    };
    assert_eq!(bank.id, "peiton");
    assert_eq!(bank.name, "Peiton DS", "the character file's name");
    assert_eq!(bank.version.as_deref(), Some("v1"));
    assert!(bank.english);
    let speakers: Vec<&str> = bank.speakers.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(speakers, ["soft", "power"]);
    assert_eq!(bank.default_speaker.as_deref(), Some("soft"));
    assert_eq!(bank.comfortable_range.range_source, "default");
    assert_eq!(
        (bank.comfortable_range.low, bank.comfortable_range.high),
        (48, 76)
    );
    assert_eq!(bank.missing_phonemes, ["zh"]);
    assert_eq!(bank.substitutions.len(), 1);
    assert_eq!(bank.substitutions[0].phoneme, "zh");
    assert_eq!(bank.substitutions[0].sung_as, "sh", "the voiceless partner");
    assert!(bank.curves.tension && bank.curves.pitch_bend);
    assert!(!bank.curves.dynamics && !bank.curves.breathiness);
    assert_eq!(bank.models, proto::VoicebankModels::default(), "ships none of its own models");
    assert_eq!(bank.vocoder.as_deref(), Some("fixture_vocoder"));
    assert!(bank.vocoder_installed);
    assert_eq!(bank.credit, None, "an unknown bank has no terms");

    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        voicebanks(&mut app).installed.is_empty(),
        "deleted means gone"
    );
}

#[test]
fn voicebanks_needs_no_project_and_bumps_no_revision() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let before = app.revision();
    let response = call(&mut app, proto::VOICEBANKS, Value::Null);
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(app.revision(), before);
}

#[test]
fn the_built_in_banks_report_their_terms_and_speakers() {
    let mut app = app_with_project();
    app.test_install_shipped_voicebanks();
    let list = voicebanks(&mut app);
    let ids: Vec<&str> = list.installed.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(ids, ["lilia", "meiji", "tiger"]);
    let bank = |id: &str| list.installed.iter().find(|b| b.id == id).unwrap();

    let tiger = bank("tiger");
    let modes: Vec<&str> = tiger.speakers.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        modes,
        ["glam", "fresh", "disco", "royal", "electric", "mystic", "vinyl"],
        "only TIGER's own modes, not the shared model's other speakers"
    );
    assert_eq!(tiger.commercial.as_deref(), Some("paid"));
    assert_eq!(tiger.comfortable_range.range_source, "builtin");

    let lilia = bank("lilia");
    assert!(lilia.speakers.is_empty());
    assert_eq!(lilia.credit.as_deref(), Some("LIEE"));
    assert_eq!(lilia.commercial.as_deref(), Some("free_with_credit"));
    // Her dictionary's replacements spell every English phone (`v` as her
    // own `w`), so nothing is missing and nothing is substituted.
    assert!(lilia.missing_phonemes.is_empty(), "{:?}", lilia.missing_phonemes);
    assert!(lilia.substitutions.is_empty(), "{:?}", lilia.substitutions);
    assert!(lilia.curves.tension && !tiger.curves.tension);

    let meiji = bank("meiji");
    assert_eq!(
        meiji.credit.as_deref(),
        Some("Gahata Meiji from LUNAI Project")
    );
    assert_eq!(meiji.commercial.as_deref(), Some("permission"));
    assert!(
        meiji.missing_phonemes.is_empty(),
        "its en/ namespace counts"
    );
}

#[test]
fn set_params_accepts_an_installed_bank_and_its_speakers() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    app.test_install_voicebank(
        "keiro",
        &VoicebankFixture {
            speakers: vec!["embeds/calm".into(), "embeds/rough".into()],
            ..VoicebankFixture::default()
        },
    );

    let got: proto::ParamsResult = set_params(&mut app, def, json!({ "voicebank": "Keiro" }))
        .result()
        .expect("an installed bank is accepted");
    assert_eq!(got.params.voicebank, "keiro");
    assert_eq!(
        got.params.speaker.as_deref(),
        Some("calm"),
        "the bank's default"
    );

    let got: proto::ParamsResult = set_params(&mut app, def, json!({ "speaker": "rough" }))
        .result()
        .expect("one of its speakers");
    assert_eq!(got.params.speaker.as_deref(), Some("rough"));

    let (kind, message) = error_of(set_params(&mut app, def, json!({ "speaker": "disco" })));
    assert_eq!(kind, ErrorKind::InvalidParams);
    assert!(
        message.contains("calm") && message.contains("rough"),
        "{message}"
    );
}

#[test]
fn set_params_refuses_an_unknown_id_but_accepts_a_known_one_not_installed() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);

    let (kind, message) = error_of(set_params(&mut app, def, json!({ "voicebank": "tigr" })));
    assert_eq!(kind, ErrorKind::InvalidParams);
    assert!(
        message.contains("tigr") && message.contains("tiger"),
        "{message}"
    );

    // A built-in bank is a valid choice before it is installed: the
    // render names it as missing then.
    let got: proto::ParamsResult = set_params(&mut app, def, json!({ "voicebank": "meiji" }))
        .result()
        .expect("a known bank");
    assert_eq!(got.params.speaker.as_deref(), Some("standard"));
}

#[test]
fn set_params_refuses_a_bank_that_cannot_sing_english() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    app.test_install_voicebank(
        "kana",
        &VoicebankFixture {
            phonemes: [
                "AP", "SP", "a", "i", "u", "e", "o", "k", "s", "t", "n", "N", "cl",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            ..VoicebankFixture::default()
        },
    );
    let listed = voicebanks(&mut app);
    let kana = listed
        .installed
        .iter()
        .find(|b| b.id == "kana")
        .expect("listed");
    assert!(!kana.english, "listed, but flagged");

    let (kind, message) = error_of(set_params(&mut app, def, json!({ "voicebank": "kana" })));
    assert_eq!(kind, ErrorKind::InvalidParams);
    assert!(message.contains("English"), "{message}");
}

/// Generate a melody on the lane (lilia, not installed), returning the
/// result.
fn generate(app: &mut Resonance) -> proto::GenerateResult {
    call(
        app,
        "vocal.generate",
        json!({ "track_id": TRACK, "seed": 7 }),
    )
    .result()
    .expect("vocal.generate succeeds: the notes are generated either way")
}

#[test]
fn render_with_a_missing_bank_fails_naming_it() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    generate(&mut app);
    let (kind, message) = error_of(call(
        &mut app,
        "vocal.render",
        json!({ "track_id": TRACK, "section_id": def }),
    ));
    assert_eq!(kind, ErrorKind::VoicebankMissing);
    assert!(message.contains("\"lilia\""), "names the bank: {message}");
    assert!(message.contains("vocal_voicebanks"), "{message}");
}

#[test]
fn render_with_a_missing_vocoder_fails_naming_the_vocoder() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    app.test_install_voicebank(
        "lilia",
        &VoicebankFixture {
            vocoder: false,
            ..VoicebankFixture::shipped("lilia")
        },
    );
    generate(&mut app);
    let (kind, message) = error_of(call(
        &mut app,
        "vocal.render",
        json!({ "track_id": TRACK, "section_id": def }),
    ));
    assert_eq!(kind, ErrorKind::VoicebankMissing);
    assert!(
        message.contains("vocoder") && message.contains("dsvocoder"),
        "{message}"
    );
}

#[test]
fn generate_reports_the_missing_bank_instead_of_succeeding_silently() {
    let mut app = app_with_project();
    vocal_lane(&mut app);
    let result = generate(&mut app);
    let error = result
        .render_error
        .expect("the render step's failure is reported");
    assert_eq!(error.kind, ErrorKind::VoicebankMissing);
    assert!(error.message.contains("lilia"), "{}", error.message);

    // With the bank installed, generate reports nothing.
    app.test_install_shipped_voicebanks();
    assert_eq!(generate(&mut app).render_error, None);
}

/// A render that would fail on a missing bank must not take the lane's
/// old audio with it: the GUI re-render and `vocal.generate`'s
/// auto-render both check the bank before queuing anything. `generate`
/// still writes its melody and reports the bank as `render_error`.
#[test]
fn a_render_with_a_missing_bank_keeps_the_old_audio() {
    use resonance_app::compose::messages::LaneInspectorMsg;
    use resonance_app::compose::ComposeMessage;
    use resonance_app::message::Message;

    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    generate(&mut app);
    let placement = app.compose_state().placements[0].id;
    let old_audio = std::env::temp_dir().join("resonance-missing-bank-old-take.wav");
    app.test_install_vocal_audio_clip(def, placement, TRACK, 900_777, old_audio);
    let epoch = app.test_vocal_render_epoch(def, TRACK);
    let rerender = Message::Compose(ComposeMessage::LaneInspector {
        definition_id: def,
        track_id: TRACK,
        msg: LaneInspectorMsg::RerenderVocalAudio,
    });

    let _ = app.update(rerender.clone());
    assert_eq!(app.test_vocal_audio_clips(TRACK), [(def, 900_777)], "old audio kept");
    assert_eq!(app.test_vocal_render_epoch(def, TRACK), epoch, "nothing queued");
    let banner = app.compose_state().last_error.clone().expect("a banner");
    assert!(banner.contains("\"lilia\"") && banner.contains("voicebanks"), "{banner}");

    let result = generate(&mut app);
    assert_eq!(
        result.render_error.map(|e| e.kind),
        Some(ErrorKind::VoicebankMissing)
    );
    assert_eq!(app.test_vocal_audio_clips(TRACK), [(def, 900_777)], "still kept");

    // Installed, the same render is queued; the old take plays until the
    // new one lands, and is replaced then (spec §7, D22).
    app.test_install_shipped_voicebanks();
    let _ = app.update(rerender);
    assert_ne!(app.test_vocal_render_epoch(def, TRACK), epoch, "a render is queued");
    assert_eq!(app.test_vocal_audio_clips(TRACK), [(def, 900_777)], "plays while rendering");
    assert!(app.test_finish_vocal_render(def, TRACK));
    let now = app.test_vocal_audio_clips(TRACK);
    assert_eq!(now.len(), 1, "{now:?}");
    assert_ne!(now[0].1, 900_777, "the new take replaced the old one");
}

#[test]
fn render_naming_an_unknown_bank_is_a_typo_not_a_missing_bank() {
    let mut app = app_with_project();
    vocal_lane(&mut app);
    generate(&mut app);
    let (kind, _) = error_of(call(
        &mut app,
        "vocal.render",
        json!({ "track_id": TRACK, "voicebank": "nope" }),
    ));
    assert_eq!(kind, ErrorKind::InvalidParams);
}

/// Give the lane `notes` quarter notes at its placement.
fn add_clip(app: &mut Resonance, notes: u64) {
    use resonance_audio::types::{AudioEvent, MidiNote, TICKS_PER_QUARTER_NOTE};
    let bar = app.compose_state().placements[0].start_bar;
    let start_sample = app.test_tempo_map().bar_to_sample(bar);
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id: 900_061,
        track_id: TRACK,
        start_sample,
        duration_ticks: 4 * 4 * TICKS_PER_QUARTER_NOTE,
        name: "Verse · Vox".to_owned(),
        notes: (0..notes)
            .map(|i| MidiNote {
                note: 64,
                velocity: 0.8,
                start_tick: i * TICKS_PER_QUARTER_NOTE,
                duration_ticks: TICKS_PER_QUARTER_NOTE,
            })
            .collect(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
}

/// `song.vocal` resolves its lines, its counts and its notes through the
/// same dictionary layers — the lane's bank's own dictionary included —
/// so a word the bank reads differently from CMU reads the same in all
/// three, and the same as the render sings it.
#[test]
fn song_vocal_lines_and_notes_agree_on_a_bank_dictionary_word() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    // CMU sings `tomato` with `ey`; this Lilia reads it with `aa`.
    app.test_install_voicebank(
        "lilia",
        &VoicebankFixture {
            english_dict: vec![(
                "tomato".to_owned(),
                ["t", "ah", "m", "aa", "t", "ow"].map(str::to_owned).to_vec(),
            )],
            ..VoicebankFixture::shipped("lilia")
        },
    );
    call(
        &mut app,
        "vocal.set_lyrics",
        json!({ "track_id": TRACK, "section_id": def, "text": "to·ma·to" }),
    )
    .result::<Value>()
    .expect("lyrics set");
    add_clip(&mut app, 3);

    let view: resonance_control::methods::song::VocalView =
        call(&mut app, "song.vocal", json!({ "track_id": TRACK }))
            .result()
            .expect("song.vocal succeeds");
    let lane = &view.lanes[0];
    let from_lines: Vec<Vec<String>> = lane.lines[0]
        .syllables
        .iter()
        .map(|s| s.phonemes.clone())
        .collect();
    let from_notes: Vec<Vec<String>> = lane.notes.iter().map(|n| n.phonemes.clone()).collect();
    assert_eq!(from_lines, from_notes);
    assert_eq!(from_lines[1], ["m", "aa"], "the bank's reading, not CMU's");
    assert_eq!(lane.syllable_count, 3);
    assert!(!lane.counts_mismatch, "{lane:?}");
}

/// `vocal.voicebanks` says which of its own models (duration, pitch,
/// variance) a bank ships beside the acoustic one — what a lane's
/// `bank_timing` / `bank_pitch` use.
#[test]
fn voicebanks_reports_the_banks_own_models() {
    let mut app = app_with_project();
    let dir = app.test_user_voicebank_dir("modelled");
    resonance_app::write_voicebank_fixture(
        &dir,
        &VoicebankFixture {
            bank_models: true,
            ..VoicebankFixture::default()
        },
    );
    let list = voicebanks(&mut app);
    let bank = list.installed.iter().find(|b| b.id == "modelled").expect("listed");
    assert_eq!(
        bank.models,
        proto::VoicebankModels {
            duration: true,
            pitch: true,
            variance: true,
        }
    );
    // Without its pitch model it reports the others.
    std::fs::remove_file(dir.join("dspitch/pitch.onnx")).unwrap();
    let list = voicebanks(&mut app);
    let bank = list.installed.iter().find(|b| b.id == "modelled").expect("listed");
    assert!(bank.models.duration && !bank.models.pitch && bank.models.variance);
}
