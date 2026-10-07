//! Where a vocal line rests and where it breathes
//! (vocal-mcp-control-and-quality.md §8).
//!
//! **Rests.** A gap between a note's end and the next note's onset of at
//! least the lane's `rest_threshold_beats` (converted at the lane's tempo)
//! is silence; a shorter gap is sung legato, the note holding until the
//! next onset. One [`RestRule`] answers that for all three places that ask
//! — the duration builder, the render-unit splitter and the articulation
//! report — so they never disagree about where the silences are.
//!
//! **Breaths.** A syllable marked `breath_before` (a standalone `AP` in the
//! lyrics) gets a [`BREATH_PHONEME`] at the end of the gap before its note,
//! up to [`BREATH_MAX_SEC`]; when the gap is shorter than that (or there is
//! none — a legato join), up to [`BREATH_TAIL_FRACTION`] of the previous
//! note's tail, at most [`BREATH_TAIL_MAX_SEC`], is given to the breath as
//! well. Plain rests and the segment pads are [`REST_PHONEME`], which the
//! post-processing gates to silence; breaths are not gated.

use resonance_music_theory::VocalParams;

/// The phoneme a plain rest and the segment pads are sung as. `SP`, not
/// `AP`: rendered with the real Lilia and TIGER banks, an `AP` span comes
/// out as audible breath noise and an `SP` span as near-silence (see the
/// measurement in `docs/features/compose/vocal-rendering.md`), so only a
/// written breath should be `AP`.
pub(crate) const REST_PHONEME: &str = "SP";

/// The phoneme a written breath is sung as.
pub(crate) const BREATH_PHONEME: &str = "AP";

/// Longest breath, taken from the end of the gap before its note.
pub(crate) const BREATH_MAX_SEC: f64 = 0.35;

/// Share of the previous note a breath may take when the gap is too short.
pub(crate) const BREATH_TAIL_FRACTION: f64 = 0.30;

/// Most a breath may take from the previous note's tail.
pub(crate) const BREATH_TAIL_MAX_SEC: f64 = 0.25;

/// Silence the leading pad keeps in front of a breath on the segment's
/// first note, so the model still starts from rest.
pub(crate) const BREATH_PAD_LEAD_SEC: f64 = 0.05;

/// Equal-within-rounding slack for the threshold compare, so a written
/// eighth rest at a ½-beat threshold is a rest however the ticks round.
const EPS_SEC: f64 = 1e-6;

/// The lane's rest threshold at one tempo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RestRule {
    threshold_sec: f64,
}

impl RestRule {
    pub(crate) fn new(params: &VocalParams, bpm: f32) -> Self {
        Self::from_beats(params.rest_threshold_beats, bpm)
    }

    pub(crate) fn from_beats(rest_threshold_beats: f32, bpm: f32) -> Self {
        let (lo, hi) = VocalParams::REST_THRESHOLD_BEATS;
        let beats = rest_threshold_beats.clamp(lo, hi) as f64;
        Self {
            threshold_sec: beats * 60.0 / bpm.max(1.0) as f64,
        }
    }

    /// Is the gap between a note sung for `sing_sec` and the next onset
    /// `slot_sec` after its own a rest?
    pub(crate) fn is_rest(self, slot_sec: f64, sing_sec: f64) -> bool {
        slot_sec - sing_sec >= self.threshold_sec - EPS_SEC
    }
}

/// How far before each note's onset its written breath starts, in
/// seconds (0 for a note without one, and for the first note, whose
/// breath sits in the leading pad). Mirrors the duration builder's
/// `place_breaths` on the notes alone: the breath takes the end of a rest
/// before the note (up to [`BREATH_MAX_SEC`]), then up to
/// [`BREATH_TAIL_FRACTION`] of the previous note (at most
/// [`BREATH_TAIL_MAX_SEC`]). Ignores the minimum-slot floors, so it can
/// overstate a breath on a very short previous note — the post gain only
/// uses it to hand the breath to the note it leads into.
pub fn breath_lead_secs(
    notes: &[resonance_audio::types::MidiNote],
    assigned: &[resonance_music_theory::g2p::AssignedSyllable],
    ticks_per_quarter: u32,
    bpm: f32,
    params: &VocalParams,
) -> Vec<f64> {
    let rule = RestRule::new(params, bpm);
    let spt = 60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter.max(1) as f64);
    (0..notes.len())
        .map(|i| {
            let breathes = assigned
                .get(i)
                .is_some_and(|a| a.breath_before && !a.is_slur);
            if i == 0 || !breathes {
                return 0.0;
            }
            let prev = &notes[i - 1];
            let slot = notes[i].start_tick.saturating_sub(prev.start_tick) as f64 * spt;
            let cap = (prev.duration_ticks as f64 * spt).max(0.05);
            let (sing, gap) = if rule.is_rest(slot, cap) {
                (cap, slot - cap)
            } else {
                (slot, 0.0)
            };
            let from_gap = gap.min(BREATH_MAX_SEC);
            let from_tail = (BREATH_MAX_SEC - from_gap)
                .min(sing * BREATH_TAIL_FRACTION)
                .clamp(0.0, BREATH_TAIL_MAX_SEC);
            from_gap + from_tail
        })
        .collect()
}
