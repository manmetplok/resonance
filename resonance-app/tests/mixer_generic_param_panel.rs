//! The generic parameter panel is reachable for every plugin, GUI or
//! not (ba todo #1306, plugin-audit finding X4, doc #276 item 2.4).
//!
//! `view_plugin_slot_row` used to route a slot click to
//! `TogglePluginPanel` only when `has_gui == false`. All eleven bundled
//! plugins declare a GUI, so that one condition made the generic panel
//! unreachable for the entire fleet — the only surface that shows a
//! plugin's parameters as plain numbers, and the only thing left when a
//! floating editor fails to open. The control API never had the
//! restriction (`track.plugin_params` / `set_plugin_param` work on any
//! plugin), so it was another GUI/MCP asymmetry.
//!
//! The fix is not to take the editor away: the name button now always
//! opens the parameter panel and the floating editor moves to a control
//! of its own, so which surface a click reaches no longer depends on a
//! property of the plugin the user cannot see.
//!
//! These drive the real widget tree through `iced_test`, not a helper —
//! the bug was in which message the button carried, so a test that
//! doesn't press the button cannot see it.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, PluginMessage, UiMessage};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::{theme, Resonance, STARTUP_TAB};
use resonance_audio::types::{ParamInfo, TrackType};

const TRACK: u64 = 1;
const INSTANCE: u64 = 77;
/// Short enough to survive the strip's 14-character name truncation, so
/// the selector matches the label the user actually sees.
const PLUGIN: &str = "Resonance EQ";
const PARAM: &str = "Low Gain";

fn app_with_plugin(has_gui: bool) -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE,
            PLUGIN.to_owned(),
            "com.resonance.eq".to_owned(),
            "/plugins/eq.clap".to_owned(),
            vec![ParamInfo {
                id: 1,
                name: PARAM.to_owned(),
                min_value: -24.0,
                max_value: 24.0,
                default_value: 0.0,
                current_value: 3.0,
            }],
            has_gui,
        ),
    );
    // The inspector renders the selected track's CHAIN group, which
    // would put a second copy of the plugin name on screen; leave the
    // selection clear so the strip's slot is the only match.
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(None)));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
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
    // Tall enough that the bottom parameter panel is inside the
    // viewport — `click` refuses a target that is not visible, and at
    // 900 px the panel header sits just past the bottom edge.
    Simulator::with_size(settings, Size::new(1440.0, 1200.0), app.view())
}

/// Click `label` in the mixer and return the messages the view raised.
fn click(app: &Resonance, label: &str) -> Vec<Message> {
    let mut ui = simulator(app);
    ui.click(label)
        .unwrap_or_else(|e| panic!("{label} should be clickable: {e:?}"));
    ui.into_messages().collect()
}

// ---------------------------------------------------------------------------

/// The finding itself: a plugin that declares a GUI must still be able
/// to show its parameters.
#[test]
fn clicking_a_gui_plugins_slot_opens_the_generic_param_panel() {
    let app = app_with_plugin(true);
    assert!(
        matches!(
            click(&app, PLUGIN).as_slice(),
            [Message::Plugin(PluginMessage::TogglePluginPanel(INSTANCE))]
        ),
        "the slot must reach the parameter panel even though has_gui is true"
    );
}

/// And the same click does the same thing when there is no editor to
/// compete with — the two kinds of plugin behave alike now.
#[test]
fn clicking_a_non_gui_plugins_slot_opens_the_same_panel() {
    let app = app_with_plugin(false);
    assert!(matches!(
        click(&app, PLUGIN).as_slice(),
        [Message::Plugin(PluginMessage::TogglePluginPanel(INSTANCE))]
    ));
}

/// Reaching the panel is only half of it — the panel has to actually
/// draw the parameters for a plugin that declares a GUI.
#[test]
fn the_panel_renders_a_gui_plugins_parameters() {
    let mut app = app_with_plugin(true);
    {
        let mut ui = simulator(&app);
        assert!(
            ui.find(PARAM).is_err(),
            "no parameter panel before the slot is clicked"
        );
    }

    app.test_dispatch(Message::Plugin(PluginMessage::TogglePluginPanel(INSTANCE)));

    let mut ui = simulator(&app);
    ui.find(PARAM)
        .expect("the generic panel should list the plugin's parameters");
}

/// The floating editor is not lost to the change: the panel header keeps
/// offering it, so a GUI plugin has both surfaces rather than one.
#[test]
fn a_gui_plugin_still_has_a_route_to_its_floating_editor() {
    let mut app = app_with_plugin(true);
    app.test_dispatch(Message::Plugin(PluginMessage::TogglePluginPanel(INSTANCE)));

    assert!(
        matches!(
            click(&app, "Open Editor").as_slice(),
            [Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE))]
        ),
        "the panel header's editor button must open the floating window"
    );
}

/// A plugin with no editor must not be offered one — the button is the
/// only thing `has_gui` still decides.
#[test]
fn a_non_gui_plugin_is_offered_no_editor_button() {
    let mut app = app_with_plugin(false);
    app.test_dispatch(Message::Plugin(PluginMessage::TogglePluginPanel(INSTANCE)));

    let mut ui = simulator(&app);
    assert!(
        ui.find("Open Editor").is_err(),
        "a plugin with no GUI has no editor window to open"
    );
}

/// Toggling closes it again, so the panel is not a one-way door.
#[test]
fn clicking_the_slot_again_closes_the_panel() {
    let mut app = app_with_plugin(true);
    app.test_dispatch(Message::Plugin(PluginMessage::TogglePluginPanel(INSTANCE)));
    app.test_dispatch(Message::Plugin(PluginMessage::TogglePluginPanel(INSTANCE)));

    let mut ui = simulator(&app);
    assert!(ui.find(PARAM).is_err(), "the panel closed again");
}
