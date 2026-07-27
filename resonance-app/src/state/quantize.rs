//! Project-scoped MIDI quantize state: the user's groove library and the
//! last-used quantize / humanize settings (ba todo #395).
//!
//! Both halves persist in the project file. The runtime types carry serde
//! derives and are written into [`crate::project::ProjectFile`] directly —
//! exactly like `DrumGroup` and the tempo / signature events — so they
//! ride the save/load round-trip *and* the undo snapshot (which is built
//! from the same `ProjectFile` shape) without a parallel on-disk mirror.

use serde::{Deserialize, Serialize};

use resonance_audio::quantize::{Division, GridValue, GrooveTemplate, QuantizeMode};

/// One user-extracted groove template in the project's groove library,
/// addressed by a stable per-project id.
///
/// Stock grooves are **not** stored here — they live in code
/// ([`resonance_audio::quantize::stock_grooves`]) and are referenced by
/// index via [`GrooveSelection::Stock`], so a project file never
/// duplicates a built-in groove's data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserGroove {
    /// Stable per-project id, referenced by [`GrooveSelection::User`].
    pub id: u64,
    /// User-facing display name.
    pub name: String,
    /// The extracted feel.
    pub template: GrooveTemplate,
}

/// Which groove the last quantize / groove apply targeted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum GrooveSelection {
    /// Plain grid quantize — no groove feel applied.
    None,
    /// A built-in stock groove, addressed by its index into
    /// [`resonance_audio::quantize::stock_grooves`].
    Stock { index: usize },
    /// A user-extracted groove, addressed by [`UserGroove::id`].
    User { id: u64 },
}

impl Default for GrooveSelection {
    fn default() -> Self {
        GrooveSelection::None
    }
}

/// Last-used quantize + humanize settings, restored as the quantize
/// panel's defaults when the user reopens it (and persisted so they
/// survive save / load and undo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuantizeSettings {
    /// Grid the notes snap to.
    pub division: Division,
    /// Blend toward the grid, `0.0..=1.0` (`1.0` snaps exactly).
    pub strength: f32,
    /// Swing applied to odd grid steps, `0.0..=1.0`.
    pub swing: f32,
    /// Whether starts only, or starts + length, are quantized.
    pub mode: QuantizeMode,
    /// Snap note-offs to the grid as well as note-ons.
    pub quantize_ends: bool,
    /// Apply the strength blend iteratively for a softer pull.
    pub iterative: bool,
    /// Humanize timing jitter — maximum absolute offset, in ticks.
    pub humanize_timing_ticks: u32,
    /// Humanize velocity jitter fraction, `0.0..=1.0`.
    pub humanize_velocity: f32,
    /// Groove feel selected for the last apply.
    pub groove: GrooveSelection,
    /// Strength of the groove feel, `0.0..=1.0`.
    pub groove_strength: f32,
}

impl Default for QuantizeSettings {
    fn default() -> Self {
        QuantizeSettings {
            division: Division::straight(GridValue::Sixteenth),
            strength: 1.0,
            swing: 0.0,
            mode: QuantizeMode::StartOnly,
            quantize_ends: false,
            iterative: false,
            humanize_timing_ticks: 0,
            humanize_velocity: 0.0,
            groove: GrooveSelection::None,
            groove_strength: 1.0,
        }
    }
}

/// Project-scoped quantize state: the user's extracted groove library plus
/// the last-used quantize / humanize settings. Held on
/// `Resonance::quantize`, rebuilt on project load; both halves persist in
/// the project file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuantizeState {
    /// User-extracted groove templates, in insertion order.
    pub groove_library: Vec<UserGroove>,
    /// Last-used quantize / humanize settings.
    pub settings: QuantizeSettings,
}

impl QuantizeState {
    /// The smallest unused groove id (max existing id + 1, or `0` when the
    /// library is empty). Lets an "extract groove" handler mint a stable
    /// id without threading a separate counter across save / load.
    pub fn next_groove_id(&self) -> u64 {
        self.groove_library
            .iter()
            .map(|g| g.id)
            .max()
            .map_or(0, |m| m + 1)
    }

    /// Look up a user groove by id.
    pub fn user_groove(&self, id: u64) -> Option<&UserGroove> {
        self.groove_library.iter().find(|g| g.id == id)
    }
}
