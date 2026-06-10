//! Drum pad mapping used by the Compose drumroll view.
//!
//! Note numbers come from the shared GM contract in
//! `resonance_common::drum_map`, which the `resonance-drums` plugin also
//! consumes; this module only picks which pads the drumroll shows and in
//! what order.
//!
//! The map is a runtime value (not a `const`) so a future `load_from_file`
//! loader can replace the default at a single construction site in
//! `DrumrollViewState::default`.

use resonance_common::drum_map as gm;

/// One pad in the drumroll grid.
#[derive(Debug, Clone)]
pub struct DrumPad {
    /// General MIDI drum note number (36 = kick, 38 = snare, …).
    pub note: u8,
}

/// Ordered list of pads shown top-to-bottom in each drum track row.
#[derive(Debug, Clone)]
pub struct DrumPadMap {
    pub pads: Vec<DrumPad>,
}

impl DrumPadMap {
    /// The built-in 12-pad General MIDI layout.
    pub fn default_map() -> Self {
        Self {
            pads: [
                gm::KICK,
                gm::SNARE,
                gm::RIMSHOT,
                gm::SNARE_SIDESTICK,
                gm::HIHAT_CLOSED,
                gm::HIHAT_OPEN,
                gm::TOM_HIGH,
                gm::TOM_MID,
                gm::TOM_LOW,
                gm::CRASH_16_EDGE,
                gm::RIDE_EDGE,
                gm::COWBELL,
            ]
            .into_iter()
            .map(|note| DrumPad { note })
            .collect(),
        }
    }

    pub fn get(&self, index: usize) -> Option<&DrumPad> {
        self.pads.get(index)
    }
}

impl Default for DrumPadMap {
    fn default() -> Self {
        Self::default_map()
    }
}
