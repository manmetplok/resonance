//! The srdc section-layout pass.
//!
//! One responsibility: giving a section a real form by re-singing the
//! statement line's contour on the restatement (and, in aaba groups,
//! the conclusion), so four lines stop reading as four independent
//! arcs.

use crate::derive::vocal::style::cap_interval;
use crate::derive::GeneratedNote;
use crate::rng::XorShift;
use crate::scale::Scale;

use super::primitives::snap_to_scale;

/// Pop srdc section layout (statement–restatement–departure–
/// conclusion, OMT phrase archetypes): lyric lines group in fours as
/// aaba / aabc. The restatement re-sings the statement's contour
/// (offsets from the line's first pitch, index-scaled across differing
/// syllable counts, scale-snapped and adjacency-capped), the departure
/// keeps its own walked material so the group has real contrast, and
/// the conclusion either restates the statement (aaba, ~50% per group,
/// seeded) or keeps its own material (aabc). Trailing 3-line groups
/// are s r c; pairs and singles are left alone.
///
/// Runs before the per-line climax and cadence passes, so the copied
/// lines still get a valid single climax and the conclusion's
/// weak→strong ending swap: a copied conclusion ends on the
/// statement's *open* contour until `super::cadence` rewrites its final
/// two notes toward PAC — the period principle (same material,
/// different ending) at section level.
pub(in crate::derive::vocal) fn apply_srdc_layout(
    notes: &mut [GeneratedNote],
    line_syllables: &[u32],
    scale: Option<Scale>,
    range: (u8, u8),
    seed: u64,
) {
    // Per-line note spans, in lyric order (one note per syllable).
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(line_syllables.len());
    let mut cursor = 0usize;
    for &syl in line_syllables {
        let n = (syl as usize).min(notes.len().saturating_sub(cursor));
        spans.push((cursor, cursor + n));
        cursor += n;
    }
    let mut rng = XorShift::new(seed.wrapping_add(0x00AA_BAAA_BC5D_C0DE));
    let total = spans.len();
    let mut g = 0usize;
    while g < total {
        let group_len = (total - g).min(4);
        // One draw per group, taken unconditionally so the aaba/aabc
        // shape of later groups doesn't depend on earlier group sizes.
        let aaba = rng.next_f32() < 0.5;
        if group_len >= 3 {
            copy_line_contour(notes, spans[g], spans[g + 1], scale, range);
            if group_len == 4 && aaba {
                copy_line_contour(notes, spans[g], spans[g + 3], scale, range);
            }
        }
        g += group_len;
    }
}

/// Re-skin `dst`'s pitches with `src`'s contour: signed offsets from
/// the source line's first pitch, index-scaled onto the destination's
/// syllable count, anchored at the destination's own first pitch (so
/// register continuity from the previous line survives), snapped to
/// scale and capped to the SVS adjacency limit. The destination's
/// first note is the anchor and keeps its walked pitch.
fn copy_line_contour(
    notes: &mut [GeneratedNote],
    src: (usize, usize),
    dst: (usize, usize),
    scale: Option<Scale>,
    range: (u8, u8),
) {
    let (lo, hi) = range;
    let src_n = src.1 - src.0;
    let dst_n = dst.1 - dst.0;
    if src_n == 0 || dst_n < 2 {
        return;
    }
    let src_first = notes[src.0].note as i16;
    let offsets: Vec<i16> = (src.0..src.1)
        .map(|i| notes[i].note as i16 - src_first)
        .collect();
    let anchor = notes[dst.0].note as i16;
    let mut prev = notes[dst.0].note;
    for s in 1..dst_n {
        let idx = s * src_n / dst_n;
        let raw = (anchor + offsets[idx]).clamp(lo as i16, hi as i16) as u8;
        let snapped = snap_to_scale(raw, scale, lo, hi);
        let pitch = cap_interval(prev, snapped, lo, hi, scale);
        notes[dst.0 + s].note = pitch;
        prev = pitch;
    }
}
