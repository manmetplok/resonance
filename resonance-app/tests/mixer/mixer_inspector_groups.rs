//! The restructured mixer inspector (mixer-cleanup.md §3, slices S1 +
//! S3): group order, SENDS as its own group, the TRACK group's mono and
//! bounce, the AUTOMATION group's lanes, master selection and the master
//! / bus inspectors' own actions.
//!
//! Every press goes through the rendered view (`Simulator::click`) and
//! asserts on the message the view raised, so these pin what the GUI
//! sends rather than a second copy of the wiring. The one exception is
//! the `+ Add lane` option: a closed `pick_list` renders only its
//! placeholder, so the option is resolved through the picker's own
//! option list (`test_inspector_add_lane_message`).

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{
    AutomationMessage, BusMessage, Message, ProjectIoMessage, TrackMessage, UiMessage,
};
use resonance_app::state::{MidiClipState, MixerInspectorGroup, PluginSlotState, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::types::TrackType;
use resonance_common::AutomationTarget;

const AUDIO: u64 = 1;
const INST: u64 = 2;
const BUS: u64 = 1;
const PLUGIN: u64 = 40;

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

/// Tall enough that the whole inspector stack is inside the viewport —
/// `click` refuses a target that is not visible.
fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(1440.0, 2000.0), app.view())
}

fn click(app: &Resonance, label: &str) -> Vec<Message> {
    let mut ui = simulator(app);
    ui.click(label)
        .unwrap_or_else(|e| panic!("{label} should be clickable: {e:?}"));
    ui.into_messages().collect()
}

fn ui(app: &mut Resonance, m: UiMessage) {
    let _ = app.update(Message::Ui(m));
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_add_track(INST, TrackType::Instrument);
    app.test_add_bus(BUS, "Drum Bus");
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn eq_slot() -> PluginSlotState {
    PluginSlotState::new(
        PLUGIN,
        "com.resonance.eq".into(),
        "com.resonance.eq".into(),
        "/plugins/eq.clap".into(),
        Vec::new(),
        false,
    )
}

/// The vertical position of the first widget reading `label`.
fn y_of(ui: &mut Simulator<'_, Message>, label: &str) -> f32 {
    ui.find(label)
        .unwrap_or_else(|e| panic!("{label} should render: {e:?}"))
        .bounds()
        .y
}

// ---------------------------------------------------------------------------
// Track groups
// ---------------------------------------------------------------------------

#[test]
fn track_groups_render_chain_sends_routing_automation_track() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(INST)));
    let mut sim = simulator(&app);
    let order = ["CHAIN", "SENDS", "ROUTING", "AUTOMATION", "TRACK"];
    let ys: Vec<f32> = order.iter().map(|l| y_of(&mut sim, l)).collect();
    for (pair, y) in order.windows(2).zip(ys.windows(2)) {
        assert!(y[0] < y[1], "{} must sit above {} ({ys:?})", pair[0], pair[1]);
    }
    assert!(sim.find("SIGNAL").is_err(), "SIGNAL was dropped");
    assert!(sim.find("PEAK").is_err(), "and its tiles with it");
}

/// SENDS folds on its own key — collapsing it hides the send list but
/// not ROUTING's pickers, and collapsing ROUTING leaves SENDS open.
#[test]
fn sends_is_its_own_group() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(AUDIO)));
    simulator(&app).find("No sends").expect("SENDS open by default");

    ui(&mut app, UiMessage::ToggleMixerInspectorGroup(MixerInspectorGroup::Sends));
    {
        let mut sim = simulator(&app);
        assert!(sim.find("No sends").is_err(), "SENDS folded");
        sim.find("INPUT DEVICE").expect("ROUTING still open");
    }

    ui(&mut app, UiMessage::ToggleMixerInspectorGroup(MixerInspectorGroup::Sends));
    ui(&mut app, UiMessage::ToggleMixerInspectorGroup(MixerInspectorGroup::Routing));
    let mut sim = simulator(&app);
    sim.find("No sends").expect("SENDS survives ROUTING folding");
    assert!(sim.find("INPUT DEVICE").is_err(), "ROUTING folded");
}

#[test]
fn track_group_mono_dispatches_toggle_mono() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(AUDIO)));
    let messages = click(&app, "MONO");
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Track(TrackMessage::ToggleTrackMono(id)) if *id == AUDIO)),
        "MONO raises ToggleTrackMono: {messages:?}"
    );
    // An audio track has nothing to bounce in place.
    assert!(simulator(&app).find("BOUNCE").is_err());
}

/// Bounce follows `classify_bounce`: disabled (with its reason shown)
/// on an instrument track with no MIDI, live once there is a clip and a
/// synth to render it.
#[test]
fn track_group_bounce_follows_classify_bounce() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(INST)));
    {
        let messages = click(&app, "BOUNCE");
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, Message::Track(TrackMessage::BounceInPlace(_)))),
            "a track with no MIDI clips must not bounce: {messages:?}"
        );
        simulator(&app)
            .find("Source track has no MIDI clips to bounce")
            .expect("the reason is shown, not a dead button");
    }

    app.test_push_track_plugin(INST, eq_slot());
    app.test_push_midi_clip(MidiClipState {
        id: 900,
        track_id: INST,
        start_sample: 0,
        duration_ticks: 3840,
        name: "riff".into(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    let messages = click(&app, "BOUNCE");
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Track(TrackMessage::BounceInPlace(id)) if *id == INST)),
        "BOUNCE raises BounceInPlace: {messages:?}"
    );
}

/// The fingerprint keys the TRACK group's Bounce state, or the button
/// would stay disabled after the first clip lands.
#[test]
fn track_fingerprint_tracks_bounce_inputs() {
    let mut app = app();
    app.test_push_track_plugin(INST, eq_slot());
    let before = app.test_inspector_fingerprint(INST).unwrap();
    app.test_push_midi_clip(MidiClipState {
        id: 901,
        track_id: INST,
        start_sample: 0,
        duration_ticks: 3840,
        name: "riff".into(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    assert_ne!(before, app.test_inspector_fingerprint(INST).unwrap());
}

// ---------------------------------------------------------------------------
// AUTOMATION
// ---------------------------------------------------------------------------

#[test]
fn automation_add_lane_then_read_toggle() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(AUDIO)));
    simulator(&app)
        .find("No automation lanes")
        .expect("empty AUTOMATION placeholder");
    let fp_empty = app.test_inspector_fingerprint(AUDIO).unwrap();

    // The picker's "Volume" option adds the track's gain lane.
    let add = app
        .test_inspector_add_lane_message(Some(AUDIO), "Volume")
        .expect("Volume is offered");
    assert!(matches!(
        &add,
        Message::Automation(AutomationMessage::AddLane(AutomationTarget::TrackGain(id))) if *id == AUDIO
    ));
    let _ = app.update(add);
    let fp_lane = app.test_inspector_fingerprint(AUDIO).unwrap();
    assert_ne!(fp_empty, fp_lane, "a new lane redraws the group");

    // The lane row renders and its READ toggle raises ToggleRead.
    let messages = click(&app, "READ");
    let toggle = messages
        .into_iter()
        .find(|m| {
            matches!(
                m,
                Message::Automation(AutomationMessage::ToggleRead(AutomationTarget::TrackGain(id))) if *id == AUDIO
            )
        })
        .expect("READ raises ToggleRead for the gain lane");
    let _ = app.update(toggle);
    let lane = app
        .test_automation()
        .lanes
        .get(&AutomationTarget::TrackGain(AUDIO))
        .expect("lane exists");
    assert!(!lane.enabled, "Read toggled off");
    assert_ne!(
        fp_lane,
        app.test_inspector_fingerprint(AUDIO).unwrap(),
        "the Read state is part of the key"
    );
}

/// The inspector lists every lane on the channel, not only the strip
/// header's primary one, and names plugin params by plugin and param.
#[test]
fn automation_lists_every_lane() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(AUDIO)));
    for label in ["Volume", "Pan", "Mute"] {
        let add = app.test_inspector_add_lane_message(Some(AUDIO), label).unwrap();
        let _ = app.update(add);
    }
    let mut sim = simulator(&app);
    let (v, p, m) = (y_of(&mut sim, "Volume"), y_of(&mut sim, "Pan"), y_of(&mut sim, "Mute"));
    assert!(v < p && p < m, "priority order gain, pan, mute");
}

// ---------------------------------------------------------------------------
// Master
// ---------------------------------------------------------------------------

#[test]
fn master_selection_is_exclusive_with_track_and_bus() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(AUDIO)));
    ui(&mut app, UiMessage::SelectMaster);
    assert!(app.test_selected_master());
    assert_eq!(app.test_selected_track(), None);
    assert!(app.test_selected_tracks().is_empty());

    ui(&mut app, UiMessage::SelectBus(Some(BUS)));
    assert!(!app.test_selected_master(), "bus takes it");
    assert_eq!(app.test_selected_bus(), Some(BUS));

    ui(&mut app, UiMessage::SelectMaster);
    assert_eq!(app.test_selected_bus(), None, "master takes it back");

    // Deselect-all (empty-space click) leaves the master alone, as it
    // does a bus.
    ui(&mut app, UiMessage::SelectTrack(None));
    assert!(app.test_selected_master());

    ui(&mut app, UiMessage::SelectTrack(Some(INST)));
    assert!(!app.test_selected_master(), "a track takes it");
    assert_eq!(app.test_selected_track(), Some(INST));
}

#[test]
fn clicking_the_master_strip_selects_master() {
    let app = app();
    let messages = click(&app, "MASTER");
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Ui(UiMessage::SelectMaster))),
        "the master strip raises SelectMaster: {messages:?}"
    );
}

#[test]
fn master_inspector_shows_chain_automation_and_bounce() {
    let mut app = app();
    app.test_push_master_plugin(eq_slot());
    ui(&mut app, UiMessage::SelectMaster);
    {
        let mut sim = simulator(&app);
        let (c, a, m) = (
            y_of(&mut sim, "CHAIN"),
            y_of(&mut sim, "AUTOMATION"),
            y_of(&mut sim, "BOUNCE TO WAV"),
        );
        assert!(c < a && a < m, "CHAIN, AUTOMATION, MASTER");
        sim.find("com.resonance.eq").expect("master insert row");
    }
    let messages = click(&app, "BOUNCE TO WAV");
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::ProjectIo(ProjectIoMessage::BounceToWav))),
        "the MASTER group's Bounce raises BounceToWav: {messages:?}"
    );

    // Master automation is gain only, and the lane lands in the group.
    assert!(app.test_inspector_add_lane_message(None, "Pan").is_none());
    let before = app.test_master_inspector_fingerprint();
    let add = app.test_inspector_add_lane_message(None, "Volume").unwrap();
    let _ = app.update(add);
    assert_ne!(before, app.test_master_inspector_fingerprint());
    simulator(&app).find("Volume").expect("master gain lane row");
}

/// The master inspector on the demo project, with a gain lane: CHAIN,
/// AUTOMATION (one lane row), MASTER (Bounce).
#[test]
fn master_inspector_golden() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    resonance_app::demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app.test_push_master_plugin(eq_slot());
    ui(&mut app, UiMessage::SelectMaster);
    let add = app.test_inspector_add_lane_message(None, "Volume").unwrap();
    let _ = app.update(add);
    let mut sim = Simulator::with_size(sim_settings(), Size::new(1440.0, 900.0), app.view());
    let snap = sim
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    crate::common::assert_golden(&snap, "tests/snapshots/mixer_inspector_master.png");
}

// ---------------------------------------------------------------------------
// Bus
// ---------------------------------------------------------------------------

#[test]
fn bus_inspector_delete_bus_raises_remove_bus() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectBus(Some(BUS)));
    {
        let mut sim = simulator(&app);
        let (c, r, a, b) = (
            y_of(&mut sim, "CHAIN"),
            y_of(&mut sim, "ROUTING"),
            y_of(&mut sim, "AUTOMATION"),
            y_of(&mut sim, "DELETE BUS"),
        );
        assert!(c < r && r < a && a < b, "CHAIN, ROUTING, AUTOMATION, BUS");
        assert!(sim.find("SIGNAL").is_err());
    }
    let messages = click(&app, "DELETE BUS");
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Bus(BusMessage::RemoveBus(id)) if *id == BUS)),
        "DELETE BUS raises RemoveBus: {messages:?}"
    );
}
