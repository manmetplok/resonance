//! Transient state for the **drag-to-timeline placement** gesture
//! (design doc #175, epic #35, todo #605).
//!
//! Dragging a row out of the media browser and over the arrangement is the
//! primary way audio lands on the timeline. While a drag is in flight this
//! struct mirrors just enough to paint the placement affordances the design
//! calls for: a drag pill trailing the cursor, the lit target lane, a
//! dashed grid-snapped ghost clip, the drop tooltip (target track + bar +
//! any sample-rate conversion), and the "create a new audio track" drop
//! zone below the last lane.
//!
//! Like every other browser-adjacent surface (folder navigation, audition,
//! collapse state) this is **session UI state**: it is never persisted in
//! the project file and never lands on the undo stack. The single undoable
//! effect is the drop itself, which fans out into a
//! [`PoolMessage::ImportAndPlace`](crate::message::PoolMessage) — that
//! message carries the whole import + placement as one undoable action.
//!
//! The cursor and the resolved drop target are refreshed by the timeline
//! canvas as the pointer moves (it owns the pixel↔sample geometry); this
//! module only holds the values so the draw pass — and the golden-image
//! test — can render a deterministic snapshot of the drag state.

use std::path::PathBuf;

use iced::Point;
use resonance_common::audio_probe::AudioFileEntry;

use crate::message::DropTarget;

/// A file lifted out of the media browser and being dragged onto the
/// timeline. Captured once at drag-start so the preview never has to touch
/// the filesystem again mid-gesture.
#[derive(Debug, Clone, PartialEq)]
pub struct DraggedAsset {
    /// Absolute source path — handed to the placement orchestration on drop.
    pub path: PathBuf,
    /// Display name (the file stem), shown on the drag pill and used as the
    /// placed clip's name.
    pub name: String,
    /// The source file's sample rate, so the tooltip can flag a conversion.
    pub source_sample_rate: u32,
    /// Clip length in **project** frames (the source frame count rescaled
    /// to the project rate), used to size the dashed ghost clip.
    pub duration_samples: u64,
    /// Pre-computed conversion note (e.g. `"→ 48 kHz"`) shown in the drop
    /// tooltip when the source rate differs from the project rate; `None`
    /// when the rates already match and nothing is converted.
    pub conversion: Option<String>,
}

impl DraggedAsset {
    /// Build a dragged-asset from a browser file row, rescaling its length
    /// to `project_sample_rate` and computing the conversion note.
    pub fn from_entry(entry: &AudioFileEntry, project_sample_rate: u32) -> Self {
        let info = &entry.info;
        let name = std::path::Path::new(&entry.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&entry.path)
            .to_string();
        let duration_samples = rescale_frames(
            info.frames,
            info.sample_rate,
            project_sample_rate,
        );
        let conversion = conversion_note(info.sample_rate, project_sample_rate);
        Self {
            path: PathBuf::from(&entry.path),
            name,
            source_sample_rate: info.sample_rate,
            duration_samples,
            conversion,
        }
    }
}

/// Rescale a source frame count to a target sample rate. Returns `frames`
/// unchanged when the rates match (or either is zero), otherwise
/// `frames * target / source` in f64 to avoid overflow on long files.
pub fn rescale_frames(frames: u64, source_rate: u32, target_rate: u32) -> u64 {
    if source_rate == 0 || target_rate == 0 || source_rate == target_rate {
        return frames;
    }
    (frames as f64 * target_rate as f64 / source_rate as f64).round() as u64
}

/// The tooltip conversion note for a source rate against the project rate,
/// or `None` when they match. Rendered as `"→ 48 kHz"`.
pub fn conversion_note(source_rate: u32, project_rate: u32) -> Option<String> {
    if source_rate == 0 || project_rate == 0 || source_rate == project_rate {
        return None;
    }
    Some(format!("→ {} kHz", khz_label(project_rate)))
}

/// Format a sample rate in kHz, dropping a trailing `.0` (48000 → "48",
/// 44100 → "44.1").
fn khz_label(rate: u32) -> String {
    let khz = rate as f32 / 1000.0;
    if (khz.fract()).abs() < f32::EPSILON {
        format!("{}", khz as u32)
    } else {
        // One decimal is enough for the common 44.1 / 88.2 rates.
        let s = format!("{khz:.1}");
        s.trim_end_matches(".0").to_string()
    }
}

/// The drop target resolved from the current cursor position against the
/// timeline geometry (which lane, which grid-snapped sample). Recomputed by
/// the timeline canvas on every pointer move while a drag is active.
#[derive(Debug, Clone, PartialEq)]
pub struct DropResolution {
    /// Where a drop right now would place the clip — an existing lane or the
    /// new-audio-track zone. Handed straight to the placement orchestration.
    pub target: DropTarget,
    /// Arrange-row index of the targeted existing lane, or `None` when the
    /// cursor is over the "create a new audio track" drop zone. Drives which
    /// lane lights up.
    pub lane_index: Option<usize>,
    /// Human-readable snapped position, e.g. `"Bar 5.1"` (bar.beat, both
    /// 1-based). Shown in the drop tooltip.
    pub bar_label: String,
}

/// A drag-to-timeline placement in flight. Present only between drag-start
/// and the drop / cancel that ends the gesture.
#[derive(Debug, Clone, PartialEq)]
pub struct DragPlacement {
    /// The file being dragged.
    pub asset: DraggedAsset,
    /// Cursor position in **timeline-canvas content coordinates** (the same
    /// space `sample_to_x` works in). The drag pill + tooltip anchor here.
    pub cursor: Point,
    /// The resolved drop target for `cursor`, or `None` before the first
    /// pointer move has landed the drag over the lanes.
    pub resolved: Option<DropResolution>,
}

impl DragPlacement {
    /// Start a drag for `asset` with no resolved target yet.
    pub fn new(asset: DraggedAsset) -> Self {
        Self {
            asset,
            cursor: Point::ORIGIN,
            resolved: None,
        }
    }
}
