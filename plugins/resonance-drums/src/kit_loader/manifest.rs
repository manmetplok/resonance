//! Drumkit manifest parsing types.
//!
//! Mirrors the shape observed in drummica's `drum_samples.json` and
//! provides the per-pad user-mic-choice container that the editor
//! persists through plugin state.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Top-level: drum piece name -> map of mic-setup name -> mic setup data.
pub type KitManifest = BTreeMap<String, BTreeMap<String, MicSetup>>;

#[derive(Deserialize)]
#[allow(dead_code)] // brand/channel/mic fields come from the manifest but we only use `position` + `rounds`
pub struct MicSetup {
    pub brand: String,
    pub channel: String,
    pub mic: String,
    pub position: String,
    /// RR name -> velocity name -> relative filename.
    pub rounds: BTreeMap<String, BTreeMap<String, String>>,
}

// ---------------------------------------------------------------------------
// Per-pad mic-choice state — kept outside the loader so the editor can
// persist it via ExtraStateSaver.
// ---------------------------------------------------------------------------

/// User-chosen setup keys per close-mic position for one pad. If an
/// entry is missing the loader picks the first available setup for that
/// position from the manifest.
#[derive(Debug, Clone, Default)]
pub struct PadMicChoices {
    pub close_setups: BTreeMap<String, String>,
}

/// Parse a "VelNN" key into its numeric suffix.
#[doc(hidden)]
pub fn parse_vel_index(key: &str) -> Option<u32> {
    let digits = key.strip_prefix("Vel")?;
    digits.parse().ok()
}
