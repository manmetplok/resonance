//! Post-walk note cleanup.
//!
//! One responsibility: guaranteeing the one-note-per-syllable timeline
//! the SVS pipeline assumes — no two notes may occupy the same time
//! window.

use crate::derive::GeneratedNote;

/// Final pass: each note's `start_tick + duration_ticks` must not
/// exceed the next note's `start_tick`. The `phrase_start_offset`
/// (negative pickup / anacrusis) can shift line N+1 to start before
/// line N's terminal sustain ends, which previously surfaced as
/// "doubled" notes — the SVS pipeline indexes phonemes by note slot,
/// so an overlap means two syllables claim the same time window and
/// the second one's pitch fights the first's tail.
///
/// We compute the time order via a permutation (instead of sorting
/// the notes themselves) so the original lyric order survives — the
/// app's `vocal_phrase_spans` walks notes in lyric order to recover
/// per-line phrase intervals, and a sort would mix lines together
/// when `phrase_start_offset` shifts a later line back into an
/// earlier one's tail. We trim each note's duration to leave at
/// least `tpb / 16` (a 64th note) of silence into the next-in-time
/// note's onset.
pub(in crate::derive::vocal) fn enforce_no_overlap(notes: &mut [GeneratedNote], tpb: u64) {
    if notes.len() < 2 {
        return;
    }
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by_key(|&i| notes[i].start_tick);
    let min_gap = (tpb / 16).max(1);
    for w in order.windows(2) {
        let (cur_idx, next_idx) = (w[0], w[1]);
        let next_start = notes[next_idx].start_tick;
        let cur_start = notes[cur_idx].start_tick;
        let cur_end = cur_start + notes[cur_idx].duration_ticks;
        if cur_end + min_gap > next_start {
            let new_dur = next_start.saturating_sub(cur_start).saturating_sub(min_gap);
            notes[cur_idx].duration_ticks = new_dur.max(1);
        }
    }
}
