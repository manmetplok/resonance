//! Update handlers for the MIDI Import modal.
//!
//! Scope here is the shared shell: open/close plumbing plus the
//! review-stage field setters. The parse task, tempo-conflict resolution,
//! and the actual import land in the follow-up todos (doc #158), so the
//! interaction arms only update the dialog's transient state — they don't
//! yet touch the project or the audio engine.

use iced::Task;

use crate::message::{ImportMessage, Message};
use crate::state::{ImportDialogState, ImportStage};
use crate::Resonance;

pub fn handle(app: &mut Resonance, message: ImportMessage) -> Task<Message> {
    match message {
        ImportMessage::Open => {
            app.import_dialog = Some(ImportDialogState::new());
        }
        ImportMessage::Cancel => {
            app.import_dialog = None;
        }
        // A file was chosen or dropped: remember it and move to Parsing.
        // The parse task itself is wired in a follow-up todo; it reports
        // back via `ParseCompleted`.
        ImportMessage::FileChosen(path) | ImportMessage::FileDropped(path) => {
            if let Some(d) = app.import_dialog.as_mut() {
                d.source_path = Some(path);
                d.stage = ImportStage::Parsing;
                d.error = None;
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
