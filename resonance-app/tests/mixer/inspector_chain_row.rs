//! The inspector CHAIN row (mixer-cleanup.md §3.2, slice S2) and the
//! header colour swatch (§3.1, §6).
//!
//! `⠿ ● Name   ↗ ☰ ×` on track, bus and master chains alike: every press
//! goes through the rendered view (`Simulator::click`) and asserts on the
//! message the view raised, then feeds it back through `update` where the
//! outcome matters — so these pin what the GUI sends, not a copy of the
//! wiring.

use iced::Size;
use iced_test::selector::{Candidate, Target};
use iced_test::simulator::Simulator;
use resonance_app::message::{
    BusMessage, ChainUiMessage, MasterMessage, Message, PluginMessage, TrackMessage, UiMessage,
};
use resonance_app::state::{PluginSlotState, SubTrackLink, TrackState, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioEvent, ParamInfo, ScannedPlugin, TrackType};

const TRACK: u64 = 1;
const INST: u64 = 2;
const BUS: u64 = 1;
const TRACK_FX: u64 = 71;
const TRACK_FX2: u64 = 72;
const BUS_FX: u64 = 73;
const MASTER_FX: u64 = 74;

const GLYPH_GRIP: &str = "\u{f58e}";
const GLYPH_OPEN: &str = "\u{f35d}";
const GLYPH_MENU: &str = "\u{f0c9}";

fn slot(instance: u64, name: &str) -> PluginSlotState {
    PluginSlotState::new(
        instance,
        name.to_owned(),
        format!("com.resonance.p{instance}"),
        format!("/plugins/p{instance}.clap"),
        vec![ParamInfo {
            id: 1,
            name: "Mix".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            ..Default::default()
        }],
        false,
    )
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    // Undo records only for a project with a home.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/inspector-chain-row.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_add_track(INST, TrackType::Instrument);
    app.test_add_bus(BUS, "Drum Bus");
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/eq.clap".to_owned(),
            clap_plugin_id: "com.resonance.eq".to_owned(),
            name: "Resonance EQ".to_owned(),
            vendor: "Resonance".to_owned(),
            ..Default::default()
        }],
    });
    app.test_push_track_plugin(TRACK, slot(TRACK_FX, "Tape Comp"));
    app.test_push_track_plugin(TRACK, slot(TRACK_FX2, "Plate Verb"));
    app.test_push_bus_plugin(BUS, slot(BUS_FX, "Bus Glue"));
    app.test_push_master_plugin(slot(MASTER_FX, "Limiter"));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    // The CHAIN rows are the inspector's: their transient state lives on
    // the channel it describes (`chain_ui::settle`).
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    app
}

fn ui(app: &mut Resonance, m: UiMessage) {
    let _ = app.update(Message::Ui(m));
}

fn chain_ui(app: &mut Resonance, m: ChainUiMessage) {
    let _ = app.update(Message::Plugin(PluginMessage::ChainUi(m)));
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
    Simulator::with_size(settings, Size::new(1440.0, 2000.0), app.view())
}

/// Left edge of the inspector pane (the right-most column).
const INSPECTOR_LEFT: f32 = 1440.0 - theme::INSPECTOR_WIDTH;

/// Top of the inspector pane: the "INSPECTOR" caption. The transport
/// bar above it draws icons in the same column (its settings button is
/// the same ☰ glyph).
fn inspector_top(app: &Resonance) -> f32 {
    simulator(app)
        .find("INSPECTOR")
        .expect("the inspector renders")
        .bounds()
        .y
}

/// The `nth` text reading `label` inside the inspector — the strips draw
/// some of the same names, and every row draws the same icon glyphs.
fn in_inspector(
    label: &str,
    nth: usize,
    top: f32,
) -> impl FnMut(Candidate<'_>) -> Option<Target> + Send {
    let label = label.to_owned();
    let mut seen = 0;
    move |c: Candidate<'_>| {
        let hit = matches!(
            &c,
            Candidate::Text { content, bounds, .. }
                if *content == label && bounds.x >= INSPECTOR_LEFT && bounds.y >= top
        );
        if !hit {
            return None;
        }
        seen += 1;
        (seen == nth + 1).then(|| Target::from(c))
    }
}

/// Click the `nth` `label` in the inspector; the raised messages.
fn click(app: &Resonance, label: &str, nth: usize) -> Vec<Message> {
    let top = inspector_top(app);
    let mut sim = simulator(app);
    sim.click(in_inspector(label, nth, top))
        .unwrap_or_else(|e| panic!("{label} #{nth} should be clickable in the inspector: {e:?}"));
    sim.into_messages().collect()
}

fn one(messages: Vec<Message>) -> Message {
    let [m] = <[Message; 1]>::try_from(messages).unwrap_or_else(|m| panic!("one message: {m:?}"));
    m
}

/// The slot a ⠿ click armed, when the click raised exactly the
/// handle's press (`DragStart`) and its release (`DragDrop`).
fn grip_messages(messages: Vec<Message>) -> Option<u64> {
    match messages.as_slice() {
        [Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::DragStart(i))), Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::DragDrop))] => {
            Some(*i)
        }
        m => panic!("⠿ click: {m:?}"),
    }
}

fn shows(app: &Resonance, label: &str) -> bool {
    let top = inspector_top(app);
    simulator(app).find(in_inspector(label, 0, top)).is_ok()
}

// ---------------------------------------------------------------------------
// The row's own controls, per owner
// ---------------------------------------------------------------------------

/// Select each owner in turn and press every control on its first row.
#[test]
fn every_row_control_raises_its_message_on_track_bus_and_master() {
    let mut app = app();
    let owners: [(UiMessage, u64, &str); 3] = [
        (UiMessage::SelectTrack(Some(TRACK)), TRACK_FX, "Tape Comp"),
        (UiMessage::SelectBus(Some(BUS)), BUS_FX, "Bus Glue"),
        (UiMessage::SelectMaster, MASTER_FX, "Limiter"),
    ];
    for (select, id, name) in owners {
        ui(&mut app, select);

        match one(click(&app, "\u{25cf}", 0)) {
            Message::Plugin(PluginMessage::SetPluginBypass {
                instance_id,
                bypassed: true,
            }) if instance_id == id => {}
            m => panic!("● bypasses {name}: {m:?}"),
        }
        assert!(matches!(
            one(click(&app, name, 0)),
            Message::Plugin(PluginMessage::FocusSlot(i)) if i == id
        ));
        assert!(matches!(
            one(click(&app, GLYPH_OPEN, 0)),
            Message::Plugin(PluginMessage::OpenPluginWindow(i)) if i == id
        ));
        let menu = one(click(&app, GLYPH_MENU, 0));
        assert!(
            matches!(
                menu,
                Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::ToggleSlotMenu(i))) if i == id
            ),
            "☰ opens {name}'s menu: {menu:?}"
        );
        // A click on ⠿ arms the drag and, its release landing on the
        // handle too, drops it again at once (nothing hovered: no move)
        // — so a press and release in one event batch never leave a
        // drag armed (code review C3).
        let grip = grip_messages(click(&app, GLYPH_GRIP, 0));
        assert_eq!(grip, Some(id), "⠿ arms a drag of {name}, then disarms it");
        let remove = one(click(&app, "\u{00d7}", 0));
        let removes_this = match &remove {
            Message::Plugin(PluginMessage::RemovePluginFromTrack(t, i)) => {
                *t == TRACK && *i == id
            }
            Message::Bus(BusMessage::RemovePluginFromBus(b, i)) => *b == BUS && *i == id,
            Message::Master(MasterMessage::RemovePluginFromMaster(i)) => *i == id,
            _ => false,
        };
        assert!(removes_this, "× removes {name} from its own chain: {remove:?}");
    }
}

/// The old row's BYP and Params are gone — they live in ● and ☰ now
/// (as do ▲▼, whose caret glyphs the group headers' fold carets share,
/// so they are pinned through the menu instead).
#[test]
fn the_row_no_longer_carries_byp_or_params() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    for gone in ["BYP", "Params"] {
        assert!(!shows(&app, gone), "{gone} left the CHAIN row");
    }
}

/// A bypassed slot reads ○, and pressing it re-engages the slot.
#[test]
fn a_bypassed_slot_reads_hollow_and_its_dot_unbypasses() {
    let mut app = app();
    // The mirror moves on the engine's echo, not the request.
    app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
        instance_id: TRACK_FX,
        bypassed: true,
        own_bypass_param: false,
    });
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    match one(click(&app, "\u{25cb}", 0)) {
        Message::Plugin(PluginMessage::SetPluginBypass {
            instance_id: TRACK_FX,
            bypassed: false,
        }) => {}
        m => panic!("○ re-engages the slot: {m:?}"),
    }
}

/// The instrument slot is fixed: its handle grabs nothing.
#[test]
fn the_instrument_slots_handle_does_not_drag() {
    let mut app = app();
    app.test_push_track_plugin(INST, slot(90, "Some Synth"));
    app.test_push_track_plugin(INST, slot(91, "Chorus"));
    ui(&mut app, UiMessage::SelectTrack(Some(INST)));
    let top = inspector_top(&app);
    let mut sim = simulator(&app);
    sim.click(in_inspector(GLYPH_GRIP, 0, top)).expect("the handle renders");
    assert!(
        sim.into_messages().next().is_none(),
        "pressing the instrument's handle arms nothing"
    );
    assert_eq!(grip_messages(click(&app, GLYPH_GRIP, 1)), Some(91));
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

/// The focused row is redrawn with the accent border: `FocusSlot` (a
/// strip click) changes the lazy body's key on every owner, so the
/// highlight follows it.
#[test]
fn focus_invalidates_the_owner_inspector_so_the_highlight_follows() {
    let mut app = app();
    let track = app.test_inspector_fingerprint(TRACK).unwrap();
    let bus = app.test_bus_inspector_fingerprint(BUS).unwrap();
    let master = app.test_master_inspector_fingerprint();
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(TRACK_FX2)));
    assert_ne!(app.test_inspector_fingerprint(TRACK).unwrap(), track);
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(BUS_FX)));
    assert_ne!(app.test_bus_inspector_fingerprint(BUS).unwrap(), bus);
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(MASTER_FX)));
    assert_ne!(app.test_master_inspector_fingerprint(), master);
}

/// Opening a menu, a replace, a prompt or a drag each redraw the body.
#[test]
fn chain_ui_state_is_part_of_the_fingerprint() {
    let mut app = app();
    let mut last = app.test_inspector_fingerprint(TRACK).unwrap();
    let steps = [
        ChainUiMessage::ToggleSlotMenu(TRACK_FX),
        ChainUiMessage::BeginReplace(TRACK_FX),
        ChainUiMessage::BeginPresetSave(TRACK_FX),
        ChainUiMessage::DragStart(TRACK_FX),
        ChainUiMessage::DragOver(TRACK_FX2),
    ];
    for step in steps {
        let label = format!("{step:?}");
        chain_ui(&mut app, step);
        let now = app.test_inspector_fingerprint(TRACK).unwrap();
        assert_ne!(now, last, "{label} redraws the CHAIN");
        last = now;
    }
}

// ---------------------------------------------------------------------------
// The ☰ menu
// ---------------------------------------------------------------------------

fn labels(app: &Resonance, id: u64) -> Vec<String> {
    app.test_slot_menu_entries(id)
        .into_iter()
        .map(|(l, _)| l)
        .collect()
}

#[test]
fn the_slot_menu_lists_parameters_presets_replace_moves_and_remove() {
    let app = app();
    assert_eq!(
        labels(&app, TRACK_FX),
        [
            "Parameters\u{2026}",
            "Browse presets\u{2026}",
            "Previous preset",
            "Next preset",
            "Save preset\u{2026}",
            "Replace\u{2026}",
            "Move up",
            "Move down",
            "Remove",
        ]
    );
}

/// The menu renders under its row, and each entry dispatches through
/// `Pick`, which closes the menu first.
#[test]
fn menu_entries_render_and_close_the_menu_when_picked() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectBus(Some(BUS)));
    assert!(!shows(&app, "Replace\u{2026}"), "closed until ☰ is pressed");
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(BUS_FX));
    assert!(shows(&app, "Replace\u{2026}"));

    let remove = one(click(&app, "Remove", 0));
    match &remove {
        Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::Pick(inner))) => assert!(
            matches!(**inner, Message::Bus(BusMessage::RemovePluginFromBus(BUS, BUS_FX))),
            "Remove removes from the bus: {inner:?}"
        ),
        m => panic!("a menu entry picks: {m:?}"),
    }
    let _ = app.update(remove);
    assert_eq!(app.test_slot_menu(), None);

    // ☰ again on an open menu closes it.
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(MASTER_FX));
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(MASTER_FX));
    assert_eq!(app.test_slot_menu(), None);
}

/// "Browse presets…" focuses the slot before opening the browser on it.
#[test]
fn browse_presets_focuses_the_slot() {
    let mut app = app();
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(TRACK_FX)));
    let browse = app
        .test_slot_menu_entries(MASTER_FX)
        .into_iter()
        .find(|(l, _)| l == "Browse presets\u{2026}")
        .and_then(|(_, m)| m)
        .expect("Browse presets is enabled on a loaded plugin");
    let _ = app.update(browse);
    assert_eq!(app.test_focused_slot(), Some(MASTER_FX));
    assert!(app.test_selected_master());
}

/// "Replace…" turns the group's add picker into a replace picker for the
/// slot; Cancel turns it back.
#[test]
fn replace_turns_the_add_picker_into_a_replace_picker() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    // A pick_list's placeholder is not a text widget the selector can
    // see, so replace mode is read off its Cancel button.
    assert!(!shows(&app, "Cancel"));
    let replace = app
        .test_slot_menu_entries(TRACK_FX2)
        .into_iter()
        .find(|(l, _)| l == "Replace\u{2026}")
        .and_then(|(_, m)| m)
        .expect("Replace is always offered");
    let _ = app.update(replace);
    assert_eq!(app.test_replacing_slot(), Some(TRACK_FX2));
    assert!(matches!(
        one(click(&app, "Cancel", 0)),
        Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::CancelReplace))
    ));
    chain_ui(&mut app, ChainUiMessage::CancelReplace);
    assert_eq!(app.test_replacing_slot(), None);
    assert!(!shows(&app, "Cancel"));
}

/// "Save preset…" opens a name prompt under the row; saving arms the same
/// capture `*.save_plugin_preset` does, and closes the prompt.
#[test]
fn save_preset_prompts_for_a_name_and_arms_the_capture() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    chain_ui(&mut app, ChainUiMessage::BeginPresetSave(TRACK_FX));
    let prompt = app.test_slot_preset_save().expect("the prompt is open").clone();
    assert_eq!(prompt.instance_id, TRACK_FX);
    assert_eq!(prompt.name, "Tape Comp", "seeded with the plugin's name");
    assert!(shows(&app, "Save"));

    chain_ui(&mut app, ChainUiMessage::PresetSaveName("Warm Glue".to_owned()));
    assert!(matches!(
        one(click(&app, "Save", 0)),
        Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::CommitPresetSave))
    ));
    chain_ui(&mut app, ChainUiMessage::CommitPresetSave);
    assert_eq!(
        app.test_pending_plugin_preset_save(TRACK_FX),
        Some("Warm Glue".to_owned())
    );
    assert!(app.test_slot_preset_save().is_none());
}

// ---------------------------------------------------------------------------
// Missing plugin: recovery inline under the row (Q16)
// ---------------------------------------------------------------------------

fn make_missing(app: &mut Resonance, id: u64) {
    app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(id),
        clap_plugin_id: format!("com.resonance.p{id}"),
        clap_file_path: format!("/plugins/p{id}.clap"),
        reason: "Failed to load plugin: no such file".to_owned(),
    });
    // The load warning is modal; the inspector is under it.
    ui(app, UiMessage::DismissMissingPlugins);
}

#[test]
fn a_missing_plugin_shows_its_recovery_under_its_row() {
    let mut app = app();
    make_missing(&mut app, TRACK_FX2);
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    assert!(shows(
        &app,
        "\u{26a0} Plate Verb is not available on this machine"
    ));
    let rescan = click(&app, "Rescan", 0);
    assert!(
        matches!(rescan.as_slice(), [Message::Plugin(PluginMessage::RescanPlugins)]),
        "Rescan looks for the plugin again: {rescan:?}"
    );
    assert!(matches!(
        one(click(&app, "Remove", 0)),
        Message::Plugin(PluginMessage::RemovePluginFromTrack(TRACK, TRACK_FX2))
    ));
    // Presets need a plugin behind the slot.
    assert!(!labels(&app, TRACK_FX2).contains(&"Save preset\u{2026}".to_owned()));
}

/// The generic window no longer carries the recovery: it points at the
/// inspector.
#[test]
fn the_window_of_a_missing_plugin_points_at_the_inspector() {
    let mut app = app();
    make_missing(&mut app, MASTER_FX);
    let _ = app.update(Message::Plugin(PluginMessage::OpenPluginWindow(MASTER_FX)));
    simulator(&app)
        .find("\u{26a0} Limiter is missing \u{2014} see the inspector to replace or remove it")
        .expect("the window says where the recovery is");
}

// ---------------------------------------------------------------------------
// Header colour swatch (§3.1, §6)
// ---------------------------------------------------------------------------

fn color_of(app: &Resonance, id: u64) -> [u8; 3] {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == id)
        .unwrap()
        .color
}

#[test]
fn the_swatch_opens_the_palette_and_a_pick_sets_the_colour() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    let before = color_of(&app, TRACK);
    let fingerprint = app.test_inspector_fingerprint(TRACK).unwrap();
    let undo_depth = app.test_undo_history().undo_len();

    let mut sim = simulator(&app);
    sim.click(iced::widget::Id::new("inspector-color-swatch"))
        .expect("the header shows the swatch");
    let open = one(sim.into_messages().collect());
    assert!(matches!(
        open,
        Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::ToggleColorPalette(TRACK)))
    ));
    let _ = app.update(open);
    assert_eq!(app.test_color_palette(), Some(TRACK));

    let target = theme::TRACK_PALETTE
        .iter()
        .position(|c| *c != before)
        .unwrap();
    let mut sim = simulator(&app);
    sim.click(iced::widget::Id::from(format!("inspector-palette-{target}")))
        .expect("the palette lists every hue");
    let pick = one(sim.into_messages().collect());
    match &pick {
        Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::Pick(inner))) => assert!(
            matches!(**inner, Message::Track(TrackMessage::SetTrackColor(TRACK, c))
                if c == theme::TRACK_PALETTE[target]),
            "the swatch sets the colour: {inner:?}"
        ),
        m => panic!("a swatch picks: {m:?}"),
    }
    let _ = app.update(pick);
    assert_eq!(color_of(&app, TRACK), theme::TRACK_PALETTE[target]);
    assert_eq!(app.test_color_palette(), None, "a pick closes the palette");
    assert_ne!(
        app.test_inspector_fingerprint(TRACK).unwrap(),
        fingerprint,
        "the colour is hashed"
    );

    assert_eq!(
        app.test_undo_history().undo_len(),
        undo_depth + 1,
        "the pick is one undo entry (the palette's open/close are none)"
    );
}

/// The palette is per track: selecting another track does not show it.
#[test]
fn the_palette_belongs_to_the_track_it_was_opened_on() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    chain_ui(&mut app, ChainUiMessage::ToggleColorPalette(TRACK));
    assert!(simulator(&app).find(iced::widget::Id::new("inspector-palette-0")).is_ok());
    ui(&mut app, UiMessage::SelectTrack(Some(INST)));
    assert!(simulator(&app).find(iced::widget::Id::new("inspector-palette-0")).is_err());
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

fn golden(app: &Resonance, path: &str) {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![theme::ICON_FONT_BYTES.into()];
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    let settings = iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    };
    let mut sim = Simulator::with_size(settings, Size::new(1440.0, 900.0), app.view());
    let snap = sim
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    crate::common::assert_golden(&snap, path);
}

/// The demo track's CHAIN with its first slot focused (accent border)
/// and the last slot's ☰ menu open.
#[test]
fn chain_row_focus_and_menu_golden() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    resonance_app::demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    let ids = app.test_track_plugin_instance_ids(TRACK);
    let (&first, &last) = (ids.first().unwrap(), ids.last().unwrap());
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(first)));
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(last));
    assert_eq!(app.test_slot_menu(), Some(last));
    golden(&app, "tests/snapshots/mixer_inspector_chain_focus_menu.png");
}

/// A missing plugin's inline recovery, a bypassed slot, a drag armed
/// from the first row over the last (the accent drop line under it),
/// and the header's colour palette open.
#[test]
fn chain_row_missing_bypassed_drag_and_palette_golden() {
    let mut app = app();
    app.test_push_track_plugin(TRACK, slot(75, "Granular Delay With A Long Name"));
    make_missing(&mut app, TRACK_FX2);
    app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
        instance_id: TRACK_FX,
        bypassed: true,
        own_bypass_param: false,
    });
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    chain_ui(&mut app, ChainUiMessage::DragStart(TRACK_FX));
    chain_ui(&mut app, ChainUiMessage::DragOver(75));
    chain_ui(&mut app, ChainUiMessage::ToggleColorPalette(TRACK));
    golden(&app, "tests/snapshots/mixer_inspector_chain_missing_drag.png");
}

/// A sub-track follows its parent's colour, so its header has no swatch.
#[test]
fn a_sub_track_has_no_swatch() {
    let mut app = app();
    const SUB: u64 = 9;
    let mut sub = TrackState::new_instrument(SUB, 2);
    sub.sub_track = Some(SubTrackLink {
        parent_track_id: INST,
        output_port_index: 1,
    });
    app.test_push_track(sub);
    ui(&mut app, UiMessage::SelectTrack(Some(SUB)));
    assert!(simulator(&app)
        .find(iced::widget::Id::new("inspector-color-swatch"))
        .is_err());
    ui(&mut app, UiMessage::SelectTrack(Some(INST)));
    assert!(simulator(&app)
        .find(iced::widget::Id::new("inspector-color-swatch"))
        .is_ok());
}

// ---------------------------------------------------------------------------
// Code-review fixes (mixer cleanup batch 2)
// ---------------------------------------------------------------------------

fn press_event() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
}

fn release_event() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))
}

fn moved(at: iced::Point) -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: at })
}

fn centre(b: iced::Rectangle) -> iced::Point {
    iced::Point::new(b.x + b.width / 2.0, b.y + b.height / 2.0)
}

/// The visible bounds of the `nth` `label` inside the inspector.
fn inspector_bounds(app: &Resonance, label: &str, nth: usize) -> iced::Rectangle {
    let top = inspector_top(app);
    simulator(app)
        .find(in_inspector(label, nth, top))
        .unwrap_or_else(|e| panic!("{label} #{nth} should render in the inspector: {e:?}"))
        .bounds()
}

fn track_chain(app: &Resonance) -> Vec<u64> {
    app.test_chain_slots(resonance_app::TestChain::Track(TRACK))
        .into_iter()
        .map(|(id, _, _)| id)
        .collect()
}

/// Arm a drag of the first row by pressing its ⠿ (press only — the
/// button stays down), then walk the pointer over the rows along
/// `path`, all through the rendered view; dispatch what the rows
/// raised. Returns those messages.
fn drag_through_view(app: &mut Resonance, path: &[iced::Point]) -> Vec<Message> {
    let grip = inspector_bounds(app, GLYPH_GRIP, 0);
    let armed: Vec<Message> = {
        let mut sim = simulator(app);
        sim.point_at(centre(grip));
        let _ = sim.simulate([press_event()]);
        sim.into_messages().collect()
    };
    assert!(
        matches!(
            armed.as_slice(),
            [Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::DragStart(TRACK_FX)))]
        ),
        "{armed:?}"
    );
    for m in armed {
        let _ = app.update(m);
    }
    // One simulator for the whole walk: the rows' hover state lives in
    // its widget tree.
    let walked: Vec<Message> = {
        let mut sim = simulator(app);
        for &at in path {
            sim.point_at(at);
            let _ = sim.simulate([moved(at)]);
        }
        sim.into_messages().collect()
    };
    for m in walked.clone() {
        let _ = app.update(m);
    }
    walked
}

/// The window-level release, as the drag's subscription maps it.
fn release(app: &mut Resonance) {
    let drop = resonance_app::update::chain_ui::drag_end_event(&release_event())
        .expect("a release ends an armed drag");
    let _ = app.update(drop);
}

/// C1: entering a row makes it the drop target and LEAVING it clears
/// that — so a release after the pointer left every row moves nothing
/// (it used to move the plugin to the last row entered).
#[test]
fn a_release_after_leaving_the_rows_moves_nothing() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    let before = track_chain(&app);
    let verb = inspector_bounds(&app, "Plate Verb", 0);
    let header = inspector_bounds(&app, "CHAIN", 0);
    let walked = drag_through_view(&mut app, &[centre(verb), centre(header)]);
    assert!(
        walked.iter().any(|m| matches!(
            m,
            Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::DragOver(TRACK_FX2)))
        )),
        "entering the row is reported: {walked:?}"
    );
    assert!(
        walked.iter().any(|m| matches!(
            m,
            Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::DragLeave(TRACK_FX2)))
        )),
        "and so is leaving it: {walked:?}"
    );
    assert_eq!(app.test_chain_drag().and_then(|d| d.over), None);

    let revision = app.revision();
    release(&mut app);
    assert_eq!(track_chain(&app), before, "released off the rows: no move");
    assert_eq!(app.revision(), revision);
    assert_eq!(app.test_chain_drag(), None, "and the drag is over");
}

/// C1, the other half: released while over a row, the drop lands there.
#[test]
fn a_release_over_a_row_drops_there() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    let verb = inspector_bounds(&app, "Plate Verb", 0);
    drag_through_view(&mut app, &[centre(verb)]);
    assert_eq!(app.test_chain_drag().and_then(|d| d.over), Some(TRACK_FX2));
    release(&mut app);
    assert_eq!(track_chain(&app), vec![TRACK_FX2, TRACK_FX]);
}

/// C1: an armed drag does not survive the inspector changing owner or
/// the view switching.
#[test]
fn selection_change_and_view_switch_drop_an_armed_drag() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    chain_ui(&mut app, ChainUiMessage::DragStart(TRACK_FX));
    ui(&mut app, UiMessage::SelectBus(Some(BUS)));
    assert_eq!(app.test_chain_drag(), None, "another owner");

    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    chain_ui(&mut app, ChainUiMessage::DragStart(TRACK_FX));
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    assert!(app.test_chain_drag().is_some(), "re-selecting the owner keeps it");
    ui(&mut app, UiMessage::SwitchView(ViewMode::Arrange));
    assert_eq!(app.test_chain_drag(), None, "a view switch");
}

/// C3: a click on ⠿ whose press and release arrive in one event batch
/// reaches the handle as both, before the window-level release listener
/// exists — dispatched in order, it leaves no drag armed.
#[test]
fn a_click_on_the_handle_leaves_no_drag_armed() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    let before = track_chain(&app);
    for m in click(&app, GLYPH_GRIP, 0) {
        let _ = app.update(m);
    }
    assert_eq!(app.test_chain_drag(), None);
    assert_eq!(track_chain(&app), before);
}

/// C3: a press while a drag is armed means its release was lost — the
/// drag disarms. Unless that press is the one that just re-armed it on
/// another handle: the handle's `DragStart` arrives before the press
/// listener's message for the same press, and is not undone by it.
#[test]
fn a_press_disarms_a_stuck_drag_but_not_the_one_it_starts() {
    let pressed = resonance_app::update::chain_ui::drag_end_event(&press_event())
        .expect("a press is reported while a drag is armed");
    assert!(matches!(
        pressed,
        Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::DragPointerPressed))
    ));

    let mut app = app();
    // Stuck: armed, its release never came, then a press elsewhere.
    chain_ui(&mut app, ChainUiMessage::DragStart(TRACK_FX));
    let _ = app.update(pressed.clone());
    assert_eq!(app.test_chain_drag(), None, "the stuck drag is gone");

    // Re-grabbed on another handle while stuck: the new drag survives
    // its own press, and the next press (the lost release again) ends it.
    chain_ui(&mut app, ChainUiMessage::DragStart(TRACK_FX));
    chain_ui(&mut app, ChainUiMessage::DragStart(TRACK_FX2));
    let _ = app.update(pressed.clone());
    assert_eq!(app.test_chain_drag().map(|d| d.instance_id), Some(TRACK_FX2));
    let _ = app.update(pressed);
    assert_eq!(app.test_chain_drag(), None);
}

fn esc(app: &mut Resonance, captured: bool) {
    use resonance_app::commands::{KeyChord, Mods, NamedKey};
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured,
    }));
}

/// C4: Esc closes the CHAIN's transient state one layer per press: a
/// drag, then the preset prompt (whose field captured the key), then
/// replace mode, then the slot menu.
#[test]
fn escape_closes_drag_prompt_replace_and_menu_in_turn() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    chain_ui(&mut app, ChainUiMessage::DragStart(TRACK_FX));
    esc(&mut app, false);
    assert_eq!(app.test_chain_drag(), None);

    chain_ui(&mut app, ChainUiMessage::BeginReplace(TRACK_FX2));
    chain_ui(&mut app, ChainUiMessage::BeginPresetSave(TRACK_FX));
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(TRACK_FX2));
    esc(&mut app, true);
    assert!(app.test_slot_preset_save().is_none(), "the prompt first");
    assert!(app.test_replacing_slot().is_some());
    esc(&mut app, false);
    assert_eq!(app.test_replacing_slot(), None, "then replace mode");
    assert_eq!(app.test_slot_menu(), Some(TRACK_FX2));
    esc(&mut app, false);
    assert_eq!(app.test_slot_menu(), None, "then the menu");

    chain_ui(&mut app, ChainUiMessage::ToggleColorPalette(TRACK));
    esc(&mut app, false);
    assert_eq!(app.test_color_palette(), None, "and the palette");
}

/// C4: a press anywhere closes an open slot menu or palette (click-away),
/// except the press that just opened another one in its place.
#[test]
fn a_press_elsewhere_closes_the_menu_but_not_the_one_it_opens() {
    use resonance_app::update::chain_ui::popover_press_event;
    let mut app = app();
    let dismiss = popover_press_event(&press_event()).expect("a press is a click-away");

    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(TRACK_FX));
    let _ = app.update(dismiss.clone());
    assert_eq!(app.test_slot_menu(), None, "click-away");

    // Menu A open; ☰ of B pressed: the widget's toggle comes first, the
    // listener's dismiss for that same press after it.
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(TRACK_FX));
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(TRACK_FX2));
    let _ = app.update(dismiss.clone());
    assert_eq!(app.test_slot_menu(), Some(TRACK_FX2), "B stays open");
    let _ = app.update(dismiss.clone());
    assert_eq!(app.test_slot_menu(), None, "the next press closes it");

    chain_ui(&mut app, ChainUiMessage::ToggleColorPalette(TRACK));
    let _ = app.update(dismiss);
    assert_eq!(app.test_color_palette(), None);
}

/// C5: "Save preset…" puts the caret in the name field (a focus +
/// select-all task on the field's id), and Enter in it saves.
#[test]
fn the_preset_prompt_takes_focus_and_enter_saves() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    let task = app.update(Message::Plugin(PluginMessage::ChainUi(
        ChainUiMessage::BeginPresetSave(TRACK_FX),
    )));
    assert!(task.units() > 0, "opening the prompt focuses its field");

    let mut sim = simulator(&app);
    sim.click(Resonance::test_preset_name_input_id())
        .expect("the name field carries the focus target's id");
    let _ = sim.tap_key(iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter));
    let messages: Vec<Message> = sim.into_messages().collect();
    assert!(
        messages.iter().any(|m| matches!(
            m,
            Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::CommitPresetSave))
        )),
        "Enter commits: {messages:?}"
    );
}

/// A private preset root, removed when the test ends.
struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "resonance-chain-presets-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The plugin hands its state back, as the engine does after an armed
/// capture: the pending save is written.
fn state_saved(app: &mut Resonance) {
    app.test_apply_engine_event(AudioEvent::PluginPresetStateSaved {
        instance_id: TRACK_FX,
        data: br#"{"version":1,"params":{"1":0.5}}"#.to_vec(),
        preset_form: true,
        first_party: true,
    });
}

fn gui_save(app: &mut Resonance, name: &str) {
    chain_ui(app, ChainUiMessage::BeginPresetSave(TRACK_FX));
    chain_ui(app, ChainUiMessage::PresetSaveName(name.to_owned()));
    chain_ui(app, ChainUiMessage::CommitPresetSave);
}

/// C6: a GUI save never replaces a save still waiting on the same
/// plugin's state — say one the control API armed. It is refused with a
/// banner, the prompt stays open, and the pending save is untouched.
#[test]
fn a_gui_save_waits_for_a_pending_save_of_the_same_plugin() {
    let root = TempRoot::new("pending");
    let mut app = app();
    app.test_set_plugin_preset_root(root.0.clone());
    let armed = crate::common::call(
        &mut app,
        "track.save_plugin_preset",
        serde_json::json!({
            "track_id": TRACK,
            "plugin_id": format!("com.resonance.p{TRACK_FX}"),
            "name": "From Agent",
            "overwrite": false,
        }),
    );
    assert!(armed.error.is_none(), "{:?}", armed.error);
    assert_eq!(
        app.test_pending_plugin_preset_save(TRACK_FX),
        Some("From Agent".to_owned())
    );

    gui_save(&mut app, "From Gui");
    assert_eq!(
        app.test_pending_plugin_preset_save(TRACK_FX),
        Some("From Agent".to_owned()),
        "the control API's save is not replaced"
    );
    assert!(app.test_slot_preset_save().is_some(), "the prompt stays open");
    assert!(app.test_error_message_is_set(), "and says why");

    // Once that save lands, the same commit goes through.
    state_saved(&mut app);
    chain_ui(&mut app, ChainUiMessage::CommitPresetSave);
    assert_eq!(
        app.test_pending_plugin_preset_save(TRACK_FX),
        Some("From Gui".to_owned())
    );
}

/// C6: whether the name is taken is asked again at commit time. A name
/// saved since the prompt last looked turns the button into "Overwrite"
/// and needs a second press; that press overwrites the preset in place
/// (same id), it does not add a second one of the same name.
#[test]
fn a_name_taken_since_the_last_keystroke_needs_a_confirmed_overwrite() {
    let root = TempRoot::new("overwrite");
    let mut app = app();
    app.test_set_plugin_preset_root(root.0.clone());
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));

    // The prompt is typed while the name is still free...
    chain_ui(&mut app, ChainUiMessage::BeginPresetSave(TRACK_FX));
    chain_ui(&mut app, ChainUiMessage::PresetSaveName("Warm Glue".to_owned()));
    assert!(!app.test_slot_preset_save().unwrap().exists, "the button reads Save");

    // ...then, with the prompt still open, the control API saves a
    // preset of that very name, and it lands.
    let armed = crate::common::call(
        &mut app,
        "track.save_plugin_preset",
        serde_json::json!({
            "track_id": TRACK,
            "plugin_id": format!("com.resonance.p{TRACK_FX}"),
            "name": "Warm Glue",
            "overwrite": false,
        }),
    );
    assert!(armed.error.is_none(), "{:?}", armed.error);
    state_saved(&mut app);
    let id = app
        .test_user_preset_id(TRACK_FX, "Warm Glue")
        .expect("the control API's save wrote the preset");

    // The prompt's press, made while it still read "Save", does not
    // overwrite that preset unannounced.
    chain_ui(&mut app, ChainUiMessage::CommitPresetSave);
    assert_eq!(app.test_pending_plugin_preset_save(TRACK_FX), None, "nothing armed");
    let now = app.test_slot_preset_save().expect("the prompt stays open");
    assert!(now.exists, "the button now reads Overwrite");
    assert!(shows(&app, "Overwrite"));

    // The confirmed press overwrites the same preset.
    chain_ui(&mut app, ChainUiMessage::CommitPresetSave);
    assert_eq!(
        app.test_pending_plugin_preset_save(TRACK_FX),
        Some("Warm Glue".to_owned())
    );
    state_saved(&mut app);
    assert_eq!(
        app.test_user_preset_id(TRACK_FX, "Warm Glue"),
        Some(id),
        "overwritten in place, not added again"
    );
}

/// C2: the inspector draws the instrument slot where the chain holds the
/// instrument. A sub-track's chain is effects only — its first row drags
/// like any effect — and an effect ahead of an instrument drags while
/// the instrument's handle is fixed.
#[test]
fn the_instrument_slot_is_the_instrument_not_row_zero() {
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/p80.clap".to_owned(),
                clap_plugin_id: "com.resonance.p80".to_owned(),
                name: "Sub EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                ..Default::default()
            },
            ScannedPlugin {
                clap_file_path: "/plugins/p82.clap".to_owned(),
                clap_plugin_id: "com.resonance.p82".to_owned(),
                name: "Synth".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
                ..Default::default()
            },
        ],
    });

    // A sub-track (instrument-typed) with a scanned effect on it.
    const SUB: u64 = 9;
    let mut sub = TrackState::new_instrument(SUB, 2);
    sub.sub_track = Some(SubTrackLink {
        parent_track_id: INST,
        output_port_index: 1,
    });
    app.test_push_track(sub);
    app.test_push_track_plugin(SUB, slot(80, "Sub EQ"));
    ui(&mut app, UiMessage::SelectTrack(Some(SUB)));
    assert_eq!(
        grip_messages(click(&app, GLYPH_GRIP, 0)),
        Some(80),
        "a sub-track's first row is an effect: its handle drags"
    );

    // An instrument track with an effect ahead of its instrument.
    app.test_push_track_plugin(INST, slot(80 + 1, "Pre Drive"));
    app.test_push_track_plugin(INST, slot(82, "Synth"));
    ui(&mut app, UiMessage::SelectTrack(Some(INST)));
    // "Pre Drive" (p81) is not scanned, but it is not slot 0's only
    // claim: the scanned instrument further down is the instrument.
    assert_eq!(grip_messages(click(&app, GLYPH_GRIP, 0)), Some(81));
    let top = inspector_top(&app);
    let mut sim = simulator(&app);
    sim.click(in_inspector(GLYPH_GRIP, 1, top)).expect("the second handle renders");
    assert!(
        sim.into_messages().next().is_none(),
        "the instrument's handle is fixed, wherever it sits"
    );
}

/// S6: focusing a slot is not a selection gesture. A plain click on a
/// slot of a track inside a multi-selection keeps the selection (that
/// track becomes the primary one); an additive one adds its track.
#[test]
fn focusing_a_slot_keeps_the_multi_selection() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(TRACK)));
    let additive = iced::keyboard::Modifiers::SHIFT;
    ui(&mut app, UiMessage::ModifiersChanged(additive));
    ui(&mut app, UiMessage::SelectTrack(Some(INST)));
    ui(&mut app, UiMessage::ModifiersChanged(iced::keyboard::Modifiers::empty()));
    assert_eq!(app.test_selected_tracks(), &[TRACK, INST]);

    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(TRACK_FX)));
    assert_eq!(app.test_focused_slot(), Some(TRACK_FX));
    assert_eq!(app.test_selected_tracks(), &[INST, TRACK], "both still selected");
    assert_eq!(app.test_selected_track(), Some(TRACK), "the slot's track leads");

    // A plain click on a slot of a track outside the selection selects
    // that track alone, as a click on its strip does.
    const OTHER: u64 = 3;
    app.test_add_track(OTHER, TrackType::Audio);
    app.test_push_track_plugin(OTHER, slot(79, "Other FX"));
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(79)));
    assert_eq!(app.test_selected_tracks(), &[OTHER]);

    // An additive one adds it.
    ui(&mut app, UiMessage::ModifiersChanged(additive));
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(TRACK_FX)));
    assert_eq!(app.test_selected_tracks(), &[OTHER, TRACK]);
}
