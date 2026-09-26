//! Update handlers for the MIDI Import modal.
//!
//! The shared shell (open/close, file chooser, background parse — code
//! review VIEW-25), the review-stage field setters, the tempo-conflict
//! choice, and Confirm, which lands the import (code review FU-V2a).
//!
//! Confirm is the ONE message of the flow that edits the project: it is
//! classified `Record` in `undo/classify.rs`, refused up front by
//! `gates_message` when [`confirm_blocker`] objects, and fans out through
//! `update()` inside [`Resonance::continue_as_one_undo`] so the new tracks,
//! clips, notes and any adopted tempo are a single undo entry. The clip
//! write is [`create_clip_with_notes`], shared with the control API's
//! `notes.import_midi`, so GUI and control land notes the same way.
//!
//! Tempo (doc #158): a file whose tempo differs from the project's stops at
//! the TempoConflict stage, which asks for one of
//! - **keep the project tempo, match bars** — the notes keep their
//!   bar/beat positions and play at the project tempo;
//! - **keep the project tempo, match time** — the notes keep their
//!   wall-clock timing, rescaled from the file's tempo map onto the
//!   project tempo at the import point;
//! - **use the file tempo** — the project's tempo map is replaced by the
//!   file's, and the notes keep their bar/beat positions.

use std::path::{Path, PathBuf};

use std::sync::Arc;

use iced::Task;

use resonance_audio::midi_io::{parse_midi_file, ImportedSmf, SmfFormat, TempoEvent};
use resonance_audio::types::{ClipId, MidiNote, TrackId, TrackType, TICKS_PER_QUARTER_NOTE};

use crate::message::{ImportMessage, Message, MidiClipMessage, MidiEditorMessage, TrackMessage};
use crate::state::{
    ControlTrackKind, ImportDialogState, ImportResultSummary, ImportSource, ImportStage,
    ImportSummary, ImportTrackKind, ParsedImport, PlacementMode, PlacementStart, PreviewNote,
    TempoAlignment, TempoChoice, TrackImportRow,
};
use crate::Resonance;

/// Notes kept per row for the Review stage's preview strip.
const PREVIEW_NOTES: usize = 128;
/// A file tempo within this many BPM of the project's is not a conflict.
const TEMPO_TOLERANCE_BPM: f32 = 0.05;
/// GM percussion channel (10, 0-based 9).
const GM_DRUM_CHANNEL: u8 = 9;

/// Parse the MIDI file at `path` into the dialog's review model. Blocking
/// file I/O + parse; the dialog runs it off the UI thread (see
/// [`spawn_parse`]). `project_bpm` decides whether the file's tempo
/// conflicts with the project's.
pub fn parse_import_file(path: &Path, project_bpm: f32) -> Result<ParsedImport, String> {
    let smf = parse_midi_file(path)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    Ok(parsed_import(&smf, file_name, project_bpm))
}

/// Map the parser's [`ImportedSmf`] onto the app-level review model.
fn parsed_import(smf: &ImportedSmf, file_name: String, project_bpm: f32) -> ParsedImport {
    let rows: Vec<TrackImportRow> = smf
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| TrackImportRow {
            selected: !t.is_conductor && t.note_count > 0,
            name: t
                .name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| format!("Track {}", i + 1)),
            channel: t.channels.first().copied().unwrap_or(0),
            kind: if t.channels.contains(&GM_DRUM_CHANNEL) {
                ImportTrackKind::Drum
            } else {
                ImportTrackKind::Instrument
            },
            note_count: t.note_count,
            pitch_min: t.pitch_min,
            pitch_max: t.pitch_max,
            is_conductor: t.is_conductor,
            preview: t
                .notes
                .iter()
                .take(PREVIEW_NOTES)
                .map(|n| PreviewNote {
                    pitch: n.note,
                    start_tick: n.start_tick,
                    duration_ticks: n.duration_ticks,
                    velocity: n.velocity,
                })
                .collect(),
        })
        .collect();
    let file_tempo_bpm = smf.tempo_events.first().map(|e| e.bpm);
    let tempo_conflict = smf
        .tempo_events
        .iter()
        .any(|e| (e.bpm - project_bpm).abs() > TEMPO_TOLERANCE_BPM);
    ParsedImport {
        summary: ImportSummary {
            file_name,
            smf_format: Some(match smf.format {
                SmfFormat::Format0 => 0,
                SmfFormat::Format1 => 1,
            }),
            track_count: smf.track_count,
            ppq: Some(smf.source_ppq),
            length_bars: Some(smf.length_bars),
            total_notes: smf.tracks.iter().map(|t| t.note_count).sum(),
            file_tempo_bpm,
            tempo_bpm_min: file_tempo_bpm.map(|_| smf.tempo_min_bpm),
            tempo_bpm_max: file_tempo_bpm.map(|_| smf.tempo_max_bpm),
            tempo_conflict,
        },
        rows,
        source: Some(ImportSource(Arc::new(smf.clone()))),
    }
}

/// Parse `path` on a blocking thread and report back as
/// [`ImportMessage::Parsed`], tagged with the path so a superseded parse
/// can be told apart.
fn spawn_parse(path: PathBuf, project_bpm: f32) -> Task<Message> {
    Task::perform(
        async move {
            let job_path = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                parse_import_file(&job_path, project_bpm)
            })
            .await
            .unwrap_or_else(|e| Err(format!("MIDI parse task failed: {e}")));
            (path, result)
        },
        |(path, result)| Message::Import(ImportMessage::Parsed { path, result }),
    )
}

/// The OS file chooser, filtered to Standard MIDI Files. A cancel yields
/// no message.
fn choose_file() -> Task<Message> {
    Task::perform(
        async {
            rfd::AsyncFileDialog::new()
                .set_title("Import MIDI")
                .add_filter("MIDI files", &["mid", "midi"])
                .pick_file()
                .await
                .map(|f| f.path().to_path_buf())
        },
        |picked| picked,
    )
    .and_then(|path| Task::done(Message::Import(ImportMessage::FileChosen(path))))
}

/// True when `path` looks like a Standard MIDI File by extension
/// (`.mid`/`.midi`, case-insensitive). The file-drop subscription uses
/// this to ignore non-MIDI drops so only MIDI files start an import.
pub fn is_midi_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mid") || ext.eq_ignore_ascii_case("midi"))
}

pub fn handle(app: &mut Resonance, message: ImportMessage) -> Task<Message> {
    match message {
        ImportMessage::Open => {
            app.import_dialog = Some(ImportDialogState::new());
        }
        ImportMessage::Cancel => {
            app.import_dialog = None;
        }
        // A MIDI file is being dragged over the window: surface the drop
        // target by opening the modal at the Drop stage. A no-op when a
        // dialog is already open, so it never disturbs an in-flight review.
        // Tagged `opened_by_hover` so a stray drag-out can dismiss it.
        ImportMessage::HoverFile => {
            if app.import_dialog.is_none() {
                let mut dialog = ImportDialogState::new();
                dialog.opened_by_hover = true;
                app.import_dialog = Some(dialog);
            }
        }
        // The drag left the window without a drop: close the dialog only if
        // the hover itself opened it and nothing has happened since (still
        // empty at the Drop stage). A dialog the user opened deliberately —
        // or one already parsing a dropped file — is left untouched.
        ImportMessage::HoverLeft => {
            if let Some(d) = app.import_dialog.as_ref() {
                if d.opened_by_hover
                    && d.stage == ImportStage::Drop
                    && d.source_path.is_none()
                {
                    app.import_dialog = None;
                }
            }
        }
        ImportMessage::Choose => return choose_file(),
        // A file was chosen or dropped: remember it, move to Parsing and
        // parse it off-thread. A drop can arrive before the modal is open
        // (dropped straight onto the arrangement), so open it on demand.
        ImportMessage::FileChosen(path) | ImportMessage::FileDropped(path) => {
            let d = app.import_dialog.get_or_insert_with(ImportDialogState::new);
            d.source_path = Some(path.clone());
            d.stage = ImportStage::Parsing;
            d.error = None;
            d.opened_by_hover = false;
            let project_bpm = app
                .tempo_events
                .first()
                .map(|e| e.bpm)
                .unwrap_or(app.transport.bpm);
            return spawn_parse(path, project_bpm);
        }
        // Only the parse of the file the dialog is still waiting on counts.
        ImportMessage::Parsed { path, result } => {
            let current = app.import_dialog.as_ref().is_some_and(|d| {
                d.stage == ImportStage::Parsing && d.source_path.as_deref() == Some(&*path)
            });
            if current {
                return handle(app, ImportMessage::ParseCompleted(result));
            }
        }
        ImportMessage::ParseCompleted(result) => {
            if let Some(d) = app.import_dialog.as_mut() {
                match result {
                    Ok(parsed) => {
                        // A tempo mismatch routes through the conflict
                        // step first; otherwise straight to review.
                        d.stage = if parsed.summary.tempo_conflict {
                            ImportStage::TempoConflict
                        } else {
                            ImportStage::Review
                        };
                        d.summary = Some(parsed.summary);
                        d.rows = parsed.rows;
                        d.source = parsed.source;
                        d.error = None;
                    }
                    Err(reason) => {
                        d.stage = ImportStage::Error;
                        d.error = Some(reason);
                    }
                }
            }
        }
        ImportMessage::ToggleTrack(index) => {
            if let Some(d) = app.import_dialog.as_mut() {
                if let Some(row) = d.rows.get_mut(index) {
                    row.selected = !row.selected;
                }
            }
        }
        ImportMessage::SetAllTracks(selected) => {
            if let Some(d) = app.import_dialog.as_mut() {
                for row in &mut d.rows {
                    row.selected = selected;
                }
            }
        }
        ImportMessage::RenameTrack(index, name) => {
            if let Some(d) = app.import_dialog.as_mut() {
                if let Some(row) = d.rows.get_mut(index) {
                    row.name = name;
                }
            }
        }
        ImportMessage::SetTempoChoice(choice) => {
            if let Some(d) = app.import_dialog.as_mut() {
                d.tempo_choice = choice;
            }
        }
        ImportMessage::SetPlacementStart(start) => {
            if let Some(d) = app.import_dialog.as_mut() {
                d.placement.start = start;
            }
        }
        ImportMessage::SetPlacementMode(mode) => {
            if let Some(d) = app.import_dialog.as_mut() {
                d.placement.mode = mode;
            }
        }
        ImportMessage::SetMergeTarget(target) => {
            if let Some(d) = app.import_dialog.as_mut() {
                d.placement.merge_target = target;
            }
        }
        ImportMessage::SetConflictAlignment(alignment) => {
            if let Some(d) = app.import_dialog.as_mut() {
                d.tempo_alignment = alignment;
            }
        }
        ImportMessage::ChooseTempo(choice, alignment) => {
            if let Some(d) = app.import_dialog.as_mut() {
                d.tempo_choice = choice;
                d.tempo_alignment = alignment;
            }
        }
        ImportMessage::ResolveTempo => {
            if let Some(d) = app.import_dialog.as_mut() {
                if d.stage == ImportStage::TempoConflict {
                    d.stage = ImportStage::Review;
                }
            }
        }
        ImportMessage::Confirm => return confirm(app),
    }
    Task::none()
}

/// Why Confirm cannot import right now, or `None` when it can. The one
/// predicate behind both the `gates_message` refusal and the Import
/// button's enabled state, so the two never disagree.
pub(crate) fn confirm_blocker(app: &Resonance) -> Option<String> {
    let Some(d) = app.import_dialog.as_ref() else {
        return Some("No import in progress.".to_owned());
    };
    if d.stage != ImportStage::Review {
        return Some("Finish the current step first.".to_owned());
    }
    if d.source.is_none() {
        return Some("Nothing has been parsed to import.".to_owned());
    }
    if d.selected_count() == 0 {
        return Some("Select at least one track to import.".to_owned());
    }
    if d.placement.mode == PlacementMode::MergeIntoSelected {
        let Some(target) = d.placement.merge_target else {
            return Some("Choose a track to merge into.".to_owned());
        };
        let Some(track) = app.registry.tracks.iter().find(|t| t.id == target) else {
            return Some("The merge target track no longer exists.".to_owned());
        };
        if !matches!(track.track_type, TrackType::Instrument | TrackType::Vocal) {
            return Some("MIDI can only merge into an instrument or vocal track.".to_owned());
        }
        if app.freeze.status(target).is_frozen() {
            return Some("The merge target is frozen; unfreeze it first.".to_owned());
        }
    }
    None
}

/// Land the reviewed import. Only reached when [`confirm_blocker`] had no
/// objection (the gate drops it otherwise), and the Confirm message has
/// already recorded the pre-import undo snapshot.
fn confirm(app: &mut Resonance) -> Task<Message> {
    let Some(d) = app.import_dialog.as_ref() else {
        return Task::none();
    };
    let Some(ImportSource(smf)) = d.source.clone() else {
        return Task::none();
    };
    let conflict = d.summary.as_ref().is_some_and(|s| s.tempo_conflict);
    let adopt_file_tempo = conflict && d.tempo_choice == TempoChoice::AdoptFile;
    let match_time = conflict
        && d.tempo_choice == TempoChoice::KeepProject
        && d.tempo_alignment == TempoAlignment::MatchTime;
    let placement = d.placement;
    let file_name = d
        .summary
        .as_ref()
        .map(|s| s.file_name.clone())
        .unwrap_or_else(|| "Imported MIDI".to_owned());
    // Row `i` was built from the file's track `i`.
    let picked: Vec<(String, ImportTrackKind, usize)> = d
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.selected && !r.is_conductor)
        .map(|(i, r)| (r.name.clone(), r.kind, i))
        .collect();

    app.continue_as_one_undo(|app| {
        let mut tasks = Vec::new();
        if adopt_file_tempo && !smf.tempo_points.is_empty() {
            let mut points = smf.tempo_points.clone();
            if points[0].bar != 0 {
                let bpm = points[0].bpm;
                points.insert(0, crate::state::TempoEvent { bar: 0, bpm });
            }
            app.tempo_events = points;
            app.rebuild_and_send_tempo();
            app.sync_tempo_display();
        }

        let start_sample = match placement.start {
            PlacementStart::Bar1 => app.tempo_map.bar_to_sample(0),
            PlacementStart::Playhead => app.transport.playhead,
        };
        let project_bpm = app
            .tempo_map
            .tempo_at_sample(start_sample, app.sample_rate)
            .0;
        let notes_of = |index: usize| -> Vec<MidiNote> {
            let notes = smf
                .tracks
                .get(index)
                .map(|t| t.notes.clone())
                .unwrap_or_default();
            if match_time {
                retime_notes(&notes, &smf.tempo_events, project_bpm)
            } else {
                notes
            }
        };

        let mut result = ImportResultSummary {
            tracks_created: 0,
            clips_added: 0,
            notes_imported: 0,
        };
        match placement.mode {
            PlacementMode::NewTracks => {
                for (name, kind, index) in &picked {
                    let notes = notes_of(*index);
                    let track_id = app.allocate_track_id();
                    tasks.push(app.update(Message::Track(TrackMessage::AddControlTrack {
                        id: track_id,
                        kind: match kind {
                            ImportTrackKind::Instrument => ControlTrackKind::Instrument,
                            ImportTrackKind::Drum => ControlTrackKind::Drums,
                            ImportTrackKind::Vocal => ControlTrackKind::Vocal,
                        },
                        name: Some(name.clone()),
                    })));
                    result.tracks_created += 1;
                    result.notes_imported += notes.len();
                    let (_, task) =
                        create_clip_with_notes(app, track_id, start_sample, notes, name.clone());
                    tasks.push(task);
                    result.clips_added += 1;
                }
            }
            PlacementMode::MergeIntoSelected => {
                // `confirm_blocker` guarantees the target.
                if let Some(target) = placement.merge_target {
                    let mut notes: Vec<MidiNote> =
                        picked.iter().flat_map(|(_, _, i)| notes_of(*i)).collect();
                    notes.sort_by_key(|n| (n.start_tick, n.note));
                    result.notes_imported = notes.len();
                    let (_, task) =
                        create_clip_with_notes(app, target, start_sample, notes, file_name);
                    tasks.push(task);
                    result.clips_added = 1;
                }
            }
        }

        if let Some(d) = app.import_dialog.as_mut() {
            d.stage = ImportStage::Imported;
            d.result = Some(result);
            d.error = None;
        }
        Task::batch(tasks)
    })
}

/// Create a clip on `track_id` at `start_sample` holding `notes` (engine
/// ticks, clip-relative), sized up to whole bars. The one clip write both
/// importers use — the dialog's Confirm and the control API's
/// `notes.import_midi` — so they land notes identically: an id-hinted
/// `CreateEmptyClip`, then the bulk `SetClipNotes`, mirrored optimistically
/// so the notes are readable before the engine echo.
pub(crate) fn create_clip_with_notes(
    app: &mut Resonance,
    track_id: TrackId,
    start_sample: u64,
    notes: Vec<MidiNote>,
    name: String,
) -> (ClipId, Task<Message>) {
    let length_ticks = notes
        .iter()
        .map(|n| n.start_tick + n.duration_ticks)
        .max()
        .unwrap_or(0);
    let bar_ticks = app.transport.time_sig_num.max(1) as u64 * TICKS_PER_QUARTER_NOTE;
    let duration_ticks = length_ticks.div_ceil(bar_ticks).max(1) * bar_ticks;
    let clip_id = app.compose.fresh_derived_clip_id();
    let create = app.update(Message::MidiClip(MidiClipMessage::CreateEmptyClip {
        clip_id,
        track_id,
        start_sample,
        duration_ticks,
        name,
    }));
    let write = app.update(Message::MidiEditor(MidiEditorMessage::SetClipNotes {
        clip_id,
        notes: notes.clone(),
    }));
    crate::engine_events::midi::optimistic_set_notes(app, clip_id, notes);
    (clip_id, Task::batch([create, write]))
}

/// Rescale notes from the file's tempo map onto a constant `project_bpm`
/// so each note keeps its wall-clock onset and length ("keep project
/// tempo, match time"). Constant across the clip: a project tempo change
/// inside the imported span is not followed.
fn retime_notes(notes: &[MidiNote], file_tempo: &[TempoEvent], project_bpm: f32) -> Vec<MidiNote> {
    let to_project_ticks = |tick: u64| -> u64 {
        let seconds = file_seconds_at(tick, file_tempo);
        (seconds * project_bpm as f64 / 60.0 * TICKS_PER_QUARTER_NOTE as f64).round() as u64
    };
    notes
        .iter()
        .map(|n| {
            let start = to_project_ticks(n.start_tick);
            let end = to_project_ticks(n.start_tick + n.duration_ticks);
            MidiNote {
                start_tick: start,
                duration_ticks: end.saturating_sub(start).max(1),
                ..n.clone()
            }
        })
        .collect()
}

/// Seconds from the file's start to `tick` under its tick-positioned tempo
/// map; the SMF default of 120 BPM applies before the first event.
fn file_seconds_at(tick: u64, tempo: &[TempoEvent]) -> f64 {
    let tpq = TICKS_PER_QUARTER_NOTE as f64;
    let mut seconds = 0.0;
    let mut at = 0u64;
    let mut bpm = 120.0f64;
    for event in tempo {
        if event.tick >= tick {
            break;
        }
        seconds += (event.tick - at) as f64 / tpq * 60.0 / bpm;
        at = event.tick;
        bpm = event.bpm as f64;
    }
    seconds + (tick - at) as f64 / tpq * 60.0 / bpm
}
