//! Drum pad mapping used by the Compose drumroll view.
//!
//! Deliberately mirrors the General MIDI layout shipped by the
//! `resonance-drums` plugin (kick, snare, hats, toms, cymbals, rim/clap/cowbell)
//! but does not depend on that crate — both sides independently target GM.
//!
//! The map is a runtime value (not a `const`) so a future `load_from_file`
//! loader can replace the default at a single construction site in
//! `DrumrollViewState::default`.

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
            pads: [36, 38, 37, 39, 42, 46, 50, 47, 45, 49, 51, 56]
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
