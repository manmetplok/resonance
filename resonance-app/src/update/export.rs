//! Update handler for the Export modal (design doc #155).
//!
//! Owns the modal lifecycle — open, close, the mode-tab switch, the
//! Audio-stems body (source checklist, range, destination) — and the
//! stem render itself: `Confirm` sends [`AudioCommand::ExportStems`] and
//! the dialog follows the engine's `StemExport*` events
//! (`engine_events::export`) through Rendering → Done / Error /
//! Cancelled (code review ARCH2-01). The MIDI tab has no exporter on the
//! engine side yet, so its primary action stays disabled.

use std::path::{Path, PathBuf};

use iced::Task;
use resonance_audio::types::{AudioCommand, StemBitDepth, StemSource, StemTarget};

use crate::message::Message;
use crate::state::{
    ExportBitDepth, ExportDialogState, ExportMode, ExportOverwrite, ExportPhase, ExportRange,
    ExportSource,
};
use crate::Resonance;

/// User actions in the Export modal (design doc #155).
#[derive(Debug, Clone)]
pub enum ExportMessage {
    /// Open the modal in its default (Audio-stems) state.
    Open,
    /// Dismiss the modal, discarding the transient selection. Ignored
    /// while a render is in flight (cancel it first).
    Close,
    /// Switch the active mode tab (Audio stems / MIDI).
    SetMode(ExportMode),
    /// Tick / untick one source in the checklist.
    ToggleSource(ExportSource),
    /// Render the whole project or only the loop range.
    SetRange(ExportRange),
    /// Open the folder picker for the destination.
    ChooseDestination,
    /// The folder picker answered (`None`: dismissed).
    DestinationChosen(Option<PathBuf>),
    /// Footer primary action - render the selected sources.
    Confirm,
    /// Stop the running stem export between targets
    /// (`AudioCommand::CancelStemExport`).
    CancelRender,
}

impl ExportMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // The export dialog reads the project; it never edits it.
            Self::Open
            | Self::Close
            | Self::SetMode(..)
            | Self::ToggleSource(..)
            | Self::SetRange(..)
            | Self::ChooseDestination
            | Self::DestinationChosen(..)
            | Self::Confirm
            | Self::CancelRender => UndoAction::Skip,
        }
    }
}

pub fn handle(r: &mut Resonance, msg: ExportMessage) -> Task<Message> {
    match msg {
        ExportMessage::Open => {
            let mut dialog = ExportDialogState::new();
            dialog.destination = r.io.project_path.as_ref().map(|p| p.join("stems"));
            r.modals.export_dialog = Some(dialog);
        }
        ExportMessage::Close => {
            if !r.modals.export_dialog.as_ref().is_some_and(|d| d.is_rendering()) {
                r.modals.export_dialog = None;
            }
        }
        ExportMessage::SetMode(mode) => {
            if let Some(dialog) = setup_dialog(r) {
                dialog.mode = mode;
            }
        }
        ExportMessage::ToggleSource(source) => {
            if let Some(dialog) = setup_dialog(r) {
                if !dialog.selected_sources.remove(&source) {
                    dialog.selected_sources.insert(source);
                }
            }
        }
        ExportMessage::SetRange(range) => {
            if let Some(dialog) = setup_dialog(r) {
                dialog.range = range;
            }
        }
        ExportMessage::ChooseDestination => {
            let start = r
                .modals
                .export_dialog
                .as_ref()
                .and_then(|d| d.destination.clone())
                .or_else(|| r.io.project_path.clone());
            return Task::perform(
                async move {
                    let mut dialog = rfd::AsyncFileDialog::new().set_title("Export stems to");
                    if let Some(dir) = start {
                        dialog = dialog.set_directory(dir);
                    }
                    dialog.pick_folder().await.map(|f| f.path().to_path_buf())
                },
                |path| Message::Export(ExportMessage::DestinationChosen(path)),
            );
        }
        ExportMessage::DestinationChosen(path) => {
            if let (Some(path), Some(dialog)) = (path, setup_dialog(r)) {
                dialog.destination = Some(path);
            }
        }
        ExportMessage::Confirm => start_stem_export(r),
        ExportMessage::CancelRender => {
            if let Some(dialog) = r.modals.export_dialog.as_mut() {
                if dialog.is_rendering() && !dialog.cancel_requested {
                    dialog.cancel_requested = true;
                    let _ = r.engine.send(AudioCommand::CancelStemExport);
                }
            }
        }
    }
    Task::none()
}

/// The open dialog while it is still editable (`Setup`).
fn setup_dialog(r: &mut Resonance) -> Option<&mut ExportDialogState> {
    r.modals
        .export_dialog
        .as_mut()
        .filter(|d| matches!(d.phase, ExportPhase::Setup))
}

/// `Confirm`: build one [`StemTarget`] per ticked source and hand the
/// queue to the engine's stem renderer.
fn start_stem_export(r: &mut Resonance) {
    let Some(dialog) = r.modals.export_dialog.as_ref() else {
        return;
    };
    if !dialog.can_export() {
        return;
    }
    // One offline renderer at a time: a stem export drives the same live
    // plugin instances as a mixdown, a bounce in place, a freeze or a
    // control measurement (see `offline_render_in_progress`).
    if r.offline_render_in_progress() {
        r.banners.error_message =
            Some("An offline render is in progress; export again when it finishes".into());
        return;
    }
    let Some(dir) = dialog.destination.clone() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        if let Some(dialog) = r.modals.export_dialog.as_mut() {
            dialog.phase = ExportPhase::Error {
                written: Vec::new(),
                message: format!("Can't create {}: {e}", dir.display()),
                remaining: dialog.selected_sources.len(),
            };
        }
        return;
    }
    let targets = stem_targets(r, dialog, &dir);
    let range = match dialog.range {
        ExportRange::WholeProject => None,
        ExportRange::LoopOrSelection => {
            let t = &r.transport;
            (t.loop_range_set && t.loop_out > t.loop_in).then_some((t.loop_in, t.loop_out))
        }
    };
    let format = dialog.format;
    let total = targets.len();
    let command = AudioCommand::ExportStems {
        targets,
        range,
        sample_rate: format.sample_rate,
        bit_depth: match format.bit_depth {
            ExportBitDepth::Int16 => StemBitDepth::Int16,
            ExportBitDepth::Int24 => StemBitDepth::Int24,
            ExportBitDepth::Float32 => StemBitDepth::Float32,
        },
        include_fx_tail: format.include_fx_tail,
    };
    if let Some(dialog) = r.modals.export_dialog.as_mut() {
        dialog.phase = ExportPhase::Rendering { done: 0, total };
        dialog.target_errors.clear();
        dialog.cancel_requested = false;
    }
    let _ = r.engine.send(command);
}

/// One target per selected source, in the selection's (deterministic)
/// order: `NN-<name>.wav` in `dir`, where `NN` is the 1-based queue
/// position, so two tracks with the same name never share a file. Under
/// [`ExportOverwrite::Suffix`] (and `Ask`, which has no prompt yet) a
/// name already on disk gets `_1`, `_2`, …; `Overwrite` replaces it.
pub fn stem_targets(r: &Resonance, dialog: &ExportDialogState, dir: &Path) -> Vec<StemTarget> {
    dialog
        .selected_sources
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let (stem, label) = match *source {
                ExportSource::Track(id) => (
                    StemSource::Track(id),
                    r.registry
                        .tracks
                        .iter()
                        .find(|t| t.id == id)
                        .map_or_else(|| format!("Track {id}"), |t| t.name.clone()),
                ),
                ExportSource::Bus(id) => (
                    StemSource::Bus(id),
                    r.registry
                        .busses
                        .iter()
                        .find(|b| b.id == id)
                        .map_or_else(|| format!("Bus {id}"), |b| b.name.clone()),
                ),
                ExportSource::Master => (StemSource::Master, "Master".to_owned()),
            };
            let base = format!("{:02}-{}", i + 1, sanitize_file_stem(&label));
            let mut path = dir.join(format!("{base}.wav"));
            if dialog.overwrite != ExportOverwrite::Overwrite {
                let mut n = 1;
                while path.exists() {
                    path = dir.join(format!("{base}_{n}.wav"));
                    n += 1;
                }
            }
            StemTarget {
                source: stem,
                path: path.to_string_lossy().into_owned(),
            }
        })
        .collect()
}

/// A track / bus name made safe as a file stem: path separators and
/// other characters file systems reject become `_`.
fn sanitize_file_stem(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.trim_matches('.').is_empty() {
        "stem".to_owned()
    } else {
        cleaned
    }
}
