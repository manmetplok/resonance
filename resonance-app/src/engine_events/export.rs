//! Stem export (`AudioCommand::ExportStems`) events, mirrored into the
//! Export modal's phase (code review ARCH2-01). The modal is the only
//! sender of `ExportStems`, so with the modal closed (which it can't be
//! mid-render) an event is only logged.

use std::path::PathBuf;

use resonance_audio::types::EngineError;

use crate::state::ExportPhase;
use crate::Resonance;

/// `StemExportProgress`: target `target_index` (0-based) is starting.
pub(super) fn progress(r: &mut Resonance, target_index: usize, total: usize) {
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
    if let Some(dialog) = r.modals.export_dialog.as_mut() {
        dialog.target_errors.push(format!("Stem {}: {message}", index + 1));
    }
}

/// `StemExportComplete`: the queue finished. Done when every target
/// wrote; Error (keeping the written stems) when any failed.
pub(super) fn complete(r: &mut Resonance, files: Vec<String>) {
    let written: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    tracing::info!("Stem export complete: {} file(s)", written.len());
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
    if let Some(dialog) = r.modals.export_dialog.as_mut() {
        dialog.phase = ExportPhase::Cancelled(files.into_iter().map(PathBuf::from).collect());
        dialog.cancel_requested = false;
    }
}
