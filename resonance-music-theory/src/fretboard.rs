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
    /// Display anchor for a chord-diagram renderer, not the literal
    /// lowest fretted fret: `0` means "render from the nut" and every
    /// fretted note fits in frets `1..=WINDOW_FRETS`; any other value
    /// `s` means "render a boxed window" with fretted notes in
    /// `s..=s + WINDOW_FRETS - 1`.
    ///
    /// Voicings whose lowest fretted note is fret 1 are deliberately
    /// anchored to the nut (`start_fret == 0`) — that is the standard
    /// chord-chart convention (open C and E major both finger fret 1;
    /// an F barre is likewise drawn at the nut), and open strings
    /// (`Some(0)` in [`frets`](Self::frets)) only make sense against a
    /// drawn nut. Whether fret 1 is fingered or open is distinguished
    /// by `frets` (`Some(1)` vs `Some(0)`), never by `start_fret`.
    pub start_fret: u8,
}

// -- Voicing -----------------------------------------------------------------

/// Width of the fret window a voicing may span: every fretted note of
/// a [`FretboardVoicing`] lies within `WINDOW_FRETS` consecutive frets
/// anchored at [`FretboardVoicing::start_fret`] (treating the nut
/// anchor `0` as fret 1). Four frets is the standard chord-box height
/// and the comfortable hand span; renderers can rely on it.
pub const WINDOW_FRETS: u8 = 4;

/// Highest window start the voicing search will consider.
///
/// Search windows are [`WINDOW_FRETS`] wide and repeat their
/// pitch-class content every 12 frets, so starts `0..=11` already
/// cover every distinct chord shape; allowing up to 15 makes those
/// shapes reachable an octave up (top fret 18 — within the fretted
/// range of every tuning in [`ALL_TUNINGS`], and the chord-diagram
/// renderer labels any `start_fret`).
pub const MAX_START_FRET: u8 = 15;

/// Compute a playable chord voicing for the given tuning, preferring
/// the open position (lowest playable window).
pub fn voicing(chord: &Chord, tuning: &Tuning) -> FretboardVoicing {
    voicing_from(chord, tuning, 0)
}

/// Like [`voicing`], but search no window lower than `min_start`
/// (clamped to [`MAX_START_FRET`]) — this is how upper-register
/// variations of a chord are reached, e.g. `min_start = 5` yields the
/// E-shape A-major barre at fret 5 and `min_start = 12` its
/// second-octave shapes above fret 11.
///
/// When `min_start > 0` the result is fully fretted (barre-style):
/// open strings are not considered, since a zero-cost open string
/// would otherwise always beat the fretted note the caller asked to
/// be voiced up the neck.
pub fn voicing_from(chord: &Chord, tuning: &Tuning, min_start: u8) -> FretboardVoicing {
    let pcs: Vec<u8> = chord.pitch_classes().map(|pc| pc.to_semitone()).collect();
    let root_pc = chord.bass.unwrap_or(chord.root).to_semitone();
    let n = tuning.string_count();
    let min_start = min_start.min(MAX_START_FRET);

    let mut best_frets = vec![None; n];
    let mut best_start = min_start;
    let mut best_score = 0i32;

    for start in min_start..=MAX_START_FRET {
        let mut frets = vec![None; n];
        let mut score = 0i32;

        for (s, &open) in tuning.open.iter().enumerate() {
            let mut best_for_string: Option<u8> = None;
            let mut best_cost = u8::MAX;

            // Open string (only in an open-position search)
            if min_start == 0 && pcs.contains(&(open % 12)) {
                best_for_string = Some(0);
                best_cost = 0;
            }

            // Frets in window
            let lo = start.max(1);
            let hi = start + WINDOW_FRETS - 1;
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

    // Anchor fret-1 voicings at the nut (see `FretboardVoicing::start_fret`
    // for why this collapse is intentional). It is lossless for a
    // `WINDOW_FRETS`-tall nut-anchored diagram: the winning window was
    // either `start = 0` (frets 1..=WINDOW_FRETS - 1 plus opens) or
    // `start = 1` (frets 1..=WINDOW_FRETS), so every fretted note still
    // fits frets 1..=WINDOW_FRETS when drawn from the nut.
    let actual_start = best_frets.iter().filter_map(|f| *f).filter(|&f| f > 0).min().unwrap_or(0);
    let start_fret = if actual_start <= 1 { 0 } else { actual_start };

    FretboardVoicing { frets: best_frets, start_fret }
}

