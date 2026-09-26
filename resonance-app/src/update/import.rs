//! Update handlers for the MIDI Import modal.
//!
//! Scope here is the shared shell: open/close plumbing, the file chooser,
//! the background parse (code review VIEW-25) and the review-stage field
//! setters. Tempo-conflict resolution and the actual import land in the
//! follow-up todos (doc #158), so the interaction arms only update the
//! dialog's transient state — they don't yet touch the project or the
//! audio engine.

use std::path::{Path, PathBuf};

use iced::Task;

use resonance_audio::midi_io::{parse_midi_file, ImportedSmf, SmfFormat};

use crate::message::{ImportMessage, Message};
use crate::state::{
    ImportDialogState, ImportStage, ImportSummary, ImportTrackKind, ParsedImport, PreviewNote,
    TrackImportRow,
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
        // The parse→import orchestration lands in a follow-up todo
        // (doc #158). For now Confirm is a no-op placeholder so the shell
        // compiles and routes cleanly; it does not yet mutate the project.
        ImportMessage::Confirm => {}
    }
    Task::none()
}
