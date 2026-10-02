//! The generic parameter window: "Open always opens a window"
//! (mixer-cleanup.md §4), retargeted from the former bottom panel's tests
//! (ba todo #1306, plugin-audit finding X4, doc #276 item 2.4).
//!
//! A strip's slot line focuses the slot on a click (`FocusSlot`) and
//! sends `OpenPluginWindow` on a double-click (mixer-cleanup.md §2.1).
//! A plugin with its own
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
/// Short enough to survive the slot line's name truncation, so
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
/// shares the name's vertical band and sits inside the strip's width.
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

/// Press the slot line's name `clicks` times in a row and return the
/// messages the view raised. Two presses inside the double-click window
/// are a double-click.
fn press_slot_line(app: &Resonance, clicks: usize) -> Vec<Message> {
    let (_, bounds) = slot_row_controls(app)
        .into_iter()
        .find(|(content, _)| content == PLUGIN)
        .expect("the strip draws the slot line");
    let mut ui = simulator(app);
    ui.point_at(Point::new(
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    ));
    for _ in 0..clicks {
        let _ = ui.simulate(iced_test::simulator::click());
    }
    ui.into_messages().collect()
}

// ---------------------------------------------------------------------------

/// A click on a slot line focuses the slot (and selects its owner) for
/// the inspector — it opens nothing.
#[test]
fn clicking_a_slot_line_focuses_the_slot() {
    for has_gui in [true, false] {
        let app = app_with_plugin(has_gui);
        assert!(
            matches!(
                press_slot_line(&app, 1).as_slice(),
                [Message::Plugin(PluginMessage::FocusSlot(INSTANCE))]
            ),
            "has_gui = {has_gui}: a click must send FocusSlot only"
        );
    }
}

/// A double-click on the slot line opens the plugin's window, for every
/// plugin — which window that is is the update's decision, not the
/// strip's.
#[test]
fn double_clicking_a_slot_line_sends_open_plugin_window_gui_or_not() {
    for has_gui in [true, false] {
        let app = app_with_plugin(has_gui);
        let messages = press_slot_line(&app, 2);
        assert!(
            messages.iter().any(|m| matches!(
                m,
                Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE))
            )),
            "has_gui = {has_gui}: the double-click must send OpenPluginWindow: {messages:?}"
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
    // Opening the window selects its track; select it up front so the
    // inspector header's copy of the name is in both counts.
    app.test_dispatch(Message::Ui(resonance_app::message::UiMessage::SelectTrack(Some(TRACK))));
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

// --- the slot line carries no controls ---------------------------------
//
// The strip used to draw an editor toggle (the sliders glyph), ▲▼, ⏻ and
// × beside every slot's name. They moved to the inspector's CHAIN row
// (mixer-cleanup.md §2.2); the strip's route to a GUI plugin's floating
// editor is now the slot line's double-click.

/// Nothing but the plugin's name shares the slot line's band — for a GUI
/// plugin as for one without.
#[test]
fn the_slot_line_draws_only_the_plugin_name() {
    for has_gui in [true, false] {
        let glyphs = slot_row_glyphs(&app_with_plugin(has_gui));
        assert_eq!(
            glyphs,
            vec![PLUGIN.to_string()],
            "has_gui = {has_gui}: the slot line shows state, not controls"
        );
    }
}

/// The strip's route to a GUI plugin's own window: a double-click on its
/// slot line, dispatched as the view raised it, opens the floating editor
/// on the engine.
#[test]
fn double_clicking_a_gui_slot_line_opens_the_floating_editor() {
    let (mut app, rx) = app_with_plugin_capture(true);
    for m in press_slot_line(&app, 2) {
        app.test_dispatch(m);
    }
    let sent: Vec<AudioCommand> = rx.try_iter().collect();
    assert!(
        sent.iter().any(|c| matches!(
            c,
            AudioCommand::OpenPluginEditor { instance_id } if *instance_id == INSTANCE
        )),
        "the double-click must open the plugin's own editor: {sent:?}"
    );
    assert_eq!(app.test_focused_slot(), Some(INSTANCE));
}

/// Font Awesome `up-right-from-square`: a CHAIN row's `↗`.
const GLYPH_OPEN: &str = "\u{f35d}";

/// Press the `↗` of the first CHAIN row in the inspector (whatever owner
/// is selected) and return the messages the view raised. The transport
/// bar draws icons in the same column, so only glyphs below the
/// inspector's caption count.
fn press_chain_open(app: &Resonance) -> Vec<Message> {
    use iced_test::selector::Target;
    let inspector_left = 1440.0 - theme::INSPECTOR_WIDTH;
    let top = simulator(app)
        .find("INSPECTOR")
        .expect("the inspector renders")
        .bounds()
        .y;
    let mut ui = simulator(app);
    ui.click(move |c: Candidate<'_>| {
        let hit = matches!(
            &c,
            Candidate::Text { content, bounds, .. }
                if *content == GLYPH_OPEN && bounds.x >= inspector_left && bounds.y >= top
        );
        hit.then(|| Target::from(c))
    })
    .expect("the CHAIN row draws its ↗");
    ui.into_messages().collect()
}

fn gui_slot(instance: u64, name: &str) -> PluginSlotState {
    PluginSlotState::new(
        instance,
        name.to_owned(),
        format!("com.example.{instance}"),
        format!("/plugins/{instance}.clap"),
        vec![],
        true,
    )
}

/// The CHAIN row's `↗` on a GUI plugin toggles its floating editor, on a
/// track, a bus and the master alike: pressed closed it opens the
/// editor; once the ENGINE reports the editor open the glyph is accented
/// and the same press closes it (ba todo #1347 — the tint follows the
/// engine's report, never the press, so a window that refused to open
/// leaves the glyph down instead of lit over nothing).
///
/// Every press goes through the rendered inspector; the tint is read
/// through the view's own decision (`test_chain_open_toggle`), since
/// `iced_test` cannot read a colour.
#[test]
fn the_chain_open_toggle_follows_the_editor_on_every_chain() {
    const BUS: u64 = 5;
    const BUS_FX: u64 = INSTANCE + 10;
    const MASTER_FX: u64 = INSTANCE + 20;
    let mut app = app_with_plugin(true);
    app.test_add_bus(BUS, "Gtr Bus");
    app.test_push_bus_plugin(BUS, gui_slot(BUS_FX, "Bus Glue"));
    app.test_push_master_plugin(gui_slot(MASTER_FX, "Limiter"));

    let owners = [
        (UiMessage::SelectTrack(Some(TRACK)), INSTANCE),
        (UiMessage::SelectBus(Some(BUS)), BUS_FX),
        (UiMessage::SelectMaster, MASTER_FX),
    ];
    for (select, id) in owners {
        app.test_dispatch(Message::Ui(select));
        let tint = |app: &Resonance| app.test_chain_open_toggle(id).map(|(_, t)| t);
        assert_eq!(tint(&app), Some(theme::TEXT_DIM), "{id}: closed sits back");

        let pressed = press_chain_open(&app);
        assert!(
            matches!(pressed.as_slice(), [Message::Plugin(PluginMessage::OpenPluginEditor(i))] if *i == id),
            "{id}: ↗ opens the editor: {pressed:?}"
        );
        for m in pressed {
            app.test_dispatch(m);
        }
        assert_eq!(
            tint(&app),
            Some(theme::TEXT_DIM),
            "{id}: the press alone tints nothing until the engine confirms"
        );

        app.test_apply_engine_event(AudioEvent::PluginEditorState {
            instance_id: id,
            open: true,
            failure: None,
        });
        assert_eq!(tint(&app), Some(theme::ACCENT), "{id}: open is accented");
        let pressed = press_chain_open(&app);
        assert!(
            matches!(pressed.as_slice(), [Message::Plugin(PluginMessage::ClosePluginEditor(i))] if *i == id),
            "{id}: the same ↗ closes the open editor: {pressed:?}"
        );
        app.test_apply_engine_event(AudioEvent::PluginEditorState {
            instance_id: id,
            open: false,
            failure: None,
        });
        assert_eq!(tint(&app), Some(theme::TEXT_DIM), "{id}: and back down");
    }
}

/// A plugin with no GUI: `↗` opens the generic window, reads accented
/// while that window shows this plugin, and closes it.
#[test]
fn the_chain_open_toggle_follows_the_generic_window() {
    let mut app = app_with_plugin(false);
    app.test_dispatch(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    let tint = |app: &Resonance| app.test_chain_open_toggle(INSTANCE).map(|(_, t)| t);
    assert_eq!(tint(&app), Some(theme::TEXT_DIM));

    let pressed = press_chain_open(&app);
    assert!(
        matches!(
            pressed.as_slice(),
            [Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE))]
        ),
        "{pressed:?}"
    );
    for m in pressed {
        app.test_dispatch(m);
    }
    assert_eq!(app.test_plugin_window(), Some(INSTANCE));
    assert_eq!(tint(&app), Some(theme::ACCENT), "lit while its window is up");

    // Off the inspector, so the press below reaches the row.
    app.test_place_plugin_window(Point::new(20.0, 20.0));
    let pressed = press_chain_open(&app);
    assert!(
        matches!(
            pressed.as_slice(),
            [Message::Plugin(PluginMessage::ClosePluginWindow(INSTANCE))]
        ),
        "the lit ↗ closes the window: {pressed:?}"
    );
    for m in pressed {
        app.test_dispatch(m);
    }
    assert_eq!(app.test_plugin_window(), None);
    assert_eq!(tint(&app), Some(theme::TEXT_DIM));
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

    // The CHAIN row's ↗, pressed for real (the fallback window moved
    // off the inspector first).
    app.test_dispatch(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    app.test_place_plugin_window(Point::new(20.0, 20.0));
    assert!(
        matches!(
            press_chain_open(&app).as_slice(),
            [Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE))]
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

    app.test_dispatch(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    assert!(
        matches!(
            press_chain_open(&app).as_slice(),
            [Message::Plugin(PluginMessage::OpenPluginEditor(INSTANCE))]
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
