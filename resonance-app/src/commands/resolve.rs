//! State-aware half of the registry: whether a command can run right now,
//! and the [`Message`] it dispatches (command-palette.md §3.1, §3.4).
//!
//! [`CommandId::availability`] is the authority on "can this run". It feeds
//! the palette's dimmed rows and the shortcut path, which drops an
//! unavailable command without dispatching anything. Its reasons are never
//! stricter than the reducer: a reducer that would no-op anyway may still
//! report `Yes`.
//!
//! [`CommandId::to_message`] builds the message, resolving a selection
//! target where the command acts on one. It returns `None` only when there
//! is no target to name; a single-target command emits the existing
//! id-carrying message and a multi-target command emits one `…Selected`
//! reducer message, so every command lands as exactly one undo entry.

use super::{Available, CommandId};
use crate::message::*;
use crate::state::ViewMode;
use crate::Resonance;

impl CommandId {
    /// Whether this command can run against `r`, with the reason when not.
    pub fn availability(self, r: &Resonance) -> Available {
        use CommandId::*;
        match self {
            Undo if !r.session.undo.can_undo() => Available::No("Nothing to undo"),
            Redo if !r.session.undo.can_redo() => Available::No("Nothing to redo"),
            TransportRecord if !r.registry.tracks.iter().any(|t| t.record_armed) => {
                Available::No("Arm a track to record")
            }
            TransportPlayFromLoopStart | PlayheadToLoopStart | PlayheadToLoopEnd
                if r.transport.loop_in == r.transport.loop_out =>
            {
                Available::No("Set a loop range first")
            }
            LoopSectionAtPlayhead if section_at_playhead(r).is_none() => {
                Available::No("No section at the playhead")
            }
            PrevSectionStart | NextSectionStart if r.compose.placements.is_empty() => {
                Available::No("The song has no sections")
            }
            OpenSelectedMidiClip if r.ui.interaction.selected_midi_clip.is_none() => {
                Available::No("Select a MIDI clip first")
            }
            CloseMidiEditor if r.ui.interaction.editing_midi_clip.is_none() => {
                Available::No("No MIDI editor is open")
            }
            NextMarker | PrevMarker if r.markers.is_empty() => {
                Available::No("The project has no markers")
            }
            ExitPerformanceMode if r.ui.view_mode != ViewMode::Performance => {
                Available::No("Not in Performance mode")
            }
            GroupSelectedTracks if r.ui.interaction.selected_tracks.len() < 2 => {
                Available::No("Select two or more tracks")
            }
            FreezeSelectedTracks
                if crate::update::freeze::selected_freezable_tracks(r).is_empty() =>
            {
                Available::No("Select a freezable track first")
            }
            CommandPalette
                if r.root_overlay().is_some_and(|o| !o.allows_palette() && !o.is_palette()) =>
            {
                Available::No("Not available here")
            }
            _ => Available::Yes,
        }
    }

    /// Build the [`Message`] this command dispatches, or `None` when it
    /// needs a target that `r` doesn't have. A fresh `Message` is built per
    /// call, so commands never need `Message: Clone`.
    pub fn to_message(self, r: &Resonance) -> Option<Message> {
        use crate::update::transport_nav::{LoopEdge, SeekTarget};
        use CommandId::*;
        let seek = |target| Message::Transport(TransportMessage::SeekTo(target));
        let message = match self {
            TransportTogglePlay => Message::Transport(TransportMessage::TogglePlay),
            TransportPlayPause if r.transport.playing => {
                Message::Transport(TransportMessage::Pause)
            }
            TransportPlayPause => Message::Transport(TransportMessage::Play),
            TransportPlayFromLoopStart => Message::Transport(TransportMessage::PlayFromLoopStart),
            TransportPlay => Message::Transport(TransportMessage::Play),
            TransportStop => Message::Transport(TransportMessage::Stop),
            TransportRecord => Message::Transport(TransportMessage::Record),
            TransportSkipBack => Message::Transport(TransportMessage::SkipBack),
            TransportSkipForward => Message::Transport(TransportMessage::SkipForward),
            TransportToggleLoop => Message::Transport(TransportMessage::ToggleLoop),
            TransportToggleMetronome => Message::Transport(TransportMessage::ToggleMetronome),
            TransportCycleTimeSignature => {
                Message::Transport(TransportMessage::CycleTimeSignature)
            }
            PlayheadToStart => seek(SeekTarget::ProjectStart),
            PlayheadToEnd => seek(SeekTarget::ProjectEnd),
            PlayheadToLoopStart => seek(SeekTarget::LoopStart),
            PlayheadToLoopEnd => seek(SeekTarget::LoopEnd),
            SetLoopStartAtPlayhead => {
                Message::Transport(TransportMessage::SetLoopPoint { edge: LoopEdge::Start })
            }
            SetLoopEndAtPlayhead => {
                Message::Transport(TransportMessage::SetLoopPoint { edge: LoopEdge::End })
            }
            LoopSectionAtPlayhead => {
                let (loop_in, loop_out) = section_at_playhead(r)?;
                Message::Transport(TransportMessage::SetLoopRange {
                    loop_in,
                    loop_out,
                    enabled: Some(true),
                })
            }
            NudgeBackBar => seek(SeekTarget::NudgeBars(-1)),
            NudgeForwardBar => seek(SeekTarget::NudgeBars(1)),
            NudgeBackBeat => seek(SeekTarget::NudgeBeats(-1)),
            NudgeForwardBeat => seek(SeekTarget::NudgeBeats(1)),
            NextMarker => Message::Marker(MarkerMessage::JumpToNext),
            PrevMarker => Message::Marker(MarkerMessage::JumpToPrev),
            PrevSectionStart => seek(SeekTarget::PrevSection),
            NextSectionStart => seek(SeekTarget::NextSection),
            AddMarkerAtPlayhead => Message::Marker(MarkerMessage::AddAtPlayhead),
            ToggleFollowPlayhead => Message::Ui(UiMessage::ToggleFollowPlayhead),

            Undo => Message::Undo,
            Redo => Message::Redo,
            OpenSelectedMidiClip => {
                Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip)
            }
            CloseMidiEditor => Message::MidiEditor(MidiEditorMessage::CloseMidiEditor),

            ViewArrange => Message::Ui(UiMessage::SwitchView(ViewMode::Arrange)),
            ViewMixer => Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)),
            ViewCompose => Message::Ui(UiMessage::SwitchView(ViewMode::Compose)),
            TogglePerformanceMode => Message::Ui(UiMessage::TogglePerformanceMode),
            ExitPerformanceMode => Message::Ui(UiMessage::ExitPerformanceMode),
            ZoomIn => Message::Viewport(ViewportMessage::ZoomIn),
            ZoomOut => Message::Viewport(ViewportMessage::ZoomOut),
            ToggleGlobalTracks => Message::Ui(UiMessage::ToggleGlobalTracks),

            ComposeCreateSection => {
                Message::Compose(crate::compose::ComposeMessage::OpenCreateSectionDialog)
            }
            ComposeCollapseTrack => {
                Message::Compose(crate::compose::ComposeMessage::CollapseTrack)
            }
            ComposeClearChordSelection => {
                Message::Compose(crate::compose::ComposeMessage::ClearChordSelection)
            }

            AddAudioTrack => Message::Track(TrackMessage::AddTrack),
            AddInstrumentTrack => Message::Track(TrackMessage::AddInstrumentTrack),
            AddVocalTrack => Message::Track(TrackMessage::AddVocalTrack),
            AddBus => Message::Bus(BusMessage::AddBus),
            OpenAddTrackMenu => Message::Ui(UiMessage::OpenAddTrackMenu),
            ToggleMasterFxBypass => Message::Master(MasterMessage::ToggleMasterFxBypass),
            GroupSelectedTracks => Message::Group(GroupMessage::CreateGroupFromSelection),
            FreezeSelectedTracks => Message::Freeze(FreezeMessage::FreezeSelectedTracks),
            FreezeAllTracks => Message::Freeze(FreezeMessage::FreezeAllTracks),

            CommandPalette => {
                Message::Ui(UiMessage::OpenPalette(crate::palette::PaletteMode::Commands))
            }
            NewProject => Message::Ui(UiMessage::StartNewProject),
            OpenProject => Message::ProjectIo(ProjectIoMessage::OpenProject),
            SaveProject => Message::ProjectIo(ProjectIoMessage::SaveProject),
            SaveProjectAs => Message::ProjectIo(ProjectIoMessage::SaveProjectAs),
            BounceToWav => Message::ProjectIo(ProjectIoMessage::BounceToWav),
            ExportChordSheet => Message::ProjectIo(ProjectIoMessage::ExportChordSheet),
            OpenSettings => Message::Ui(UiMessage::OpenSettings),
        };
        Some(message)
    }
}

/// The `[start, end)` samples of the section placement under the playhead.
pub(crate) fn section_at_playhead(r: &Resonance) -> Option<(u64, u64)> {
    let pos = r.transport.playhead;
    r.compose.placements.iter().find_map(|p| {
        let def = r.compose.find_definition(p.definition_id)?;
        let start = r.tempo_map.bar_to_sample(p.start_bar);
        let end = r.tempo_map.bar_to_sample(p.start_bar + def.length_bars);
        (start <= pos && pos < end).then_some((start, end))
    })
}
