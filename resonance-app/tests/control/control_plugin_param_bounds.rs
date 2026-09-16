//! `track.set_plugin_param` and f32-declared parameter bounds (ba doc
//! #273, todo #1235).
//!
//! The wire carries f64, but plugins declare their ranges in f32:
//! nih-plug's `FloatRange::Skewed { min: 0.1, .. }` for the compressor's
//! attack widens to `0.10000000149011612`. An exact comparison therefore
//! rejected `0.1` — the number `track.plugin_params` had just
//! documented — as out of range. These assert the tolerance band accepts
//! a value that rounds onto a bound, clamps it into the true range
//! before it reaches the DSP, and still refuses anything genuinely
//! outside.

use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_control::methods::track::PluginParamsView;
use resonance_control::{ErrorKind, MutationAck, Response};
use crate::common::call;

const TRACK: u64 = 1;
const COMPRESSOR: u64 = 20;

/// `0.1f32` widened to f64 — what a plugin's f32-declared minimum
/// actually reaches the control API as.
const F32_MIN_TENTH: f64 = 0.1f32 as f64;
/// `20000.0f32` is exact; `0.9f32` is not — the max-bound counterpart.
const F32_MAX_NINE_TENTHS: f64 = 0.9f32 as f64;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-param-bounds.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/compressor.clap".to_owned(),
            clap_plugin_id: "com.resonance.compressor".to_owned(),
            name: "Resonance Compressor".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
        ..Default::default()
}],
    });
    // A chain whose bounds are exactly what nih-plug's f32 declarations
    // widen to, including one range (20..20000) big enough that a fixed
    // absolute epsilon would be the wrong tolerance.
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: COMPRESSOR,
        plugin_name: "Resonance Compressor".to_owned(),
        clap_plugin_id: "com.resonance.compressor".to_owned(),
        clap_file_path: "/plugins/compressor.clap".to_owned(),
        params: vec![
            ParamInfo {
                id: 1,
                name: "Attack".to_owned(),
                min_value: F32_MIN_TENTH,
                max_value: 200.0f32 as f64,
                default_value: 10.0,
                current_value: 10.0,
                ..Default::default()
            },
            ParamInfo {
                id: 2,
                name: "Mix".to_owned(),
                min_value: 0.0,
                max_value: F32_MAX_NINE_TENTHS,
                default_value: 0.5,
                current_value: 0.5,
                ..Default::default()
            },
            ParamInfo {
                id: 3,
                name: "Cutoff".to_owned(),
                min_value: 20.0,
                max_value: 20000.0,
                default_value: 1000.0,
                current_value: 1000.0,
                ..Default::default()
            },
        ],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    app
}

fn set(app: &mut Resonance, param: &str, value: f64) -> Response {
    call(
        app,
        "track.set_plugin_param",
        serde_json::json!({
            "track_id": TRACK,
            "plugin_id": "com.resonance.compressor",
            "param": param,
            "value": value,
        }),
    )
}

/// The value the engine was actually told to apply for `param_id`.
fn dispatched(rx: &crossbeam_channel::Receiver<AudioCommand>, param_id: u32) -> f64 {
    std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::SetPluginParam {
                param_id: id,
                value,
                ..
            } if id == param_id => Some(value),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no SetPluginParam for param {param_id} reached the engine"))
}

#[test]
fn the_reported_minimum_carries_the_f32_widening_tail() {
    let mut app = app();
    let view: PluginParamsView = call(
        &mut app,
        "track.plugin_params",
        serde_json::json!({"track_id": TRACK}),
    )
    .result()
    .expect("track.plugin_params succeeds");
    let attack = view.plugins[0]
        .params
        .iter()
        .find(|p| p.name == "Attack")
        .expect("Attack is reported");
    assert!(
        attack.min > 0.1,
        "the premise of this todo: the reported minimum {} is strictly above the 0.1 a caller \
         types",
        attack.min
    );
}

#[test]
fn a_value_that_rounds_onto_the_minimum_is_accepted_and_clamped_up() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let _: MutationAck = set(&mut app, "Attack", 0.1)
        .result()
        .expect("0.1 is the documented minimum and must be accepted");
    let value = dispatched(&rx, 1);
    assert_eq!(
        value, F32_MIN_TENTH,
        "the accepted value is clamped UP to the plugin's true minimum, so the DSP never sees \
         a value below what it declared"
    );
}

#[test]
fn a_value_that_rounds_onto_the_maximum_is_accepted_and_clamped_down() {
    let mut app = app();
    let rx = app.test_capture_engine();
    // 0.9 as f64 is strictly ABOVE 0.9f32-widened, so this is the
    // max-bound mirror of the attack case.
    assert!(0.9f64 > F32_MAX_NINE_TENTHS);
    let _: MutationAck = set(&mut app, "Mix", 0.9)
        .result()
        .expect("0.9 rounds onto the reported maximum and must be accepted");
    let value = dispatched(&rx, 2);
    assert_eq!(
        value, F32_MAX_NINE_TENTHS,
        "clamped DOWN to the plugin's true maximum"
    );
}

#[test]
fn a_value_comfortably_inside_the_range_passes_through_untouched() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let _: MutationAck = set(&mut app, "Attack", 12.5).result().expect("succeeds");
    assert_eq!(
        dispatched(&rx, 1),
        12.5,
        "an in-range value is never nudged"
    );
}

#[test]
fn clearly_out_of_range_values_are_still_refused_with_the_range() {
    let mut app = app();
    for (param, value) in [("Attack", 0.0), ("Attack", 500.0), ("Mix", -60.0)] {
        let error = set(&mut app, param, value)
            .error
            .unwrap_or_else(|| panic!("{param}={value} must be refused"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {param}={value}");
        assert!(
            error.message.contains("must be within"),
            "the rejection still names the range: {}",
            error.message
        );
    }
}

/// The tolerance scales with the magnitude of the range: on 20..20000 an
/// f32 ULP is ~0.001, so a hair over 20000 is accepted, while a value
/// that is wrong by any musically meaningful amount is not.
#[test]
fn the_tolerance_scales_with_the_range_rather_than_being_absolute() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let just_over = 20000.0f64 + 1e-4;
    let _: MutationAck = set(&mut app, "Cutoff", just_over)
        .result()
        .expect("a sub-ULP overshoot on a 20..20000 range is inside the tolerance");
    assert_eq!(
        dispatched(&rx, 3),
        20000.0,
        "and is clamped back onto the bound"
    );

    let error = set(&mut app, "Cutoff", 20001.0)
        .error
        .expect("1 Hz over is a real overshoot, not float noise");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    // The same absolute slack must NOT be granted on a 0..1 range.
    let error = set(&mut app, "Mix", 0.9 + 1e-4)
        .error
        .expect("1e-4 is far outside f32 precision at this magnitude");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
}

/// A plugin that reports a NaN bound must not panic the update loop.
///
/// `ParamInfo` takes `min_value`/`max_value` straight from the plugin
/// (`resonance-audio/src/clap_host/instance.rs`) with no sanitisation, so
/// a third-party CLAP can put a NaN here. The malformed-range guard has
/// to be written `!(min <= max)` rather than `min > max`, because NaN
/// fails every comparison and would otherwise reach
/// `f64::clamp`, which *asserts* `min <= max`.
#[test]
fn a_nan_bound_is_treated_as_a_malformed_range_rather_than_panicking() {
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: 21,
        plugin_name: "Rogue".to_owned(),
        clap_plugin_id: "com.example.rogue".to_owned(),
        clap_file_path: "/plugins/rogue.clap".to_owned(),
        params: vec![
            ParamInfo {
                id: 10,
                name: "NanMin".to_owned(),
                min_value: f64::NAN,
                max_value: 1.0,
                default_value: 0.5,
                current_value: 0.5,
                ..Default::default()
            },
            ParamInfo {
                id: 11,
                name: "NanMax".to_owned(),
                min_value: 0.0,
                max_value: f64::NAN,
                default_value: 0.5,
                current_value: 0.5,
                ..Default::default()
            },
        ],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    let rx = app.test_capture_engine();

    for (param, param_id) in [("NanMin", 10u32), ("NanMax", 11)] {
        let response = call(
            &mut app,
            "track.set_plugin_param",
            serde_json::json!({
                "track_id": TRACK,
                "plugin_id": "com.example.rogue",
                "param": param,
                "value": 0.25,
            }),
        );
        let _: MutationAck = response
            .result()
            .unwrap_or_else(|e| panic!("{param} must not be refused over a bogus bound: {e:?}"));
        assert_eq!(
            dispatched(&rx, param_id),
            0.25,
            "an unusable range means the value goes through as-is, unclamped",
        );
    }
}
