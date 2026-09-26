//! Mixer-inspector lazy fingerprints cover every field they draw
//! (review VIEW-08).
//!
//! The ROUTING + CHAIN groups sit in a `lazy(fp, …)` region, so a field
//! the region renders but the fingerprint leaves out freezes on screen.
//! The worst case was the chain's BYP button: it renders `bypassed` and
//! builds its press message from it, so after the first click's echo the
//! retained button still said "not bypassed" and kept sending
//! `bypassed: true` — the user could never un-bypass from the inspector.
//!
//! Each case asserts on the fingerprint directly, because a fresh
//! `Simulator` built from a fresh `view()` bypasses the retained cache and
//! would pass even with a stale key.

use crate::common::call;

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, UiMessage};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, AuxSend, SendSource, TrackType};
use resonance_common::PlaybackSource;

const TRACK: u64 = 1;
const BUS: u64 = 1;
const PLUGIN: u64 = 10;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app
}

fn plugin_added(track_id: u64) -> AudioEvent {
    AudioEvent::PluginAdded {
        track_id,
        instance_id: PLUGIN,
        plugin_name: "com.resonance.eq".to_owned(),
        clap_plugin_id: "com.resonance.eq".to_owned(),
        clap_file_path: "/plugins/eq.clap".to_owned(),
        params: Vec::new(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    }
}

fn bypass_echo(bypassed: bool) -> AudioEvent {
    AudioEvent::PluginBypassChanged {
        instance_id: PLUGIN,
        bypassed,
        own_bypass_param: false,
    }
}

#[test]
fn track_fingerprint_tracks_plugin_bypass() {
    let mut app = app();
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_apply_engine_event(plugin_added(TRACK));
    let live = app.test_inspector_fingerprint(TRACK).unwrap();

    app.test_apply_engine_event(bypass_echo(true));
    assert_eq!(app.test_plugin_bypass_flags().get(&PLUGIN), Some(&true));
    let bypassed = app.test_inspector_fingerprint(TRACK).unwrap();
    assert_ne!(live, bypassed, "the bypass echo must redraw the BYP button");

    app.test_apply_engine_event(bypass_echo(false));
    assert_eq!(app.test_inspector_fingerprint(TRACK).unwrap(), live);
}

#[test]
fn bus_fingerprint_tracks_plugin_bypass() {
    let mut app = app();
    app.test_add_bus(BUS, "Drum Bus");
    app.test_push_bus_plugin(
        BUS,
        resonance_app::state::PluginSlotState::new(
            PLUGIN,
            "com.resonance.eq".into(),
            "com.resonance.eq".into(),
            "/plugins/eq.clap".into(),
            Vec::new(),
            false,
        ),
    );
    let live = app.test_bus_inspector_fingerprint(BUS).unwrap();
    app.test_apply_engine_event(bypass_echo(true));
    assert_ne!(live, app.test_bus_inspector_fingerprint(BUS).unwrap());
}

#[test]
fn bus_fingerprint_tracks_send_source_names() {
    let mut app = app();
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_add_bus(BUS, "Verb");
    app.test_seed_aux_send(AuxSend {
        id: 1,
        source: SendSource::Track(TRACK),
        dest: BUS,
        level_db: -6.0,
        pre_fader: false,
        enabled: true,
    });
    let before = app.test_bus_inspector_fingerprint(BUS).unwrap();
    let reply = call(
        &mut app,
        "track.rename",
        serde_json::json!({ "track_id": TRACK, "name": "Renamed" }),
    );
    assert!(reply.result::<serde_json::Value>().is_ok(), "rename succeeds");
    assert_ne!(
        before,
        app.test_bus_inspector_fingerprint(BUS).unwrap(),
        "SENDS IN names its source track"
    );
}

#[test]
fn external_fingerprint_tracks_every_drawn_field() {
    let mut app = app();
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));
    let fp = |app: &Resonance| app.test_inspector_fingerprint(TRACK).unwrap();

    let base = fp(&app);
    let _ = app.update(Message::ExternalInstrument(Eim::SetPlaybackSource(
        TRACK,
        PlaybackSource::Recorded,
    )));
    let recorded = fp(&app);
    assert_ne!(base, recorded, "playback_source");

    let _ = app.update(Message::ExternalInstrument(Eim::DetectLatency(TRACK)));
    assert!(app.test_external_instrument(TRACK).unwrap().latency_detect_in_progress);
    let detecting = fp(&app);
    assert_ne!(recorded, detecting, "latency_detect_in_progress");

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyDetectFailed {
        track_id: TRACK,
        reason: "no signal".into(),
    });
    let failed = fp(&app);
    assert_ne!(detecting, failed, "latency_detect_error");

    app.test_set_transport_playing(true);
    assert_ne!(failed, fp(&app), "transport.playing gates the Detect button");
}

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(resonance_app::theme::ICON_FONT_BYTES.into());
    for face in resonance_app::theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: resonance_app::theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// The inspector CHAIN with a bypassed insert: BYP lit on that row.
#[test]
fn inspector_bypassed_plugin_golden() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    resonance_app::demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    let &instance_id = app
        .test_track_plugin_instance_ids(TRACK)
        .last()
        .expect("demo track 1 has a chain");
    app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
        instance_id,
        bypassed: true,
        own_bypass_param: false,
    });
    let mut ui = iced_test::simulator::Simulator::with_size(
        sim_settings(),
        iced::Size::new(1440.0, 900.0),
        app.view(),
    );
    let snap = ui
        .snapshot(&resonance_app::theme::resonance_theme())
        .expect("snapshot should render");
    crate::common::assert_golden(&snap, "tests/snapshots/mixer_inspector_plugin_bypassed.png");
}
