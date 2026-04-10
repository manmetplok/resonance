/// Drum pad mapping used by the Compose drumroll view.
///
/// Deliberately mirrors the General MIDI layout shipped by the
/// `resonance-drums` plugin (kick, snare, hats, toms, cymbals, rim/clap/cowbell)
/// but does not depend on that crate — both sides independently target GM.
///
/// The map is a runtime value (not a `const`) so a future `load_from_file`
/// loader can replace the default at a single construction site in
/// `DrumrollViewState::default`.

/// One pad in the drumroll grid.
#[derive(Debug, Clone)]
pub struct DrumPad {
    /// General MIDI drum note number (36 = kick, 38 = snare, …).
    pub note: u8,
    pub name: &'static str,
    /// Cell tint hue in linear sRGB; groups of related pads share a hue so
    /// the user can eyeball the grid layout.
    pub color: [f32; 3],
}

/// Ordered list of pads shown top-to-bottom in each drum track row.
#[derive(Debug, Clone)]
pub struct DrumPadMap {
    pub pads: Vec<DrumPad>,
}

impl DrumPadMap {
    /// The built-in 12-pad General MIDI layout.
    pub fn default_map() -> Self {
        // Color groups:
        //   kick/snare/rim/clap = drum red
        //   hats                = hat yellow
        //   toms                = tom orange
        //   cymbals             = cymbal cyan
        //   cowbell             = misc purple
        let kit = [0.88, 0.38, 0.34];
        let hat = [0.92, 0.82, 0.36];
        let tom = [0.92, 0.56, 0.28];
        let cym = [0.42, 0.78, 0.90];
        let misc = [0.74, 0.54, 0.92];
        Self {
            pads: vec![
                DrumPad { note: 36, name: "Kick", color: kit },
                DrumPad { note: 38, name: "Snare", color: kit },
                DrumPad { note: 37, name: "Rimshot", color: kit },
                DrumPad { note: 39, name: "Clap", color: kit },
                DrumPad { note: 42, name: "Hi-Hat Closed", color: hat },
                DrumPad { note: 46, name: "Hi-Hat Open", color: hat },
                DrumPad { note: 50, name: "Tom High", color: tom },
                DrumPad { note: 47, name: "Tom Mid", color: tom },
                DrumPad { note: 45, name: "Tom Low", color: tom },
                DrumPad { note: 49, name: "Crash", color: cym },
                DrumPad { note: 51, name: "Ride", color: cym },
                DrumPad { note: 56, name: "Cowbell", color: misc },
            ],
        }
    }

    pub fn len(&self) -> usize {
        self.pads.len()
    }

    pub fn get(&self, index: usize) -> Option<&DrumPad> {
        self.pads.get(index)
    }

    /// Reverse lookup: find the pad index for a given MIDI note. Returns
    /// `None` if the note isn't in the map (caller should skip unmapped
    /// notes rather than guess).
    pub fn index_for_note(&self, note: u8) -> Option<usize> {
        self.pads.iter().position(|p| p.note == note)
    }
}

impl Default for DrumPadMap {
    fn default() -> Self {
        Self::default_map()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_map_has_twelve_pads() {
        assert_eq!(DrumPadMap::default_map().len(), 12);
    }

    #[test]
    fn kick_is_first() {
        let map = DrumPadMap::default_map();
        assert_eq!(map.get(0).unwrap().note, 36);
        assert_eq!(map.get(0).unwrap().name, "Kick");
    }

    #[test]
    fn index_for_note_roundtrip() {
        let map = DrumPadMap::default_map();
        for (i, pad) in map.pads.iter().enumerate() {
            assert_eq!(map.index_for_note(pad.note), Some(i));
        }
    }

    #[test]
    fn unmapped_note_returns_none() {
        let map = DrumPadMap::default_map();
        // 60 = middle C, not a GM drum
        assert_eq!(map.index_for_note(60), None);
    }
}
