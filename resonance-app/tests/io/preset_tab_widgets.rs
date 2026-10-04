//! The media browser's Presets tab and the preset browser overlay, driven
//! through the real widget tree (`iced_test::Simulator`) rather than their
//! messages (review round 2, B1/B2): a double-click on a row loads it, a
//! press-move-release onto a track header adds the plugin with the
//! preset, and a plain click arms nothing that a later click could fire.
//!
//! Every app has a private preset root and marks store.

use iced::{mouse, Event, Point, Size};
use iced_test::simulator::Simulator;
use resonance_app::message::{BrowserMessage, Message, PluginMessage, PresetUiMessage, UiMessage};
use resonance_app::state::{BrowserTab, PluginSlotState, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::types::{ChainOwner, AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_common::factory_presets::FactoryPresetEntry;

const TRACK: u64 = 5;
const INSTANCE: u64 = 960;
const PLUGIN_ID: &str = "com.resonance.test-eq";

fn clap_id(s: &str) -> u32 {
    resonance_plugin::stable_hash(s)
}

fn params() -> Vec<ParamInfo> {
    ["gain", "freq"]
        .into_iter()
        .map(|id| ParamInfo {
            id: clap_id(id),
            name: id.to_owned(),
            min_value: 0.0,
            max_value: 20_000.0,
            default_value: 1.0,
            current_value: 1.0,
            ..Default::default()
        })
        .collect()
}

fn scanned() -> ScannedPlugin {
    let factory = |id: &str, name: &str, gain: f64| FactoryPresetEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        json: format!(r#"{{"version":1,"params":{{"gain":{gain},"freq":440.0}}}}"#),
        meta: None,
    };
    ScannedPlugin {
        clap_file_path: "/nonexistent/test-eq.clap".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        name: "Test EQ".to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: false,
        factory_presets: vec![factory("warm", "Warm", 3.0), factory("bright", "Bright", 7.0)],
    }
}

/// Arrange view, media browser open on Presets, one audio track with the
/// EQ as the focused slot.
fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/preset-tab-widgets.rprj"));
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![scanned()],
    });
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE,
            "Test EQ".to_owned(),
            PLUGIN_ID.to_owned(),
            "/nonexistent/test-eq.clap".to_owned(),
            params(),
            false,
        ),
    );
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Arrange)));
    let _ = app.update(Message::Plugin(PluginMessage::OpenPluginWindow(INSTANCE)));
    // The plugin's generic window is what makes it the load target; park
    // it in the bottom-right corner, off the browser rows and the track
    // headers this file presses.
    app.test_place_plugin_window(Point::new(960.0, 640.0));
    let _ = app.update(Message::Browser(BrowserMessage::ToggleVisible));
    let _ = app.update(Message::Browser(BrowserMessage::SelectTab(BrowserTab::Presets)));
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
    Simulator::with_size(settings, Size::new(1440.0, 900.0), app.view())
}

/// The centre of the first visible text `label`.
fn centre_of(app: &Resonance, label: &str) -> Point {
    let mut ui = simulator(app);
    let found = ui
        .find(label)
        .unwrap_or_else(|e| panic!("{label:?} should be on screen: {e:?}"));
    iced_test::selector::Bounded::visible_bounds(&found)
        .unwrap_or_else(|| panic!("{label:?} should be visible"))
        .center()
}

/// The centre of the right-most visible text `label` (all matches walked).
fn rightmost(app: &Resonance, label: &str) -> Point {
    use iced_test::selector::Candidate;
    use std::sync::{Arc, Mutex};
    let found: Arc<Mutex<Vec<iced::Rectangle>>> = Arc::default();
    {
        let found = Arc::clone(&found);
        let label = label.to_string();
        let collect = move |candidate: Candidate<'_>| -> Option<()> {
            if let Candidate::Text {
                content,
                visible_bounds: Some(bounds),
                ..
            } = candidate
            {
                if content == label {
                    found.lock().unwrap().push(bounds);
                }
            }
            None
        };
        let _ = simulator(app).find(collect);
    }
    let all = found.lock().unwrap().clone();
    all.into_iter()
        .max_by(|a, b| a.center_x().total_cmp(&b.center_x()))
        .unwrap_or_else(|| panic!("{label:?} should be on screen"))
        .center()
}

/// Run `events` at `at` against the current view and apply every message
/// the widgets raised, in order.
fn at(app: &mut Resonance, point: Point, events: Vec<Event>) {
    let messages: Vec<Message> = {
        let mut ui = simulator(app);
        ui.point_at(point);
        let _ = ui.simulate(events);
        ui.into_messages().collect()
    };
    for m in messages {
        let _ = app.update(m);
    }
}

fn press() -> Event {
    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
}

fn release() -> Event {
    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
}

fn moved(p: Point) -> Event {
    Event::Mouse(mouse::Event::CursorMoved { position: p })
}

fn gain(app: &mut Resonance, instance: u64) -> Option<f64> {
    app.test_plugin_param(instance, clap_id("gain"))
}

fn plugins_on_track(app: &Resonance) -> usize {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .map_or(0, |t| t.plugins.len())
}

fn track_name(app: &Resonance) -> String {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .unwrap()
        .name
        .clone()
}

/// B1: a double-click on a Presets-tab row loads it onto the selected slot.
#[test]
fn a_double_click_on_a_preset_row_loads_it() {
    let mut app = app();
    let row = centre_of(&app, "Bright");
    let messages: Vec<Message> = {
        let mut ui = simulator(&app);
        ui.point_at(row);
        let _ = ui.simulate(iced_test::simulator::click());
        let _ = ui.simulate(iced_test::simulator::click());
        ui.into_messages().collect()
    };
    assert!(
        messages.iter().any(|m| matches!(
            m,
            Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::MediaLoad(_)))
        )),
        "the double-click must reach the row: {messages:?}"
    );
    for m in messages {
        let _ = app.update(m);
    }
    assert_eq!(gain(&mut app, INSTANCE), Some(7.0));
}

/// B1 in the overlay: a double-click keeps the row, a single click only
/// auditions it.
#[test]
fn a_double_click_in_the_overlay_keeps_the_preset() {
    let mut app = app();
    let _ = app.update(Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::OpenBrowser(
        INSTANCE,
    ))));
    // "Warm" is drawn twice: in the tab (under the backdrop, on the left)
    // and in the overlay's list (centred). Aim at the overlay's.
    let row = rightmost(&app, "Warm");
    assert!(row.x > 460.0, "the overlay's row: {row:?}");
    let messages: Vec<Message> = {
        let mut ui = simulator(&app);
        ui.point_at(row);
        let _ = ui.simulate(iced_test::simulator::click());
        let _ = ui.simulate(iced_test::simulator::click());
        ui.into_messages().collect()
    };
    assert!(
        messages.iter().any(|m| matches!(
            m,
            Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::BrowserAudition(_)))
        )),
        "the click reached the row, not the backdrop: {messages:?}"
    );
    assert!(messages.iter().any(|m| matches!(
        m,
        Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::CloseBrowser { keep: true }))
    )));
    for m in messages {
        let _ = app.update(m);
    }
    assert!(app.test_presets().host_browser.is_none(), "kept and closed");
    assert_eq!(gain(&mut app, INSTANCE), Some(3.0), "Warm is what was kept");
}

/// B2: press on a row, move to a track header, release: the plugin is
/// added there with the preset.
#[test]
fn a_real_drag_onto_a_track_header_adds_the_plugin_with_the_preset() {
    let mut app = app();
    let row = centre_of(&app, "Bright");
    let header = centre_of(&app, &track_name(&app));
    let next = app.test_next_plugin_id();

    at(&mut app, row, vec![press()]);
    assert!(app.test_presets().dragging.is_some(), "the press arms a drag");
    let mid = Point::new((row.x + header.x) / 2.0, (row.y + header.y) / 2.0);
    at(&mut app, row, vec![moved(row)]);
    at(&mut app, mid, vec![moved(mid)]);
    at(&mut app, header, vec![moved(header), release()]);
    assert!(app.test_presets().dragging.is_none());
    assert_eq!(plugins_on_track(&app), 2, "added to the track's chain");

    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Track(TRACK),
        instance_id: next,
        plugin_name: "Test EQ".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        clap_file_path: "/nonexistent/test-eq.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: Vec::new(),
    });
    assert_eq!(gain(&mut app, next), Some(7.0));
}

/// B2: a plain click on a row arms nothing that survives its own release,
/// so a later click on a track header adds nothing.
#[test]
fn a_click_then_a_click_on_a_header_adds_nothing() {
    let mut app = app();
    let row = centre_of(&app, "Bright");
    let header = centre_of(&app, &track_name(&app));
    at(&mut app, row, vec![press()]);
    at(&mut app, row, vec![release()]);
    assert!(app.test_presets().dragging.is_none(), "a click disarms on release");
    at(&mut app, header, vec![press(), release()]);
    assert_eq!(plugins_on_track(&app), 1);

    // A press released over a header without moving is still a click.
    at(&mut app, row, vec![press()]);
    at(&mut app, header, vec![release()]);
    assert_eq!(plugins_on_track(&app), 1);
}

/// An effect dragged onto an instrument track with no instrument yet is
/// refused (it would take the instrument slot and hide the picker).
#[test]
fn an_effect_is_not_dropped_into_an_empty_instrument_slot() {
    let mut app = app();
    app.test_add_track(6, TrackType::Instrument);
    let name = app
        .test_registry()
        .tracks
        .iter()
        .find(|t| t.id == 6)
        .unwrap()
        .name
        .clone();
    let row = centre_of(&app, "Bright");
    let header = centre_of(&app, &name);
    at(&mut app, row, vec![press()]);
    at(&mut app, row, vec![moved(row)]);
    at(&mut app, header, vec![moved(header), release()]);
    let count = app
        .test_registry()
        .tracks
        .iter()
        .find(|t| t.id == 6)
        .map_or(0, |t| t.plugins.len());
    assert_eq!(count, 0, "refused");
    assert!(app.test_presets().dragging.is_none());
}

/// An armed drag ends on Esc, on a press no row took (its release was
/// lost outside the window), and when the pointer leaves the window — so a
/// later click on a header never drops it.
#[test]
fn a_lost_drag_is_disarmed_and_never_drops_later() {
    use resonance_app::commands::{KeyChord, Mods, NamedKey};
    let mut app = app();
    let row = centre_of(&app, "Bright");
    let header = centre_of(&app, &track_name(&app));
    let arm = |app: &mut Resonance| {
        at(app, row, vec![press()]);
        at(app, row, vec![moved(row)]);
        at(app, header, vec![moved(header)]);
        assert!(app.test_presets().dragging.as_ref().is_some_and(|d| d.moved));
    };

    arm(&mut app);
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured: false,
    }));
    assert!(app.test_presets().dragging.is_none(), "Esc");

    arm(&mut app);
    let outside = Point::new(-20.0, -20.0);
    let messages: Vec<Message> = {
        let mut ui = simulator(&app);
        ui.point_at(header);
        let _ = ui.simulate(vec![moved(header)]);
        ui.point_at(outside);
        let _ = ui.simulate(vec![moved(outside), Event::Mouse(mouse::Event::CursorLeft)]);
        ui.into_messages().collect()
    };
    for m in messages {
        let _ = app.update(m);
    }
    assert!(app.test_presets().dragging.is_none(), "the pointer left the window");

    arm(&mut app);
    // The release happened outside; the next thing is a click on a header.
    // The press goes through the app's window-level listener first (it
    // sees presses a header's buttons capture), then the widgets.
    for event in [press(), Event::Window(iced::window::Event::Unfocused)] {
        let end = resonance_app::update::plugin_preset_ui::drag_end_event(&event);
        assert!(end.is_some(), "{event:?} ends a drag");
    }
    let _ = app.update(resonance_app::update::plugin_preset_ui::drag_end_event(&press()).unwrap());
    at(&mut app, header, vec![press(), release()]);
    assert!(app.test_presets().dragging.is_none());
    assert_eq!(plugins_on_track(&app), 1, "and nothing was dropped");
}
