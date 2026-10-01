//! The mixer's transient CHAIN / strip state belongs to the channel the
//! inspector describes (`update::chain_ui::settle`, run after every
//! outermost update): the focused slot, a ☰ menu, replace mode, the
//! preset prompt, a drag, the colour palette and the instrument-picker
//! cue. Whatever moves the inspector off that channel — a click, a key,
//! `FocusSlot`, a removal, a control call — drops them, and so does the
//! thing they name going away. Also: every view change runs the same
//! reset (Performance mode included), and Esc reaches the CHAIN only
//! when no other widget spent it.

use resonance_app::commands::{KeyChord, Mods, NamedKey};
use resonance_app::message::{ChainUiMessage, Message, PluginMessage, UiMessage};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::TrackType;

use crate::common::call;

const AUDIO: u64 = 1;
const SYNTH: u64 = 2;
const EMPTY_SYNTH: u64 = 3;
const BUS: u64 = 1;

const EQ: u64 = 101;
const COMP: u64 = 102;
const WAVE: u64 = 201;
const BUS_FX: u64 = 301;
const MASTER_FX: u64 = 401;

fn slot(instance: u64, name: &str) -> PluginSlotState {
    PluginSlotState::new(
        instance,
        name.to_owned(),
        format!("com.resonance.{instance}"),
        "/plugins/x.clap".to_owned(),
        Vec::new(),
        false,
    )
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_add_track(SYNTH, TrackType::Instrument);
    app.test_add_track(EMPTY_SYNTH, TrackType::Instrument);
    app.test_add_bus(BUS, "Verb");
    app.test_push_track_plugin(AUDIO, slot(EQ, "Resonance EQ"));
    app.test_push_track_plugin(AUDIO, slot(COMP, "Tape Comp"));
    app.test_push_track_plugin(SYNTH, slot(WAVE, "Resonance Wave"));
    app.test_push_bus_plugin(BUS, slot(BUS_FX, "Plate"));
    app.test_push_master_plugin(slot(MASTER_FX, "Limiter"));
    ui(&mut app, UiMessage::SelectTrack(None));
    ui(&mut app, UiMessage::SwitchView(ViewMode::Mixer));
    app
}

fn ui(app: &mut Resonance, m: UiMessage) {
    let _ = app.update(Message::Ui(m));
}

fn chain_ui(app: &mut Resonance, m: ChainUiMessage) {
    let _ = app.update(Message::Plugin(PluginMessage::ChainUi(m)));
}

fn focus(app: &mut Resonance, id: u64) {
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(id)));
}

fn esc(app: &mut Resonance, captured: bool) {
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured,
    }));
}

// ---------------------------------------------------------------------------
// F2: the focused slot follows the inspector's owner
// ---------------------------------------------------------------------------

#[test]
fn the_focused_slot_goes_when_the_inspector_moves_to_another_channel() {
    let mut app = app();
    focus(&mut app, EQ);
    assert_eq!(app.test_selected_track(), Some(AUDIO), "focus selects the owner");
    assert_eq!(app.test_focused_slot(), Some(EQ));
    assert_eq!(app.test_preset_target(), Some(EQ));

    // Another slot of the same owner keeps the owner, moves the focus.
    focus(&mut app, COMP);
    assert_eq!(app.test_focused_slot(), Some(COMP));
    // Reselecting the same track is no owner change.
    ui(&mut app, UiMessage::SelectTrack(Some(AUDIO)));
    assert_eq!(app.test_focused_slot(), Some(COMP));

    for other in [
        UiMessage::SelectTrack(Some(SYNTH)),
        UiMessage::SelectBus(Some(BUS)),
        UiMessage::SelectMaster,
        UiMessage::SelectTrack(None),
    ] {
        focus(&mut app, COMP);
        let label = format!("{other:?}");
        ui(&mut app, other);
        assert_eq!(app.test_focused_slot(), None, "{label} drops the focus");
        assert_eq!(app.test_preset_target(), None, "{label}: no preset target");
        let (_, _, _, focused, _) = app.test_strip_slot_line(COMP).unwrap();
        assert!(!focused, "{label}: the strip highlight goes");
    }
}

/// Focusing a slot on another channel is itself an owner change: the
/// old owner's ☰ menu closes, the new owner's focus stays.
#[test]
fn focus_slot_on_another_channel_prunes_the_old_owners_state() {
    let mut app = app();
    focus(&mut app, EQ);
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(COMP));
    chain_ui(&mut app, ChainUiMessage::BeginReplace(EQ));
    assert_eq!(app.test_slot_menu(), Some(COMP));
    focus(&mut app, BUS_FX);
    assert_eq!(app.test_selected_bus(), Some(BUS));
    assert_eq!(app.test_focused_slot(), Some(BUS_FX));
    assert_eq!(app.test_slot_menu(), None);
    assert_eq!(app.test_replacing_slot(), None);
}

/// "Parameters…" / an editor fallback opens the generic window; it
/// selects the slot's channel, so the window, the inspector and the
/// strip highlight all name one plugin.
#[test]
fn opening_the_generic_window_selects_the_slots_channel() {
    let mut app = app();
    let _ = app.update(Message::Plugin(PluginMessage::OpenGenericParams(BUS_FX)));
    assert_eq!(app.test_plugin_window(), Some(BUS_FX));
    assert_eq!(app.test_selected_bus(), Some(BUS));
    assert_eq!(app.test_focused_slot(), Some(BUS_FX));
    assert_eq!(app.test_preset_target(), Some(BUS_FX));

    let _ = app.update(Message::Plugin(PluginMessage::OpenGenericParams(MASTER_FX)));
    assert!(app.test_selected_master());
    assert_eq!(app.test_selected_bus(), None);
    assert_eq!(app.test_focused_slot(), Some(MASTER_FX));
}

/// The preset commands never act on a slot the user cannot see is
/// targeted: Performance mode hides both the window and the inspector.
#[test]
fn the_preset_target_needs_the_slot_on_screen() {
    let mut app = app();
    focus(&mut app, EQ);
    assert_eq!(app.test_preset_target(), Some(EQ));
    ui(&mut app, UiMessage::SwitchView(ViewMode::Performance));
    assert_eq!(app.test_preset_target(), None);
    ui(&mut app, UiMessage::SwitchView(ViewMode::Mixer));
    assert_eq!(app.test_preset_target(), Some(EQ), "the focus outlives the view");
}

// ---------------------------------------------------------------------------
// F5: pruning on paths that are not UI messages
// ---------------------------------------------------------------------------

#[test]
fn deleting_the_inspected_track_by_control_call_prunes_its_state() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(AUDIO)));
    focus(&mut app, EQ);
    chain_ui(&mut app, ChainUiMessage::ToggleColorPalette(AUDIO));
    assert_eq!(app.test_color_palette(), Some(AUDIO));
    let response = call(
        &mut app,
        "track.delete",
        serde_json::json!({ "track_id": AUDIO, "confirm": true }),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(app.test_color_palette(), None, "the palette for a deleted track closes");
    assert_eq!(app.test_focused_slot(), None);
}

#[test]
fn deleting_the_inspected_bus_by_control_call_prunes_its_state() {
    let mut app = app();
    focus(&mut app, BUS_FX);
    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(BUS_FX));
    chain_ui(&mut app, ChainUiMessage::DragStart(BUS_FX));
    let response = call(
        &mut app,
        "bus.delete",
        serde_json::json!({ "bus_id": BUS, "confirm": true }),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(app.test_focused_slot(), None);
    assert_eq!(app.test_slot_menu(), None);
    assert_eq!(app.test_chain_drag(), None);
}

/// A palette open on a track that a removal deselected under it (no
/// owner change message ran) still closes.
#[test]
fn a_removed_tracks_palette_closes_whatever_removed_it() {
    let mut app = app();
    ui(&mut app, UiMessage::SelectTrack(Some(SYNTH)));
    chain_ui(&mut app, ChainUiMessage::ToggleColorPalette(SYNTH));
    let _ = app.update(Message::Track(
        resonance_app::message::TrackMessage::RequestRemoveTrack(SYNTH),
    ));
    let _ = app.update(Message::Track(
        resonance_app::message::TrackMessage::ConfirmRemoveTrack,
    ));
    assert!(app.test_registry().tracks.iter().all(|t| t.id != SYNTH));
    assert_eq!(app.test_color_palette(), None);
}

// ---------------------------------------------------------------------------
// F3: every view change resets
// ---------------------------------------------------------------------------

#[test]
fn performance_mode_drops_the_chain_state_both_ways() {
    for enter in [UiMessage::TogglePerformanceMode, UiMessage::SwitchView(ViewMode::Performance)] {
        let mut app = app();
        focus(&mut app, EQ);
        chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(EQ));
        ui(&mut app, enter.clone());
        assert_eq!(app.test_slot_menu(), None, "{enter:?} closes the menu");

        ui(&mut app, UiMessage::TogglePerformanceMode);
        assert_eq!(app.test_view_mode(), ViewMode::Mixer, "back where it came from");
        chain_ui(&mut app, ChainUiMessage::BeginReplace(EQ));
        ui(&mut app, UiMessage::TogglePerformanceMode);
        assert_eq!(app.test_replacing_slot(), None, "entering by toggle resets");
        chain_ui(&mut app, ChainUiMessage::BeginReplace(EQ));
        ui(&mut app, UiMessage::ExitPerformanceMode);
        assert_eq!(app.test_view_mode(), ViewMode::Mixer);
    }
}

/// Leaving Performance commits an open inline rename like any other view
/// switch (the field is a blur).
#[test]
fn leaving_performance_mode_commits_a_rename() {
    use resonance_app::state::{RenameSurface, RenameTarget};
    let mut app = app();
    ui(&mut app, UiMessage::BeginRename(RenameTarget::Track(AUDIO), RenameSurface::Strip));
    ui(&mut app, UiMessage::RenameInput("Vox".to_owned()));
    ui(&mut app, UiMessage::TogglePerformanceMode);
    assert_eq!(app.test_renaming(), None);
    let name = &app.test_registry().tracks.iter().find(|t| t.id == AUDIO).unwrap().name;
    assert_eq!(name, "Vox");
}

#[test]
fn esc_reaches_the_chain_only_in_the_mixer() {
    let mut app = app();
    focus(&mut app, EQ);
    chain_ui(&mut app, ChainUiMessage::BeginReplace(EQ));
    // Off the Mixer the CHAIN is not drawn, so Esc is not its key. (The
    // switch itself resets replace mode; re-arm it behind the view.)
    ui(&mut app, UiMessage::SwitchView(ViewMode::Arrange));
    chain_ui(&mut app, ChainUiMessage::BeginReplace(EQ));
    esc(&mut app, false);
    assert_eq!(app.test_replacing_slot(), Some(EQ), "Esc in Arrange leaves it");
}

// ---------------------------------------------------------------------------
// F4: the instrument-picker cue
// ---------------------------------------------------------------------------

#[test]
fn the_picker_cue_goes_once_the_track_has_an_instrument() {
    let mut app = app();
    chain_ui(&mut app, ChainUiMessage::CueInstrumentPicker(EMPTY_SYNTH));
    assert_eq!(app.test_instrument_picker_cue(), Some(EMPTY_SYNTH));
    // Any unclassified plugin in slot 0 is the instrument.
    app.test_push_track_plugin(EMPTY_SYNTH, slot(999, "Some Synth"));
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_instrument_picker_cue(), None);

    // So Esc means what it means next — here, closing the window.
    let _ = app.update(Message::Plugin(PluginMessage::OpenGenericParams(999)));
    assert!(app.test_plugin_window().is_some());
    esc(&mut app, false);
    assert_eq!(app.test_plugin_window(), None, "Esc was not eaten by a stale cue");
}

// ---------------------------------------------------------------------------
// F6: a captured Esc is spent
// ---------------------------------------------------------------------------

/// The replace picker's dropdown captures Esc to close itself; replace
/// mode stays. The preset prompt's field captures Esc too, and that one
/// does close the prompt.
#[test]
fn a_captured_esc_closes_only_the_preset_prompt() {
    let mut app = app();
    focus(&mut app, EQ);
    chain_ui(&mut app, ChainUiMessage::BeginReplace(EQ));
    esc(&mut app, true);
    assert_eq!(app.test_replacing_slot(), Some(EQ), "the dropdown spent the key");
    esc(&mut app, false);
    assert_eq!(app.test_replacing_slot(), None);

    chain_ui(&mut app, ChainUiMessage::ToggleSlotMenu(EQ));
    esc(&mut app, true);
    assert_eq!(app.test_slot_menu(), Some(EQ));

    chain_ui(&mut app, ChainUiMessage::BeginPresetSave(EQ));
    assert!(app.test_slot_preset_save().is_some());
    esc(&mut app, true);
    assert!(app.test_slot_preset_save().is_none(), "the prompt's field captured it");
}
