//! The Preferences › Keyboard panel's reducer and the persisted keymap
//! (command-palette.md §8): a DAW preset plus the user's overrides, replayed
//! in order onto the preset's table.

use iced::Task;

use crate::commands::{BindingMap, CommandId, KeyChord, KeymapPreset};
use crate::message::Message;
use crate::settings::{KeymapOverride, KeymapSettings};
use crate::Resonance;

/// The panel's state types and the preset lookup live in
/// `state::keymap_editor` (`UiTransientState` holds the state — ARCH2-05);
/// re-exported so this module stays their import path.
pub use crate::state::keymap_editor::{preset_of, KeymapConflict, KeymapEditorState, SettingsTab};

/// Keyboard-panel interaction, routed as `UiMessage::Keymap`.
#[derive(Debug, Clone)]
pub enum KeymapMsg {
    SetTab(SettingsTab),
    SetPreset(KeymapPreset),
    Filter(String),
    /// Start listening for a new chord for this command.
    BeginRebind(CommandId),
    /// The key pressed while listening.
    Captured(KeyChord),
    /// Take the chord from its current owner.
    ConfirmConflict,
    CancelRebind,
    /// Leave the command with no chord at all.
    Unbind(CommandId),
    /// Back to the preset's chord for this command.
    Reset(CommandId),
    ResetAll,
}

/// Build the active keymap: the preset, then every override in order.
/// Overrides naming an unknown command or an unparsable chord are skipped.
pub fn resolve(settings: &KeymapSettings) -> BindingMap {
    let mut map = preset_of(settings).bindings();
    for o in &settings.overrides {
        let Some(id) = CommandId::from_key(&o.command) else {
            continue;
        };
        match o.chord.as_deref().map(KeyChord::parse) {
            // A rebind replaces the primary chord; alternates stay.
            Some(Some(chord)) => map.set_primary(id, chord),
            Some(None) => {}
            None => map.clear(id),
        }
    }
    map
}

/// Rebuild the live keymap from settings and persist them.
fn apply(r: &mut Resonance) {
    r.ui.keymap = resolve(&r.settings.keymap);
    let preset = preset_of(&r.settings.keymap);
    r.ui.keymap_editor.baseline = preset.bindings();
    r.ui.keymap_editor.unbound_by_preset = preset.unbound();
    crate::settings::persist(&r.settings);
}

/// Record `command → chord` (or unbound) as the newest override.
fn push_override(r: &mut Resonance, command: CommandId, chord: Option<KeyChord>) {
    let overrides = &mut r.settings.keymap.overrides;
    overrides.retain(|o| o.command != command.key());
    overrides.push(KeymapOverride {
        command: command.key().to_string(),
        chord: chord.map(|c| c.format_tokens()),
    });
    apply(r);
}

pub(crate) fn handle(r: &mut Resonance, msg: KeymapMsg) -> Task<Message> {
    let editor = &mut r.ui.keymap_editor;
    match msg {
        KeymapMsg::SetTab(tab) => {
            editor.tab = tab;
            editor.capturing = None;
            editor.conflict = None;
            if tab == SettingsTab::Keyboard {
                let preset = preset_of(&r.settings.keymap);
                editor.baseline = preset.bindings();
                editor.unbound_by_preset = preset.unbound();
            }
        }
        KeymapMsg::SetPreset(preset) => {
            r.settings.keymap.preset = preset.key().to_string();
            apply(r);
        }
        KeymapMsg::Filter(text) => editor.filter = text,
        KeymapMsg::BeginRebind(command) => {
            editor.capturing = Some(command);
            editor.conflict = None;
        }
        KeymapMsg::Captured(chord) => {
            let Some(command) = editor.capturing.take() else {
                return Task::none();
            };
            match r.ui.keymap.command_for(command.scope(), chord) {
                Some(owner) if owner != command => {
                    r.ui.keymap_editor.conflict = Some(KeymapConflict {
                        command,
                        chord,
                        owner,
                    });
                }
                _ => push_override(r, command, Some(chord)),
            }
        }
        KeymapMsg::ConfirmConflict => {
            if let Some(c) = editor.conflict.take() {
                push_override(r, c.command, Some(c.chord));
            }
        }
        KeymapMsg::CancelRebind => {
            editor.capturing = None;
            editor.conflict = None;
        }
        KeymapMsg::Unbind(command) => push_override(r, command, None),
        KeymapMsg::Reset(command) => {
            r.settings.keymap.overrides.retain(|o| o.command != command.key());
            apply(r);
        }
        KeymapMsg::ResetAll => {
            r.settings.keymap.overrides.clear();
            apply(r);
        }
    }
    Task::none()
}

/// While the panel listens for a chord, every key press is the answer:
/// Esc cancels, anything else is captured. Returns `None` when not
/// listening.
pub(crate) fn key(r: &mut Resonance, chord: KeyChord, repeat: bool) -> Option<Task<Message>> {
    r.ui.keymap_editor.capturing?;
    if repeat {
        return Some(Task::none());
    }
    let esc = KeyChord::named(crate::commands::NamedKey::Escape, crate::commands::Mods::NONE);
    let msg = if chord == esc {
        KeymapMsg::CancelRebind
    } else {
        KeymapMsg::Captured(chord)
    };
    Some(handle(r, msg))
}
