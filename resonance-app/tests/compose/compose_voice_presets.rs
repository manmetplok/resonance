//! The vocal lane inspector's Voice preset block
//! (`docs/features/compose/voice-presets.md`): the cached picker, the
//! preset a lane matches (or Custom), Save… / Rename… / Delete, and a
//! preset on a bank that is not installed — with goldens of each.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::messages::LaneInspectorMsg;
use resonance_app::compose::vocal_svs::voice_presets;
use resonance_app::compose::{ComposeMessage, RailPanelKey, SelectedLane};
use resonance_app::message::{Message, VoicePresetMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};
use resonance_audio::types::TrackId;

const TALL_WINDOW: (f32, f32) = (1440.0, 1600.0);

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

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(
        sim_settings(),
        Size::new(TALL_WINDOW.0, TALL_WINDOW.1),
        app.view(),
    )
}

fn snapshot(app: &Resonance, path: &str) {
    let mut ui = simulator(app);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// The demo on the Compose tab with a TIGER stand-in installed, its vocal
/// lane selected and every vocal card but Voice & delivery folded:
/// `(app, definition_id, vocal_track_id)`.
fn vocal_rail() -> (Resonance, u64, TrackId) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_install_voicebank("tiger", &resonance_app::VoicebankFixture::shipped("tiger"));
    let track = app
        .track_registry()
        .tracks
        .iter()
        .find(|t| {
            matches!(t.track_type, resonance_audio::types::TrackType::Vocal)
                && t.sub_track.is_none()
        })
        .map(|t| t.id)
        .expect("demo seeds a vocal track");
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(track),
    )));
    let def = app
        .compose_state()
        .selected_placement()
        .expect("demo selects a placement")
        .definition_id;
    for key in [
        RailPanelKey::VocalLyrics(track),
        RailPanelKey::VocalDraft(track),
        RailPanelKey::VocalMelody(track),
    ] {
        let _ = app.update(Message::Compose(ComposeMessage::ToggleRailPanel(key)));
    }
    (app, def, track)
}

fn send(app: &mut Resonance, m: VoicePresetMessage) {
    let _ = app.update(Message::VoicePreset(m));
}

fn pick(app: &mut Resonance, def: u64, track: TrackId, id: &str) {
    send(
        app,
        VoicePresetMessage::Pick {
            definition_id: def,
            track_id: track,
            id: id.into(),
        },
    );
}

fn matched(app: &Resonance, def: u64, track: TrackId) -> Option<String> {
    let params = app.test_vocal_lane_params(def, track)?;
    let user = app.test_user_voice_presets();
    voice_presets::matching(&user, &params).map(|p| p.id.clone())
}

#[test]
fn the_picker_lists_built_ins_best_first_and_shows_the_matched_preset() {
    let (mut app, def, track) = vocal_rail();
    let choices = app.test_voice_preset_choices();
    assert_eq!(choices.len(), 31);
    assert_eq!(
        choices[0],
        ("tiger-mystic".into(), "5/5 \u{00B7} TIGER Mystic".into())
    );
    assert!(
        choices.iter().any(|(id, label)| id == "katyusha-standard"
            && label == "4/5 \u{00B7} Katyusha \u{2014} Alto lead"),
        "{choices:?}"
    );

    pick(&mut app, def, track, "tiger-mystic");
    assert_eq!(matched(&app, def, track).as_deref(), Some("tiger-mystic"));
    {
        let mut ui = simulator(&app);
        ui.find("Voice preset")
            .expect("the block is in the voice card");
        ui.find("Save\u{2026}").expect("Save… beside the picker");
    }
    snapshot(&app, "tests/snapshots/compose_voice_preset_matched.png");

    // Moving one slider makes the lane Custom.
    let _ = app.update(Message::Compose(ComposeMessage::LaneInspector {
        definition_id: def,
        track_id: track,
        msg: LaneInspectorMsg::SetVocalVibrato(0.61),
    }));
    assert_eq!(matched(&app, def, track), None);
}

#[test]
fn a_preset_on_a_missing_bank_applies_and_points_at_the_library() {
    let (mut app, def, track) = vocal_rail();
    pick(&mut app, def, track, "katyusha-standard");
    assert_eq!(
        app.test_vocal_voicebank(def, track),
        Some(resonance_music_theory::VoicebankId::new("katyusha"))
    );
    let mut ui = simulator(&app);
    ui.find(
        "katyusha is not installed \u{2014} Library\u{2026} downloads it; the lane renders once it is",
    )
    .expect("the card says the bank is missing");
    snapshot(
        &app,
        "tests/snapshots/compose_voice_preset_not_installed.png",
    );
}

#[test]
fn save_rename_and_delete_a_user_preset_from_the_card() {
    let (mut app, def, track) = vocal_rail();
    pick(&mut app, def, track, "tiger-vinyl");
    let _ = app.update(Message::Compose(ComposeMessage::LaneInspector {
        definition_id: def,
        track_id: track,
        msg: LaneInspectorMsg::SetVocalVibrato(0.5),
    }));
    let undo_len = app.test_undo_history().undo_len();

    // Save…: the prompt opens empty; a built-in's name is refused.
    send(
        &mut app,
        VoicePresetMessage::OpenSave {
            definition_id: def,
            track_id: track,
        },
    );
    assert_eq!(app.test_voice_preset_prompt().as_deref(), Some(""));
    send(
        &mut app,
        VoicePresetMessage::NameChanged("TIGER Vinyl".into()),
    );
    send(&mut app, VoicePresetMessage::Confirm);
    assert!(app.test_voice_preset_error().is_some());
    assert!(app.test_user_voice_presets().is_empty());

    send(
        &mut app,
        VoicePresetMessage::NameChanged("Warm lead".into()),
    );
    send(&mut app, VoicePresetMessage::Confirm);
    assert_eq!(
        app.test_voice_preset_prompt(),
        None,
        "a save closes the prompt"
    );
    assert_eq!(app.test_voice_preset_error(), None);
    let user = app.test_user_voice_presets();
    assert_eq!(user.len(), 1);
    assert_eq!(user[0].settings.vibrato, 0.5);
    assert!(app
        .test_voice_preset_dir()
        .unwrap()
        .join("Warm lead.json")
        .exists());
    assert_eq!(
        app.test_undo_history().undo_len(),
        undo_len,
        "saving is not a project edit"
    );
    assert_eq!(matched(&app, def, track).as_deref(), Some("user:Warm lead"));
    assert!(app
        .test_voice_preset_choices()
        .iter()
        .any(|(id, label)| id == "user:Warm lead" && label == "User \u{2039} Warm lead"));

    // Saving again under the same name offers to overwrite.
    send(
        &mut app,
        VoicePresetMessage::OpenSave {
            definition_id: def,
            track_id: track,
        },
    );
    send(
        &mut app,
        VoicePresetMessage::NameChanged("Warm lead".into()),
    );
    {
        let mut ui = simulator(&app);
        ui.find("Overwrite").expect("the button reads Overwrite");
        ui.find("Rename\u{2026}")
            .expect("a user preset can be renamed");
    }
    snapshot(
        &app,
        "tests/snapshots/compose_voice_preset_user_overwrite.png",
    );
    send(&mut app, VoicePresetMessage::Cancel);

    // Rename…
    send(
        &mut app,
        VoicePresetMessage::OpenRename {
            definition_id: def,
            track_id: track,
            name: "Warm lead".into(),
        },
    );
    assert_eq!(app.test_voice_preset_prompt().as_deref(), Some("Warm lead"));
    send(
        &mut app,
        VoicePresetMessage::NameChanged("Vinyl lead".into()),
    );
    send(&mut app, VoicePresetMessage::Confirm);
    let names: Vec<String> = app
        .test_user_voice_presets()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Vinyl lead"]);
    assert_eq!(
        matched(&app, def, track).as_deref(),
        Some("user:Vinyl lead")
    );

    // Delete asks with a second press.
    send(&mut app, VoicePresetMessage::Delete("Vinyl lead".into()));
    assert_eq!(
        app.test_user_voice_presets().len(),
        1,
        "the first press only arms"
    );
    send(&mut app, VoicePresetMessage::Delete("Vinyl lead".into()));
    assert!(app.test_user_voice_presets().is_empty());
    assert_eq!(app.test_voice_preset_choices().len(), 31);
    assert_eq!(app.test_undo_history().undo_len(), undo_len);
}
