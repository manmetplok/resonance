//! The lane inspector's voice preset controls
//! (`docs/features/compose/voice-presets.md`): pick a preset, save the
//! lane's voice as a user preset, rename or delete one.
//!
//! Picking re-enters `update()` as the lane inspector's own
//! [`LaneInspectorMsg::ApplyVoicePreset`], so it is one undoable lane edit
//! (and arms the debounced re-render like any other voice setting). Saving,
//! renaming and deleting write the user's preset folder — machine state,
//! not the project — so every message here is `UndoAction::Skip`.

use iced::Task;
use resonance_audio::types::TrackId;

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::vocal_svs::voice_presets::{self, builtin_presets};
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::state::voice_presets::{PromptMode, VoicePresetPrompt};
use crate::Resonance;

#[derive(Debug, Clone, PartialEq)]
pub enum VoicePresetMessage {
    /// Apply preset `id` to the lane.
    Pick {
        definition_id: u64,
        track_id: TrackId,
        id: String,
    },
    /// Open the name prompt to save the lane's voice as a user preset.
    OpenSave {
        definition_id: u64,
        track_id: TrackId,
    },
    /// Open the name prompt to rename user preset `name`.
    OpenRename {
        definition_id: u64,
        track_id: TrackId,
        name: String,
    },
    /// The prompt's text changed.
    NameChanged(String),
    /// Save or rename under the prompt's name.
    Confirm,
    /// Close the prompt (or disarm a delete).
    Cancel,
    /// Delete user preset `name`: the first press arms, a second press of
    /// the same preset deletes.
    Delete(String),
}

impl VoicePresetMessage {
    /// Exhaustive on purpose (ARCH-06 A6-4): none of these touches the
    /// project. `Pick` re-enters `update()` as the lane inspector's own
    /// message, which records its undo entry there.
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            Self::Pick { .. }
            | Self::OpenSave { .. }
            | Self::OpenRename { .. }
            | Self::NameChanged(_)
            | Self::Confirm
            | Self::Cancel
            | Self::Delete(_) => UndoAction::Skip,
        }
    }
}

pub fn handle(app: &mut Resonance, message: VoicePresetMessage) -> Task<Message> {
    use VoicePresetMessage as M;
    match message {
        M::Pick {
            definition_id,
            track_id,
            id,
        } => {
            app.voice_presets.prompt = None;
            app.voice_presets.confirm_delete = None;
            app.voice_presets.error = None;
            let Some(preset) = voice_presets::find(&app.voice_presets.user, &id) else {
                return Task::none();
            };
            let settings = Box::new(preset.settings.clone());
            return app.update(Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::ApplyVoicePreset(settings),
            }));
        }
        M::OpenSave {
            definition_id,
            track_id,
        } => {
            app.voice_presets.confirm_delete = None;
            app.voice_presets.error = None;
            app.voice_presets.prompt = Some(VoicePresetPrompt {
                definition_id,
                track_id,
                mode: PromptMode::Save,
                name: String::new(),
            });
        }
        M::OpenRename {
            definition_id,
            track_id,
            name,
        } => {
            app.voice_presets.confirm_delete = None;
            app.voice_presets.error = None;
            app.voice_presets.prompt = Some(VoicePresetPrompt {
                definition_id,
                track_id,
                mode: PromptMode::Rename(name.clone()),
                name,
            });
        }
        M::NameChanged(name) => {
            if let Some(prompt) = app.voice_presets.prompt.as_mut() {
                prompt.name = name;
            }
        }
        M::Confirm => confirm(app),
        M::Cancel => {
            app.voice_presets.prompt = None;
            app.voice_presets.confirm_delete = None;
            app.voice_presets.error = None;
        }
        M::Delete(name) => {
            app.voice_presets.prompt = None;
            app.voice_presets.error = None;
            if app.voice_presets.confirm_delete.as_deref() != Some(name.as_str()) {
                app.voice_presets.confirm_delete = Some(name);
                return Task::none();
            }
            app.voice_presets.confirm_delete = None;
            if let Err(e) = app.voice_presets.store.delete(&name) {
                app.voice_presets.error = Some(e);
            }
            app.voice_presets.reload();
            app.refresh_vocal_choices();
        }
    }
    Task::none()
}

/// Save or rename under the prompt's name. A built-in's name is refused;
/// an existing user preset's name is replaced (the button reads
/// "Overwrite" first).
fn confirm(app: &mut Resonance) {
    let Some(prompt) = app.voice_presets.prompt.clone() else {
        return;
    };
    let name = prompt.name.trim().to_owned();
    if let Err(e) = check_name(&name) {
        app.voice_presets.error = Some(e);
        return;
    }
    let result = match &prompt.mode {
        PromptMode::Save => {
            let Some(params) = lane_params(app, prompt.definition_id, prompt.track_id) else {
                app.voice_presets.prompt = None;
                return;
            };
            let preset = voice_presets::capture(&name, String::new(), Vec::new(), &params);
            app.voice_presets.store.save(&preset).map(|_| ())
        }
        PromptMode::Rename(from) => app.voice_presets.store.rename(from, &name),
    };
    match result {
        Ok(()) => {
            app.voice_presets.prompt = None;
            app.voice_presets.error = None;
        }
        Err(e) => app.voice_presets.error = Some(e),
    }
    app.voice_presets.reload();
    app.refresh_vocal_choices();
}

/// Why `name` cannot name a user preset: empty, or a built-in's.
pub(crate) fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("name the voice preset".to_owned());
    }
    if builtin_presets()
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(name.trim()))
    {
        return Err(format!(
            "{name:?} is a built-in voice preset; pick another name"
        ));
    }
    Ok(())
}

/// The lane's current settings.
pub(crate) fn lane_params(
    app: &Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Option<resonance_music_theory::VocalParams> {
    app.compose
        .find_definition(definition_id)
        .and_then(|d| d.lane_generators.get(&track_id))
        .and_then(|c| match &c.kind {
            crate::compose::LaneGeneratorKind::Vocal(p) => Some(p.clone()),
            _ => None,
        })
}
