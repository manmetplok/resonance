//! Top-level dispatcher for `ComposeMessage`. Each arm hands the work off
//! to the appropriate submodule:
//!
//! - [`section`] — section + placement CRUD and the new/edit dialog forms.
//! - [`chord`] — chord-state ops (add / edit / move / resize / delete).
//! - [`chord_inspector`] — chord-lane inspector messages (Markov knobs +
//!   shared motif knobs).
//! - [`lane_inspector`] — per-track lane inspector messages (generator
//!   choice, Bass/Melody/Pad/Drum params, lane Regenerate).
//! - [`regenerate`] — derive notes for one lane and the cascade helpers
//!   that fan chord changes / motif-seed bumps to every dependent lane.
//! - [`expand`] — expanded piano-roll viewport (open track, scroll, zoom).
//! - [`expression`] — vocal Expression-dock curve edits (breakpoints, pen/
//!   snap tool state, depth/smoothing, reset-to-generated).

use iced::Task;

use crate::compose::ComposeMessage;
use crate::message::Message;

mod chord;
mod chord_inspector;
pub(crate) mod drum_groups;
mod expand;
mod expression;
mod lane_inspector;
pub(crate) mod regenerate;
mod section;
mod vocal_audio_install;
pub mod vocal_audio_io;
mod vocal_control;
mod vocal_lyrics;
mod vocal_midi_install;
pub(crate) mod vocal_render;
pub mod vocal_render_plan;

/// Pure drum-note builder — exposed so integration tests in `tests/` can
/// assert the materialized `MidiNote` sequence for an arrangement's
/// resolved spans without booting a whole `Resonance`.
pub use drum_groups::build_drum_notes;

pub(crate) use section::next_default_color;

/// When the freshly installed clip becomes visible in `r.midi_clips`.
///
/// The engine always echoes `LoadMidiClipDirect` back as
/// `MidiClipCreated`, and `engine_events::midi::clip_created` mirrors it
/// into `r.midi_clips` — so for a GUI edit, which only has to be right by
/// the next repaint, [`Self::OnEcho`] is the whole story.
///
/// A control-endpoint generate cannot wait for that round trip: it
/// returns the clip id in its reply and the client's *very next* request
/// may address it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipVisibility {
    /// Mirror into `r.midi_clips` now, before the reply is sent.
    Immediate,
    /// Leave it to the engine's `MidiClipCreated` echo.
    OnEcho,
}

/// One derived MIDI clip about to be installed on the timeline.
pub(crate) struct DerivedMidiClip<'a> {
    pub definition_id: u64,
    pub placement_id: u64,
    pub track_id: resonance_audio::types::TrackId,
    pub start_sample: u64,
    pub duration_ticks: u64,
    pub notes: Vec<resonance_audio::types::MidiNote>,
    pub name: &'a str,
    pub visibility: ClipVisibility,
}

/// Replace the derived clip on `(definition, placement, track)`: tear the
/// previous one down (engine + app mirror), allocate a fresh derived id,
/// send `LoadMidiClipDirect`, and register the new clip in
/// `compose.derived_clips`. Returns the new clip id.
///
/// **Why [`ClipVisibility::Immediate`] exists (ba todo #1162):** the
/// engine's `MidiClipCreated` echo is asynchronous, but `generate.part` /
/// `generate.drums` return the clip id in their reply *synchronously*.
/// Without a mirror, a client that did `id = generate.drums(...)` then
/// `song.notes(id)` on the very next request got `no MIDI clip with id
/// ...` for roughly the length of one engine round trip (~0.3 s
/// measured), and only a sleep made it work. `notes.create_clip`
/// (`update/midi_clip.rs`) and the vocal installer already did this; the
/// two generators did not. The echo's `clip_created` handler skips ids
/// already present, so the round trip stays an idempotent no-op once it
/// lands.
///
/// Removing the torn-down clip from `r.midi_clips` is unconditional: a
/// stale mirror entry would otherwise keep a deleted clip visible to
/// `song.*` (and get re-serialized into the next save) forever, whichever
/// way the replacement became visible.
pub(crate) fn install_derived_midi_clip(
    r: &mut crate::Resonance,
    clip: DerivedMidiClip<'_>,
) -> resonance_audio::types::ClipId {
    use resonance_audio::types::AudioCommand;

    let key = (clip.definition_id, clip.placement_id, clip.track_id);
    let reused = r.compose.derived_clips.remove(&key);
    if let Some(old_id) = reused {
        let _ = r
            .engine
            .send(AudioCommand::DeleteMidiClip { clip_id: old_id });
        r.midi_clips.retain(|c| c.id != old_id);
    }

    // Regenerating the SAME (section, placement, track) slot keeps the
    // slot's clip id (ba doc #275 P1.7). The clip is still torn down and
    // rebuilt — its notes, length and name are all replaced — but a
    // client that cached the id from `generate.part` or `song.tracks`
    // can still address it afterwards. Handing out a fresh id every time
    // silently invalidated every cached id whenever anything re-derived
    // the lane, including a track rename (the generated name embeds the
    // track's name, so renaming re-derives).
    let clip_id = reused.unwrap_or_else(|| r.compose.fresh_derived_clip_id());
    let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
        clip_id,
        track_id: clip.track_id,
        start_sample: clip.start_sample,
        duration_ticks: clip.duration_ticks,
        notes: clip.notes.clone(),
        name: clip.name.to_owned(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    r.compose.derived_clips.insert(key, clip_id);
    if clip.visibility == ClipVisibility::Immediate
        && !r.midi_clips.iter().any(|c| c.id == clip_id)
    {
        r.midi_clips.push(crate::state::MidiClipState {
            id: clip_id,
            track_id: clip.track_id,
            start_sample: clip.start_sample,
            duration_ticks: clip.duration_ticks,
            name: clip.name.to_owned(),
            notes: clip.notes,
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        });
    }
    clip_id
}

/// The meter a section is composed in: the signature numerator and the
/// BPM the tempo map has at the section's earliest placement (bar 0 when
/// it is not placed). Compose never reads `transport.time_sig_num` /
/// `transport.bpm` — those follow the playhead during playback, so an
/// edit's clip lengths, chord-fit checks and vocal render tempo would
/// depend on where the song happens to be playing (code review VIEW-13).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SectionMeter {
    /// Beats per bar.
    pub numerator: u8,
    pub bpm: f32,
}

/// [`SectionMeter`] at 0-based `bar`.
pub(crate) fn meter_at_bar(r: &crate::Resonance, bar: u32) -> SectionMeter {
    let sample = r.tempo_map.bar_to_sample(bar);
    SectionMeter {
        numerator: r.tempo_map.numerator_at_bar(bar).max(1),
        bpm: r.tempo_map.bpm_at(sample, r.sample_rate),
    }
}

/// [`SectionMeter`] of a section definition — see the type docs.
pub(crate) fn section_meter(r: &crate::Resonance, definition_id: u64) -> SectionMeter {
    let bar = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| p.start_bar)
        .min()
        .unwrap_or(0);
    meter_at_bar(r, bar)
}

/// Whether `track_id` is a live track. Lane derivation checks this so a
/// generator left pointing at a removed track never loads a clip onto it
/// (the engine accepts any track id) — code review VIEW-12.
pub(crate) fn track_exists(r: &crate::Resonance, track_id: resonance_audio::types::TrackId) -> bool {
    r.registry.tracks.iter().any(|t| t.id == track_id)
}

/// Drop every Compose lane of a removed track (code review VIEW-12):
/// its generator in each section, its derived MIDI clips (engine + app
/// mirror — the engine's `RemoveTrack` only drops audio clips), its
/// installed vocal audio, and the per-lane side tables. The render epoch
/// is bumped rather than removed so a render still in flight for the
/// lane is discarded on completion. Called from the track-removal echo;
/// an undo of the delete restores the generators from the project
/// snapshot.
pub(crate) fn forget_track(r: &mut crate::Resonance, track_id: resonance_audio::types::TrackId) {
    use resonance_audio::types::AudioCommand;

    for def in r.compose.definitions.iter_mut() {
        def.lane_generators.remove(&track_id);
    }
    let midi: Vec<_> = r
        .compose
        .derived_clips
        .iter()
        .filter(|((_, _, t), _)| *t == track_id)
        .map(|(_, id)| *id)
        .collect();
    r.compose.derived_clips.retain(|(_, _, t), _| *t != track_id);
    for clip_id in midi {
        let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id });
        r.midi_clips.retain(|c| c.id != clip_id);
        r.compose.vocal_audio.clip_lyrics.remove(&clip_id);
    }
    let audio = &mut r.compose.vocal_audio;
    let audio_ids: Vec<_> = audio
        .clips
        .iter()
        .filter(|((_, _, t), _)| *t == track_id)
        .map(|(_, (id, _))| *id)
        .collect();
    audio.clips.retain(|(_, _, t), _| *t != track_id);
    for ((_, t), epoch) in audio.render_epoch.iter_mut() {
        if *t == track_id {
            *epoch += 1;
        }
    }
    audio.render_cache.retain(|(_, t), _| *t != track_id);
    for clip_id in audio_ids {
        let _ = r.engine.send(AudioCommand::DeleteClip { clip_id });
        r.clips.retain(|c| c.id != clip_id);
    }
    r.compose.vocal_bulk_lyrics.retain(|(_, t), _| *t != track_id);
    r.compose.expression_curves.retain(|(_, t), _| *t != track_id);
    if r.compose.expanded_track_id == Some(track_id) {
        r.compose.expanded_track_id = None;
    }
    if r.compose.details_track_id() == Some(track_id) {
        r.compose.selected_lane = crate::compose::SelectedLane::Chords;
    }
}

pub fn handle(r: &mut crate::Resonance, msg: ComposeMessage) -> Task<Message> {
    let num = |r: &crate::Resonance, definition_id: u64| section_meter(r, definition_id).numerator;

    match msg {
        ComposeMessage::DrumGroups(m) => return drum_groups::handle(r, m),
        ComposeMessage::Arrangement(m) => return drum_groups::handle_arrangement(r, m),

        // Tiling-ribbon span selection — pure view state, drives the
        // right-rail Entry inspector. No mutation, no undo.
        ComposeMessage::SelectArrangementEntry(index) => {
            r.compose.drumroll.selected_entry_index = index;
        }

        ComposeMessage::CreateMidiClipInSection {
            track_id,
            start_sample,
            length_bars,
        } => section::handle_create_midi_clip(r, track_id, start_sample, length_bars),

        // Create-section dialog
        ComposeMessage::OpenCreateSectionDialog => section::handle_open_create_dialog(r),
        ComposeMessage::CancelCreateSectionDialog => section::handle_cancel_create_dialog(r),
        ComposeMessage::SetNewSectionName(name) => section::handle_set_new_name(r, name),
        ComposeMessage::SetNewSectionLength(input) => section::handle_set_new_length(r, input),
        ComposeMessage::ConfirmCreateSection => section::handle_confirm_create(r),

        // Edit-section dialog
        ComposeMessage::OpenEditSectionDialog { definition_id } => {
            section::handle_open_edit_dialog(r, definition_id)
        }
        ComposeMessage::CancelEditSectionDialog => section::handle_cancel_edit_dialog(r),
        ComposeMessage::SetEditSectionName(name) => section::handle_set_edit_name(r, name),
        ComposeMessage::SetEditSectionLength(input) => section::handle_set_edit_length(r, input),
        ComposeMessage::ConfirmEditSection => return section::handle_confirm_edit(r),
        ComposeMessage::CycleSectionColor { definition_id } => {
            section::handle_cycle_color(r, definition_id)
        }

        // Section CRUD
        ComposeMessage::CreateSection {
            name,
            length_bars,
            color,
            place,
        } => section::handle_create(r, name, length_bars, color, place),
        ComposeMessage::RenameSection {
            definition_id,
            name,
        } => section::handle_rename(r, definition_id, name),
        ComposeMessage::ResizeSection {
            definition_id,
            length_bars,
        } => {
            let time_sig_num = num(r, definition_id);
            return section::handle_resize(r, definition_id, length_bars, time_sig_num);
        }
        ComposeMessage::SetSectionScale {
            definition_id,
            scale,
        } => section::handle_set_scale(r, definition_id, scale),
        ComposeMessage::DeleteSectionDefinition { definition_id } => {
            section::handle_delete_definition(r, definition_id)
        }
        ComposeMessage::DeleteSectionWithPlacements { definition_id } => {
            section::handle_delete_with_placements(r, definition_id)
        }
        ComposeMessage::GenerateSectionPart {
            definition_id,
            track_id,
            config,
        } => return control_generate_part(r, definition_id, track_id, *config).1,
        ComposeMessage::SetLaneGenerator {
            definition_id,
            track_id,
            config,
        } => control_set_lane_generator(r, definition_id, track_id, config.map(|c| *c)),
        ComposeMessage::GenerateSectionDrums {
            definition_id,
            pattern_id,
            builtin,
            density,
            seed,
        } => {
            let _ = drum_groups::control_generate_drums(
                r,
                definition_id,
                pattern_id,
                builtin,
                density,
                seed,
            );
        }
        ComposeMessage::ControlGenerateVocal {
            definition_id,
            track_id,
            seed,
            lyrics,
        } => return control_generate_vocal(r, definition_id, track_id, seed, lyrics),
        ComposeMessage::ControlSetVocalLyrics {
            definition_id,
            track_id,
            text,
        } => vocal_control::control_set_lyrics(r, definition_id, track_id, &text),
        ComposeMessage::ControlSetVocalLine {
            definition_id,
            track_id,
            line_index,
            text,
        } => {
            let _ =
                vocal_control::control_set_line(r, definition_id, track_id, line_index, &text);
        }
        ComposeMessage::ControlSetPronunciation { word, phonemes } => {
            control_set_pronunciation(r, word, phonemes);
        }
        ComposeMessage::ControlClearPronunciation { word } => {
            control_clear_pronunciation(r, &word);
        }
        ComposeMessage::ControlRenderVocal {
            definition_id,
            track_id,
            voicebank,
        } => return vocal_control::control_render(r, definition_id, track_id, voicebank),

        // Placement CRUD
        ComposeMessage::PlaceSection {
            definition_id,
            start_bar,
        } => section::handle_place(r, definition_id, start_bar),
        ComposeMessage::DeleteSectionPlacement { placement_id } => {
            section::handle_delete_placement(r, placement_id)
        }
        ComposeMessage::SelectSectionPlacement { placement_id } => {
            section::handle_select_placement(r, placement_id)
        }

        // Chord lane selection
        ComposeMessage::SelectChord { chord_id } => {
            r.compose.selected_chord_id = Some(chord_id);
            r.compose.selected_lane = crate::compose::SelectedLane::Chords;
        }
        ComposeMessage::ClearChordSelection => {
            r.compose.selected_chord_id = None;
        }
        ComposeMessage::SelectLane(lane) => {
            r.compose.selected_lane = lane;
            ensure_vocal_bulk_lyrics_for_selection(r);
        }

        // Collapse toggles — runtime UI state, never persisted.
        ComposeMessage::ToggleRailPanel(key) => {
            let set = &mut r.compose.collapsed_rail_panels;
            if !set.remove(&key) {
                set.insert(key);
            }
        }
        ComposeMessage::ToggleWorkspaceGroup(group) => match group {
            crate::compose::WorkspaceGroup::Section => {
                r.compose.section_lanes_collapsed = !r.compose.section_lanes_collapsed;
            }
            crate::compose::WorkspaceGroup::Tracks => {
                r.compose.track_lanes_collapsed = !r.compose.track_lanes_collapsed;
            }
        },

        // Expanded piano-roll viewport
        ComposeMessage::ExpandTrack { track_id } => expand::handle_expand(r, track_id),
        ComposeMessage::CollapseTrack => expand::handle_collapse(r),
        ComposeMessage::ExpandedScrollX(delta) => expand::handle_scroll_x(r, delta),
        ComposeMessage::ExpandedScrollY(delta) => expand::handle_scroll_y(r, delta),
        ComposeMessage::ExpandedZoomY(delta) => expand::handle_zoom_y(r, delta),

        // Chord ops
        ComposeMessage::AddChord {
            definition_id,
            start_beat,
            duration_beats,
            root,
            quality,
        } => chord::handle_add(
            r,
            definition_id,
            start_beat,
            duration_beats,
            root,
            quality,
            num(r, definition_id),
        ),
        ComposeMessage::EditChord {
            definition_id,
            chord_id,
            chord,
        } => chord::handle_edit(r, definition_id, chord_id, chord),
        ComposeMessage::MoveChord {
            definition_id,
            chord_id,
            start_beat,
        } => {
            let time_sig_num = num(r, definition_id);
            chord::handle_move(r, definition_id, chord_id, start_beat, time_sig_num)
        }
        ComposeMessage::ResizeChord {
            definition_id,
            chord_id,
            duration_beats,
        } => {
            let time_sig_num = num(r, definition_id);
            chord::handle_resize(r, definition_id, chord_id, duration_beats, time_sig_num)
        }
        ComposeMessage::ReplaceSectionChords {
            definition_id,
            chords,
        } => {
            let time_sig_num = num(r, definition_id);
            chord::handle_replace(r, definition_id, chords, time_sig_num)
        }
        ComposeMessage::DeleteChord {
            definition_id,
            chord_id,
        } => chord::handle_delete(r, definition_id, chord_id),

        // Inspectors
        ComposeMessage::ChordInspector { definition_id, msg } => {
            chord_inspector::handle(r, definition_id, msg)
        }
        ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg,
        } => return lane_inspector::handle(r, definition_id, track_id, msg),

        ComposeMessage::Expression {
            definition_id,
            track_id,
            msg,
        } => return expression::handle(r, definition_id, track_id, msg),

        // Vocal audio render completion (dispatched from the background
        // SVS task that `lane_inspector::handle` queued).
        ComposeMessage::VocalAudioReady(data) => {
            // Install FIRST: the epoch check inside decides whether this
            // render is current or was superseded (a later `vocal.render`
            // or GUI regeneration re-queued the lane while it was in
            // flight). Only an accepted install may tick the lane off the
            // control-initiated render jobs (doc #265, todo #1156) —
            // ticking before the check let a superseded render resolve a
            // later job `done` while the audio that job asked for was
            // then discarded as stale, so a client that waited on the job
            // read old/no audio back. A track-level `vocal.render` covers
            // every lane on the track, so the job only resolves once the
            // last of them lands; no-op when no control job covers the
            // lane (a GUI-driven render).
            let (definition_id, track_id) = (data.definition_id, data.track_id);
            let render_epoch = data.render_epoch;
            // Sung at a tempo the section no longer has (its placement
            // moved into another tempo region, or the tempo was edited,
            // while the render ran): re-render instead of installing
            // audio that drifts against the grid (FU-V2d).
            if let Some(task) = rerender_on_tempo_change(r, &data) {
                vocal_audio_io::unlink_if_exists(&data.wav_path);
                vocal_audio_install::settle_render_event(
                    r,
                    definition_id,
                    track_id,
                    render_epoch,
                    false,
                );
                return task;
            }
            let accepted = vocal_audio_install::handle_vocal_audio_ready(r, *data);
            if accepted {
                r.mark_edited_without_history();
                r.control
                    .jobs
                    .complete_vocal_lane(definition_id, track_id, r.revision());
            }
            // A discarded render with no successor in flight fails the
            // jobs waiting on it instead of stranding them (UPD-08).
            vocal_audio_install::settle_render_event(
                r,
                definition_id,
                track_id,
                render_epoch,
                accepted,
            );
        }
        ComposeMessage::VocalAudioFailed {
            definition_id,
            track_id,
            render_epoch,
            error,
        } => {
            // Same epoch gate as the success path: a failure from a
            // superseded render is moot — a newer render for the lane is
            // already in flight, and *its* outcome is what the lane (and
            // any job waiting on it) will get. Only a current-epoch
            // failure means the lane's audio is genuinely not coming, so
            // only then does it fail the jobs still waiting on this lane
            // — and only those: before the message carried the lane, a
            // GUI regeneration of lane B erroring killed a control job
            // that covered only lane A.
            let current = render_epoch
                == vocal_audio_install::current_render_epoch(r, definition_id, track_id);
            if current {
                r.control
                    .jobs
                    .fail_vocal_lane(definition_id, track_id, error.clone());
                r.compose.last_error = Some(error);
            }
            vocal_audio_install::settle_render_event(
                r,
                definition_id,
                track_id,
                render_epoch,
                current,
            );
        }
        ComposeMessage::VocalAudioUnavailable {
            definition_id,
            track_id,
            render_epoch,
        } => {
            // No voicebank: the lane plays its MIDI, which is the silent
            // fallback, so no `last_error` banner. A job waiting on the
            // lane asked for audio that is not coming, so it fails with
            // the reason; a stale epoch settles like any stale event.
            let current = render_epoch
                == vocal_audio_install::current_render_epoch(r, definition_id, track_id);
            if current {
                r.control.jobs.fail_vocal_lane(
                    definition_id,
                    track_id,
                    "no SVS voicebank is installed, so the vocal lane plays its MIDI \
                     notes only; install a voicebank to render sung audio",
                );
            }
            vocal_audio_install::settle_render_event(
                r,
                definition_id,
                track_id,
                render_epoch,
                current,
            );
        }
    }
    Task::none()
}

/// FU-V2d: when a *current* render finished at a tempo the section no
/// longer has, queue a re-render at the section's tempo now and return
/// its task. `None` — install as usual — when the render is stale anyway
/// (the epoch check discards it), the tempo still matches, or no
/// re-render could be queued (then the old-tempo audio is still better
/// than none, and the error the re-render reported is left showing).
fn rerender_on_tempo_change(
    r: &mut crate::Resonance,
    data: &crate::compose::messages::VocalAudioReadyData,
) -> Option<Task<Message>> {
    let (definition_id, track_id) = (data.definition_id, data.track_id);
    if data.render_epoch
        != vocal_audio_install::current_render_epoch(r, definition_id, track_id)
        || !r.compose.placements.iter().any(|p| p.definition_id == definition_id)
    {
        return None;
    }
    let now = section_meter(r, definition_id).bpm;
    if (now - data.bpm).abs() <= 1e-3 {
        return None;
    }
    let task = vocal_render::rerender_vocal_audio(r, definition_id, track_id);
    let requeued = vocal_audio_install::current_render_epoch(r, definition_id, track_id)
        != data.render_epoch;
    requeued.then_some(task)
}

/// Outcome of a control-endpoint melodic-part generation
/// ([`control_generate_part`]).
pub(crate) enum ControlPartOutcome {
    /// The lane was generated; MIDI clips now exist across the section's
    /// placements.
    Generated,
    /// The section has no chords, so there is nothing to derive from.
    NoChords,
    /// The section definition id was not found.
    NoSection,
}

/// Install a Bass / Melody / Pad generator on `track_id` within
/// `definition_id` and derive its notes onto every placement of the
/// section (control endpoint `generate.part`, ba todo #1154).
///
/// `config` is the fully-built [`LaneGeneratorConfig`] (kind + params +
/// seed) the caller assembled from the wire params; this reuses the
/// exact `regenerate_lane` path the lane inspector drives, so the
/// generated clips match the GUI's. Returns [`ControlPartOutcome::NoChords`]
/// without touching the lane when the section has no chords (the
/// generators read the chord grid and would silently produce nothing).
pub(crate) fn control_generate_part(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
    config: crate::compose::LaneGeneratorConfig,
) -> (ControlPartOutcome, Task<Message>) {
    let Some(def) = r.compose.find_definition(definition_id) else {
        return (ControlPartOutcome::NoSection, Task::none());
    };
    if def.chords.is_empty() {
        return (ControlPartOutcome::NoChords, Task::none());
    }
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.lane_generators.insert(track_id, config);
    }
    let task = regenerate::regenerate_lane(r, definition_id, track_id);
    r.compose.last_error = None;
    (ControlPartOutcome::Generated, task)
}

/// Install (or clear) the generator on a `(section definition, track)`
/// lane, without deriving any MIDI (control endpoint
/// `section.set_lane_generator`, ba doc #268 / todo #1168).
///
/// `config` is the fully-built [`crate::compose::LaneGeneratorConfig`]
/// the caller assembled from the wire params; `None` is the Manual kind
/// and removes the lane's entry. This is deliberately the *whole*
/// mutation: unlike [`control_generate_part`] there is no chord gate and
/// no `regenerate_lane` call, because a vocal lane carries no
/// chord-derived material at install time (its melody and lyrics arrive
/// later via `vocal.*`) and the melodic kinds keep `generate.part` for
/// the generate-now verb.
///
/// Mirrors the GUI's `lane_inspector::set_generator` insert/remove.
pub(crate) fn control_set_lane_generator(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
    config: Option<crate::compose::LaneGeneratorConfig>,
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        match config {
            Some(config) => {
                def.lane_generators.insert(track_id, config);
            }
            None => {
                def.lane_generators.remove(&track_id);
            }
        }
        r.compose.last_error = None;
    }
}

/// `vocal.generate` (ba doc #269 FR-2): generate a vocal lane's melody
/// into its derived clip, optionally rolling fresh lyrics first.
///
/// Reuses the exact paths the lane inspector's generate buttons drive,
/// so material produced over the wire matches the GUI's:
/// `roll_vocal_lyrics` + `roll_vocal_melody` for the full generate, and
/// `roll_vocal_melody` alone for melody-only.
///
/// `lyrics = false` leaves an existing draft alone — generation writes
/// both melody and lyrics from the lane's theme brief by default, which
/// would silently discard lyrics a client had just written.
///
/// An explicit `seed` is installed on the lane config before generating
/// (reproducible); `None` bumps it, as the GUI buttons do, so repeated
/// calls give new material.
pub(crate) fn control_generate_vocal(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: resonance_audio::types::TrackId,
    seed: Option<u64>,
    lyrics: bool,
) -> Task<Message> {
    if let Some(seed) = seed {
        if let Some(cfg) = r
            .compose
            .find_definition_mut(definition_id)
            .and_then(|d| d.lane_generators.get_mut(&track_id))
        {
            cfg.seed = seed;
        }
    }
    if lyrics {
        // Seed already pinned above when explicit: use it as given
        // (VIEW-35 — `bump_seed` always moves, even with mix 0).
        let mix = if seed.is_some() { None } else { Some(0xBF58476D1CE4E5B9) };
        vocal_render::draft_vocal_lyrics(r, definition_id, track_id, mix);
        vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
    }
    if seed.is_none() {
        if let Some(cfg) = r
            .compose
            .find_definition_mut(definition_id)
            .and_then(|d| d.lane_generators.get_mut(&track_id))
        {
            cfg.seed = crate::util::bump_seed(cfg.seed, 0x94D049BB133111EB);
        }
    }
    let task = vocal_render::roll_vocal_melody(r, definition_id, track_id);
    r.compose.last_error = None;
    task
}

/// `vocal.set_pronunciation`: set (or replace) a per-word override in
/// the project pronunciation dictionary. Phonemes are already
/// canonicalised. Word match is case-insensitive (the dictionary key is
/// the cleaned, lowercased spelling).
fn control_set_pronunciation(
    r: &mut crate::Resonance,
    word: String,
    phonemes: Vec<&'static str>,
) {
    use crate::compose::vocal_svs::{clean_word, DictionaryEntry, DictionaryScope};
    let key = clean_word(&word);
    let dict = &mut r.compose.pronunciation.project_dictionary;
    dict.retain(|e| e.word != key);
    dict.push(DictionaryEntry::from_canonical(
        &word,
        phonemes,
        DictionaryScope::Project,
    ));
    r.compose.last_error = None;
}

/// `vocal.clear_pronunciation`: remove a per-word project override.
fn control_clear_pronunciation(r: &mut crate::Resonance, word: &str) {
    use crate::compose::vocal_svs::clean_word;
    let key = clean_word(word);
    r.compose
        .pronunciation
        .project_dictionary
        .retain(|e| e.word != key);
    r.compose.last_error = None;
}

/// Ensure the bulk-lyrics text-editor buffer exists for the currently
/// selected vocal lane. The view layer hands a `&Content` to the iced
/// widget, so the entry must be present before the first paint to avoid
/// rendering a dead fallback. Seeded from the lane's current `params.draft`
/// so the user sees their existing lyrics in the editor.
pub(crate) fn ensure_vocal_bulk_lyrics_for_selection(r: &mut crate::Resonance) {
    use crate::compose::{LaneGeneratorKind, SelectedLane};
    let track_id = match r.compose.selected_lane {
        SelectedLane::Instrument(id) => id,
        _ => return,
    };
    let Some(placement_id) = r.compose.selected_placement_id else {
        return;
    };
    let Some(placement) = r
        .compose
        .placements
        .iter()
        .find(|p| p.id == placement_id)
        .cloned()
    else {
        return;
    };
    let definition_id = placement.definition_id;
    let Some(def) = r.compose.find_definition(definition_id) else {
        return;
    };
    let Some(cfg) = def.lane_generators.get(&track_id) else {
        return;
    };
    let LaneGeneratorKind::Vocal(params) = &cfg.kind else {
        return;
    };
    let key = (definition_id, track_id);
    if r.compose.vocal_bulk_lyrics.contains_key(&key) {
        return;
    }
    let body = params
        .draft
        .iter()
        .map(|l| l.text.replace('\u{00B7}', "").replace("  ", " "))
        .collect::<Vec<_>>()
        .join("\n");
    r.compose
        .vocal_bulk_lyrics
        .insert(key, iced::widget::text_editor::Content::with_text(&body));
}
