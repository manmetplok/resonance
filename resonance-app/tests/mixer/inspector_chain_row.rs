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
        assert!(matches!(
            one(click(&app, GLYPH_GRIP, 0)),
            Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::DragStart(i))) if i == id
        ));
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
    assert!(matches!(
        one(click(&app, GLYPH_GRIP, 1)),
        Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::DragStart(91)))
    ));
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
        app.test_pending_plugin_preset_save(),
        Some((TRACK_FX, "Warm Glue".to_owned()))
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
