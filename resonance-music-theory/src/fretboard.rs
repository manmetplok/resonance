//! Fretboard voicing computation for any stringed instrument.
//!
//! Given a [`Chord`] and a tuning (array of open-string MIDI notes),
//! compute playable fret positions. The algorithm prefers open-position
//! voicings with the root on the lowest sounding string.

use crate::chord::Chord;

// -- Tunings -----------------------------------------------------------------

/// 6-string guitar, standard tuning.
pub const GUITAR_6: Tuning = Tuning {
    name: "Guitar (6-string)",
    short: "Guitar 6",
    open: &[40, 45, 50, 55, 59, 64], // E2 A2 D3 G3 B3 E4
    labels: &["E", "A", "D", "G", "B", "e"],
};

/// 8-string guitar, standard tuning.
pub const GUITAR_8: Tuning = Tuning {
    name: "Guitar (8-string)",
    short: "Guitar 8",
    open: &[30, 35, 40, 45, 50, 55, 59, 64], // F#1 B1 E2 A2 D3 G3 B3 E4
    labels: &["F#", "B", "E", "A", "D", "G", "B", "e"],
};

/// 4-string bass, standard tuning.
pub const BASS_4: Tuning = Tuning {
    name: "Bass (4-string)",
    short: "Bass 4",
    open: &[28, 33, 38, 43], // E1 A1 D2 G2
    labels: &["E", "A", "D", "G"],
};

/// 5-string bass, standard tuning.
pub const BASS_5: Tuning = Tuning {
    name: "Bass (5-string)",
    short: "Bass 5",
    open: &[23, 28, 33, 38, 43], // B0 E1 A1 D2 G2
    labels: &["B", "E", "A", "D", "G"],
};

/// All tunings in display order.
pub const ALL_TUNINGS: &[&Tuning] = &[&GUITAR_6, &GUITAR_8, &BASS_4, &BASS_5];

// -- Types -------------------------------------------------------------------

/// An instrument tuning: open-string MIDI notes and display labels.
pub struct Tuning {
    pub name: &'static str,
    pub short: &'static str,
    pub open: &'static [u8],
    pub labels: &'static [&'static str],
}

impl Tuning {
    pub fn string_count(&self) -> usize {
        self.open.len()
    }
}

/// Chord voicing on a fretboard: one fret per string, `None` = muted.
#[derive(Debug, Clone)]
pub struct FretboardVoicing {
    pub frets: Vec<Option<u8>>,
    pub start_fret: u8,
}

// -- Voicing -----------------------------------------------------------------

/// Compute a playable chord voicing for the given tuning.
pub fn voicing(chord: &Chord, tuning: &Tuning) -> FretboardVoicing {
    let pcs: Vec<u8> = chord.pitch_classes().iter().map(|pc| pc.to_semitone()).collect();
    let root_pc = chord.bass.unwrap_or(chord.root).to_semitone();
    let n = tuning.string_count();

    let mut best_frets = vec![None; n];
    let mut best_start = 0u8;
    let mut best_score = 0i32;

    for start in 0..=7u8 {
        let mut frets = vec![None; n];
        let mut score = 0i32;

        for (s, &open) in tuning.open.iter().enumerate() {
            let mut best_for_string: Option<u8> = None;
            let mut best_cost = u8::MAX;

            // Open string
            if pcs.contains(&(open % 12)) {
                best_for_string = Some(0);
                best_cost = 0;
            }

            // Frets in window
            let lo = start.max(1);
            let hi = start + 4;
            for fret in lo..=hi {
                let note_pc = (open + fret) % 12;
                if pcs.contains(&note_pc) && fret < best_cost {
                    best_for_string = Some(fret);
                    best_cost = fret;
                }
            }

            frets[s] = best_for_string;
            if best_for_string.is_some() {
                score += 1;
            }
        }

        if score > best_score || (score == best_score && start < best_start) {
            best_score = score;
            best_frets = frets;
            best_start = start;
        }
    }

    // Mute strings below the root
    if let Some(root_idx) = best_frets.iter().enumerate().position(|(s, f)| {
        f.map(|fret| (tuning.open[s] + fret) % 12 == root_pc).unwrap_or(false)
    }) {
        for slot in best_frets.iter_mut().take(root_idx) {
            *slot = None;
        }
    }

    let actual_start = best_frets.iter().filter_map(|f| *f).filter(|&f| f > 0).min().unwrap_or(0);
    let start_fret = if actual_start <= 1 { 0 } else { actual_start };

    FretboardVoicing { frets: best_frets, start_fret }
}

