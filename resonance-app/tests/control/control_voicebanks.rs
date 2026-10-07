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
    app.test_without_dev_voicebanks();
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
    // Development models only by explicit opt-in: the source tree's
    // `resonance-svs/models/` is never scanned implicitly.
    let opted_in: Vec<std::path::PathBuf> = std::env::var_os("RESONANCE_SVS_MODELS_DIR")
        .filter(|v| !v.is_empty())
        .map(Into::into)
        .into_iter()
        .collect();
    assert_eq!(roots.dev_models, opted_in);
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
    assert_eq!(lilia.missing_phonemes, ["v"]);
    assert_eq!(lilia.substitutions[0].sung_as, "f", "Lilia's own hint");
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
