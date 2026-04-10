use resonance_audio::types::ClipId;

use super::humanize::{AccentPattern, HumanizeScope};

/// Messages produced by the Compose drumroll view. Routed through
/// `ComposeMessage::Drumroll` → `crate::update::drumroll::handle`.
#[derive(Debug, Clone)]
pub enum DrumrollMessage {
    /// User clicked a step cell. Toggles the hit on/off: if an existing
    /// note for this pad lies in this step it is removed, otherwise a new
    /// note is added at the snapped tick.
    ToggleStep {
        clip_id: ClipId,
        pad_index: usize,
        step: u32,
    },
    /// Sidebar: pick a pad to focus (euclidean Apply, Clear pad, etc.
    /// target the selected pad).
    SelectPad { pad_index: usize },
    /// Change the step-grid resolution. Only {4, 8, 16, 32} are accepted.
    SetStepsPerBar(u32),
    /// Velocity used for newly added hits on the selected pad.
    SetDefaultVelocity(f32),
    /// Buffered text inputs for the euclidean form.
    SetEuclidSteps(String),
    SetEuclidHits(String),
    SetEuclidRotation(String),
    /// Apply the current euclidean parameters to the selected pad on the
    /// given clip. Replaces any existing hits on that pad.
    GenerateEuclideanPad {
        clip_id: ClipId,
        pad_index: usize,
    },
    /// Remove every note on the given pad in the given clip.
    ClearPad {
        clip_id: ClipId,
        pad_index: usize,
    },

    // --- Humanizer sliders + pickers ---
    SetHumanizeVelocity(f32),
    SetHumanizeTiming(f32),
    SetHumanizeSwing(f32),
    SetHumanizeAccent(AccentPattern),
    SetHumanizeAccentAmount(f32),
    SetHumanizeScope(HumanizeScope),
    /// Apply the humanizer to every in-scope note in the clip. Notes are
    /// re-snapped to the grid first so repeated Apply clicks produce
    /// independent realizations rather than compounding drift.
    ApplyHumanize { clip_id: ClipId },
}
