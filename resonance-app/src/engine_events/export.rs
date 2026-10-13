//! Stem export (`AudioCommand::ExportStems`) events, mirrored into the
//! Export modal's phase (code review ARCH2-01) or into a control client's
//! `render.stems` job. Two senders, one queue: the modal and the control
//! method refuse to start while the other renders
//! (`stem_export_in_progress`), so while
//! `ControlEndpointState::pending_stems` is set every event belongs to
//! that job, and otherwise to the modal. With neither (the modal can't
//! close mid-render) an event is only logged.

use std::path::PathBuf;

use resonance_audio::types::EngineError;

use crate::state::ExportPhase;
use crate::update::control;
use crate::Resonance;

/// `StemExportProgress`: target `target_index` (0-based) is starting.
pub(super) fn progress(r: &mut Resonance, target_index: usize, total: usize, fraction: f32) {
    if control::stems_progress(r, target_index, total, fraction) {
        return;
    }
    if let Some(dialog) = r.modals.export_dialog.as_mut() {
        if dialog.is_rendering() {
            dialog.phase = ExportPhase::Rendering {
                done: target_index.min(total),
                total,
            };
        }
    }
}

/// `StemExportTargetDone`: one stem is on disk.
pub(super) fn target_done(r: &mut Resonance, index: usize, path: String) {
    tracing::info!("Stem {} written: {path}", index + 1);
    if control::stems_target_done(r, index) {
        return;
    }
    if let Some(dialog) = r.modals.export_dialog.as_mut() {
        if let ExportPhase::Rendering { done, total } = dialog.phase {
            dialog.phase = ExportPhase::Rendering {
                done: (index + 1).max(done).min(total),
                total,
            };
        }
    }
}

/// `StemExportTargetError`: one stem failed; the queue goes on.
pub(super) fn target_error(r: &mut Resonance, index: usize, message: String) {
    tracing::warn!("Stem {} failed: {message}", index + 1);
    if control::stems_target_error(r, index, &message) {
        return;
    }
    if let Some(dialog) = r.modals.export_dialog.as_mut() {
        dialog.target_errors.push(format!("Stem {}: {message}", index + 1));
    }
}

/// `StemExportComplete`: the queue finished. Done when every target
/// wrote; Error (keeping the written stems) when any failed.
pub(super) fn complete(r: &mut Resonance, files: Vec<String>) {
    tracing::info!("Stem export complete: {} file(s)", files.len());
    if control::stems_complete(r, &files) {
        return;
    }
    let written: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    let Some(dialog) = r.modals.export_dialog.as_mut() else {
        return;
    };
    dialog.phase = if dialog.target_errors.is_empty() {
        ExportPhase::Done(written)
    } else {
        let remaining = dialog.target_errors.len();
        ExportPhase::Error {
            written,
            message: dialog.target_errors.join("\n"),
            remaining,
        }
    };
}

/// `StemExportError`: the export could not start; nothing was written.
pub(super) fn error(r: &mut Resonance, e: EngineError) {
    tracing::warn!(kind = ?e.kind, "Stem export failed: {}", e.message);
    if control::stems_error(r, &e) {
        return;
    }
    match r.modals.export_dialog.as_mut() {
        Some(dialog) => {
            let remaining = match dialog.phase {
                ExportPhase::Rendering { total, .. } => total,
                _ => dialog.selected_sources.len(),
            };
            dialog.phase = ExportPhase::Error {
                written: Vec::new(),
                message: e.message,
                remaining,
            };
        }
        None => r.banners.error_message = Some(format!("Stem export failed: {}", e.message)),
    }
}

/// `StemExportCancelled`: stopped between targets on the user's cancel.
pub(super) fn cancelled(r: &mut Resonance, files: Vec<String>) {
    tracing::info!("Stem export cancelled after {} file(s)", files.len());
    if control::stems_cancelled(r, &files) {
        return;
    }
    if let Some(dialog) = r.modals.export_dialog.as_mut() {
        dialog.phase = ExportPhase::Cancelled(files.into_iter().map(PathBuf::from).collect());
        dialog.cancel_requested = false;
    }
}
