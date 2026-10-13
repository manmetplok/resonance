//! `vocal.voice_presets` / `vocal.apply_voice_preset` /
//! `vocal.save_voice_preset` (`docs/features/compose/voice-presets.md`),
//! and the GUI's pick through the same lane edit.

use resonance_app::message::{Message, VoicePresetMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_control::methods::song::VocalRenderState;
use resonance_control::methods::vocal as proto;
use resonance_control::ErrorKind;
use resonance_music_theory::{VocalCurvePoint, VocalStyle, VoicebankId};
use serde_json::json;

use crate::control_vocal_render_state::{
    app_with_project, call, lane_state, rendered_lane, vocal_view, TRACK,
};

fn presets(app: &mut Resonance) -> Vec<proto::VoicePresetView> {
    call(app, proto::VOICE_PRESETS, &json!({}))
        .result::<proto::VoicePresetList>()
        .expect("vocal.voice_presets succeeds")
        .presets
}

fn apply(app: &mut Resonance, def: u64, preset: &str) -> resonance_control::Response {
    call(
        app,
        proto::APPLY_VOICE_PRESET,
        &json!({ "track_id": TRACK, "section_id": def, "preset": preset }),
    )
}

#[test]
fn voice_presets_lists_the_built_ins_best_first_without_a_project() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_install_shipped_voicebanks();
    let list = presets(&mut app);
    assert_eq!(list.len(), 31, "no user presets yet");
    assert_eq!(list[0].id, "tiger-mystic");
    assert_eq!(list[0].score, Some(5));
    assert!(list.iter().all(|p| p.builtin && p.score.is_some()));
    assert!(
        list.windows(2).all(|w| w[0].score >= w[1].score),
        "best first"
    );
    // The test app has the three shipped banks; the LUNAI singers are not.
    let by_id = |id: &str| list.iter().find(|p| p.id == id).unwrap();
    assert!(by_id("tiger-mystic").installed);
    assert!(by_id("lilia").installed);
    assert!(!by_id("katyusha-standard").installed);
    let rock = by_id("liam_thorne-rock");
    assert_eq!(rock.voicebank, "liam_thorne");
    assert_eq!(rock.speaker.as_deref(), Some("rock"));
    assert_eq!(rock.settings.voice, proto::VocalVoice::Baritone);
    assert_eq!(
        (rock.settings.range.low, rock.settings.range.high),
        (45, 65)
    );
    assert!(rock.tags.contains(&"aggressive".to_owned()));
    assert_eq!(by_id("lilia").speaker, None);
}

#[test]
fn apply_sets_the_voice_and_delivery_and_nothing_else_in_one_undo_step() {
    let mut app = app_with_project();
    let def = rendered_lane(&mut app, "Verse", 1);
    // A finished part: its own style, theme, curve and lyrics.
    let mut before = app.test_vocal_lane_params(def, TRACK).unwrap();
    before.style = VocalStyle::Folk;
    before.theme = "kept".into();
    before.curves.tension.push(VocalCurvePoint {
        beat: 2.0,
        value: 0.4,
        stepped: false,
    });
    app.test_set_vocal_lane_params(def, TRACK, before.clone());
    let undo_len = app.test_undo_history().undo_len();
    let revision = app.revision();

    let result: proto::ApplyVoicePresetResult = apply(&mut app, def, "lilia")
        .result()
        .expect("vocal.apply_voice_preset succeeds");
    assert_eq!(result.preset_id, "lilia");
    assert_eq!(result.preset_name, "LIEE Lilia");
    assert!(result.installed);
    assert_eq!(result.warning, None);
    assert_eq!(result.params.voicebank, "lilia");
    assert_eq!(result.params.speaker, None);
    assert_eq!(result.revision, revision + 1, "one revision bump");
    assert_eq!(
        app.test_undo_history().undo_len(),
        undo_len + 1,
        "one undo entry"
    );

    let after = app.test_vocal_lane_params(def, TRACK).unwrap();
    let preset = resonance_app::compose::vocal_svs::voice_presets::builtin_presets()
        .iter()
        .find(|p| p.id == "lilia")
        .unwrap();
    let mut expected = before.clone();
    preset.settings.apply_to(&mut expected);
    assert_eq!(after, expected, "exactly the preset's fields changed");
    assert_eq!(after.draft, before.draft);
    assert_eq!(after.curves, before.curves);
    assert_eq!(after.style, VocalStyle::Folk);
    assert_eq!(after.theme, "kept");

    // The lane is stale (a control edit never renders) and nothing queued.
    assert_eq!(
        lane_state(&vocal_view(&mut app), def),
        VocalRenderState::Stale
    );
    assert!(
        !app.test_finish_vocal_render(def, TRACK),
        "no render was queued"
    );

    // One undo takes the whole preset back.
    call(&mut app, "edit.undo", &json!({}))
        .result::<serde_json::Value>()
        .expect("edit.undo succeeds");
    assert_eq!(app.test_vocal_lane_params(def, TRACK).unwrap(), before);
}

#[test]
fn a_preset_on_a_bank_that_is_not_installed_still_applies_and_says_so() {
    let mut app = app_with_project();
    let def = rendered_lane(&mut app, "Verse", 1);
    let result: proto::ApplyVoicePresetResult =
        apply(&mut app, def, "Katyusha \u{2014} Strong alto")
            .result()
            .expect("applies by name");
    assert_eq!(result.preset_id, "katyusha-strong");
    assert!(!result.installed);
    let warning = result.warning.expect("a warning names what is missing");
    assert!(warning.contains("katyusha"), "{warning}");
    assert!(
        warning.contains("vocal_voicebank_install") || warning.contains("Library"),
        "{warning}"
    );
    assert_eq!(
        app.test_vocal_voicebank(def, TRACK),
        Some(VoicebankId::new("katyusha"))
    );
    assert_eq!(result.params.speaker.as_deref(), Some("strong"));
    // And a render says the same.
    let render = call(&mut app, "vocal.render", &json!({ "track_id": TRACK }));
    assert_eq!(
        render.error.expect("refused").kind(),
        ErrorKind::VoicebankMissing
    );
}

#[test]
fn apply_refuses_an_unknown_preset_and_a_track_without_a_lane() {
    let mut app = app_with_project();
    let def = rendered_lane(&mut app, "Verse", 1);
    let error = apply(&mut app, def, "nobody").error.expect("refused");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(
        error.message.contains("tiger-mystic"),
        "lists the ids: {}",
        error.message
    );
    let error = call(
        &mut app,
        proto::APPLY_VOICE_PRESET,
        &json!({ "track_id": 999, "preset": "lilia" }),
    )
    .error
    .expect("refused");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn save_writes_a_user_preset_without_touching_the_project() {
    let mut app = app_with_project();
    let def = rendered_lane(&mut app, "Verse", 1);
    apply(&mut app, def, "meiji-phantom")
        .result::<serde_json::Value>()
        .unwrap();
    let mut p = app.test_vocal_lane_params(def, TRACK).unwrap();
    p.vibrato = 0.55;
    app.test_set_vocal_lane_params(def, TRACK, p.clone());
    let undo_len = app.test_undo_history().undo_len();
    let revision = app.revision();

    let saved: proto::SaveVoicePresetResult = call(
        &mut app,
        proto::SAVE_VOICE_PRESET,
        &json!({ "track_id": TRACK, "section_id": def, "name": "Ghost lead",
                 "description": "verses", "tags": ["husky"] }),
    )
    .result()
    .expect("vocal.save_voice_preset succeeds");
    assert_eq!(saved.preset.id, "user:Ghost lead");
    assert!(!saved.preset.builtin && saved.preset.score.is_none());
    assert!(!saved.replaced);
    assert_eq!(saved.preset.voicebank, "meiji");
    assert_eq!(saved.preset.settings.vibrato, 0.55);
    assert_eq!(saved.preset.description, "verses");
    assert_eq!(
        app.revision(),
        revision,
        "a library write bumps no revision"
    );
    assert_eq!(
        app.test_undo_history().undo_len(),
        undo_len,
        "and records no undo entry"
    );

    // On disk, in the app's own (hermetic) folder, and listed last.
    let dir = app
        .test_voice_preset_dir()
        .expect("a test app has a folder");
    assert!(dir.join("Ghost lead.json").exists());
    let list = presets(&mut app);
    assert_eq!(list.last().unwrap().id, "user:Ghost lead");
    assert_eq!(app.test_user_voice_presets().len(), 1);

    // A second save of the name asks first; overwrite replaces.
    let error = call(
        &mut app,
        proto::SAVE_VOICE_PRESET,
        &json!({ "track_id": TRACK, "name": "Ghost lead" }),
    )
    .error
    .expect("refused");
    assert_eq!(error.kind(), ErrorKind::NeedsConfirmation);
    let again: proto::SaveVoicePresetResult = call(
        &mut app,
        proto::SAVE_VOICE_PRESET,
        &json!({ "track_id": TRACK, "name": "Ghost lead", "overwrite": true }),
    )
    .result()
    .unwrap();
    assert!(again.replaced);

    // A built-in's name is refused.
    let error = call(
        &mut app,
        proto::SAVE_VOICE_PRESET,
        &json!({ "track_id": TRACK, "name": "tiger mystic" }),
    )
    .error
    .expect("refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    // The saved voice goes onto another lane by name.
    let chorus = rendered_lane(&mut app, "Chorus", 5);
    let result: proto::ApplyVoicePresetResult =
        apply(&mut app, chorus, "ghost lead").result().unwrap();
    assert_eq!(result.preset_id, "user:Ghost lead");
    let chorus_params = app.test_vocal_lane_params(chorus, TRACK).unwrap();
    assert_eq!(chorus_params.speaker.as_deref(), Some("phantom"));
    assert_eq!(chorus_params.vibrato, 0.55);
}

#[test]
fn the_gui_pick_is_the_same_lane_edit() {
    let mut app = app_with_project();
    let def = rendered_lane(&mut app, "Verse", 1);
    let undo_len = app.test_undo_history().undo_len();
    let _ = app.update(Message::VoicePreset(VoicePresetMessage::Pick {
        definition_id: def,
        track_id: TRACK,
        id: "lilia".into(),
    }));
    assert_eq!(app.test_undo_history().undo_len(), undo_len + 1);
    assert_eq!(
        app.test_vocal_voicebank(def, TRACK),
        Some(VoicebankId::new("lilia"))
    );
    // A GUI edit arms the debounced re-render of a rendered lane.
    assert!(app.test_vocal_auto_render_pending(def, TRACK).is_some());
}
