//! The built `resonance-color` bundle (warmth-width-depth.md §6.1) loads
//! through the engine's real add handlers on a track and on a bus, reports
//! its whole parameter surface, a GUI and no sidechain port, and publishes
//! the factory bank skills name.
//!
//! Skips rather than fails when the workspace hasn't been bundled
//! (`scripts/bundle.sh`) — the contract every test under `tests/clap_host/`
//! that needs a built plugin follows.

use resonance_audio::test_support::{ClapBundle, EngineHandlerHarness};
use resonance_audio::types::AudioEvent;
use resonance_audio::Track;

const COLOR_CLAP_ID: &str = "com.resonance.color";
const TRACK: u64 = 1;
const BUS: u64 = 7;

/// The ids `plugins/resonance-color/src/params.rs` declares, by display
/// name as the bridge reports them.
const PARAM_NAMES: [&str; 11] = [
    "Mode",
    "Drive",
    "Bias",
    "Response",
    "Tone",
    "Mix",
    "Auto Gain",
    "Output",
    "Oversample",
    "Speed",
    "Flutter",
];

fn bundle_path() -> Option<std::path::PathBuf> {
    let candidates = [
        std::path::PathBuf::from("target/bundled/resonance-color.clap"),
        std::path::PathBuf::from("../target/bundled/resonance-color.clap"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

#[test]
fn the_color_bundle_loads_on_a_track_and_a_bus() {
    let Some(path) = bundle_path() else {
        eprintln!("target/bundled/resonance-color.clap absent — run scripts/bundle.sh; skipping");
        return;
    };
    let path = path.to_string_lossy().into_owned();

    let mut harness = EngineHandlerHarness::new();
    harness.push_track(Track::new(TRACK, "Vocal".to_string()));
    harness.add_bus(BUS, Some("Drum Bus".to_string()));
    harness.drain_events();

    harness.add_plugin(TRACK, path.clone(), COLOR_CLAP_ID.to_string(), 100);
    let track_events = harness.drain_events();
    let bus_events = harness.add_plugin_to_bus(BUS, path, COLOR_CLAP_ID.to_string(), 101);

    let track_params = track_events
        .iter()
        .find_map(|e| match e {
            AudioEvent::PluginAdded {
                track_id,
                clap_plugin_id,
                params,
                has_gui,
                has_sidechain_input,
                ..
            } if *track_id == TRACK && clap_plugin_id == COLOR_CLAP_ID => {
                assert!(*has_gui, "the plugin ships an editor");
                assert!(!*has_sidechain_input, "the plugin has no key port");
                Some(params.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no PluginAdded for the track: {track_events:?}"));
    let bus_params = bus_events
        .iter()
        .find_map(|e| match e {
            AudioEvent::BusPluginAdded {
                bus_id,
                clap_plugin_id,
                params,
                ..
            } if *bus_id == BUS && clap_plugin_id == COLOR_CLAP_ID => Some(params.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no BusPluginAdded for the bus: {bus_events:?}"));

    for params in [&track_params, &bus_params] {
        let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, PARAM_NAMES, "the declared surface, in order");
        let mode = &params[0];
        assert!(mode.stepped, "Mode is a choice parameter");
        assert_eq!((mode.min_value, mode.max_value), (0.0, 4.0), "five modes");
    }
    assert_eq!(harness.plugin_instance_count(), 2);
}

#[test]
fn the_color_bundle_publishes_the_spec_preset_names() {
    let Some(path) = bundle_path() else {
        eprintln!("target/bundled/resonance-color.clap absent — run scripts/bundle.sh; skipping");
        return;
    };
    let bundle = ClapBundle::load(&path).expect("the Color bundle should load");
    let names: Vec<&str> = bundle.factory_presets().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "Bus — Warm Glue",
            "Bass — Iron",
            "Vocal — Tube Air",
            "Drums — Tape 15",
            "Master — Subtle Tape"
        ]
    );
}
