//! Inversion decorations for generated progressions (research §2C).
//!
//! Runs after the Markov walk and the harmonic-rhythm split pass, and
//! rewrites the sampled (never the locked) material with the classical
//! pre-dominant bass idioms that root-position-only harmony cannot
//! express:
//!
//! - **IV precedes ii** — when an accelerated predominant slot came out
//!   as `ii IV`, the halves are swapped to the normative `IV ii`
//!   ordering (predominants intensify toward the dominant, and IV → ii
//!   is common while ii → IV is rare).
//! - **ii6** — a root-position supertonic directly before a
//!   root-position V is rendered in first inversion with seeded
//!   probability, putting scale degree 4 in the bass so the bass walks
//!   4 → 5 into the cadence (and holds when IV came right before).
//! - **Cadential 6/4** — a phrase-final V is decorated with the tonic
//!   triad in second inversion on the front half of its slot, using the
//!   existing [`SplitChord`] machinery: the 6/4 lands on the bar's
//!   downbeat over the dominant bass and resolves to V on the weak
//!   half. The same-voice 6→5 / 4→3 resolutions are enforced by the
//!   SATB voice-leading pass (`crate::satb`).
//!
//! All decisions draw from the generator's seeded RNG, so the pass is
//! as deterministic as the walk itself.

use crate::chord::ChordQuality;
use crate::rng::XorShift;

use super::degree::Degree;
use super::table::{HarmonicFunction, MarkovTable};
use super::{GeneratedChord, SplitChord};

/// Probability that a root-position ii directly before a root-position
/// V is rendered in first inversion (ii6). High because classical
/// practice prefers the first-inversion supertonic at cadences, but not
/// 1.0 so root-position ii still appears.
const P_FIRST_INVERSION_PREDOMINANT: f32 = 0.6;

/// Probability that an eligible phrase-final V is decorated with a
/// cadential 6/4. Moderate: it is a strong classical marker and should
/// color cadences, not stamp every one.
const P_CADENTIAL_SIX_FOUR: f32 = 0.35;

/// Apply the inversion decorations described in the module docs.
///
/// `prefixed` marks slots fixed before the walk (locks and start/end
/// constraints) — those are never rewritten. `plans` are the per-slot
/// function-level windows from the phrase-model overlay; a `(2, 2)`
/// window marks the cadential-dominant slot of its phrase.
pub(super) fn decorate_inversions(
    chords: &mut [GeneratedChord],
    splits: &mut Vec<SplitChord>,
    prefixed: &[Option<Degree>],
    plans: &[(u8, u8)],
    table: &MarkovTable,
    rng: &mut XorShift,
) {
    // --- 1. IV precedes ii ----------------------------------------------
    for split in splits.iter_mut() {
        let slot = split.slot as usize;
        if prefixed.get(slot).is_some_and(|p| p.is_some()) {
            continue;
        }
        let front = chords[slot].degree;
        if is_predominant_on_root(table, front, 2) && is_predominant_on_root(table, split.degree, 4)
        {
            chords[slot].degree = split.degree;
            split.degree = front;
        }
    }

    // --- 2. ii6 before V --------------------------------------------------
    // The chord *adjacent* to the dominant gets the inversion: the back
    // half when the slot was split, the whole slot otherwise.
    for slot in 0..chords.len() {
        let next_is_v = chords
            .get(slot + 1)
            .is_some_and(|c| is_root_position_v(table, c.degree));
        if !next_is_v {
            continue;
        }
        if let Some(split) = splits.iter_mut().find(|s| s.slot as usize == slot) {
            if is_predominant_on_root(table, split.degree, 2)
                && rng.next_f32() < P_FIRST_INVERSION_PREDOMINANT
            {
                split.degree = split.degree.with_inversion(1);
            }
        } else if prefixed[slot].is_none()
            && is_predominant_on_root(table, chords[slot].degree, 2)
            && rng.next_f32() < P_FIRST_INVERSION_PREDOMINANT
        {
            chords[slot].degree = chords[slot].degree.with_inversion(1);
        }
    }

    // --- 3. Cadential 6/4 ---------------------------------------------------
    for slot in 0..chords.len() {
        if plans.get(slot) != Some(&(2, 2)) || prefixed[slot].is_some() {
            continue;
        }
        if splits.iter().any(|s| s.slot as usize == slot) {
            continue;
        }
        let dominant = chords[slot].degree;
        if !is_root_position_v(table, dominant) {
            continue;
        }
        let Some(tonic) = cadential_tonic(table) else {
            continue;
        };
        if rng.next_f32() >= P_CADENTIAL_SIX_FOUR {
            continue;
        }
        chords[slot].degree = tonic.with_inversion(2);
        splits.push(SplitChord {
            slot: slot as u8,
            degree: dominant,
        });
    }
    splits.sort_by_key(|s| s.slot);
}

/// Is `degree` a root-position predominant on the given scale-degree
/// root? (Quality-agnostic: ii, ii7, ii° and IV, iv, IVΔ7 all count.)
fn is_predominant_on_root(table: &MarkovTable, degree: Degree, root: u8) -> bool {
    degree.root == root
        && !degree.flat
        && degree.inversion == 0
        && table.function_of(degree) == HarmonicFunction::Predominant
}

/// Is `degree` a root-position dominant on scale degree 5 with a
/// major-type quality? The classical cadence idioms (bass 4→5, the
/// cadential 6/4) target a real V — never the subtonic bVII or vii°.
fn is_root_position_v(table: &MarkovTable, degree: Degree) -> bool {
    degree.root == 5
        && !degree.flat
        && degree.inversion == 0
        && matches!(degree.quality, ChordQuality::Maj | ChordQuality::Dom7)
        && table.function_of(degree) == HarmonicFunction::Dominant
}

/// The tonic triad used for the cadential 6/4, derived from the table's
/// own root-position tonic so minor tables decorate with i6/4 and major
/// tables with I6/4. `None` when the table has no usable degree-1 tonic
/// (the decoration is then skipped).
fn cadential_tonic(table: &MarkovTable) -> Option<Degree> {
    let tonic = table.degrees().into_iter().find(|d| {
        d.root == 1
            && !d.flat
            && d.inversion == 0
            && table.function_of(*d) == HarmonicFunction::Tonic
    })?;
    let quality = match tonic.quality {
        ChordQuality::Maj | ChordQuality::Maj7 | ChordQuality::Maj6 | ChordQuality::Add9 => {
            ChordQuality::Maj
        }
        ChordQuality::Min | ChordQuality::Min7 | ChordQuality::MinMaj7 | ChordQuality::Min6 => {
            ChordQuality::Min
        }
        _ => return None,
    };
    Some(Degree {
        root: 1,
        flat: false,
        quality,
        inversion: 0,
    })
}
