//! Re-fit a melody's pitches to a new harmony without touching its
//! rhythm (vocal-melody-on-chord-change, BUG-70).
//!
//! A chord edit under a hand-written (or hand-tuned) vocal melody leaves
//! notes clashing with the new chords. Re-deriving the melody would throw
//! the user's work away; re-fitting keeps everything about it except the
//! pitches that no longer fit:
//!
//! - **Never changed:** note count, order, start, duration, velocity. The
//!   app's per-note lyric side table is index-aligned with the notes, so
//!   lyrics and `_` / `+` holds stay on the notes they belong to.
//! - **Strong-beat and long notes** (a note starting on the bar's first
//!   beat, on the middle beat of an even bar of four or more beats, or on
//!   a chord change; or a note at least a beat long) move to the nearest
//!   **chord tone** of the chord sounding at its start.
//! - **Every other note** moves to the nearest **scale tone**. With no
//!   scale the key is inferred from the chords ([`infer_scale`]); with a
//!   chromatic one the note keeps its pitch (every pitch is in it).
//! - **Contour.** Each note keeps the direction it had from the previous
//!   note — up, down or a repeat — measured against the previous note *as
//!   re-fitted*: of the fitting pitches within a tritone, the nearest one
//!   that keeps that direction wins, and only when none does the nearest
//!   fitting pitch overall. So a note that already fits stays put unless
//!   the previous note's move would flip its direction, and a repeated note
//!   stays a repeat whenever the repeat fits. Equal distances take the
//!   lower pitch.
//!
//! A note under no chord (a gap in the grid) uses the scale rule.

use super::{GeneratedNote, TimedChord};
use crate::Scale;

/// Re-fit `notes` (sorted by `start_tick`, section-relative) to `chords`
/// in `scale`. Returns the notes in the same order with only `note`
/// changed. `ticks_per_beat` is the grid resolution, `beats_per_bar` the
/// section's meter numerator.
pub fn refit_melody(
    notes: &[GeneratedNote],
    chords: &[TimedChord],
    scale: Option<Scale>,
    ticks_per_beat: u32,
    beats_per_bar: u32,
) -> Vec<GeneratedNote> {
    let tpb = u64::from(ticks_per_beat.max(1));
    let bpb = u64::from(beats_per_bar.max(1));
    let scale = scale
        .or_else(|| infer_scale(chords))
        .filter(|s| s.mode != crate::Mode::Chromatic);
    let mut out: Vec<GeneratedNote> = Vec::with_capacity(notes.len());
    for (i, n) in notes.iter().enumerate() {
        let chord = chord_at(chords, n.start_tick, tpb);
        let strong = is_strong(n.start_tick, tpb, bpb, chords) || n.duration_ticks >= tpb;
        let targets: Option<[bool; 12]> = match (strong, chord, scale) {
            (true, Some(c), _) => {
                Some(pitch_class_set(c.chord.pitch_classes().map(|p| p.to_semitone())))
            }
            (_, _, Some(s)) => Some(pitch_class_set((0u8..12).filter(|pc| s.contains(*pc)))),
            _ => None,
        };
        let Some(targets) = targets else {
            out.push(*n);
            continue;
        };
        let prev = (i > 0).then(|| (notes[i - 1].note, out[i - 1].note));
        let note = nearest_fitting(n.note, &targets, prev);
        out.push(GeneratedNote { note, ..*n });
    }
    out
}

/// The major or natural-minor key that holds the most of `chords`' tones,
/// weighted by how long each chord lasts; on a tie, the key on the first
/// chord's root, then major. `None` without chords.
pub fn infer_scale(chords: &[TimedChord]) -> Option<Scale> {
    use crate::{Mode, PitchClass};
    let first_root = chords.first()?.chord.root;
    let mut best: Option<(u32, u8, Scale)> = None;
    for semitone in 0u8..12 {
        let root = PitchClass::from_semitone(semitone);
        for (mode, mode_rank) in [(Mode::Major, 1u8), (Mode::Minor, 0)] {
            let scale = Scale::new(root, mode);
            let score: u32 = chords
                .iter()
                .map(|c| {
                    let inside = c
                        .chord
                        .pitch_classes()
                        .filter(|p| scale.contains(p.to_semitone()))
                        .count() as u32;
                    inside * c.duration_beats.max(1)
                })
                .sum();
            let rank = u8::from(root == first_root) * 2 + mode_rank;
            if best.is_none_or(|(s, r, _)| (score, rank) > (s, r)) {
                best = Some((score, rank, scale));
            }
        }
    }
    best.map(|(_, _, scale)| scale)
}

/// The chord whose span holds `tick`'s beat.
fn chord_at(chords: &[TimedChord], tick: u64, tpb: u64) -> Option<&TimedChord> {
    let beat = tick / tpb;
    chords.iter().find(|c| {
        let start = u64::from(c.start_beat);
        beat >= start && beat < start + u64::from(c.duration_beats)
    })
}

/// A note start that carries metric weight: on a beat that is the bar's
/// first, the middle of an even bar of four or more beats, or a chord
/// change.
fn is_strong(tick: u64, tpb: u64, bpb: u64, chords: &[TimedChord]) -> bool {
    if tick % tpb != 0 {
        return false;
    }
    let beat = tick / tpb;
    let in_bar = beat % bpb;
    in_bar == 0
        || (bpb >= 4 && bpb % 2 == 0 && in_bar == bpb / 2)
        || chords.iter().any(|c| u64::from(c.start_beat) == beat)
}

fn pitch_class_set(pcs: impl Iterator<Item = u8>) -> [bool; 12] {
    let mut set = [false; 12];
    for pc in pcs {
        set[usize::from(pc % 12)] = true;
    }
    set
}

/// The fitting pitch for `pitch` (see the module docs): within a tritone,
/// nearest first, preferring the candidates that keep the direction from
/// the previous note. `prev` is `(previous original pitch, previous
/// refitted pitch)`.
fn nearest_fitting(pitch: u8, targets: &[bool; 12], prev: Option<(u8, u8)>) -> u8 {
    if !targets.iter().any(|t| *t) {
        return pitch;
    }
    let candidates: Vec<u8> = (i16::from(pitch) - 6..=i16::from(pitch) + 6)
        .filter(|p| (0..=127).contains(p) && targets[(*p as usize) % 12])
        .map(|p| p as u8)
        .collect();
    let nearest = |pool: &mut dyn Iterator<Item = u8>| {
        pool.min_by_key(|c| (c.abs_diff(pitch), *c))
    };
    if let Some((prev_orig, prev_new)) = prev {
        let direction = pitch.cmp(&prev_orig);
        let mut keeping = candidates.iter().copied().filter(|c| c.cmp(&prev_new) == direction);
        if let Some(best) = nearest(&mut keeping) {
            return best;
        }
    }
    nearest(&mut candidates.iter().copied()).unwrap_or(pitch)
}
