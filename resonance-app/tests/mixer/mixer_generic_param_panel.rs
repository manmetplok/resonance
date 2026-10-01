//! The generic parameter window: "Open always opens a window"
//! (mixer-cleanup.md §4), retargeted from the former bottom panel's tests
//! (ba todo #1306, plugin-audit finding X4, doc #276 item 2.4).
//!
//! A slot's name button sends `OpenPluginWindow`. A plugin with its own
//! GUI opens that editor; one without opens the host-drawn generic
//! window. A GUI plugin's generic parameters stay reachable as the
//! fallback when its editor refuses to open (ba todo #1347) — the only
//! surface that shows a plugin's parameters as plain numbers, and the
//! only thing left when a floating editor fails.
//!
//! These drive the real widget tree through `iced_test`, not a helper —
//! the bug class is in which message a button carries, so a test that
//! doesn't press the button cannot see it.

use std::sync::{Arc, Mutex};

use iced::{Point, Rectangle, Size};
use iced_test::selector::Candidate;
use iced_test::simulator::Simulator;
use resonance_app::commands::{KeyChord, Mods, NamedKey};
use resonance_app::message::{Message, PluginMessage, PluginWindowDrag, UiMessage};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_audio::types::AudioEvent;
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioCommand, ParamInfo, TrackType};

const TRACK: u64 = 1;
const INSTANCE: u64 = 77;
/// Short enough to survive the strip's 14-character name truncation, so
/// the selector matches the label the user actually sees.
const PLUGIN: &str = "Resonance EQ";
const PARAM: &str = "Low Gain";

fn app_with_plugin(has_gui: bool) -> Resonance {
    let (app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    with_plugin(app, has_gui)
}

/// [`app_with_plugin`], keeping the engine's command queue to assert on.
fn app_with_plugin_capture(
    has_gui: bool,
) -> (Resonance, crossbeam_channel::Receiver<AudioCommand>) {
    let (app, _task, rx) = Resonance::new_for_test_with_capture();
    let app = with_plugin(app, has_gui);
    while rx.try_recv().is_ok() {}
    (app, rx)
}

fn with_plugin(mut app: Resonance, has_gui: bool) -> Resonance {
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
    // The same tall viewport the old bottom panel needed; the floating
    // window sits well inside it.
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

/// The slot's name button opens the plugin's window, for every plugin —
/// which window that is is the update's decision, not the strip's.
#[test]
fn clicking_a_slot_sends_open_plugin_window_gui_or_not() {
    for has_gui in [true, false] {
        let app = app_with_plugin(has_gui);
        assert!(
            matches!(
                click(&app, PLUGIN).as_slice(),
                [Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE))]
            ),
            "has_gui = {has_gui}: the slot must send OpenPluginWindow"
        );
    }
}

/// A plugin with its own GUI opens THAT: the existing editor path, and no
/// generic window on top of it.
#[test]
fn opening_a_gui_plugin_routes_to_its_editor() {
    let (mut app, rx) = app_with_plugin_capture(true);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));

    let sent: Vec<AudioCommand> = rx.try_iter().collect();
    assert!(
        sent.iter().any(|c| matches!(
            c,
            AudioCommand::OpenPluginEditor { instance_id } if *instance_id == INSTANCE
        )),
        "a GUI plugin's window is its own editor: {sent:?}"
    );
    assert_eq!(app.test_plugin_window(), None, "no generic window for it");
    let mut ui = simulator(&app);
    assert!(ui.find(PARAM).is_err(), "and no parameter list drawn");
}

/// A plugin without a GUI opens the host-drawn generic window, which lists
/// its parameters — and no editor command goes out.
#[test]
fn opening_a_non_gui_plugin_shows_the_generic_window() {
    let (mut app, rx) = app_with_plugin_capture(false);
    {
        let mut ui = simulator(&app);
        assert!(ui.find(PARAM).is_err(), "no window before Open");
    }

    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));

    assert_eq!(app.test_plugin_window(), Some(INSTANCE));
    let sent: Vec<AudioCommand> = rx.try_iter().collect();
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, AudioCommand::OpenPluginEditor { .. })),
        "a GUI-less plugin has no editor to open: {sent:?}"
    );
    let mut ui = simulator(&app);
    ui.find(PARAM)
        .expect("the generic window should list the plugin's parameters");
}

/// The title bar names the plugin's owner beside the plugin.
#[test]
fn the_window_title_names_the_owner() {
    let mut app = app_with_plugin(false);
    let owner = app
        .test_registry()
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .expect("track")
        .name
        .clone();
    let count = |app: &Resonance| {
        let sink: Arc<Mutex<usize>> = Arc::default();
        let s2 = Arc::clone(&sink);
        let owner = owner.clone();
        let _ = simulator(app).find(move |c: Candidate<'_>| -> Option<()> {
            if let Candidate::Text { content, .. } = c {
                if content == owner {
                    *s2.lock().unwrap() += 1;
                }
            }
            None
        });
        let n = *sink.lock().unwrap();
        n
    };
    let before = count(&app);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    assert_eq!(count(&app), before + 1, "the title bar adds the owner's name");
}

/// A refused editor falls back to the generic window, and that window
/// draws a GUI plugin's parameters too.
#[test]
fn a_refused_editor_falls_back_to_a_window_that_draws_its_parameters() {
    let mut app = app_with_plugin(true);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: false,
        failure: Some(resonance_audio::types::PluginEditorFailure::CreateFailed),
    });

    let mut ui = simulator(&app);
    ui.find(PARAM)
        .expect("the fallback window should list the plugin's parameters");
}

/// The floating editor is not lost in the fallback window: its title bar
/// offers it again.
#[test]
fn a_gui_plugin_still_has_a_route_to_its_floating_editor() {
    let mut app = app_with_plugin(true);
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: false,
        failure: Some(resonance_audio::types::PluginEditorFailure::CreateFailed),
    });

    assert!(
        matches!(
            click(&app, "Open Editor").as_slice(),
            [Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE))]
        ),
        "the window's editor button must open the floating window"
    );
}

/// A plugin with no editor must not be offered one — the button is the
/// only thing `has_gui` decides inside the window.
#[test]
fn a_non_gui_plugin_is_offered_no_editor_button() {
    let mut app = app_with_plugin(false);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));

    let mut ui = simulator(&app);
    assert!(
        ui.find("Open Editor").is_err(),
        "a plugin with no GUI has no editor window to open"
    );
}

// --- the strip's own editor toggle ------------------------------------
//
// The strip's dedicated editor glyph. The tests above press the generic
// WINDOW's "Open Editor" text button, which is a different widget in a
// different module — nothing there touches the sliders glyph on the strip.

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

/// The visible bounds of the text `label` in the open generic window's
/// title bar (the strips under it draw some of the same labels).
fn in_window(app: &Resonance, label: &'static str) -> Rectangle {
    let origin = app.test_plugin_window_state().expect("open").position;
    let sink: Arc<Mutex<Vec<Rectangle>>> = Arc::default();
    let s2 = Arc::clone(&sink);
    let _ = simulator(app).find(move |c: Candidate<'_>| -> Option<()> {
        if let Candidate::Text {
            content,
            visible_bounds: Some(b),
            ..
        } = c
        {
            if content == label {
                s2.lock().unwrap().push(b);
            }
        }
        None
    });
    let all = sink.lock().unwrap().clone();
    // The window layer is traversed after the base view, so its copy is
    // the last one in its title-bar band.
    all.into_iter()
        .filter(|b| b.x >= origin.x && b.y >= origin.y && b.y < origin.y + 40.0)
        .last()
        .unwrap_or_else(|| panic!("the window draws {label:?}"))
}

/// The window's × closes it.
#[test]
fn the_close_button_closes_the_window() {
    let mut app = app_with_plugin(false);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));

    // The strip has a × of its own (remove); the window's is the one
    // inside the window.
    let close = in_window(&app, "\u{00d7}");
    let mut ui = simulator(&app);
    ui.point_at(Point::new(
        close.x + close.width / 2.0,
        close.y + close.height / 2.0,
    ));
    let _ = ui.simulate(iced_test::simulator::click());
    let messages: Vec<Message> = ui.into_messages().collect();
    assert!(
        matches!(
            messages.as_slice(),
            [Message::Plugin(PluginMessage::ClosePluginWindow(INSTANCE))]
        ),
        "the title bar's × closes the window: {messages:?}"
    );
    for m in messages {
        app.test_dispatch(m);
    }
    assert_eq!(app.test_plugin_window(), None);
    let mut ui = simulator(&app);
    assert!(ui.find(PARAM).is_err(), "the window closed");
}

/// Esc closes the window: it is non-modal, but Esc is its close key.
#[test]
fn escape_closes_the_window() {
    let mut app = app_with_plugin(false);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    assert_eq!(app.test_plugin_window(), Some(INSTANCE));

    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured: false,
    }));
    assert_eq!(app.test_plugin_window(), None, "Esc closed it");
}

/// Pressing the title bar (not one of its buttons) starts a drag, and the
/// drag moves the window by the pointer's travel.
#[test]
fn dragging_the_title_bar_moves_the_window() {
    let mut app = app_with_plugin(false);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    let start = app.test_plugin_window_state().expect("open").position;
    let drag = |step| Message::Plugin(PluginMessage::PluginWindowDrag(step));

    // The strip draws the plugin's name too; the title bar's copy is the
    // one inside the window.
    let title_name = in_window(&app, PLUGIN);
    let mut ui = simulator(&app);
    ui.point_at(Point::new(title_name.x + 2.0, title_name.y + 2.0));
    let _ = ui.simulate(iced_test::simulator::click());
    let pressed: Vec<Message> = ui.into_messages().collect();
    assert!(
        pressed.iter().any(|m| matches!(
            m,
            Message::Plugin(PluginMessage::PluginWindowDrag(PluginWindowDrag::Begin))
        )),
        "pressing the title bar starts a drag: {pressed:?}"
    );

    app.test_dispatch(drag(PluginWindowDrag::Begin));
    app.test_dispatch(drag(PluginWindowDrag::Moved(Point::new(
        start.x + 10.0,
        start.y + 5.0,
    ))));
    app.test_dispatch(drag(PluginWindowDrag::Moved(Point::new(
        start.x + 110.0,
        start.y + 55.0,
    ))));
    app.test_dispatch(drag(PluginWindowDrag::End));

    let after = app.test_plugin_window_state().expect("still open");
    assert_eq!(after.position, Point::new(start.x + 100.0, start.y + 50.0));
    assert!(after.drag.is_none(), "the release ends the drag");

    // A move with no drag in progress does nothing.
    app.test_dispatch(drag(PluginWindowDrag::Moved(Point::new(5.0, 5.0))));
    assert_eq!(
        app.test_plugin_window_state().expect("open").position,
        after.position
    );
}

/// Opening another plugin's window replaces the open one, in place.
#[test]
fn opening_another_plugin_replaces_the_window_in_place() {
    let mut app = app_with_plugin(false);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE + 1,
            "Second".to_owned(),
            "com.example.second".to_owned(),
            "/plugins/second.clap".to_owned(),
            vec![],
            false,
        ),
    );
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    app.test_place_plugin_window(Point::new(300.0, 200.0));
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE + 1)));

    let w = app.test_plugin_window_state().expect("open");
    assert_eq!(w.instance_id, INSTANCE + 1);
    assert_eq!(w.position, Point::new(300.0, 200.0), "it keeps its place");
}

/// Removing the plugin closes its window.
#[test]
fn removing_the_plugin_closes_its_window() {
    let mut app = app_with_plugin(false);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    app.test_dispatch(Message::Plugin(PluginMessage::RemovePluginFromTrack(
        TRACK, INSTANCE,
    )));
    assert_eq!(app.test_plugin_window(), None);
    let mut ui = simulator(&app);
    assert!(ui.find(PARAM).is_err(), "nothing left drawn");
}

/// Loading a project closes the window: its plugin may not exist there.
#[test]
fn loading_a_project_closes_the_window() {
    let mut app = app_with_plugin(false);
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    let file = app.test_build_project_file();
    app.test_replay_loaded_project(file);
    assert_eq!(app.test_plugin_window(), None);
}

/// A missing plugin opens the generic window even though it declares a
/// GUI. The window says it is missing and points at the recovery, which
/// lives under the slot's row in the inspector CHAIN (mixer-cleanup.md
/// §3.2, Q16 — covered by `inspector_chain_row`).
#[test]
fn a_missing_plugin_opens_the_window_with_its_recovery() {
    let mut app = app_with_plugin(true);
    app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(INSTANCE),
        clap_plugin_id: "com.resonance.eq".to_owned(),
        clap_file_path: "/plugins/eq.clap".to_owned(),
        reason: "Failed to load plugin: no such file".to_owned(),
    });
    let _ = app.update(Message::Ui(UiMessage::DismissMissingPlugins));
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    assert_eq!(
        app.test_plugin_window(),
        Some(INSTANCE),
        "a missing plugin has no editor; it gets the generic window"
    );
    let mut ui = simulator(&app);
    ui.find(
        format!(
            "\u{26a0} {PLUGIN} is missing \u{2014} see the inspector to replace or remove it"
        )
        .as_str(),
    )
    .expect("the window points at the inspector's recovery");
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
fn a_refused_open_leaves_the_slot_closed_and_offers_the_generic_window() {
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
        app.test_plugin_window(),
        Some(INSTANCE),
        "a refused editor must fall back to the generic window — \
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

/// A successful open does NOT open the generic window.
///
/// The fallback is for failures only; hijacking the panel on every
/// successful open would fight the user's own selection.
#[test]
fn a_successful_open_leaves_the_generic_window_alone() {
    let mut app = app_with_plugin(true);
    let before = app.test_plugin_window();

    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE)));
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: INSTANCE,
        open: true,
        failure: None,
    });

    assert_eq!(
        app.test_plugin_window(),
        before,
        "a working editor must not also open the generic window"
    );
}

// ---------------------------------------------------------------------------
// Golden
// ---------------------------------------------------------------------------

/// The generic window open over the mixer: title bar (plugin, owner, ×),
/// preset bar, and a parameter list, floating above the strips — which
/// keep the full height the bottom panel used to take.
#[test]
fn generic_window_over_the_mixer_golden() {
    let mut app = app_with_plugin(false);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE + 1,
            "Tape Echo".to_owned(),
            "com.example.tape-echo".to_owned(),
            "/plugins/tape-echo.clap".to_owned(),
            [
                ("Time", 0.0, 2000.0, 375.0, "375 ms"),
                ("Feedback", 0.0, 1.0, 0.45, "45 %"),
                ("Mix", 0.0, 1.0, 0.3, "30 %"),
                ("Tone", -1.0, 1.0, 0.2, ""),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (name, min, max, value, text))| ParamInfo {
                id: i as u32 + 1,
                name: name.to_owned(),
                min_value: min,
                max_value: max,
                default_value: min,
                current_value: value,
                text: text.to_owned(),
                ..Default::default()
            })
            .collect(),
            false,
        ),
    );
    app.test_dispatch(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE + 1)));
    let mut sim = simulator(&app);
    let snap = sim
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    crate::common::assert_golden(&snap, "tests/snapshots/plugin_generic_window.png");
}
