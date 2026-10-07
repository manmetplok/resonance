//! The melisma pass — `SyllableMode` Mixed and Melismatic.
//!
//! One responsibility: after the syllabic melody is final, give some
//! syllables 2–3 notes by splitting their note into a short ornament
//! (vocal-mcp-control-and-quality.md §6). Every note after the first of
//! a split is a generator-made hold. Those holds are reported in
//! [`VocalDerivation::auto_holds`] and never written into the lyrics:
//! the app stores them in the clip's per-note slur side table, so a
//! regenerate replaces them along with the melody.
//!
//! Splitting keeps each syllable's time span, so the line layout, the
//! phrase spans and the rests are unchanged; only the notes inside a
//! melisma are new. Syllabic runs no pass and draws nothing, so its
//! output is bit-identical to [`derive_vocal_with_motif`].

use crate::derive::vocal::params::{SyllableMode, VocalParams, VocalStyle};
use crate::derive::vocal::derive_vocal_with_motif;
use crate::derive::{GeneratedNote, LyricLine, TimedChord};
use crate::g2p::{resolve_draft, SyllableStress};
use crate::rng::XorShift;
use crate::scale::Scale;

use super::primitives::scale_from_chords;
use super::syllables::count_syllables;

/// A vocal melody plus which of its notes are generator-made holds.
#[derive(Debug, Clone, PartialEq)]
pub struct VocalDerivation {
    /// The notes, in lyric order.
    pub notes: Vec<GeneratedNote>,
    /// Parallel to `notes`: `true` on a note that holds the previous
    /// note's syllable because the generator made a melisma there (not
    /// because the lyrics have a `_`). Never set on a line's first note.
    pub auto_holds: Vec<bool>,
}

/// [`derive_vocal_with_motif`], then the `params.syllable_mode` melisma
/// pass. Deterministic per `seed`.
///
/// * **Syllabic** — one note per lyric unit; no auto holds.
/// * **Mixed** — at most one melisma per line: the line's last syllable
///   about half the time, otherwise sometimes a long stressed one.
/// * **Melismatic** — up to three per line, more of them three notes
///   long.
pub fn derive_vocal_with_holds(
    chords: &[TimedChord],
    params: &VocalParams,
    ticks_per_beat: u32,
    beats_per_bar: u32,
    motif_intervals: Option<&[i8]>,
    seed: u64,
) -> VocalDerivation {
    let notes = derive_vocal_with_motif(
        chords,
        params,
        ticks_per_beat,
        beats_per_bar,
        motif_intervals,
        seed,
    );
    let Some(odds) = Odds::for_mode(params.syllable_mode) else {
        let auto_holds = vec![false; notes.len()];
        return VocalDerivation { notes, auto_holds };
    };
    let scale = if params.stay_in_scale { scale_from_chords(chords) } else { None };
    let (lo, hi) = if params.range.0 <= params.range.1 {
        params.range
    } else {
        (params.range.1, params.range.0)
    };
    let pass = Melisma {
        odds,
        scale,
        lo,
        hi,
        tpb: ticks_per_beat.max(1) as u64,
        // The same adjacency caps the section-climax pass keeps.
        cap: if params.style == VocalStyle::Hymnal { 4 } else { 9 },
    };
    pass.apply(notes, &params.draft, seed)
}

/// How often each mode makes a melisma.
#[derive(Clone, Copy)]
struct Odds {
    /// Chance the line's last syllable becomes a melisma.
    phrase_final: f32,
    /// Chance a long stressed syllable inside the line does.
    stressed: f32,
    /// Chance a melisma is three notes rather than two (when it fits).
    three: f32,
    /// Most melismas in one line.
    per_line: usize,
}

impl Odds {
    fn for_mode(mode: SyllableMode) -> Option<Self> {
        match mode {
            SyllableMode::Syllabic => None,
            SyllableMode::Mixed => Some(Self {
                phrase_final: 0.5,
                stressed: 0.2,
                three: 0.25,
                per_line: 1,
            }),
            SyllableMode::Melismatic => Some(Self {
                phrase_final: 0.9,
                stressed: 0.55,
                three: 0.5,
                per_line: 3,
            }),
        }
    }
}

struct Melisma {
    odds: Odds,
    scale: Option<Scale>,
    lo: u8,
    hi: u8,
    tpb: u64,
    cap: i16,
}

/// One note-filling unit of a lyric line, as the resolver sees it.
#[derive(Clone, Copy)]
struct Unit {
    stress: SyllableStress,
    /// This unit is a draft `_` hold.
    draft_hold: bool,
}

/// The units a lyric line fills, padded to `count` (padding units have
/// no syllable and are never split).
fn line_units(line: &LyricLine, count: usize) -> Vec<Option<Unit>> {
    let mut units: Vec<Option<Unit>> = resolve_draft(std::slice::from_ref(line))
        .into_iter()
        .flat_map(|s| {
            std::iter::once(Some(Unit { stress: s.stress, draft_hold: false })).chain(
                std::iter::repeat_n(
                    Some(Unit { stress: SyllableStress::None, draft_hold: true }),
                    s.slurs_after,
                ),
            )
        })
        .collect();
    units.resize(count, None);
    units
}

impl Melisma {
    fn apply(&self, notes: Vec<GeneratedNote>, draft: &[LyricLine], seed: u64) -> VocalDerivation {
        let mut rng = XorShift::new(seed.wrapping_add(0x0E15_A11E_5EED_0001));
        let mut out = VocalDerivation {
            notes: Vec::with_capacity(notes.len() + notes.len() / 2),
            auto_holds: Vec::with_capacity(notes.len() + notes.len() / 2),
        };
        let mut cursor = 0usize;
        for line in draft {
            let n = (count_syllables(&line.text) as usize).min(notes.len() - cursor);
            let span = &notes[cursor..cursor + n];
            let units = line_units(line, n);
            let peak = span.iter().map(|x| x.note).max().unwrap_or(0);
            let picks = self.pick(span, &units, &mut rng);
            for (i, note) in span.iter().enumerate() {
                let pieces = picks.get(&i).copied().unwrap_or(1);
                let prev = out.notes.last().map(|p| p.note);
                let next = span.get(i + 1).map(|x| x.note);
                let pitches = if pieces > 1 {
                    self.ornament(note.note, pieces, prev, next, peak, i + 1 == n)
                } else {
                    vec![note.note]
                };
                self.push_split(&mut out, note, &pitches);
            }
            cursor += n;
        }
        // Notes past the draft (none in practice) stay as they are.
        for note in &notes[cursor..] {
            out.notes.push(*note);
            out.auto_holds.push(false);
        }
        out
    }

    /// Which units of one line become melismas, and into how many notes.
    /// Draws a fixed number of values per unit so later lines do not
    /// depend on earlier picks.
    fn pick(
        &self,
        span: &[GeneratedNote],
        units: &[Option<Unit>],
        rng: &mut XorShift,
    ) -> std::collections::BTreeMap<usize, usize> {
        let mut picks = std::collections::BTreeMap::new();
        let last = span.len().saturating_sub(1);
        // Phrase-final first: it is the most idiomatic place.
        let order = std::iter::once(last).chain(0..last);
        for i in order {
            let (roll, three) = (rng.next_f32(), rng.next_f32());
            if picks.len() >= self.odds.per_line || span.is_empty() {
                continue;
            }
            let Some(unit) = units[i] else { continue };
            // A note followed by a draft hold of the same syllable must
            // stay one note: the note walk gives the hold to the first
            // slur note after the syllable, so a split here would shift
            // the draft hold onto the ornament.
            if units.get(i + 1).copied().flatten().is_some_and(|u| u.draft_hold) {
                continue;
            }
            let dur = span[i].duration_ticks;
            let piece = self.min_piece();
            // "Long" for a stressed syllable: half a beat or more.
            let long = dur >= 3 * piece;
            let chance = if i == last {
                self.odds.phrase_final
            } else if unit.stress == SyllableStress::Primary && !unit.draft_hold && long {
                self.odds.stressed
            } else {
                continue;
            };
            if dur < 2 * piece || roll >= chance {
                continue;
            }
            let pieces = if three < self.odds.three && long { 3 } else { 2 };
            picks.insert(i, pieces);
        }
        picks
    }

    /// Pitches for a melisma of `pieces` notes on `p`. Tries a few
    /// ornaments and keeps the first that stays in range, below the
    /// line's peak (so the single-climax rule holds) and within the
    /// adjacency cap on both sides; falls back to repeating `p`.
    fn ornament(
        &self,
        p: u8,
        pieces: usize,
        prev: Option<u8>,
        next: Option<u8>,
        peak: u8,
        phrase_final: bool,
    ) -> Vec<u8> {
        let below = self.step(p, -1);
        let above = self.step(p, 1);
        let toward = next.and_then(|n| match n.cmp(&p) {
            std::cmp::Ordering::Greater => above,
            std::cmp::Ordering::Less => below,
            std::cmp::Ordering::Equal => None,
        });
        let mut candidates: Vec<Vec<Option<u8>>> = Vec::new();
        match (pieces, phrase_final) {
            // A line ending keeps its cadence pitch as the last note.
            (2, true) => {
                candidates.push(vec![above, Some(p)]);
                candidates.push(vec![below, Some(p)]);
            }
            (2, false) => {
                candidates.push(vec![Some(p), toward]);
                candidates.push(vec![Some(p), below]);
            }
            _ => {
                candidates.push(vec![Some(p), below, Some(p)]);
                candidates.push(vec![Some(p), above, Some(p)]);
            }
        }
        candidates
            .into_iter()
            .filter_map(|c| c.into_iter().collect::<Option<Vec<u8>>>())
            .find(|c| self.fits(c, p, prev, next, peak))
            .unwrap_or_else(|| vec![p; pieces])
    }

    fn fits(&self, pitches: &[u8], p: u8, prev: Option<u8>, next: Option<u8>, peak: u8) -> bool {
        let in_range = pitches.iter().all(|&x| (self.lo..=self.hi).contains(&x));
        let under_peak = pitches.iter().all(|&x| x == p || x < peak);
        let path: Vec<u8> = prev.into_iter().chain(pitches.iter().copied()).chain(next).collect();
        let capped = path.windows(2).all(|w| (w[0] as i16 - w[1] as i16).abs() <= self.cap);
        in_range && under_peak && capped
    }

    /// The scale tone one step above (`dir > 0`) or below `p`, within
    /// two semitones; a whole tone without a scale.
    fn step(&self, p: u8, dir: i16) -> Option<u8> {
        let candidates: &[i16] = match self.scale {
            Some(_) => &[1, 2],
            None => &[2],
        };
        candidates.iter().find_map(|&d| {
            let x = p as i16 + d * dir;
            let ok = (0..=127).contains(&x)
                && self.scale.is_none_or(|s| s.contains(x as u8));
            ok.then_some(x as u8)
        })
    }

    /// The shortest melisma note: a sixteenth-note triplet.
    fn min_piece(&self) -> u64 {
        (self.tpb / 6).max(1)
    }

    /// Split `note` evenly into `pitches.len()` notes over its own span,
    /// a hair apart (a 256th) so they stay strictly ordered while the
    /// render sings them legato.
    fn push_split(&self, out: &mut VocalDerivation, note: &GeneratedNote, pitches: &[u8]) {
        let k = pitches.len() as u64;
        if k <= 1 {
            out.notes.push(*note);
            out.auto_holds.push(false);
            return;
        }
        let gap = (self.tpb / 64).max(1);
        let piece = (note.duration_ticks / k).max(1);
        let end = note.start_tick + note.duration_ticks;
        for (j, &pitch) in pitches.iter().enumerate() {
            let start = note.start_tick + piece * j as u64;
            let duration = if j as u64 + 1 == k {
                end.saturating_sub(start).max(1)
            } else {
                piece.saturating_sub(gap).max(1)
            };
            out.notes.push(GeneratedNote {
                note: pitch,
                velocity: note.velocity,
                start_tick: start,
                duration_ticks: duration,
            });
            out.auto_holds.push(j > 0);
        }
    }
}
