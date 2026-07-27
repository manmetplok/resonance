//! Performance-mode footer selections (epic #11, todo #311, design #151).
//!
//! Performance mode draws live fingering diagrams for the chord under the
//! playhead. Which instrument those diagrams are drawn for — and whether a
//! capo is in play — is a per-session choice surfaced on the footer strip:
//! a segmented instrument/tuning selector over [`ALL_TUNINGS`] and a `Capo`
//! stepper. This struct holds that selection; the footer controls mutate it
//! (via `UiMessage::SetPerformanceTuning` / `SetPerformanceCapo`, routed
//! through `update::ui`) and the diagram todos (#308/#309) read it.
//!
//! ## Capo handling
//!
//! A capo at fret `c` raises every open string by `c` semitones and makes
//! fret `c` the lowest playable position. Rather than introduce a new
//! music-theory primitive (arch doc #152 flags that as optional), the capo
//! is applied app-side here by voicing the chord with the window search
//! pinned to start at the capo: [`fretboard_voicing_from`] with
//! `min_start = capo` only considers frets `>= capo`, so the returned
//! voicing is exactly what a player can fret above the capo, in absolute
//! fret numbers. Because the frets stay absolute (measured from the real
//! nut) against the real tuning's open notes, a renderer computes the
//! correct *sounding* note for every dot with no further adjustment —
//! `tuning.open[i] + fret` is the true pitch. With no capo the open
//! position is used, so open strings (`Some(0)`) are still available.

use resonance_music_theory::{
    fretboard_voicing, fretboard_voicing_from, Chord, FretboardVoicing, Tuning, ALL_TUNINGS,
};

/// Highest capo position the stepper allows, in frets. A capo at the 12th
/// fret already transposes the open strings a full octave; positions beyond
/// that are not musically useful and would push voicings past the diagram
/// window, so the stepper clamps here.
pub const MAX_CAPO: u8 = 12;

/// The instrument/tuning + capo selection backing the Performance-mode
/// footer and the live fingering diagrams.
///
/// The derived [`Default`] (`tuning_index: 0`, `capo: 0`) is Guitar 6 with no
/// capo — the first entry in [`ALL_TUNINGS`] and the footer scaffold's
/// default active cell (#307).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PerformanceState {
    /// Index into [`ALL_TUNINGS`] of the active instrument tuning. Always
    /// kept in range by [`set_tuning_index`](Self::set_tuning_index); read
    /// the resolved tuning with [`tuning`](Self::tuning).
    pub tuning_index: usize,
    /// Capo position in frets (`0` = no capo). Clamped to `0..=MAX_CAPO` by
    /// [`set_capo`](Self::set_capo).
    pub capo: u8,
}

impl PerformanceState {
    /// The active tuning. Defensively clamps the stored index so a stale
    /// value can never panic the diagram renderer.
    pub fn tuning(&self) -> &'static Tuning {
        let i = self.tuning_index.min(ALL_TUNINGS.len().saturating_sub(1));
        ALL_TUNINGS[i]
    }

    /// Select the instrument/tuning by [`ALL_TUNINGS`] index. Out-of-range
    /// indices are ignored (the selection is left untouched) so a bad
    /// message can't desync the footer from the diagram. The capo is left
    /// as-is — switching instruments keeps the player's capo position.
    pub fn set_tuning_index(&mut self, index: usize) {
        if index < ALL_TUNINGS.len() {
            self.tuning_index = index;
        }
    }

    /// Set the capo position, clamped to `0..=MAX_CAPO`.
    pub fn set_capo(&mut self, frets: u8) {
        self.capo = frets.min(MAX_CAPO);
    }

    /// A playable voicing of `chord` on the active tuning with the capo
    /// applied. See the module docs for why the capo is realised as a
    /// pinned window start rather than a music-theory change. Returns
    /// absolute fret numbers (measured from the real nut), so the caller
    /// renders it against the real [`tuning`](Self::tuning) open notes.
    pub fn voicing(&self, chord: &Chord) -> FretboardVoicing {
        let tuning = self.tuning();
        if self.capo == 0 {
            fretboard_voicing(chord, tuning)
        } else {
            fretboard_voicing_from(chord, tuning, self.capo)
        }
    }
}
