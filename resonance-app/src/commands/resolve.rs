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
            PlayheadToStart | PlayheadToEnd | PlayheadToLoopStart | PlayheadToLoopEnd
            | NudgeBackBar | NudgeForwardBar | NudgeBackBeat | NudgeForwardBeat
            | PrevSectionStart | NextSectionStart | PrevMarker | NextMarker | GoToBar
            | TransportPlayFromLoopStart | TransportSkipBack | TransportSkipForward
                if r.transport.is_recording() =>
            {
                Available::No("Recording")
            }
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
            SplitClipAtPlayhead if split_target(r).is_none() => {
                Available::No("Select a clip under the playhead")
            }
            DuplicateSelection if duplicate_target(r).is_none() => {
                Available::No("Select a section with room after it")
            }
            LoopSelection if selection_range(r).is_none() => {
                Available::No("Select a clip, section or marker region first")
            }
            QuantizeSelectedNotes | SelectAllNotes if r.ui.interaction.editing_midi_clip.is_none() => {
                Available::No("Open a MIDI clip first")
            }
            SelectNotesInView => Available::No("Press it in the MIDI editor"),
            DeleteSelectedNotes if !editor_is(r, false) || editor_selection(r).is_empty() => {
                Available::No("Select notes in the MIDI editor first")
            }
            VocalDeleteNote | VocalToggleSlur if vocal_note(r).is_none() => {
                Available::No("Select a note in the vocal roll first")
            }
            TimelineDeleteSelection if timeline_delete(r).is_none() => {
                Available::No("Select a clip or global event first")
            }
            ExpandedZoomIn | ExpandedZoomOut | ComposeCollapseTrack
                if r.compose.expanded_track_id.is_none() =>
            {
                Available::No("Expand a track in Compose first")
            }
            DeleteChordAtPlayhead | ToggleChordPinAtPlayhead if chord_at_playhead(r).is_none() => {
                Available::No("No chord at the playhead")
            }
            ToggleMuteSelected | ToggleSoloSelected | ToggleArmSelected
                if r.ui.interaction.selected_tracks.is_empty() =>
            {
                Available::No("Select a track first")
            }
            ToggleArmSelected
                if !r
                    .ui
                    .interaction
                    .selected_tracks
                    .iter()
                    .any(|&id| !r.freeze.status(id).is_frozen()) =>
            {
                Available::No("The selected tracks are frozen")
            }
            DeleteSelectedTrack if r.ui.interaction.selected_track.is_none() => {
                Available::No("Select a track first")
            }
            NewProject if r.io.has_active_project && r.session.dirty => {
                Available::No("Save first: the project has unsaved changes")
            }
            NewProject | OpenProject if r.io.loading || r.io.saving || r.io.save_state.is_some() => {
                Available::No("A project load or save is in progress")
            }
            ShowMissingPlugins if !r.has_missing_plugins() => {
                Available::No("No plugins are missing")
            }
            PreviousPluginPreset | NextPluginPreset | BrowsePluginPresets
                if !r.selected_plugin_available() =>
            {
                Available::No("Select a plugin first")
            }
            CommandPalette | GoToBar
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
            LoopSelection => {
                let (loop_in, loop_out) = selection_range(r)?;
                Message::Transport(TransportMessage::SetLoopRange {
                    loop_in,
                    loop_out,
                    enabled: Some(true),
                })
            }
            // Needs a fresh clip id; `build` allocates it.
            SplitClipAtPlayhead => match split_target(r)? {
                SplitTarget::Take(group_id) => {
                    Message::Take(TakeMessage::SplitCompAtPlayhead { group_id })
                }
                SplitTarget::Clip(_) => return None,
            },
            DuplicateSelection => {
                let (definition_id, start_bar) = duplicate_target(r)?;
                Message::Compose(crate::compose::ComposeMessage::PlaceSection {
                    definition_id,
                    start_bar,
                })
            }
            QuantizeSelectedNotes => {
                let q = &r.midi_quantize;
                Message::MidiEditor(MidiEditorMessage::Quantize {
                    grid: q.grid.division(),
                    strength: q.strength,
                    swing: q.swing,
                    mode: q.mode,
                    quantize_ends: q.quantize_ends,
                    iterative: q.iterative,
                })
            }
            TimelineDeleteSelection => timeline_delete(r)?,
            DeleteSelectedNotes => {
                let clip_id = r.ui.interaction.editing_midi_clip.as_ref()?.clip_id;
                Message::MidiEditor(MidiEditorMessage::RemoveSelectedNotes { clip_id })
            }
            SelectAllNotes => Message::MidiEditor(MidiEditorMessage::SelectAllNotes),
            SelectNotesInView => return None,
            VocalDeleteNote => {
                let (clip_id, note_index) = vocal_note(r)?;
                Message::MidiEditor(MidiEditorMessage::RemoveNote { clip_id, note_index })
            }
            VocalToggleSlur => {
                let (clip_id, note_index) = vocal_note(r)?;
                Message::MidiEditor(MidiEditorMessage::ToggleSlur { clip_id, note_index })
            }
            ExpandedZoomIn => Message::Compose(crate::compose::ComposeMessage::ExpandedZoomY(2.0)),
            ExpandedZoomOut => {
                Message::Compose(crate::compose::ComposeMessage::ExpandedZoomY(-2.0))
            }
            ToggleBrowser => Message::Browser(BrowserMessage::ToggleVisible),
            ToggleReferencePanel => Message::Ui(UiMessage::ToggleReferencePanel),
            ToggleMarkersOverview => Message::Ui(UiMessage::ToggleMarkersOverview),
            AddChordAtPlayhead => Message::ChordTrack(ChordTrackMessage::AddAtPlayhead),
            DeleteChordAtPlayhead => {
                Message::ChordTrack(ChordTrackMessage::Delete { id: chord_at_playhead(r)? })
            }
            ToggleChordPinAtPlayhead => {
                Message::ChordTrack(ChordTrackMessage::TogglePin { id: chord_at_playhead(r)? })
            }
            // Needs a fresh track id; `build` allocates it.
            AddDrumTrack => return None,
            ToggleMuteSelected => Message::Track(TrackMessage::ToggleMuteSelected),
            ToggleSoloSelected => Message::Track(TrackMessage::ToggleSoloSelected),
            ToggleArmSelected => Message::Track(TrackMessage::ToggleArmSelected),
            DeleteSelectedTrack => {
                Message::Track(TrackMessage::RequestRemoveTrack(r.ui.interaction.selected_track?))
            }
            RescanPlugins => Message::Plugin(PluginMessage::RescanPlugins),
            ShowMissingPlugins => Message::Ui(UiMessage::ShowMissingPlugins),
            PreviousPluginPreset | NextPluginPreset => {
                Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::Step {
                    instance_id: r.ui.mixer.plugin_window_id()?,
                    delta: if self == NextPluginPreset { 1 } else { -1 },
                }))
            }
            BrowsePluginPresets => Message::Plugin(PluginMessage::PresetUi(
                PresetUiMessage::OpenBrowser(r.ui.mixer.plugin_window_id()?),
            )),
            ExportStemsMidi => Message::Export(ExportMessage::Open),
            ImportMidi => Message::Import(ImportMessage::Open),
            ImportAudio => Message::Pool(PoolMessage::PickFiles),
            SaveAsTemplate => Message::ProjectIo(ProjectIoMessage::SaveAsTemplate {
                name: r
                    .io
                    .project_path
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "Untitled".to_string()),
                description: String::new(),
                include_markers_and_tempo: true,
                include_master_chain: true,
            }),
            RelinkMissingMedia => Message::Relink(RelinkMessage::ShowModal),

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
            GoToBar => Message::Ui(UiMessage::OpenPalette(crate::palette::PaletteMode::GoToBar)),
            // With nothing open the startup flow picks a folder; with a
            // project open it is a fresh untitled one, as `project.new` does.
            NewProject if !r.io.has_active_project => Message::Ui(UiMessage::StartNewProject),
            NewProject => Message::Ui(UiMessage::NewEmptyProject),
            OpenProject => Message::ProjectIo(ProjectIoMessage::OpenProject),
            SaveProject => Message::ProjectIo(ProjectIoMessage::SaveProject),
            SaveProjectAs => Message::ProjectIo(ProjectIoMessage::SaveProjectAs),
            BounceToWav => Message::ProjectIo(ProjectIoMessage::BounceToWav),
            ExportChordSheet => Message::ProjectIo(ProjectIoMessage::ExportChordSheet),
            OpenSettings => Message::Ui(UiMessage::OpenSettings),
        };
        Some(message)
    }

    /// [`to_message`](Self::to_message) for a command about to run: the
    /// commands whose message names an entity the command creates get a
    /// freshly allocated id here, the way the control API allocates one
    /// before it dispatches.
    pub(crate) fn build(self, r: &mut Resonance) -> Option<Message> {
        match self {
            CommandId::SplitClipAtPlayhead => match split_target(r)? {
                SplitTarget::Clip(clip_id) => {
                    let at_sample = r.transport.playhead;
                    let new_clip_id = r.media.ids.clips.allocate();
                    Some(Message::Clip(ClipMessage::SplitClipAt {
                        clip_id,
                        new_clip_id,
                        at_sample,
                    }))
                }
                SplitTarget::Take(_) => self.to_message(r),
            },
            CommandId::AddDrumTrack => {
                let id = r.allocate_track_id();
                Some(Message::Track(TrackMessage::AddControlTrack {
                    id,
                    kind: crate::state::ControlTrackKind::Drums,
                    name: None,
                }))
            }
            _ => self.to_message(r),
        }
    }
}

/// What Split at Playhead cuts.
enum SplitTarget {
    Clip(resonance_audio::types::ClipId),
    Take(resonance_common::TakeGroupId),
}

/// The selected audio clip when the playhead is strictly inside it, else a
/// take lane on the selected track whose slot holds the playhead.
fn split_target(r: &Resonance) -> Option<SplitTarget> {
    let pos = r.transport.playhead;
    if let Some(id) = r.ui.interaction.selected_clip {
        let clip = r.clips.iter().find(|c| c.id == id)?;
        if clip.start_sample < pos && pos < clip.start_sample + clip.duration_samples {
            return Some(SplitTarget::Clip(id));
        }
    }
    let track = r.ui.interaction.selected_track?;
    r.take_groups
        .groups
        .iter()
        .find(|g| g.track_id == track && g.slot.start < pos && pos < g.slot.end())
        .map(|g| SplitTarget::Take(g.id))
}

/// The selected section placement's definition and the bar right after it,
/// when that span is free.
fn duplicate_target(r: &Resonance) -> Option<(u64, u32)> {
    let placement = r.compose.selected_placement()?;
    let def = r.compose.find_definition(placement.definition_id)?;
    let start = placement.start_bar.checked_add(def.length_bars)?;
    let taken = crate::compose::invariants::placement_overlaps(
        &r.compose.placements,
        &r.compose.definitions,
        start,
        def.length_bars,
        None,
    );
    (!taken).then_some((def.id, start))
}

/// The selection's span in samples: the selected audio clip, else the
/// selected MIDI clip, else the selected section placement, else the
/// selected marker's region.
pub(crate) fn selection_range(r: &Resonance) -> Option<(u64, u64)> {
    let i = &r.ui.interaction;
    if let Some(c) = i.selected_clip.and_then(|id| r.clips.iter().find(|c| c.id == id)) {
        return Some((c.start_sample, c.start_sample + c.duration_samples));
    }
    if let Some(c) = i.selected_midi_clip.and_then(|id| r.midi_clips.iter().find(|c| c.id == id)) {
        let end = r
            .tempo_map
            .tick_to_abs_sample(c.start_sample, c.duration_ticks, r.sample_rate);
        return Some((c.start_sample, end));
    }
    if let Some(p) = r.compose.selected_placement() {
        let def = r.compose.find_definition(p.definition_id)?;
        return Some((
            r.tempo_map.bar_to_sample(p.start_bar),
            r.tempo_map.bar_to_sample(p.start_bar + def.length_bars),
        ));
    }
    let marker = i.selected_marker_id.and_then(|id| r.markers.get(id))?;
    let end = marker.end_sample.filter(|&e| e > marker.start_sample)?;
    Some((marker.start_sample, end))
}

/// Whether the open MIDI editor is the vocal roll (`true`) or the piano
/// roll (`false`); `false` for either when no editor is open.
fn editor_is(r: &Resonance, vocal: bool) -> bool {
    r.ui.interaction.editing_midi_clip.as_ref().is_some_and(|e| {
        let is_vocal = matches!(
            r.classify_editor_variant(e.track_id),
            crate::view::editor_panel::EditorVariant::Vocal
        );
        is_vocal == vocal
    })
}

fn editor_selection(r: &Resonance) -> Vec<usize> {
    r.ui.interaction
        .editing_midi_clip
        .as_ref()
        .map(|e| e.selected_notes.iter().copied().collect())
        .unwrap_or_default()
}

/// The vocal roll's selected note.
fn vocal_note(r: &Resonance) -> Option<(resonance_audio::types::ClipId, usize)> {
    if !editor_is(r, true) {
        return None;
    }
    let e = r.ui.interaction.editing_midi_clip.as_ref()?;
    Some((e.clip_id, e.primary_selected()?))
}

/// What Delete on the timeline removes, in the canvas's own order (after
/// the canvas-local automation breakpoint, which only the canvas knows).
fn timeline_delete(r: &Resonance) -> Option<Message> {
    let i = &r.ui.interaction;
    if i.selected_global_event.is_some() {
        return Some(Message::GlobalTrack(GlobalTrackMessage::DeleteSelectedEvent));
    }
    if let Some(id) = i.selected_midi_clip {
        return Some(Message::MidiClip(MidiClipMessage::DeleteMidiClip(id)));
    }
    i.selected_clip
        .map(|id| Message::Clip(ClipMessage::DeleteClip(id)))
}

/// The chord-track region under the playhead.
fn chord_at_playhead(r: &Resonance) -> Option<u64> {
    let pos = r.transport.playhead;
    r.chord_track
        .regions
        .iter()
        .find(|rg| rg.start_sample <= pos && pos < rg.end_sample)
        .map(|rg| rg.id)
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
