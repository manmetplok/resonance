//! What `*.plugin_params` says a parameter MEANS, and what
//! `*.set_plugin_param` accepts as a value (ba todo #1290, finding X8).
//!
//! Before this, a client reading a filter type saw `value: 2.0` on a
//! `0..=4` range and had no way to learn that 2 is "Band-pass", and a Q
//! read `0.71` with no unit. The plugins knew both — 100+
//! `with_value_to_string` call sites — and the wire threw all of it
//! away.
//!
//! These drive the whole slice from the app's side: the engine echo
//! fills a parameter list carrying text/unit/module/stepped/choices/
//! hidden, `track`, `bus` and `master` all report it, a write may name a
//! choice instead of its number, a wrong label comes back with the
//! accepted ones, and the engine's `PluginParamText` echo keeps the text
//! honest after the value moves.

use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_control::methods::track::PluginParamsView;
use resonance_control::{ErrorKind, MutationAck, Response};
use crate::common::call;

const TRACK: u64 = 1;
const DELAY: u64 = 20;
const PLUGIN: &str = "com.resonance.delay";

const FILTER: u32 = 1;
const MIX: u32 = 2;
const DIVISION: u32 = 3;
const INTERNAL: u32 = 4;

/// The parameter list an engine echo delivers: one choice parameter,
/// one continuous parameter with a unit, one choice parameter whose
/// range does NOT start at zero (the labels are indexed from the
/// minimum, not from 0), and one the plugin asks to keep hidden.
fn params() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: FILTER,
            name: "Filter Type".to_owned(),
            min_value: 0.0,
            max_value: 2.0,
            default_value: 0.0,
            current_value: 1.0,
            text: "Band-pass".to_owned(),
            stepped: true,
            choices: vec![
                "Low-pass".to_owned(),
                "Band-pass".to_owned(),
                "High-pass".to_owned(),
            ],
            module: "Voice/Filter".to_owned(),
            ..Default::default()
        },
        ParamInfo {
            id: MIX,
            name: "Mix".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.35,
            current_value: 0.4,
            text: "40 %".to_owned(),
            unit: "%".to_owned(),
            ..Default::default()
        },
        ParamInfo {
            id: DIVISION,
            name: "Division".to_owned(),
            min_value: 1.0,
            max_value: 3.0,
            default_value: 1.0,
            current_value: 1.0,
            text: "1/4".to_owned(),
            stepped: true,
            choices: vec!["1/4".to_owned(), "1/8".to_owned(), "1/8T".to_owned()],
            ..Default::default()
        },
        ParamInfo {
            id: INTERNAL,
            name: "Internal Trim".to_owned(),
            min_value: -1.0,
            max_value: 1.0,
            default_value: 0.0,
            current_value: 0.0,
            text: "0.00 dB".to_owned(),
            unit: "dB".to_owned(),
            hidden: true,
            ..Default::default()
        },
    ]
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-param-meta.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/delay.clap".to_owned(),
            clap_plugin_id: PLUGIN.to_owned(),
            name: "Resonance Delay".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
        ..Default::default()
}],
    });
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: DELAY,
        plugin_name: "Resonance Delay".to_owned(),
        clap_plugin_id: PLUGIN.to_owned(),
        clap_file_path: "/plugins/delay.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    app
}

fn track_params(app: &mut Resonance) -> PluginParamsView {
    call(
        app,
        "track.plugin_params",
        serde_json::json!({"track_id": TRACK}),
    )
    .result()
    .expect("track.plugin_params succeeds")
}

/// Set a parameter with any JSON value — a number or a label.
fn set(app: &mut Resonance, param: &str, value: serde_json::Value) -> Response {
    call(
        app,
        "track.set_plugin_param",
        serde_json::json!({
            "track_id": TRACK,
            "plugin_id": PLUGIN,
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

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[test]
fn a_choice_parameter_reports_its_labels_and_group() {
    let mut app = app();
    let view = track_params(&mut app);
    let filter = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == FILTER)
        .expect("Filter Type is reported");

    assert_eq!(filter.value, 1.0);
    // The three fields that turn that 1.0 into something an agent can
    // act on.
    assert_eq!(filter.text, "Band-pass");
    assert!(filter.stepped);
    assert_eq!(
        filter.choices,
        vec!["Low-pass", "Band-pass", "High-pass"],
        "the labels are what set_plugin_param accepts by name"
    );
    assert_eq!(filter.module, "Voice/Filter");
}

#[test]
fn a_continuous_parameter_reports_its_unit_and_formatting() {
    let mut app = app();
    let view = track_params(&mut app);
    let mix = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == MIX)
        .expect("Mix is reported");
    assert_eq!(mix.value, 0.4);
    assert_eq!(mix.text, "40 %", "0.40 and 40 % are the same number");
    assert_eq!(mix.unit, "%");
    assert!(!mix.stepped);
    assert!(mix.choices.is_empty());
}

#[test]
fn a_hidden_parameter_is_reported_flagged() {
    let mut app = app();
    let view = track_params(&mut app);
    let internal = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == INTERNAL)
        .expect("a hidden parameter is still readable and writable");
    assert!(internal.hidden);
    assert_eq!(
        view.plugins[0].params.iter().filter(|p| p.hidden).count(),
        1,
        "only the parameter the plugin flagged"
    );
}

// ---------------------------------------------------------------------------
// Writing by name
// ---------------------------------------------------------------------------

#[test]
fn a_choice_label_resolves_to_its_value() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let _: MutationAck = set(&mut app, "Filter Type", serde_json::json!("High-pass"))
        .result()
        .expect("a label the parameter reports must be accepted");
    assert_eq!(dispatched(&rx, FILTER), 2.0);
}

#[test]
fn a_choice_label_is_matched_case_insensitively() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let _: MutationAck = set(&mut app, "Filter Type", serde_json::json!("low-PASS"))
        .result()
        .expect("labels match case-insensitively, like parameter names");
    assert_eq!(dispatched(&rx, FILTER), 0.0);
}

#[test]
fn labels_are_indexed_from_the_parameters_minimum() {
    // Division runs 1..=3, so its second label is the value 2 — not the
    // index 1. Getting this wrong would silently pick the neighbouring
    // note division.
    let mut app = app();
    let rx = app.test_capture_engine();
    let _: MutationAck = set(&mut app, "Division", serde_json::json!("1/8"))
        .result()
        .expect("1/8 is a declared choice");
    assert_eq!(dispatched(&rx, DIVISION), 2.0);
}

#[test]
fn a_number_still_works_and_so_does_a_stringified_one() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let _: MutationAck = set(&mut app, "Filter Type", serde_json::json!(2))
        .result()
        .expect("numbers were always accepted and must stay accepted");
    assert_eq!(dispatched(&rx, FILTER), 2.0);

    let rx = app.test_capture_engine();
    let _: MutationAck = set(&mut app, "Mix", serde_json::json!("0.75"))
        .result()
        .expect("a client that stringifies its numbers is not making a choice-label mistake");
    assert_eq!(dispatched(&rx, MIX), 0.75);
}

#[test]
fn an_unknown_label_is_refused_with_the_ones_that_work() {
    let mut app = app();
    let response = set(&mut app, "Filter Type", serde_json::json!("Notch"));
    let error = response.error.expect("an unknown label is not a value");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    for expected in ["Low-pass", "Band-pass", "High-pass"] {
        assert!(
            error.message.contains(expected),
            "the error must list the accepted labels so the caller can correct itself; got {:?}",
            error.message
        );
    }
}

/// Send `track.set_plugin_param` with a label and hand back the reply
/// channel (nothing on it yet when the label went to the plugin) and the
/// token of the `ResolvePluginParamText` question the engine received.
fn set_label_deferred(
    app: &mut Resonance,
    rx: &crossbeam_channel::Receiver<AudioCommand>,
    param: &str,
    label: &str,
) -> (crossbeam_channel::Receiver<Response>, u64) {
    let (reply, replies) = resonance_app::control_socket::ReplySender::test_pair();
    let request = resonance_control::Request::new(
        7,
        "track.set_plugin_param",
        &serde_json::json!({ "track_id": TRACK, "plugin_id": PLUGIN, "param": param, "value": label }),
    )
    .unwrap();
    let _ = app.update(resonance_app::message::Message::Control(
        resonance_app::control_socket::ControlMessage::Request(
            resonance_app::control_socket::ControlRequest {
                conn: 1,
                request,
                reply,
            },
        ),
    ));
    let token = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::ResolvePluginParamText {
                instance_id,
                param_id,
                text,
                token,
            } => {
                assert_eq!(instance_id, DELAY);
                assert_eq!(param_id, MIX);
                assert_eq!(text, label);
                Some(token)
            }
            _ => None,
        })
        .expect("the label went to the plugin");
    (replies, token)
}

#[test]
fn a_label_on_a_choiceless_parameter_is_resolved_by_the_plugin_without_blocking() {
    // nam-model-library.md §9.2: with no `choices` to match, the app asks
    // the plugin (CLAP `text_to_value`) — through the engine, answering the
    // request only when the engine answers, so the update loop never waits.
    let mut app = app();
    let rx = app.test_capture_engine();
    let revision = app.revision();
    let (replies, token) = set_label_deferred(&mut app, &rx, "Mix", "half");
    assert!(replies.try_recv().is_err(), "no reply until the plugin has answered");
    assert_eq!(app.revision(), revision);

    app.test_apply_engine_event(AudioEvent::PluginParamTextResolved {
        token,
        value: Some(0.5),
    });
    let ack: MutationAck = replies
        .try_recv()
        .expect("the reply goes out with the answer")
        .result()
        .expect("the plugin knows what \"half\" means");
    assert_eq!(ack.revision, revision + 1, "one call, one revision");
    assert_eq!(dispatched(&rx, MIX), 0.5, "the plugin's answer is what the engine is told to set");
}

#[test]
fn a_label_on_a_parameter_with_no_choices_says_so() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let (replies, token) = set_label_deferred(&mut app, &rx, "Mix", "loud");
    // The engine's answer for a text the plugin does not display back (a
    // lenient plugin's "loud" → 0 does not round-trip).
    app.test_apply_engine_event(AudioEvent::PluginParamTextResolved { token, value: None });
    let error = replies.try_recv().unwrap().error.expect("Mix names no choices");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("names no choices"), "{}", error.message);
    assert!(error.message.contains("nor does the plugin recognise"), "{}", error.message);
}

#[test]
fn a_label_the_plugin_never_answers_times_out() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let (replies, _token) = set_label_deferred(&mut app, &rx, "Mix", "loud");
    app.test_expire_pending_labels(std::time::Instant::now());
    assert!(replies.try_recv().is_err(), "not before its deadline");
    app.test_expire_pending_labels(std::time::Instant::now() + std::time::Duration::from_secs(10));
    let error = replies.try_recv().unwrap().error.expect("an unanswered label is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("did not answer in time"), "{}", error.message);
}

#[test]
fn a_pending_label_holds_the_fast_tick() {
    // The answer arrives as an engine event, drained on the tick: at the
    // 200 ms idle rate the reply would wait that long for nothing.
    use resonance_app::update::tick;
    let mut app = app();
    let rx = app.test_capture_engine();
    let slow = std::time::Duration::from_millis(tick::TICK_INTERVAL_IDLE_MS);
    assert_eq!(tick::tick_interval(&app), slow);
    let (_replies, token) = set_label_deferred(&mut app, &rx, "Mix", "half");
    assert_ne!(tick::tick_interval(&app), slow, "a label in flight needs the fast tick");
    app.test_apply_engine_event(AudioEvent::PluginParamTextResolved { token, value: Some(0.5) });
    assert_eq!(tick::tick_interval(&app), slow);
}

#[test]
fn a_deferred_label_set_is_one_undo_entry() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let before = app.test_undo_history().undo_len();
    let (replies, token) = set_label_deferred(&mut app, &rx, "Mix", "half");
    app.test_apply_engine_event(AudioEvent::PluginParamTextResolved { token, value: Some(0.5) });
    replies.try_recv().unwrap().result::<MutationAck>().expect("applied");
    assert_eq!(app.test_undo_history().undo_len(), before + 1, "one call, one undo entry");
}

#[test]
fn a_label_whose_plugin_went_away_meanwhile_is_not_found() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let (replies, token) = set_label_deferred(&mut app, &rx, "Mix", "half");
    app.test_apply_engine_event(AudioEvent::PluginRemoved { track_id: TRACK, instance_id: DELAY });
    let revision = app.revision();
    app.test_apply_engine_event(AudioEvent::PluginParamTextResolved { token, value: Some(0.5) });
    let error = replies.try_recv().unwrap().error.expect("the plugin is gone");
    assert_eq!(error.kind(), ErrorKind::NotFound, "{}", error.message);
    assert_eq!(app.revision(), revision, "nothing was changed");
}

#[test]
fn a_label_whose_client_disconnected_meanwhile_changes_nothing() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let revision = app.revision();
    let (replies, token) = set_label_deferred(&mut app, &rx, "Mix", "half");
    drop(replies);
    let _ = app.update(resonance_app::message::Message::Control(
        resonance_app::control_socket::ControlMessage::Disconnected { conn: 1 },
    ));
    app.test_apply_engine_event(AudioEvent::PluginParamTextResolved { token, value: Some(0.5) });
    assert_eq!(app.revision(), revision, "no client to tell, so no edit made");
    assert!(
        !std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::SetPluginParam { param_id: MIX, .. })),
        "the engine was not told to set Mix"
    );
}

#[test]
fn only_a_round_tripping_answer_counts() {
    use resonance_audio::label_round_trips;
    assert!(label_round_trips("half", "Half"));
    assert!(label_round_trips("-6 dB", "-6.02 dB"), "numbers agree to the input's precision");
    assert!(label_round_trips("0.5", "0.50"));
    assert!(label_round_trips("friedman", "Friedman BE-100 · standard"), "a name prefix");
    assert!(!label_round_trips("loud", "0.0 %"), "JUCE parses \"loud\" as 0; that is not a match");
    assert!(!label_round_trips("-12 dB", "-6.02 dB"));
    assert!(!label_round_trips("fr", "Friedman"), "too short to be a prefix");
    assert!(!label_round_trips("", "0"));
}

#[test]
fn a_number_outside_the_range_still_reports_the_range() {
    let mut app = app();
    let response = set(&mut app, "Mix", serde_json::json!(4.0));
    let error = response.error.expect("4.0 is outside 0..=1");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("must be within"),
        "the pre-existing bounds message is unchanged; got {:?}",
        error.message
    );
}

#[test]
fn a_non_finite_value_is_still_refused() {
    let mut app = app();
    // JSON has no NaN literal, so it arrives as the string form a client
    // would send — which is neither a number nor a choice.
    let response = set(&mut app, "Mix", serde_json::json!("NaN"));
    let error = response.error.expect("NaN is not a value");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("finite"),
        "got {:?}",
        error.message
    );
}

// ---------------------------------------------------------------------------
// Keeping the text honest after a write
// ---------------------------------------------------------------------------

#[test]
fn the_engine_echo_refreshes_the_text_a_read_reports() {
    let mut app = app();
    let _: MutationAck = set(&mut app, "Filter Type", serde_json::json!("High-pass"))
        .result()
        .expect("set succeeds");

    // The app mirrors the NUMBER immediately; only the plugin can say
    // what it is called, and it does so through this echo.
    app.test_apply_engine_event(AudioEvent::PluginParamText {
        instance_id: DELAY,
        param_id: FILTER,
        value: 2.0,
        text: "High-pass".to_owned(),
    });

    let view = track_params(&mut app);
    let filter = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == FILTER)
        .expect("Filter Type");
    assert_eq!(filter.value, 2.0);
    assert_eq!(filter.text, "High-pass");
}

#[test]
fn an_echo_overtaken_by_a_newer_write_is_dropped() {
    let mut app = app();
    let _: MutationAck = set(&mut app, "Filter Type", serde_json::json!("High-pass"))
        .result()
        .expect("set succeeds");

    // The echo for a value the parameter has already left — a knob drag
    // issues one set per frame. Applying it would paint text that
    // disagrees with the number beside it.
    app.test_apply_engine_event(AudioEvent::PluginParamText {
        instance_id: DELAY,
        param_id: FILTER,
        value: 0.0,
        text: "Low-pass".to_owned(),
    });

    let view = track_params(&mut app);
    let filter = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == FILTER)
        .expect("Filter Type");
    assert_eq!(filter.value, 2.0);
    assert_eq!(
        filter.text, "Band-pass",
        "the stale echo is ignored; the next one for 2.0 corrects the text"
    );
}

#[test]
fn an_echo_reveals_a_unit_the_load_time_text_did_not_carry() {
    let mut app = app();
    // A fader that read "-inf dB" when it loaded has no unit to take
    // from its text; the first value that formats as a number gives one.
    app.test_apply_engine_event(AudioEvent::PluginParamText {
        instance_id: DELAY,
        param_id: FILTER,
        value: 1.0,
        text: "Band-pass".to_owned(),
    });
    let view = track_params(&mut app);
    let filter = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == FILTER)
        .expect("Filter Type");
    assert_eq!(
        filter.unit, "",
        "a mode name is not a unit, and none was invented from it"
    );

    let _: MutationAck = set(&mut app, "Mix", serde_json::json!(0.75))
        .result()
        .expect("set succeeds");
    app.test_apply_engine_event(AudioEvent::PluginParamText {
        instance_id: DELAY,
        param_id: MIX,
        value: 0.75,
        text: "75 %".to_owned(),
    });
    let view = track_params(&mut app);
    let mix = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == MIX)
        .expect("Mix");
    assert_eq!(mix.text, "75 %");
    assert_eq!(mix.unit, "%");
}

// ---------------------------------------------------------------------------
// The same shape on a bus and on the master
// ---------------------------------------------------------------------------
//
// A bus and the master publish the SAME `PluginParamsEntry` a track does
// — that is the promise their own docs make — so a client reads all
// three chains with one code path. Three hand-written copies of the
// mapping is how one of them would quietly stop reporting a field.

const BUS_DELAY: u64 = 30;
const MASTER_DELAY: u64 = 40;

fn bus_with_delay(app: &mut Resonance) -> u64 {
    let result: resonance_control::methods::bus::CreateResult =
        call(app, "bus.create", serde_json::json!({"name": "FX Bus"}))
            .result()
            .expect("bus.create succeeds");
    let bus_id = result.bus_id.0;
    app.test_apply_engine_event(AudioEvent::BusPluginAdded {
        bus_id,
        instance_id: BUS_DELAY,
        plugin_name: "Resonance Delay".to_owned(),
        clap_plugin_id: PLUGIN.to_owned(),
        clap_file_path: "/plugins/delay.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
    });
    bus_id
}

#[test]
fn a_bus_reports_the_same_meaning_and_takes_the_same_label() {
    let mut app = app();
    let bus_id = bus_with_delay(&mut app);

    let view: resonance_control::methods::bus::PluginParamsView = call(
        &mut app,
        "bus.plugin_params",
        serde_json::json!({"bus_id": bus_id}),
    )
    .result()
    .expect("bus.plugin_params succeeds");
    let filter = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == FILTER)
        .expect("Filter Type on the bus");
    assert_eq!(filter.text, "Band-pass");
    assert_eq!(filter.choices, vec!["Low-pass", "Band-pass", "High-pass"]);
    assert_eq!(filter.module, "Voice/Filter");
    assert!(filter.stepped);

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "bus.set_plugin_param",
        serde_json::json!({
            "bus_id": bus_id,
            "plugin_id": PLUGIN,
            "param": "Filter Type",
            "value": "High-pass",
        }),
    )
    .result()
    .expect("a bus takes a choice label too");
    assert_eq!(dispatched(&rx, FILTER), 2.0);
}

#[test]
fn the_master_reports_the_same_meaning_and_takes_the_same_label() {
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id: MASTER_DELAY,
        plugin_name: "Resonance Delay".to_owned(),
        clap_plugin_id: PLUGIN.to_owned(),
        clap_file_path: "/plugins/delay.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
    });

    let view: resonance_control::methods::master::PluginParamsView =
        call(&mut app, "master.plugin_params", serde_json::json!({}))
            .result()
            .expect("master.plugin_params succeeds");
    let mix = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == MIX)
        .expect("Mix on the master");
    assert_eq!(mix.text, "40 %");
    assert_eq!(mix.unit, "%");

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "master.set_plugin_param",
        serde_json::json!({
            "plugin_id": PLUGIN,
            "param": "Division",
            "value": "1/8T",
        }),
    )
    .result()
    .expect("the master takes a choice label too");
    assert_eq!(
        dispatched(&rx, DIVISION),
        3.0,
        "the third label of a 1..=3 parameter is the value 3"
    );
}
