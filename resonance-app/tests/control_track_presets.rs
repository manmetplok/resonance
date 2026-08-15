//! Saving a track as a preset, and stamping one out again (ba todo
//! #1303, finding P1).
//!
//! The capture pipeline was complete and unreachable: `save_user_preset`
//! worked, `finish_preset_save` worked, `pending_preset_save` was
//! declared and `take()`n on the engine echo — and nothing in the app
//! ever set it. So the preset menu could apply and delete presets that
//! only hand-written JSON could create.
//!
//! These drive the round trip the todo asks for — save, list, apply —
//! over the control API, plus the GUI message path and the overwrite
//! rule. `RESONANCE_PRESET_DIR` points the store at a temp directory, so
//! the tests never touch the machine's real preset library.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::{Message, TrackMessage};
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, TrackType};
use resonance_control::methods::track::{AddResult, PresetsView};
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const TRACK: u64 = 1;
const SYNTH: u64 = 30;

/// Point the preset store at a directory of this test binary's own, once
/// for the whole process. Tests share it, so each uses its own preset
/// names.
fn preset_dir() -> std::path::PathBuf {
    use std::sync::OnceLock;
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("resonance-preset-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // Safety: set once, before any test does preset I/O, and never
        // read by another thread in between — the alternative is writing
        // into the developer's real preset folder.
        unsafe {
            std::env::set_var(resonance_app::presets::PRESET_DIR_ENV, &dir);
        }
        dir
    })
    .clone()
}

fn app() -> Resonance {
    let _ = preset_dir();
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-track-presets.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app
}

/// A track carrying an instrument with a dialled-in parameter — the
/// thing worth saving.
fn with_instrument(app: &mut Resonance) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: SYNTH,
        plugin_name: "Resonance Wavetable".to_owned(),
        clap_plugin_id: "com.resonance.wavetable".to_owned(),
        clap_file_path: "/plugins/wavetable.clap".to_owned(),
        params: vec![ParamInfo {
            id: 1,
            name: "Cutoff".to_owned(),
            min_value: 20.0,
            max_value: 20000.0,
            default_value: 1000.0,
            current_value: 640.0,
        }],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
}

fn roundtrip(app: &mut Resonance, req: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn presets(app: &mut Resonance) -> PresetsView {
    roundtrip(app, Request::without_params(2, "track.presets"))
        .result()
        .expect("track.presets succeeds")
}

/// Play the engine echo that completes a save: the plugins' state blobs.
fn deliver_plugin_states(app: &mut Resonance) {
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved {
        states: vec![(SYNTH, vec![1, 2, 3, 4])],
    });
}

fn drain(rx: &resonance_audio::__test_support::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

// ---------------------------------------------------------------------------
// save -> list -> apply
// ---------------------------------------------------------------------------

#[test]
fn a_saved_track_becomes_a_preset_that_makes_another_track() {
    let mut app = app();
    with_instrument(&mut app);

    // Save. The reply means "capture started": the state blobs come
    // back from the engine, and only then is the file written.
    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "track.save_preset",
        serde_json::json!({"track_id": TRACK, "name": "Round Trip Pad"}),
    )
    .result()
    .expect("track.save_preset succeeds");
    assert!(
        drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::SaveAllPluginStates)),
        "the capture asks the engine for the plugins' state blobs"
    );
    deliver_plugin_states(&mut app);

    // List: it is there, it is a user preset, and it brings the
    // instrument with it.
    let view = presets(&mut app);
    let saved = view
        .presets
        .iter()
        .find(|p| p.name == "Round Trip Pad")
        .expect("the preset just saved is in the library");
    assert!(!saved.builtin);
    assert_eq!(saved.kind, resonance_control::TrackKind::Instrument);
    assert_eq!(saved.plugins, vec!["com.resonance.wavetable"]);

    // Apply: a NEW track, addressable in the same breath.
    let result: AddResult = call(
        &mut app,
        "track.apply_preset",
        serde_json::json!({"preset": "Round Trip Pad", "name": "From Preset"}),
    )
    .result()
    .expect("track.apply_preset succeeds");
    let new_id = u64::from(result.track_id);
    assert_ne!(new_id, TRACK, "applying a preset creates a track");

    // The engine was asked to make it, with the app's id and the
    // caller's name.
    let rx = app.test_capture_engine();
    let _ = drain(&rx);
}

#[test]
fn applying_a_preset_asks_the_engine_for_the_right_kind_of_track() {
    let mut app = app();
    with_instrument(&mut app);
    let _: MutationAck = call(
        &mut app,
        "track.save_preset",
        serde_json::json!({"track_id": TRACK, "name": "Kind Check"}),
    )
    .result()
    .expect("save succeeds");
    deliver_plugin_states(&mut app);

    let rx = app.test_capture_engine();
    let result: AddResult = call(
        &mut app,
        "track.apply_preset",
        serde_json::json!({"preset": "kind check"}),
    )
    .result()
    .expect("preset names match case-insensitively");
    let new_id = u64::from(result.track_id);

    let commands = drain(&rx);
    assert!(
        commands.iter().any(|c| matches!(
            c,
            AudioCommand::AddInstrumentTrack { id_hint: Some(id), .. } if *id == new_id
        )),
        "an instrument preset makes an instrument track, at the id the reply promised: \
         {commands:?}"
    );
}

#[test]
fn the_builtin_presets_are_listed_too_and_marked_as_such() {
    let mut app = app();
    let view = presets(&mut app);
    assert!(
        view.presets.iter().any(|p| p.builtin),
        "the app ships presets and they are stampable: {:?}",
        view.presets.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    // A built-in carries no chain — worth saying, because it means the
    // new track makes no sound until something is added to it.
    let builtin = view.presets.iter().find(|p| p.builtin).unwrap();
    assert!(builtin.plugins.is_empty());
}

// ---------------------------------------------------------------------------
// Overwriting
// ---------------------------------------------------------------------------

#[test]
fn overwriting_a_preset_needs_the_confirm_flag() {
    let mut app = app();
    with_instrument(&mut app);
    let save = |app: &mut Resonance, overwrite: bool| {
        call(
            app,
            "track.save_preset",
            serde_json::json!({
                "track_id": TRACK,
                "name": "Overwrite Me",
                "overwrite": overwrite,
            }),
        )
    };

    let _: MutationAck = save(&mut app, false).result().expect("first save succeeds");
    deliver_plugin_states(&mut app);

    // Second save, same name, no flag: refused, and it says why.
    let response = save(&mut app, false);
    let error = response.error.expect("a colliding name is refused");
    assert_eq!(error.kind(), ErrorKind::NeedsConfirmation);
    assert!(
        error.message.contains("already exists") && error.message.contains("overwrite"),
        "the refusal names the remedy: {:?}",
        error.message
    );

    // With the flag, it goes through.
    let _: MutationAck = save(&mut app, true)
        .result()
        .expect("overwrite: true replaces the preset");
}

#[test]
fn an_unknown_track_or_preset_is_refused_by_name() {
    let mut app = app();
    let response = call(
        &mut app,
        "track.save_preset",
        serde_json::json!({"track_id": 999, "name": "Nope"}),
    );
    assert_eq!(
        response.error.expect("no such track").kind(),
        ErrorKind::NotFound
    );

    let response = call(
        &mut app,
        "track.apply_preset",
        serde_json::json!({"preset": "Not A Preset"}),
    );
    let error = response.error.expect("no such preset");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(
        error.message.contains("have:"),
        "the error lists what does exist so the caller can correct itself: {:?}",
        error.message
    );
}

// ---------------------------------------------------------------------------
// The GUI path
// ---------------------------------------------------------------------------

#[test]
fn the_gui_prompt_arms_the_same_capture() {
    let mut app = app();
    with_instrument(&mut app);

    // Open the prompt from the track context menu: it seeds with the
    // track's own name.
    let _ = app.update(Message::Track(TrackMessage::OpenSavePresetPrompt(TRACK)));
    let _ = app.update(Message::Track(TrackMessage::SetSavePresetName(
        "GUI Path".to_owned(),
    )));

    let rx = app.test_capture_engine();
    let _ = app.update(Message::Track(TrackMessage::SaveTrackAsPreset {
        track_id: TRACK,
        name: "GUI Path".to_owned(),
        overwrite: false,
    }));
    assert!(
        drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::SaveAllPluginStates)),
        "the GUI message runs the same capture as track.save_preset"
    );
    deliver_plugin_states(&mut app);

    assert!(
        presets(&mut app).presets.iter().any(|p| p.name == "GUI Path"),
        "and the preset it wrote is in the same library the menu lists"
    );
}

#[test]
fn saving_a_preset_is_not_an_undo_step() {
    // A preset is a file on the machine; the project is exactly as it
    // was. Recording it would make Ctrl-Z pretend to un-save something.
    let mut app = app();
    with_instrument(&mut app);
    let before = app.revision();
    let _ = app.update(Message::Track(TrackMessage::SaveTrackAsPreset {
        track_id: TRACK,
        name: "Not Undoable".to_owned(),
        overwrite: true,
    }));
    deliver_plugin_states(&mut app);
    assert_eq!(app.revision(), before, "no project edit, no revision bump");
}
