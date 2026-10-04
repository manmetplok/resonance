//! MIDI Learn on screen (doc #167, W1): the right-click MIDI menu on a
//! strip's controls, its Learn / Clear entries, the learning outline and
//! binding badge, the inspector's TRACK › MIDI CONTROL section, and
//! Settings › MIDI — pressed through the rendered view, asserting on the
//! messages it raises and the engine commands they send.

use iced::{Point, Size};
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, MidiMapMessage, UiMessage};
use resonance_app::state::{MixerInspectorGroup, ViewMode};
use resonance_app::update::keymap::{KeymapMsg, SettingsTab};
use resonance_app::{theme, Resonance};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_common::{CcMode, ControlSource, MidiTarget};

use crate::common;

const ONE: u64 = 1;
const TWO: u64 = 2;

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
    Simulator::with_size(sim_settings(), Size::new(1440.0, 900.0), app.view())
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    rx.try_iter().collect()
}

/// Two audio tracks on the mixer, track 1 selected, distinct fader
/// levels so each strip's dB label names it, and the inspector folded
/// down to its TRACK group.
fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/midi-learn-ui.rproj"));
    app.test_add_track(ONE, TrackType::Audio);
    app.test_add_track(TWO, TrackType::Audio);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Track(resonance_app::message::TrackMessage::SetTrackVolume(ONE, -3.0)));
    let _ = app.update(Message::Track(resonance_app::message::TrackMessage::SetTrackVolume(TWO, -9.0)));
    app.test_select_track(ONE);
    for g in [
        MixerInspectorGroup::Chain,
        MixerInspectorGroup::Sends,
        MixerInspectorGroup::Routing,
        MixerInspectorGroup::Automation,
    ] {
        let _ = app.update(Message::Ui(UiMessage::ToggleMixerInspectorGroup(g)));
    }
    let _ = drain(&rx);
    (app, rx)
}

fn cc(cc: u8) -> ControlSource {
    ControlSource::Cc {
        channel: 0,
        cc,
        mode: CcMode::Absolute,
    }
}

fn bind(app: &mut Resonance, target: MidiTarget, source: ControlSource) {
    let _ = app.update(Message::MidiMap(MidiMapMessage::Learn(target)));
    app.test_apply_engine_event(AudioEvent::MidiLearnCaptured { target, source });
}

/// Right-click the centre of the widget showing `label`; the messages
/// it raised.
fn right_click(app: &Resonance, label: &str) -> (Point, Vec<Message>) {
    let mut ui = simulator(app);
    let target = ui
        .find(label)
        .unwrap_or_else(|e| panic!("{label} is rendered: {e:?}"));
    let at = iced_test::selector::Bounded::visible_bounds(&target)
        .expect("visible")
        .center();
    ui.point_at(at);
    let _ = ui.simulate([
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Right)),
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Right)),
    ]);
    (at, ui.into_messages().collect())
}

fn click(app: &Resonance, label: &str) -> Vec<Message> {
    let mut ui = simulator(app);
    ui.click(label)
        .unwrap_or_else(|e| panic!("{label} should be clickable: {e:?}"));
    ui.into_messages().collect()
}

fn run(app: &mut Resonance, messages: Vec<Message>) {
    for m in messages {
        let _ = app.update(m);
    }
}

#[test]
fn right_clicking_a_fader_opens_its_midi_menu_under_the_pointer() {
    let (mut app, _rx) = app();
    let (at, messages) = right_click(&app, "-9.0");
    let opened = messages.iter().find_map(|m| match m {
        Message::MidiMap(MidiMapMessage::OpenMenu { target, x, y }) => Some((*target, *x, *y)),
        _ => None,
    });
    let (target, x, y) = opened.unwrap_or_else(|| panic!("{messages:?}"));
    assert_eq!(target, MidiTarget::TrackVolume(TWO));
    assert!((x - at.x).abs() < 0.5 && (y - at.y).abs() < 0.5, "({x}, {y}) vs {at:?}");

    run(&mut app, messages);
    assert!(app.test_midi_map().menu.is_some());
    // Esc closes it like any overlay.
    let _ = app.update(Message::MidiMap(MidiMapMessage::CloseMenu));
    assert!(app.test_midi_map().menu.is_none());
}

#[test]
fn the_menu_learns_and_clears_through_the_engine() {
    let (mut app, rx) = app();
    let target = MidiTarget::TrackVolume(ONE);
    let (_, messages) = right_click(&app, "-3.0");
    run(&mut app, messages);

    let learn = click(&app, "Learn MIDI");
    assert!(learn
        .iter()
        .any(|m| matches!(m, Message::MidiMap(MidiMapMessage::Learn(t)) if *t == target)));
    run(&mut app, learn);
    assert!(app.test_midi_map().menu.is_none(), "an entry closes the menu");
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::EnterMidiLearn { target: t } if *t == target)));

    // The capture: the badge on the fader now names the control.
    app.test_apply_engine_event(AudioEvent::MidiLearnCaptured {
        target,
        source: cc(7),
    });
    let mut ui = simulator(&app);
    assert!(ui.find("CC7").is_ok(), "the fader wears its binding");
    drop(ui);

    let (_, messages) = right_click(&app, "-3.0");
    run(&mut app, messages);
    let _ = drain(&rx);
    let clear = click(&app, "Clear MIDI binding");
    run(&mut app, clear);
    assert!(app.test_midi_map().bindings.is_empty());
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::ClearMidiBinding { .. })));
}

#[test]
fn mute_solo_and_pan_open_their_own_menus() {
    let (app, _rx) = app();
    // The first strip's M (speaker-x glyph), S (headphones) and pan
    // readout.
    let mute = theme::fa::VOLUME_XMARK.to_string();
    let solo = theme::fa::HEADPHONES.to_string();
    for (label, want) in [
        (mute.as_str(), MidiTarget::TrackMute(ONE)),
        (solo.as_str(), MidiTarget::TrackSolo(ONE)),
        ("C", MidiTarget::TrackPan(ONE)),
    ] {
        let (_, messages) = right_click(&app, label);
        assert!(
            messages.iter().any(|m| matches!(
                m,
                Message::MidiMap(MidiMapMessage::OpenMenu { target, .. }) if *target == want
            )),
            "{label}: {messages:?}"
        );
    }
}

#[test]
fn learn_state_and_bindings_move_the_lazy_fingerprints() {
    let (mut app, _rx) = app();
    let strip = app.test_track_strip_fingerprint(ONE).unwrap();
    let inspector = app.test_inspector_fingerprint(ONE).unwrap();
    let _ = app.update(Message::MidiMap(MidiMapMessage::Learn(MidiTarget::TrackPan(ONE))));
    assert_ne!(app.test_track_strip_fingerprint(ONE).unwrap(), strip, "pan outline");
    assert_ne!(app.test_inspector_fingerprint(ONE).unwrap(), inspector, "listening row");
    let armed = app.test_inspector_fingerprint(ONE).unwrap();
    app.test_apply_engine_event(AudioEvent::MidiLearnCaptured {
        target: MidiTarget::TrackPan(ONE),
        source: cc(10),
    });
    assert_ne!(app.test_inspector_fingerprint(ONE).unwrap(), armed, "binding row");
}

#[test]
fn the_inspector_offers_every_target_on_the_track() {
    let (app, _rx) = app();
    let labels: Vec<String> = app.test_learn_choices(ONE).into_iter().map(|(_, l)| l).collect();
    assert_eq!(labels, ["Volume", "Pan", "Mute", "Solo"]);
}

#[test]
fn the_settings_midi_page_lists_and_clears_bindings() {
    let (mut app, rx) = app();
    bind(&mut app, MidiTarget::TrackVolume(TWO), cc(7));
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    let tab = click(&app, "MIDI");
    run(&mut app, tab);
    let mut ui = simulator(&app);
    assert!(ui.find("Track 2 \u{b7} Volume").is_ok(), "the binding is listed");
    drop(ui);
    let _ = drain(&rx);
    let clear = click(&app, "Clear all");
    run(&mut app, clear);
    assert!(app.test_midi_map().bindings.is_empty());
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::ClearAllMidiBindings)));
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

/// Track 1's fader armed (accent outline + LEARN), track 2's bound to
/// CC 7, track 1's pan bound to CC 10 — and the inspector's MIDI CONTROL
/// listing the pan binding under its "Move a control" banner.
#[test]
fn mixer_midi_learn_state() {
    let (mut app, _rx) = app();
    bind(&mut app, MidiTarget::TrackVolume(TWO), cc(7));
    bind(&mut app, MidiTarget::TrackPan(ONE), cc(10));
    let _ = app.update(Message::MidiMap(MidiMapMessage::Learn(MidiTarget::TrackVolume(ONE))));
    let mut ui = simulator(&app);
    let snap = ui.snapshot(&theme::resonance_theme()).expect("snapshot");
    common::assert_golden(&snap, "tests/snapshots/mixer_midi_learn_state.png");
}

/// The MIDI menu over a bound fader: header, Learn (replace), the bound
/// control, Clear.
#[test]
fn mixer_midi_menu() {
    let (mut app, _rx) = app();
    bind(&mut app, MidiTarget::TrackVolume(TWO), cc(7));
    let (_, messages) = right_click(&app, "-9.0");
    run(&mut app, messages);
    let mut ui = simulator(&app);
    let snap = ui.snapshot(&theme::resonance_theme()).expect("snapshot");
    common::assert_golden(&snap, "tests/snapshots/mixer_midi_menu.png");
}

/// Settings › MIDI with an unplugged control surface, two bindings, a
/// learn armed and the map-name field.
#[test]
fn settings_midi_page() {
    let (mut app, _rx) = app();
    bind(&mut app, MidiTarget::TrackVolume(TWO), cc(7));
    bind(
        &mut app,
        MidiTarget::Transport(resonance_common::TransportAction::Play),
        ControlSource::Note { channel: 9, note: 36 },
    );
    let _ = app.update(Message::MidiMap(MidiMapMessage::SetControlSurfaceInput(Some(
        "nanoKONTROL2".into(),
    ))));
    let _ = app.update(Message::MidiMap(MidiMapMessage::Learn(MidiTarget::TrackPan(ONE))));
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    let _ = app.update(Message::Ui(UiMessage::Keymap(KeymapMsg::SetTab(SettingsTab::Midi))));
    let mut ui = simulator(&app);
    let snap = ui.snapshot(&theme::resonance_theme()).expect("snapshot");
    common::assert_golden(&snap, "tests/snapshots/settings_midi_page.png");
    drop(ui);
    let _ = app.update(Message::MidiMap(MidiMapMessage::SetControlSurfaceInput(None)));
}
