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

use std::sync::{Arc, Mutex};

use iced::{Point, Rectangle, Size};
use iced_test::selector::Candidate;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, PluginMessage, UiMessage};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_audio::types::AudioEvent;
use resonance_app::{theme, Resonance};
use resonance_audio::types::{ParamInfo, TrackType};

const TRACK: u64 = 1;
const INSTANCE: u64 = 77;
/// Short enough to survive the strip's 14-character name truncation, so
/// the selector matches the label the user actually sees.
const PLUGIN: &str = "Resonance EQ";
const PARAM: &str = "Low Gain";

fn app_with_plugin(has_gui: bool) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
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
                // ba todo #1290 added text/unit/stepped/choices/module/hidden.
                // Defaulting them keeps this fixture's subject the panel's
                // routing, not its formatting: empty `text` is exactly the
                // case that falls back to the old `{:.2}` rendering.
                ..Default::default()
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

/// Every text-bearing widget of the plugin slot row, left to right.
///
/// Everything on the row after the plugin's name is a bare icon glyph
/// with no label, so the only honest way to address one is the way the
/// row lays it out — by position. A widget belongs to the slot when it
/// shares the name's vertical band and sits inside the strip's 140 px.
///
/// (`Simulator` exposes `find`, which stops at the first match, but no
/// `find_all`; a selector closure that records every candidate and
/// never matches walks the whole tree instead.)
fn slot_row_controls(app: &Resonance) -> Vec<(String, Rectangle)> {
    let sink: Arc<Mutex<Vec<(String, Rectangle)>>> = Arc::default();
    {
        let sink = Arc::clone(&sink);
        let collect = move |candidate: Candidate<'_>| -> Option<()> {
            if let Candidate::Text {
                content,
                visible_bounds: Some(bounds),
                ..
            } = candidate
            {
                sink.lock().unwrap().push((content.to_owned(), bounds));
            }
            None
        };
        // Always `Err(SelectorNotFound)` — the closure matches nothing
        // on purpose, so the traversal visits every widget.
        let _ = simulator(app).find(collect);
    }

    let all = Arc::try_unwrap(sink).unwrap().into_inner().unwrap();
    let (_, name) = all
        .iter()
        .find(|(content, _)| content == PLUGIN)
        .expect("the strip should draw the plugin's name")
        .clone();
    let band = name.y + name.height / 2.0;
    let mut row: Vec<_> = all
        .into_iter()
        .filter(|(_, b)| {
            (b.y + b.height / 2.0 - band).abs() < name.height
                && b.x > name.x - 20.0
                && b.x < name.x + theme::MIXER_STRIP_WIDTH
        })
        .collect();
    row.sort_by(|(_, a), (_, b)| a.x.total_cmp(&b.x));
    row
}

/// The glyphs the slot row draws after the plugin's name, left to right.
fn slot_row_glyphs(app: &Resonance) -> Vec<String> {
    slot_row_controls(app)
        .into_iter()
        .map(|(content, _)| content)
        .collect()
}

/// Press the slot-row control at `index` (see [`slot_row_controls`]) and
/// return the messages the view raised. Positional rather than by label
/// because these controls have no labels.
fn press_slot_control(app: &Resonance, index: usize) -> Vec<Message> {
    let controls = slot_row_controls(app);
    let (_, bounds) = controls
        .get(index)
        .unwrap_or_else(|| panic!("slot row has no control #{index}: {controls:?}"))
        .clone();
    let mut ui = simulator(app);
    ui.point_at(Point::new(
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    ));
    let _ = ui.simulate(iced_test::simulator::click());
    ui.into_messages().collect()
}

/// The strip slot's editor toggle: index 1, straight after the name.
const EDITOR_TOGGLE: usize = 1;

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

// --- the strip's own editor toggle ------------------------------------
//
// The control the name button handed the editor over to. The two tests
// above press the parameter PANEL header's "Open Editor" text button,
// which is a different widget in a different module — nothing there
// touches the sliders glyph on the strip.

/// The glyph appears exactly where a plugin has a window to open.
#[test]
fn the_strip_offers_an_editor_toggle_only_for_a_gui_plugin() {
    let sliders = theme::fa::SLIDERS.to_string();

    let with_gui = slot_row_glyphs(&app_with_plugin(true));
    assert_eq!(
        with_gui.get(EDITOR_TOGGLE),
        Some(&sliders),
        "a GUI plugin's slot draws the editor toggle right after the \
         name, before the reorder carets: {with_gui:?}"
    );

    let without = slot_row_glyphs(&app_with_plugin(false));
    assert!(
        !without.contains(&sliders),
        "a plugin with no GUI has no window to open, so the slot must \
         not draw the toggle at all: {without:?}"
    );
    assert_eq!(
        without.len() + 1,
        with_gui.len(),
        "and the toggle is the ONLY difference — the name, the two \
         reorder carets and the delete × are drawn either way: \
         {without:?} vs {with_gui:?}"
    );
}

/// The mapping the control exists for: pressing it opens the floating
/// editor. Pressed by position — it carries no label to select by.
#[test]
fn pressing_the_strip_editor_toggle_opens_the_floating_editor() {
    let app = app_with_plugin(true);
    assert!(
        matches!(
            press_slot_control(&app, EDITOR_TOGGLE).as_slice(),
            [Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE))]
        ),
        "the sliders glyph must open the plugin's own window — the name \
         button next to it opens the parameter panel instead"
    );
}

/// And it is a toggle, not a one-way open: with the editor already up,
/// the same control closes it.
#[test]
fn pressing_the_strip_editor_toggle_again_closes_the_floating_editor() {
    let mut app = app_with_plugin(true);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE)));
    // The engine's confirmation is what makes the editor "open" now
    // (ba todo #1347); the press on its own no longer moves the flag.
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: true,
        failure: None,
    });

    assert!(
        matches!(
            press_slot_control(&app, EDITOR_TOGGLE).as_slice(),
            [Message::Plugin(PluginMessage::ClosePluginEditor(INSTANCE))]
        ),
        "with the editor open the toggle must close it, not open a second"
    );
}

/// The glyph lights up while the editor is open. That tint is the only
/// feedback a press gives, so it has to track the state — and since ba
/// todo #1347 it tracks the ENGINE's report rather than the press, so a
/// window that refused to open leaves the glyph down instead of lit over
/// nothing.
///
/// Asserted through the view's own decision rather than the widget
/// tree: `iced_test` can read a text candidate's content but never its
/// colour.
#[test]
fn the_strip_editor_toggle_is_tinted_while_the_editor_is_open() {
    let mut app = app_with_plugin(true);
    assert_eq!(
        app.test_strip_editor_toggle(INSTANCE).map(|(_, tint)| tint),
        Some(theme::TEXT_DIM),
        "closed: the glyph sits back in the strip's dim icon ramp"
    );

    // The press alone no longer tints it: since ba todo #1347 the flag
    // moves only on the engine's report, so a window that failed to open
    // cannot leave the glyph lit over nothing.
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE)));
    assert_eq!(
        app.test_strip_editor_toggle(INSTANCE).map(|(_, tint)| tint),
        Some(theme::TEXT_DIM),
        "pressing open must not tint anything until the engine confirms"
    );

    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: true,
        failure: None,
    });
    assert_eq!(
        app.test_strip_editor_toggle(INSTANCE).map(|(_, tint)| tint),
        Some(theme::ACCENT),
        "open: the glyph is accented, which is the only sign the window \
         is up when it is behind the main window"
    );

    app.test_dispatch(Message::Plugin(PluginMessage::ClosePluginEditor(INSTANCE)));
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: false,
        failure: None,
    });
    assert_eq!(
        app.test_strip_editor_toggle(INSTANCE).map(|(_, tint)| tint),
        Some(theme::TEXT_DIM),
        "and it goes back down again"
    );

    assert!(
        app_with_plugin(false)
            .test_strip_editor_toggle(INSTANCE)
            .is_none(),
        "no GUI, no control to tint"
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

// ---------------------------------------------------------------------------
// editor_open reflects only what the engine reported (ba todo #1347)
// ---------------------------------------------------------------------------

/// A failed open must not leave the slot claiming the editor is up.
///
/// This is the symptom the todo was filed for: `update/plugin.rs` set
/// `editor_open = true` the moment the command went out, and the engine
/// answered a failure with `Error("Failed to open plugin editor")` — no
/// instance id — so nothing could ever correct it. The slot read "Close
/// Editor" over a window that was never there, and pressing it sent a
/// close for an editor that did not exist.
#[test]
fn a_refused_open_leaves_the_slot_closed_and_offers_the_generic_panel() {
    let mut app = app_with_plugin(true);

    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE)));
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: false,
        failure: Some(resonance_audio::types::PluginEditorFailure::CreateFailed),
    });

    assert!(
        matches!(
            app.test_strip_editor_toggle(INSTANCE).map(|(msg, _)| msg),
            Some(Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE)))
        ),
        "after a refused open the toggle must still OFFER to open, not \
         offer to close a window that is not there"
    );
    assert_eq!(
        app.test_selected_plugin(),
        Some(INSTANCE),
        "a refused editor must fall back to the generic parameter panel — \
         it is the only way left to see this plugin's parameters, and \
         without it the press appears to do nothing at all"
    );
}

/// A window the user closes from its own titlebar clears the flag.
///
/// Nothing asked the app to close it, so before #1347 there was no
/// message and no event — the slot stayed "open" for the rest of the
/// session and the toggle offered to close an already-closed window.
#[test]
fn a_titlebar_close_clears_the_flag_without_the_app_asking() {
    let mut app = app_with_plugin(true);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE)));
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: true,
        failure: None,
    });

    // No app-side message: this arrives entirely on the engine's word.
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: false,
        failure: None,
    });

    assert!(
        matches!(
            app.test_strip_editor_toggle(INSTANCE).map(|(msg, _)| msg),
            Some(Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE)))
        ),
        "the slot must go back to offering an open once the window is gone"
    );
}

/// A successful open does NOT select the generic panel.
///
/// The fallback is for failures only; hijacking the panel on every
/// successful open would fight the user's own selection.
#[test]
fn a_successful_open_leaves_the_panel_selection_alone() {
    let mut app = app_with_plugin(true);
    let before = app.test_selected_plugin();

    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE)));
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: true,
        failure: None,
    });

    assert_eq!(
        app.test_selected_plugin(),
        before,
        "a working editor must not steal the parameter panel"
    );
}
