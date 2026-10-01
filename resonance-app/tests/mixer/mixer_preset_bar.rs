//! The host's preset surfaces (plugin-preset-library.md §6.6, §6.7; slice
//! P6): the plugin panel's bar, the browser overlay it opens, the media
//! browser's Presets tab and "with preset…" adds, driven through their
//! messages, plus goldens of the bar and the overlay.
//!
//! Every app has a private preset root and marks store (a test app gets
//! both at construction), so nothing here reads the user's library.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::commands::CommandId;
use resonance_app::message::{Message, PluginMessage, PresetAddOwner, PresetUiMessage, UiMessage};
use resonance_app::state::presets::PresetAddPick;
use resonance_app::state::{Overlay, PluginSlotState, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_common::factory_presets::FactoryPresetEntry;
use resonance_control::methods::plugin_preset::PluginPresetSource;

const TRACK: u64 = 5;
const INSTANCE: u64 = 950;
const PLUGIN_ID: &str = "com.resonance.test-eq";

fn clap_id(s: &str) -> u32 {
    resonance_plugin::stable_hash(s)
}

fn params() -> Vec<ParamInfo> {
    ["gain", "freq"]
        .into_iter()
        .map(|id| ParamInfo {
            id: clap_id(id),
            name: id.to_owned(),
            min_value: 0.0,
            max_value: 20_000.0,
            default_value: 1.0,
            current_value: 1.0,
            ..Default::default()
        })
        .collect()
}

fn factory(id: &str, name: &str, gain: f64, category: &str) -> FactoryPresetEntry {
    FactoryPresetEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        json: format!(r#"{{"version":1,"params":{{"gain":{gain},"freq":440.0}}}}"#),
        meta: Some(serde_json::json!({ "category": category }).to_string()),
    }
}

fn scanned() -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: "/nonexistent/test-eq.clap".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        name: "Test EQ".to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: false,
        factory_presets: vec![
            factory("warm", "Warm", 3.0, "EQ"),
            factory("bright", "Bright", 7.0, "EQ"),
            factory("flat", "Flat", 1.0, "Utility"),
        ],
    }
}

fn app_with(app: Resonance) -> Resonance {
    let mut app = app;
    app.test_set_active_project(true);
    // The history only records once the project has a path.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/preset-bar.rprj"));
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![scanned()],
    });
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE,
            "Test EQ".to_owned(),
            PLUGIN_ID.to_owned(),
            "/nonexistent/test-eq.clap".to_owned(),
            params(),
            false,
        ),
    );
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    app
}

fn app() -> Resonance {
    let (app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app_with(app)
}

fn ui(app: &mut Resonance, m: PresetUiMessage) {
    let _ = app.update(Message::Plugin(PluginMessage::PresetUi(m)));
}

fn gain(app: &mut Resonance) -> Option<f64> {
    app.test_plugin_param(INSTANCE, clap_id("gain"))
}

fn current_id(app: &Resonance) -> Option<String> {
    app.test_presets()
        .plugin_preset_identity
        .get(&INSTANCE)
        .map(|i| i.id.clone())
}

fn row_index(app: &Resonance, id: &str) -> usize {
    app.test_presets()
        .host_browser
        .as_ref()
        .expect("browser open")
        .list
        .rows
        .iter()
        .position(|r| r.id == id)
        .unwrap_or_else(|| panic!("no row {id}"))
}

fn undo_len(app: &Resonance) -> usize {
    app.test_undo_history().undo_len()
}

/// ◀ / ▶ walk the bank in order and wrap, naming each preset; a run of
/// steps on one plugin is one undo entry (review minor), back to where it
/// started.
#[test]
fn next_and_previous_walk_the_bank_as_one_undo_entry() {
    let mut app = app();
    let before = undo_len(&app);
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: 1 });
    assert_eq!(current_id(&app).as_deref(), Some("warm"));
    assert_eq!(gain(&mut app), Some(3.0));
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: 1 });
    assert_eq!(current_id(&app).as_deref(), Some("bright"));
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: -1 });
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: -1 });
    assert_eq!(current_id(&app).as_deref(), Some("flat"), "wraps to the last");
    assert_eq!(undo_len(&app), before + 1, "one entry for the run");
    let _ = app.update(Message::Undo);
    assert_eq!(gain(&mut app), Some(1.0), "back to before the first step");
}

/// A run of steps sends the plugin one state load, for where it stopped,
/// once the run is quiet (an amp step would reload a model each time).
#[test]
fn a_step_run_sends_one_state_load() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    while rx.try_recv().is_ok() {}
    for _ in 0..3 {
        ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: 1 });
    }
    let loads = |cmds: &[AudioCommand]| {
        cmds.iter()
            .filter(|c| {
                matches!(c, AudioCommand::LoadPluginPresetState { instance_id: INSTANCE, .. })
            })
            .count()
    };
    assert_eq!(loads(&rx.try_iter().collect::<Vec<_>>()), 0, "parked during the run");
    app.test_flush_step_state();
    let cmds: Vec<_> = rx.try_iter().collect();
    assert_eq!(loads(&cmds), 1);
    assert!(capture_token(&cmds).is_some(), "the run's undo entry waits on its capture");
}

/// The audition bracket (§6.7): auditions record nothing, keeping records
/// ONE entry whose undo returns to the sound the browser opened on.
#[test]
fn auditions_record_nothing_and_keeping_is_one_undo_back_to_the_origin() {
    let mut app = app();
    let before = undo_len(&app);
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    assert_eq!(app.root_overlay(), Some(Overlay::PresetBrowser));
    let warm = row_index(&app, "warm");
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(warm));
    assert_eq!(gain(&mut app), Some(3.0));
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    assert_eq!(gain(&mut app), Some(7.0));
    assert_eq!(undo_len(&app), before, "auditions are not history");

    ui(&mut app, PresetUiMessage::CloseBrowser { keep: true });
    assert_eq!(app.root_overlay(), None);
    assert_eq!(gain(&mut app), Some(7.0));
    assert_eq!(current_id(&app).as_deref(), Some("bright"));
    assert_eq!(undo_len(&app), before + 1, "one entry for the whole bracket");

    let _ = app.update(Message::Undo);
    assert_eq!(gain(&mut app), Some(1.0), "undo returns to the origin, not to Warm");
}

/// Esc (the overlay's dismiss) reverts: the origin's values go back to
/// the engine and the mirror, and nothing is recorded.
#[test]
fn esc_reverts_the_audition() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    let before = undo_len(&app);
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    while rx.try_recv().is_ok() {}
    app.test_dismiss_overlay();
    assert_eq!(app.root_overlay(), None);
    assert_eq!(gain(&mut app), Some(1.0));
    assert_eq!(current_id(&app), None);
    let sent_gain = rx.try_iter().any(|c| {
        matches!(c, AudioCommand::SetPluginParam { instance_id: INSTANCE, param_id, value }
            if param_id == clap_id("gain") && value == 1.0)
    });
    assert!(sent_gain, "the engine hears the origin again");
    assert_eq!(undo_len(&app), before);
}

/// Stars from the browser and the bar land in the shared marks store and
/// feed the favourites filter and the add pickers' "with preset…".
#[test]
fn stars_filter_the_lists_and_feed_the_add_pickers() {
    let mut app = app();
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let flat = row_index(&app, "flat");
    ui(&mut app, PresetUiMessage::BrowserToggleRowFavorite(flat));
    ui(&mut app, PresetUiMessage::BrowserFavoritesOnly(true));
    let rows: Vec<String> = app.test_presets().host_browser.as_ref().unwrap().list.rows
        .iter()
        .map(|r| r.id.clone())
        .collect();
    assert_eq!(rows, vec!["flat".to_string()]);
    let picks: Vec<String> = app.test_presets().fx_favorite_picks.iter()
        .map(|p| p.preset_id.clone())
        .collect();
    assert_eq!(picks, vec!["flat".to_string()]);
    ui(&mut app, PresetUiMessage::CloseBrowser { keep: false });

    // The bar's star toggles the loaded preset.
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: 1 });
    ui(&mut app, PresetUiMessage::ToggleFavorite(INSTANCE));
    assert_eq!(app.test_presets().fx_favorite_picks.len(), 2);
}

/// The media tab lists every plugin's presets; a double-click loads onto
/// the selected slot of the same plugin as one recorded load.
#[test]
fn the_media_tab_searches_and_loads_onto_the_selected_plugin() {
    let mut app = app();
    ui(&mut app, PresetUiMessage::MediaSearch("bri".into()));
    let rows = &app.test_presets().media_presets.rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].plugin_name, "Test EQ");
    let before = undo_len(&app);
    ui(&mut app, PresetUiMessage::MediaLoad(0));
    assert_eq!(gain(&mut app), Some(7.0));
    assert_eq!(undo_len(&app), before + 1);
}

/// "with preset…": the add happens now, the preset lands on the echo.
#[test]
fn adding_with_a_preset_loads_it_when_the_plugin_arrives() {
    let mut app = app();
    let next = app.test_next_plugin_id();
    ui(
        &mut app,
        PresetUiMessage::AddWithPreset {
            owner: PresetAddOwner::Track(TRACK),
            pick: PresetAddPick {
                plugin: scanned(),
                preset_id: "bright".into(),
                preset_name: "Bright".into(),
                source: PluginPresetSource::Factory,
            },
        },
    );
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: next,
        plugin_name: "Test EQ".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        clap_file_path: "/nonexistent/test-eq.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: Vec::new(),
    });
    assert_eq!(app.test_plugin_param(next, clap_id("gain")), Some(7.0));
}

/// The commands follow the plugin panel's selection.
#[test]
fn the_preset_commands_need_a_selected_plugin() {
    let (mut bare, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    bare.test_set_active_project(true);
    assert!(!CommandId::NextPluginPreset.availability(&bare).is_yes());
    let mut app = app();
    assert!(CommandId::BrowsePluginPresets.availability(&app).is_yes());
    let open = CommandId::BrowsePluginPresets.to_message(&app).expect("a target");
    let _ = app.update(open);
    assert_eq!(app.root_overlay(), Some(Overlay::PresetBrowser));
    app.test_dismiss_overlay();
    let next = CommandId::NextPluginPreset.to_message(&app).expect("a target");
    let _ = app.update(next);
    assert_eq!(current_id(&app).as_deref(), Some("warm"));
}

// ---------------------------------------------------------------------------
// Discovered presets and drag-to-add (slice P8)
// ---------------------------------------------------------------------------

fn discovered(name: &str, key: &str, flags: u32) -> resonance_audio::types::DiscoveredPreset {
    resonance_audio::types::DiscoveredPreset {
        name: name.to_owned(),
        location: resonance_audio::types::DiscoveredLocation::Plugin,
        load_key: Some(key.to_owned()),
        plugin_ids: vec![PLUGIN_ID.to_owned()],
        creators: vec!["Jane".to_owned()],
        description: Some("From the plugin".to_owned()),
        features: vec!["bass".to_owned()],
        flags,
    }
}

/// A preset-discovery listing joins the library after the compiled-in
/// bank, a provider favourite is starred once, and loading one asks the
/// plugin (`clap.preset-load`) as one undo entry.
#[test]
fn discovered_presets_are_listed_and_load_through_the_plugin() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    app.test_apply_engine_event(AudioEvent::PluginPresetsDiscovered {
        plugin_id: PLUGIN_ID.to_owned(),
        presets: vec![
            discovered("Sub Drop", "bank/1", 0),
            discovered("Air Lift", "bank/2", 1 << 3),
        ],
    });
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let rows: Vec<(String, bool)> = app.test_presets().host_browser.as_ref().unwrap().list.rows
        .iter()
        .map(|r| (r.name.clone(), r.favorite))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Warm".to_owned(), false),
            ("Bright".to_owned(), false),
            ("Flat".to_owned(), false),
            ("Sub Drop".to_owned(), false),
            ("Air Lift".to_owned(), true),
        ]
    );
    ui(&mut app, PresetUiMessage::CloseBrowser { keep: false });
    while rx.try_recv().is_ok() {}

    let before = undo_len(&app);
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: -1 });
    assert_eq!(undo_len(&app), before + 1);
    assert_eq!(current_id(&app).as_deref(), Some("plugin:bank/2"));
    let asked = rx.try_iter().find_map(|c| match c {
        AudioCommand::LoadPluginPresetFromLocation { instance_id: INSTANCE, location, load_key, .. } => {
            Some((location, load_key))
        }
        _ => None,
    });
    assert_eq!(
        asked,
        Some((
            resonance_audio::types::PluginPresetLocation::Plugin,
            Some("bank/2".to_owned())
        ))
    );
}

/// Selecting a row (keyboard, programmatic) is not arming a drag: only a
/// real press on the row does (the widget tests in `io::preset_tab_widgets`
/// drive the press and the drag).
#[test]
fn selecting_a_row_does_not_arm_a_drag() {
    let mut app = app();
    ui(&mut app, PresetUiMessage::MediaSearch("bright".into()));
    ui(&mut app, PresetUiMessage::MediaSelect(0));
    assert_eq!(app.test_presets().media_presets.selected, Some(0));
    assert!(app.test_presets().dragging.is_none());
    ui(&mut app, PresetUiMessage::MediaPress(0));
    ui(&mut app, PresetUiMessage::DropOnTrack(TRACK));
    assert_eq!(app.test_registry().tracks[0].plugins.len(), 1, "no movement, no drop");
}

// ---------------------------------------------------------------------------
// The whole sound comes back (review M1)
// ---------------------------------------------------------------------------

/// The state a plugin had with model X picked by hand; the audition's
/// preset carries model Y.
const MODEL_X: &[u8] = br#"{"version":2,"params":{"gain":1.0},"model_id":"x"}"#;

fn capture_token(cmds: &[AudioCommand]) -> Option<u64> {
    cmds.iter().find_map(|c| match c {
        AudioCommand::LoadPluginPresetState {
            instance_id: INSTANCE,
            capture,
            ..
        } => *capture,
        _ => None,
    })
}

fn full_state_loads(cmds: &[AudioCommand]) -> Vec<Vec<u8>> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::LoadPluginState {
                instance_id: INSTANCE,
                data,
            } => Some(data.clone()),
            _ => None,
        })
        .collect()
}

fn captured(app: &mut Resonance, token: u64, data: &[u8]) {
    app.test_apply_engine_event(AudioEvent::PluginStateCaptured {
        instance_id: INSTANCE,
        token,
        data: data.to_vec(),
        after: false,
    });
}

fn captured_after(app: &mut Resonance, token: u64, data: &[u8]) {
    app.test_apply_engine_event(AudioEvent::PluginStateCaptured {
        instance_id: INSTANCE,
        token,
        data: data.to_vec(),
        after: true,
    });
}

/// Undo, Redo, Undo, Redo of a preset step: each undo pushes the state
/// from before the load, each redo the state the load left — for a model
/// (amp) and for user tables (wavetable), and also when the undo comes
/// before the "after" state has arrived.
#[test]
fn undo_and_redo_of_a_preset_load_round_trip_the_whole_state() {
    const AMP_BEFORE: &[u8] = br#"{"params":{},"model_id":"x"}"#;
    const AMP_AFTER: &[u8] = br#"{"params":{},"model_id":"y"}"#;
    const WT_BEFORE: &[u8] = br#"{"params":{},"user_wavetables":{"osc1":{"frames":"AAAA"}}}"#;
    const WT_AFTER: &[u8] = br#"{"params":{},"user_wavetables":{"osc1":{"frames":"BBBB"}}}"#;
    for (before, after, late) in [
        (AMP_BEFORE, AMP_AFTER, false),
        (WT_BEFORE, WT_AFTER, false),
        (WT_BEFORE, WT_AFTER, true),
    ] {
        let (app, _task, rx) = Resonance::new_for_test_with_capture();
        let mut app = app_with(app);
        app.test_seed_plugin_state(INSTANCE, b"stale".to_vec());
        while rx.try_recv().is_ok() {}
        ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: 1 });
        app.test_flush_step_state();
        let token = capture_token(&rx.try_iter().collect::<Vec<_>>()).expect("a capture");
        captured(&mut app, token, before);
        if !late {
            captured_after(&mut app, token, after);
        }
        for round in 0..2 {
            let _ = app.update(Message::Undo);
            if late && round == 0 {
                // The "after" half lands once the undo has already run.
                captured_after(&mut app, token, after);
            }
            let sent = full_state_loads(&rx.try_iter().collect::<Vec<_>>());
            assert_eq!(sent, vec![before.to_vec()], "undo {round} (late: {late})");
            let _ = app.update(Message::Redo);
            let sent = full_state_loads(&rx.try_iter().collect::<Vec<_>>());
            assert_eq!(sent, vec![after.to_vec()], "redo {round} (late: {late})");
        }
    }
}

/// An undo inside the step debounce is final: the parked state load does
/// not go out afterwards and override the restored sound.
#[test]
fn an_undo_mid_debounce_is_not_overridden() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: 1 });
    let _ = app.update(Message::Undo);
    while rx.try_recv().is_ok() {}
    app.test_flush_step_state();
    let sent: Vec<_> = rx.try_iter().collect();
    assert!(
        !sent.iter().any(|c| matches!(c, AudioCommand::LoadPluginPresetState { .. })),
        "{sent:?}"
    );
    assert_eq!(gain(&mut app), Some(1.0), "the pre-step sound");
}

/// Esc, then a new audition before the first capture arrived: the late
/// capture no longer lands on the new audition, and it is the new
/// browser's origin (Esc again restores the sound before any audition).
#[test]
fn a_late_capture_does_not_land_on_a_newer_audition() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    while rx.try_recv().is_ok() {}
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    let token = capture_token(&rx.try_iter().collect::<Vec<_>>()).unwrap();
    app.test_dismiss_overlay();
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let warm = row_index(&app, "warm");
    ui(&mut app, PresetUiMessage::BrowserAudition(warm));
    assert_eq!(capture_token(&rx.try_iter().collect::<Vec<_>>()), None, "no second capture");
    captured(&mut app, token, MODEL_X);
    assert!(
        full_state_loads(&rx.try_iter().collect::<Vec<_>>()).is_empty(),
        "the old revert does not land on the new audition"
    );
    app.test_dismiss_overlay();
    assert_eq!(full_state_loads(&rx.try_iter().collect::<Vec<_>>()), vec![MODEL_X.to_vec()]);
}

/// Model X by hand, audition a preset (model Y), Esc: the plugin gets its
/// full pre-audition state back (X plays), and that is what is cached and
/// saved.
#[test]
fn esc_after_an_audition_restores_the_whole_state_the_plugin_had() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    while rx.try_recv().is_ok() {}
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    let cmds: Vec<_> = rx.try_iter().collect();
    let token = capture_token(&cmds).expect("the first audition captures the full state");
    captured(&mut app, token, MODEL_X);

    app.test_dismiss_overlay();
    let cmds: Vec<_> = rx.try_iter().collect();
    assert_eq!(full_state_loads(&cmds), vec![MODEL_X.to_vec()], "X plays again");
    assert_eq!(
        app.test_cached_plugin_state(INSTANCE).as_deref(),
        Some(MODEL_X),
        "and is what is saved"
    );
}

/// Esc before the engine's capture arrived: the revert completes when it
/// does.
#[test]
fn a_revert_before_the_capture_lands_completes_when_it_does() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    while rx.try_recv().is_ok() {}
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    let token = capture_token(&rx.try_iter().collect::<Vec<_>>()).unwrap();
    app.test_dismiss_overlay();
    assert!(full_state_loads(&rx.try_iter().collect::<Vec<_>>()).is_empty());
    captured(&mut app, token, MODEL_X);
    assert_eq!(
        full_state_loads(&rx.try_iter().collect::<Vec<_>>()),
        vec![MODEL_X.to_vec()]
    );
}

/// Undo of a bar step and of a kept audition puts back the full state
/// the load replaced (the engine's capture), not only the params.
#[test]
fn undoing_a_preset_load_restores_the_full_state_it_replaced() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    app.test_seed_plugin_state(INSTANCE, b"stale".to_vec());
    while rx.try_recv().is_ok() {}
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: 1 });
    app.test_flush_step_state();
    let token =
        capture_token(&rx.try_iter().collect::<Vec<_>>()).expect("a recorded load captures");
    captured(&mut app, token, MODEL_X);
    let _ = app.update(Message::Undo);
    assert_eq!(
        full_state_loads(&rx.try_iter().collect::<Vec<_>>()),
        vec![MODEL_X.to_vec()]
    );

    // Kept audition: the entry returns to the origin, captured at the
    // first audition — not to the audition the engine plays when kept.
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    let token = capture_token(&rx.try_iter().collect::<Vec<_>>()).unwrap();
    captured(&mut app, token, MODEL_X);
    ui(&mut app, PresetUiMessage::CloseBrowser { keep: true });
    assert_eq!(capture_token(&rx.try_iter().collect::<Vec<_>>()), None, "no second capture");
    let _ = app.update(Message::Undo);
    assert_eq!(
        full_state_loads(&rx.try_iter().collect::<Vec<_>>()),
        vec![MODEL_X.to_vec()]
    );
}

/// A `loaded()` echo of a discovered preset keeps the library's identity
/// (review M5), and a provider favourite the user un-starred stays
/// un-starred when the plugin is discovered again.
#[test]
fn a_loaded_echo_confirms_a_discovered_identity_and_unstars_stick() {
    let mut app = app();
    let listing = || AudioEvent::PluginPresetsDiscovered {
        plugin_id: PLUGIN_ID.to_owned(),
        presets: vec![
            discovered("Sub Drop", "bank/1", 0),
            discovered("Air Lift", "bank/2", 1 << 3),
        ],
    };
    app.test_apply_engine_event(listing());
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: -1 });
    app.test_apply_engine_event(AudioEvent::PluginPresetLoaded {
        instance_id: INSTANCE,
        location: resonance_audio::types::PluginPresetLocation::Plugin,
        load_key: Some("bank/2".into()),
    });
    let identity = app.test_presets().plugin_preset_identity.get(&INSTANCE).cloned().unwrap();
    assert_eq!((identity.id.as_str(), identity.name.as_str()), ("plugin:bank/2", "Air Lift"));

    ui(&mut app, PresetUiMessage::ToggleFavorite(INSTANCE));
    app.test_apply_engine_event(listing());
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let air = app.test_presets().host_browser.as_ref().unwrap().list.rows
        .iter()
        .find(|r| r.name == "Air Lift")
        .cloned()
        .unwrap();
    assert!(!air.favorite, "the provider's favourite is seeded once, not every start");
}

// ---------------------------------------------------------------------------
// The browser's lifetime and what Keep keeps (review M12, minors)
// ---------------------------------------------------------------------------

/// The browser does not outlive its plugin: removing it closes the browser
/// without a revert, so Esc sends nothing to a dead instance and no
/// identity is re-inserted for it.
#[test]
fn removing_the_plugin_closes_its_browser_without_a_revert() {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let mut app = app_with(app);
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    app.test_apply_engine_event(AudioEvent::PluginRemoved {
        track_id: TRACK,
        instance_id: INSTANCE,
    });
    assert!(app.test_presets().host_browser.is_none());
    assert_eq!(app.root_overlay(), None);
    while rx.try_recv().is_ok() {}
    app.test_dismiss_overlay();
    ui(&mut app, PresetUiMessage::CloseBrowser { keep: false });
    let sent: Vec<_> = rx.try_iter().collect();
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, AudioCommand::SetPluginParam { instance_id: INSTANCE, .. })),
        "nothing goes to the dead instance: {sent:?}"
    );
    assert!(!app.test_presets().plugin_preset_identity.contains_key(&INSTANCE));
}

/// Keep keeps what was auditioned even when a search has filtered its row
/// out since (it used to turn into a revert).
#[test]
fn keep_keeps_the_audition_after_its_row_is_filtered_out() {
    let mut app = app();
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    ui(&mut app, PresetUiMessage::BrowserSearch("warm".into()));
    ui(&mut app, PresetUiMessage::CloseBrowser { keep: true });
    assert_eq!(current_id(&app).as_deref(), Some("bright"));
    assert_eq!(gain(&mut app), Some(7.0));
}

fn press_key(app: &mut Resonance, key: resonance_app::commands::NamedKey, captured: bool) {
    use resonance_app::commands::{KeyChord, Mods};
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(key, Mods::NONE),
        repeat: false,
        captured,
    }));
}

/// The browser's keys go through the real key dispatch: ↓ auditions the
/// next row even while the search field has focus (captured), ↵ keeps.
#[test]
fn arrows_audition_and_enter_keeps_in_the_browser() {
    use resonance_app::commands::NamedKey;
    let mut app = app();
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    press_key(&mut app, NamedKey::ArrowDown, true);
    assert_eq!(gain(&mut app), Some(3.0), "the first row (Warm) auditions");
    press_key(&mut app, NamedKey::ArrowDown, true);
    assert_eq!(gain(&mut app), Some(7.0), "then Bright");
    press_key(&mut app, NamedKey::Enter, true);
    assert!(app.test_presets().host_browser.is_none(), "kept and closed");
    assert_eq!(current_id(&app).as_deref(), Some("bright"));
}

/// The Presets tab's rows are cached behind the list's generation: an
/// unrelated update leaves it alone, a re-query that changes nothing does
/// too, and a search that changes the rows bumps it (review M10).
#[test]
fn the_tab_list_generation_moves_only_with_its_rows() {
    let mut app = app();
    ui(&mut app, PresetUiMessage::MediaSearch(String::new()));
    let g0 = app.test_presets().media_presets.generation;
    let _ = app.update(Message::Tick);
    ui(&mut app, PresetUiMessage::MediaSearch(String::new()));
    assert_eq!(app.test_presets().media_presets.generation, g0, "same rows, same widgets");
    ui(&mut app, PresetUiMessage::MediaSearch("bri".into()));
    assert_ne!(app.test_presets().media_presets.generation, g0);
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

fn snapshot_to(app: &Resonance, path: &str) {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![theme::ICON_FONT_BYTES.into()];
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    let settings = iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    };
    let mut sim = Simulator::with_size(settings, Size::new(1440.0, 1000.0), app.view());
    let snap = sim
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// The panel's bar with a preset loaded and edited: ◀ Warm • ▶ ☆ Presets….
#[test]
fn preset_bar_golden() {
    let mut app = app();
    ui(&mut app, PresetUiMessage::Step { instance_id: INSTANCE, delta: 1 });
    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(
        INSTANCE,
        clap_id("freq"),
        880.0,
    )));
    snapshot_to(&app, "tests/snapshots/plugin_preset_bar.png");
}

/// The browser mid-audition, one row starred.
#[test]
fn preset_browser_overlay_golden() {
    let mut app = app();
    ui(&mut app, PresetUiMessage::OpenBrowser(INSTANCE));
    let warm = row_index(&app, "warm");
    ui(&mut app, PresetUiMessage::BrowserToggleRowFavorite(warm));
    let bright = row_index(&app, "bright");
    ui(&mut app, PresetUiMessage::BrowserAudition(bright));
    snapshot_to(&app, "tests/snapshots/plugin_preset_browser.png");
}
