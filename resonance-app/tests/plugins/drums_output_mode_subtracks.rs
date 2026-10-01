//! Resonance Drums gets its per-pad sub-tracks only in Multi output mode
//! (drums-plugin-rework.md §8, E11/D5).
//!
//! In Stereo — the default for a new instance — every pad sums to the
//! main output and the other seven ports are silent, so sub-tracks would
//! be dead faders. Switching to Multi creates them in the same undoable
//! edit; switching back keeps them (they are the user's tracks). A state
//! load that lands in Multi (a v1 project, the "Drummica Kit" preset)
//! creates the missing ones too. Other multi-output plugins keep getting
//! theirs at once.

use resonance_app::message::{Message, PluginMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{
    AudioEvent, ParamInfo, ParamValueUpdate, PluginInstanceId, TrackType,
};

const TRACK: u64 = 1;
const DRUMS: PluginInstanceId = 50;
const PORTS: [&str; 8] = [
    "Main", "Kick", "Snare", "Hi-Hat", "Toms", "Cymbals", "Perc", "Overhead",
];

fn output_mode_id() -> u32 {
    resonance_plugin::stable_hash("output_mode")
}

fn output_mode(value: f64) -> ParamInfo {
    ParamInfo {
        id: output_mode_id(),
        name: "Output Mode".to_owned(),
        min_value: 0.0,
        max_value: 1.0,
        current_value: value,
        stepped: true,
        choices: vec!["Stereo".to_owned(), "Multi".to_owned()],
        ..Default::default()
    }
}

fn app() -> Resonance {
    let (mut app, _) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    // The history only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/drums-output-mode.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app
}

fn add(
    app: &mut Resonance,
    instance_id: PluginInstanceId,
    plugin_id: &str,
    params: Vec<ParamInfo>,
) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params,
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: PORTS.len(),
        output_port_names: PORTS.iter().map(|s| s.to_string()).collect(),
    });
}

fn sub_tracks(app: &Resonance) -> Vec<(u32, String)> {
    let mut subs: Vec<(u32, String)> = app
        .test_tracks()
        .iter()
        .filter_map(|t| {
            t.sub_track
                .filter(|l| l.parent_track_id == TRACK)
                .map(|l| (l.output_port_index, t.name.clone()))
        })
        .collect();
    subs.sort();
    subs
}

fn set_output_mode(app: &mut Resonance, value: f64) {
    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(
        DRUMS,
        output_mode_id(),
        value,
    )));
}

#[test]
fn stereo_drums_get_no_sub_tracks_and_multi_creates_them() {
    let mut app = app();
    add(
        &mut app,
        DRUMS,
        "com.resonance.drums",
        vec![output_mode(0.0)],
    );
    assert!(
        sub_tracks(&app).is_empty(),
        "Stereo: the ports carry nothing"
    );

    set_output_mode(&mut app, 1.0);
    let subs = sub_tracks(&app);
    assert_eq!(subs.len(), 7, "one per non-main port: {subs:?}");
    assert_eq!(subs[0].0, 1);
    assert!(subs[0].1.ends_with("Kick"), "{subs:?}");

    // Again: nothing doubles.
    set_output_mode(&mut app, 1.0);
    assert_eq!(sub_tracks(&app).len(), 7);

    // Back to Stereo keeps the user's tracks.
    set_output_mode(&mut app, 0.0);
    assert_eq!(
        sub_tracks(&app).len(),
        7,
        "Stereo does not delete sub-tracks"
    );
}

#[test]
fn undoing_the_switch_to_multi_takes_its_sub_tracks_back_and_redo_restores_them() {
    let mut app = app();
    add(
        &mut app,
        DRUMS,
        "com.resonance.drums",
        vec![output_mode(0.0)],
    );
    set_output_mode(&mut app, 1.0);
    assert_eq!(sub_tracks(&app).len(), 7);
    let _ = app.update(Message::Undo);
    assert!(sub_tracks(&app).is_empty(), "{:?}", sub_tracks(&app));
    assert_eq!(app.test_plugin_param(DRUMS, output_mode_id()), Some(0.0));
    let _ = app.update(Message::Redo);
    assert_eq!(sub_tracks(&app).len(), 7, "redo brings them back");
    assert_eq!(app.test_plugin_param(DRUMS, output_mode_id()), Some(1.0));
}

#[test]
fn a_state_load_that_lands_in_multi_creates_the_sub_tracks() {
    let mut app = app();
    add(
        &mut app,
        DRUMS,
        "com.resonance.drums",
        vec![output_mode(0.0)],
    );
    // A v1 state (or the "Drummica Kit" preset) recalled Multi; the
    // plugin's rescan reports it.
    app.test_apply_engine_event(AudioEvent::PluginParamValuesChanged {
        instance_id: DRUMS,
        values: vec![ParamValueUpdate {
            id: output_mode_id(),
            value: 1.0,
            text: "Multi".to_owned(),
        }],
    });
    assert_eq!(sub_tracks(&app).len(), 7);
}

#[test]
fn drums_added_in_multi_and_other_multi_out_plugins_get_sub_tracks_at_once() {
    let mut app = app();
    add(
        &mut app,
        DRUMS,
        "com.resonance.drums",
        vec![output_mode(1.0)],
    );
    assert_eq!(sub_tracks(&app).len(), 7);

    let mut other = self::app();
    add(&mut other, 51, "com.example.multiout", Vec::new());
    assert_eq!(
        sub_tracks(&other).len(),
        7,
        "not a drums instance: as before"
    );
}
