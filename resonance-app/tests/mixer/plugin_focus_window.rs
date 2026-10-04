//! The focused plugin slot (`MixerUiState::focused_slot`,
//! mixer-cleanup.md §2.1) and the generic window's life cycle.
//!
//! The preset commands (◀ / ▶ / browse) and the media tab's double-click
//! load used to target "the plugin the generic window shows" — which a
//! plugin with its own GUI never opens, so for GUI plugins they had no
//! target at all. They now act on the focused slot: set by opening any
//! window on the slot (`OpenPluginWindow`, `OpenPluginEditor`,
//! `OpenGenericParams`) or by `FocusSlot`, cleared when the slot goes
//! away or a project loads, and refused in Performance mode.
//!
//! The rest pins the window's own edges: it never shows (or eats Esc)
//! for a slot that no longer resolves, it is not drawn over the Arrange
//! track menu, a drag ends when the app window loses focus, and its
//! position stays inside the app window.

use iced::{Point, Size};
use iced_test::simulator::Simulator;
use resonance_app::commands::{CommandId, KeyChord, Mods, NamedKey};
use resonance_app::message::{
    Message, PluginMessage, PluginWindowDrag, PresetUiMessage, UiMessage,
};
use resonance_app::state::{PluginSlotState, ViewMode, PLUGIN_WINDOW_DEFAULT_POSITION};
use resonance_app::{theme, Resonance};
use resonance_audio::types::{ChainOwner, AudioEvent, ParamInfo, TrackType};

const TRACK: u64 = 1;
const BUS: u64 = 1;
const GUI: u64 = 81;
const PLAIN: u64 = 82;
const BUS_FX: u64 = 83;
const MASTER_FX: u64 = 84;
/// Drawn only by the generic window's parameter list.
const PARAM: &str = "Low Gain";

fn slot(instance: u64, name: &str, has_gui: bool) -> PluginSlotState {
    PluginSlotState::new(
        instance,
        name.to_owned(),
        format!("com.resonance.{instance}"),
        "/plugins/x.clap".to_owned(),
        vec![ParamInfo {
            id: 1,
            name: PARAM.to_owned(),
            min_value: -24.0,
            max_value: 24.0,
            ..Default::default()
        }],
        has_gui,
    )
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_add_bus(BUS, "Verb");
    app.test_push_track_plugin(TRACK, slot(GUI, "Gui Synth", true));
    app.test_push_track_plugin(TRACK, slot(PLAIN, "Plain EQ", false));
    app.test_push_bus_plugin(BUS, slot(BUS_FX, "Bus Comp", false));
    app.test_push_master_plugin(slot(MASTER_FX, "Limiter", false));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn plugin(app: &mut Resonance, m: PluginMessage) {
    let _ = app.update(Message::Plugin(m));
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![theme::ICON_FONT_BYTES.into()];
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    let settings = iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    };
    Simulator::with_size(settings, Size::new(1440.0, 1200.0), app.view())
}

fn window_drawn(app: &Resonance) -> bool {
    simulator(app).find(PARAM).is_ok()
}

fn escape(app: &mut Resonance) {
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured: false,
    }));
}

/// Whether `m` is a preset step / browse aimed at `instance`.
fn targets(m: &Message, instance: u64) -> bool {
    matches!(
        m,
        Message::Plugin(PluginMessage::PresetUi(
            PresetUiMessage::Step { instance_id, .. } | PresetUiMessage::OpenBrowser(instance_id)
        )) if *instance_id == instance
    )
}

// ---------------------------------------------------------------------------
// Focus and the preset commands
// ---------------------------------------------------------------------------

/// The headline: a GUI plugin opens its own editor (no generic window),
/// and the preset commands still have a target — the focused slot.
#[test]
fn opening_a_gui_plugin_focuses_it_for_the_preset_commands() {
    let mut app = app();
    assert_eq!(CommandId::NextPluginPreset.to_message(&app).map(|_| ()), None);

    plugin(&mut app, PluginMessage::OpenPluginWindow(GUI));
    assert_eq!(app.test_plugin_window(), None, "a GUI plugin opens its own editor");
    assert_eq!(app.test_focused_slot(), Some(GUI));
    for command in [
        CommandId::NextPluginPreset,
        CommandId::PreviousPluginPreset,
        CommandId::BrowsePluginPresets,
    ] {
        let m = command.to_message(&app).expect("the focused slot is the target");
        assert!(targets(&m, GUI), "{command:?} targets the GUI plugin: {m:?}");
    }
}

#[test]
fn open_plugin_editor_and_generic_window_focus_their_slot() {
    let mut app = app();
    plugin(&mut app, PluginMessage::OpenPluginEditor(GUI));
    assert_eq!(app.test_focused_slot(), Some(GUI));
    plugin(&mut app, PluginMessage::OpenPluginWindow(PLAIN));
    assert_eq!(app.test_plugin_window(), Some(PLAIN));
    assert_eq!(app.test_focused_slot(), Some(PLAIN));
    // Closing the window leaves the slot focused.
    plugin(&mut app, PluginMessage::ClosePluginWindow(PLAIN));
    assert_eq!(app.test_focused_slot(), Some(PLAIN));
    assert_eq!(app.test_preset_target(), Some(PLAIN));
}

/// "Params" opens the generic window even for a plugin with its own
/// GUI: the way to its parameter list and preset bar.
#[test]
fn open_generic_params_opens_the_generic_window_for_a_gui_plugin() {
    let mut app = app();
    plugin(&mut app, PluginMessage::OpenGenericParams(GUI));
    assert_eq!(app.test_plugin_window(), Some(GUI));
    assert_eq!(app.test_focused_slot(), Some(GUI));
    assert!(window_drawn(&app), "the generic window lists its parameters");
    // An unknown slot opens nothing.
    plugin(&mut app, PluginMessage::OpenGenericParams(999));
    assert_eq!(app.test_plugin_window(), Some(GUI));
}

/// `FocusSlot` focuses without opening anything, and selects the
/// channel the slot sits on — track, bus or master.
#[test]
fn focus_slot_selects_the_owner_without_opening_a_window() {
    let mut app = app();
    plugin(&mut app, PluginMessage::FocusSlot(BUS_FX));
    assert_eq!(app.test_focused_slot(), Some(BUS_FX));
    assert_eq!(app.test_selected_bus(), Some(BUS));
    assert_eq!(app.test_plugin_window(), None);

    plugin(&mut app, PluginMessage::FocusSlot(MASTER_FX));
    assert_eq!(app.test_focused_slot(), Some(MASTER_FX));
    assert!(app.test_selected_master());
    assert_eq!(app.test_selected_bus(), None);

    plugin(&mut app, PluginMessage::FocusSlot(PLAIN));
    assert_eq!(app.test_focused_slot(), Some(PLAIN));
    assert_eq!(app.test_selected_track(), Some(TRACK));
    assert!(!app.test_selected_master());

    plugin(&mut app, PluginMessage::FocusSlot(999));
    assert_eq!(app.test_focused_slot(), Some(PLAIN), "an unknown slot changes nothing");
}

/// The slot leaving its chain takes the focus (and the window) with it,
/// so a preset command cannot land on whatever plugin later reuses the
/// id.
#[test]
fn removing_the_focused_plugin_clears_the_focus() {
    let mut app = app();
    plugin(&mut app, PluginMessage::OpenGenericParams(PLAIN));
    app.test_apply_engine_event(AudioEvent::PluginRemoved {
        owner: ChainOwner::Track(TRACK),
        instance_id: PLAIN,
    });
    assert_eq!(app.test_focused_slot(), None);
    assert_eq!(app.test_plugin_window(), None);
    assert_eq!(CommandId::NextPluginPreset.to_message(&app).map(|_| ()), None);

    // A bus plugin, and a whole bus's chain.
    plugin(&mut app, PluginMessage::FocusSlot(BUS_FX));
    app.test_apply_engine_event(AudioEvent::BusRemoved { bus_id: BUS });
    assert_eq!(app.test_focused_slot(), None);
}

#[test]
fn a_project_load_clears_the_focus_and_the_window() {
    let mut app = app();
    plugin(&mut app, PluginMessage::OpenGenericParams(PLAIN));
    let file = app.test_build_project_file();
    app.test_replay_loaded_project(file);
    assert_eq!(app.test_focused_slot(), None);
    assert_eq!(app.test_plugin_window(), None);
}

/// Performance mode draws neither the window nor the inspector, so the
/// preset commands must not step a sound the user cannot see targeted.
#[test]
fn performance_mode_has_no_preset_target() {
    let mut app = app();
    plugin(&mut app, PluginMessage::OpenPluginWindow(GUI));
    assert_eq!(app.test_preset_target(), Some(GUI));
    let _ = app.update(Message::Ui(UiMessage::TogglePerformanceMode));
    assert_eq!(app.test_preset_target(), None);
    assert!(CommandId::NextPluginPreset.to_message(&app).is_none());
    assert!(!CommandId::NextPluginPreset.availability(&app).is_yes());
    let _ = app.update(Message::Ui(UiMessage::ExitPerformanceMode));
    assert_eq!(app.test_preset_target(), Some(GUI), "back on exit");
}

// ---------------------------------------------------------------------------
// Dead window state
// ---------------------------------------------------------------------------

/// An editor-failure echo for a slot that is already gone opens nothing.
#[test]
fn an_editor_failure_for_a_gone_slot_opens_no_window() {
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: 999,
        open: false,
        failure: Some(resonance_audio::types::PluginEditorFailure::CreateFailed),
    });
    assert_eq!(app.test_plugin_window(), None);
    assert_eq!(app.test_focused_slot(), None);
}

/// A window whose slot no longer resolves is neither drawn nor eats Esc.
#[test]
fn a_window_on_a_vanished_slot_is_not_visible_and_does_not_take_esc() {
    let mut app = app();
    plugin(&mut app, PluginMessage::OpenPluginWindow(PLAIN));
    assert!(window_drawn(&app));
    // Pull the slot out from under the window without the removal path.
    app.test_registry_mut()
        .tracks
        .iter_mut()
        .find(|t| t.id == TRACK)
        .unwrap()
        .plugins
        .retain(|p| p.instance_id != PLAIN);
    assert!(!window_drawn(&app));
    escape(&mut app);
    assert_eq!(
        app.test_plugin_window(),
        Some(PLAIN),
        "Esc was not taken by an invisible window"
    );
}

// ---------------------------------------------------------------------------
// The Arrange track menu
// ---------------------------------------------------------------------------

/// The track menu is drawn inside the base view, under the window's
/// layer; the window steps aside while it is up.
#[test]
fn the_window_is_not_drawn_over_the_track_menu() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Arrange)));
    plugin(&mut app, PluginMessage::OpenPluginWindow(PLAIN));
    assert!(window_drawn(&app));
    let _ = app.update(Message::Ui(UiMessage::OpenTrackMenu {
        id: TRACK,
        x: 200.0,
        y: 200.0,
    }));
    assert!(!window_drawn(&app), "hidden while the track menu is up");
    assert_eq!(app.test_plugin_window(), Some(PLAIN), "but still open");
    let _ = app.update(Message::Ui(UiMessage::CloseTrackMenu));
    assert!(window_drawn(&app), "back once the menu closes");
}

// ---------------------------------------------------------------------------
// Drag and position
// ---------------------------------------------------------------------------

fn drag(app: &mut Resonance, step: PluginWindowDrag) {
    plugin(app, PluginMessage::PluginWindowDrag(step));
}

/// Losing window focus mid-drag ends the drag: the release then goes to
/// another window and would never reach this one.
#[test]
fn unfocusing_the_app_window_ends_a_drag() {
    let mut app = app();
    plugin(&mut app, PluginMessage::OpenPluginWindow(PLAIN));
    drag(&mut app, PluginWindowDrag::Begin);
    assert!(app.test_plugin_window_state().unwrap().drag.is_some());

    let unfocused = iced::Event::Window(iced::window::Event::Unfocused);
    let end = resonance_app::update::plugin_window::drag_end_event(&unfocused)
        .expect("Unfocused ends the drag");
    let _ = app.update(end);
    assert!(app.test_plugin_window_state().unwrap().drag.is_none());

    let moved = iced::Event::Window(iced::window::Event::Focused);
    assert!(resonance_app::update::plugin_window::drag_end_event(&moved).is_none());
}

/// The drag keeps a grab strip of the title bar inside the app window,
/// on every edge, at the window's reported size.
#[test]
fn a_drag_is_clamped_to_the_app_window() {
    use resonance_app::state::{PLUGIN_WINDOW_GRAB_HEIGHT, PLUGIN_WINDOW_GRAB_WIDTH};
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::WindowResized(Size::new(1600.0, 1000.0))));
    plugin(&mut app, PluginMessage::OpenPluginWindow(PLAIN));
    let start = app.test_plugin_window_state().unwrap().position;
    drag(&mut app, PluginWindowDrag::Begin);
    // The first move fixes the grab offset at the window's corner.
    drag(&mut app, PluginWindowDrag::Moved(start));
    drag(&mut app, PluginWindowDrag::Moved(Point::new(9_000.0, 9_000.0)));
    let at = app.test_plugin_window_state().unwrap().position;
    assert_eq!(
        at,
        Point::new(1600.0 - PLUGIN_WINDOW_GRAB_WIDTH, 1000.0 - PLUGIN_WINDOW_GRAB_HEIGHT)
    );
    drag(&mut app, PluginWindowDrag::Moved(Point::new(-500.0, -500.0)));
    assert_eq!(app.test_plugin_window_state().unwrap().position, Point::ORIGIN);
    drag(&mut app, PluginWindowDrag::End);

    // Shrinking the app window pulls an open window back inside it.
    drag(&mut app, PluginWindowDrag::Begin);
    drag(&mut app, PluginWindowDrag::Moved(Point::ORIGIN));
    drag(&mut app, PluginWindowDrag::Moved(Point::new(1400.0, 900.0)));
    drag(&mut app, PluginWindowDrag::End);
    let _ = app.update(Message::Ui(UiMessage::WindowResized(Size::new(1440.0, 900.0))));
    let at = app.test_plugin_window_state().unwrap().position;
    assert!(at.x <= 1440.0 - PLUGIN_WINDOW_GRAB_WIDTH, "{at:?}");
    assert!(at.y <= 900.0 - PLUGIN_WINDOW_GRAB_HEIGHT, "{at:?}");
}

/// A stored position that is off screen is not reused: the window opens
/// at the default spot instead.
#[test]
fn an_off_screen_position_resets_on_open() {
    let mut app = app();
    plugin(&mut app, PluginMessage::OpenPluginWindow(PLAIN));
    app.test_place_plugin_window(Point::new(5_000.0, 4_000.0));
    plugin(&mut app, PluginMessage::OpenGenericParams(GUI));
    assert_eq!(
        app.test_plugin_window_state().unwrap().position,
        PLUGIN_WINDOW_DEFAULT_POSITION
    );

    // An on-screen one is kept.
    let kept = Point::new(300.0, 200.0);
    app.test_place_plugin_window(kept);
    plugin(&mut app, PluginMessage::OpenPluginWindow(PLAIN));
    assert_eq!(app.test_plugin_window_state().unwrap().position, kept);
}
