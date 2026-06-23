//! Tests for sub-clip render units + the content-addressed render cache
//! (todo #495): a vocal clip is split into independently-renderable
//! segments at genuine silences, an edit re-renders only the segment it
//! touched, untouched segments are reused from cache, and the stitched
//! output places each unit at its timeline offset.
//!
//! The acoustic + vocoder pipeline needs installed model files, so these
//! tests drive [`render_units_cached`] with a stub renderer that counts
//! invocations and returns deterministic audio — every property under
//! test (segmentation, cache reuse, the "N of M" tally, stitching) is
//! independent of the neural model itself.

use std::cell::Cell;

use resonance_app::compose::vocal_svs::{
    render_units_cached, split_render_units, SvsRenderCache,
};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::g2p::{AssignedSyllable, PhonemeProvenance, SyllableStress};
use resonance_music_theory::{VocalParams, VocalVoicebank};
use resonance_svs::ds::DsSegment;

const BPM: f32 = 120.0;
const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;

fn params() -> VocalParams {
    VocalParams {
        voicebank: VocalVoicebank::Tiger,
        ..VocalParams::default()
    }
}

/// A non-slur note assignment with explicit phonemes and syllable index.
fn syl(phonemes: &[&'static str], syllable_index: usize) -> AssignedSyllable {
    AssignedSyllable {
        label: phonemes.concat(),
        phonemes: phonemes.to_vec(),
        is_slur: false,
        is_word_end: true,
        syllable_index,
        stress: SyllableStress::None,
        provenance: PhonemeProvenance::Auto,
    }
}

fn note(start_tick: u64) -> MidiNote {
    MidiNote {
        note: 60,
        velocity: 0.8,
        start_tick,
        duration_ticks: TICKS_PER_QUARTER_NOTE,
    }
}

/// Two quarter-notes far enough apart (2 s gap) that the duration builder
/// would insert a rest between them — so they split into two units.
fn two_units_input() -> (Vec<MidiNote>, Vec<AssignedSyllable>) {
    let notes = vec![note(0), note(4 * TICKS_PER_QUARTER_NOTE)];
    let assigned = vec![syl(&["s", "ow"], 0), syl(&["l", "ow"], 1)];
    (notes, assigned)
}

// ---------------------------------------------------------------------------
// Segmentation
// ---------------------------------------------------------------------------

#[test]
fn continuous_phrase_is_a_single_unit() {
    // Back-to-back quarter notes: no gap exceeds the silence threshold,
    // so the whole clip is one render unit (matching the old whole-clip
    // build — no regression for continuous singing).
    let notes = vec![note(0), note(TICKS_PER_QUARTER_NOTE)];
    let assigned = vec![syl(&["s", "ow"], 0), syl(&["l", "ow"], 1)];

    let units = split_render_units(&notes, &params(), &assigned, TPQ, BPM);

    assert_eq!(units.len(), 1, "continuous phrase should be one unit");
    assert_eq!(units[0].note_range, 0..2);
    assert_eq!(units[0].syllable_range, 0..2);
    assert_eq!(units[0].start_sec, 0.0);
}

#[test]
fn genuine_silence_splits_into_two_units() {
    let (notes, assigned) = two_units_input();

    let units = split_render_units(&notes, &params(), &assigned, TPQ, BPM);

    assert_eq!(units.len(), 2, "a 2 s gap should split into two units");
    assert_eq!(units[0].note_range, 0..1);
    assert_eq!(units[1].note_range, 1..2);
    assert_eq!(units[0].syllable_range, 0..1);
    assert_eq!(units[1].syllable_range, 1..2);
    assert_eq!(units[0].start_sec, 0.0);
    // Second note starts at 4 quarter-notes = 2.0 s at 120 BPM.
    assert!((units[1].start_sec - 2.0).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// Cache reuse + N-of-M tally
// ---------------------------------------------------------------------------

/// A stub renderer that returns a fixed-length buffer (value = call index)
/// and tallies how many times it actually ran.
fn counting_renderer(
    calls: &Cell<usize>,
    sr: u32,
    len: usize,
) -> impl FnMut(&DsSegment) -> Result<(Vec<f32>, u32), String> + '_ {
    move |_seg: &DsSegment| {
        let n = calls.get();
        calls.set(n + 1);
        Ok((vec![0.1 * (n + 1) as f32; len], sr))
    }
}

#[test]
fn first_render_renders_every_unit() {
    let (notes, assigned) = two_units_input();
    let units = split_render_units(&notes, &params(), &assigned, TPQ, BPM);
    let mut cache = SvsRenderCache::new();
    let calls = Cell::new(0);

    let out = render_units_cached(&units, &mut cache, counting_renderer(&calls, 44_100, 64)).unwrap();

    assert_eq!(out.plan.total, 2);
    assert_eq!(out.plan.changed, 2);
    assert_eq!(out.plan.reused, 0);
    assert_eq!(calls.get(), 2, "both units rendered on first pass");
    assert_eq!(cache.last_plan(), Some(out.plan));
    assert_eq!(cache.len(), 2);
}

#[test]
fn identical_rerender_reuses_all_units() {
    let (notes, assigned) = two_units_input();
    let mut cache = SvsRenderCache::new();
    let calls = Cell::new(0);

    // Populate.
    let units = split_render_units(&notes, &params(), &assigned, TPQ, BPM);
    render_units_cached(&units, &mut cache, counting_renderer(&calls, 44_100, 64)).unwrap();
    assert_eq!(calls.get(), 2);

    // Re-render the very same clip: nothing changed, everything reused.
    let units2 = split_render_units(&notes, &params(), &assigned, TPQ, BPM);
    let out = render_units_cached(&units2, &mut cache, counting_renderer(&calls, 44_100, 64)).unwrap();

    assert_eq!(out.plan.changed, 0, "no unit changed");
    assert_eq!(out.plan.reused, 2);
    assert_eq!(calls.get(), 2, "renderer not invoked again");
}

#[test]
fn editing_one_syllable_rerenders_only_that_unit() {
    let (notes, assigned) = two_units_input();
    let mut cache = SvsRenderCache::new();
    let calls = Cell::new(0);

    let before = split_render_units(&notes, &params(), &assigned, TPQ, BPM);
    render_units_cached(&before, &mut cache, counting_renderer(&calls, 44_100, 64)).unwrap();
    assert_eq!(calls.get(), 2);

    // Edit the second syllable's phonemes; the first is untouched.
    let edited = vec![syl(&["s", "ow"], 0), syl(&["m", "iy"], 1)];
    let after = split_render_units(&notes, &params(), &edited, TPQ, BPM);

    // Only the changed unit's content key moves.
    assert_eq!(before[0].key, after[0].key, "untouched unit key is stable");
    assert_ne!(before[1].key, after[1].key, "edited unit key changed");

    let out = render_units_cached(&after, &mut cache, counting_renderer(&calls, 44_100, 64)).unwrap();

    assert_eq!(out.plan.total, 2);
    assert_eq!(out.plan.changed, 1, "only the edited segment re-renders");
    assert_eq!(out.plan.reused, 1, "the untouched segment is reused");
    assert_eq!(calls.get(), 3, "renderer ran exactly once more");
    // Stale entry for the old second unit is evicted; cache tracks the
    // clip's current two units.
    assert_eq!(cache.len(), 2);
}

// ---------------------------------------------------------------------------
// Stitching: units land at their timeline offsets
// ---------------------------------------------------------------------------

#[test]
fn units_are_stitched_at_their_sample_offsets() {
    let (notes, assigned) = two_units_input();
    let units = split_render_units(&notes, &params(), &assigned, TPQ, BPM);
    let mut cache = SvsRenderCache::new();
    let calls = Cell::new(0);

    // Tiny sample rate keeps the offset arithmetic exact and obvious:
    // unit 1 starts at 2.0 s → sample offset 20.
    let sr = 10;
    let len = 5;
    let out = render_units_cached(&units, &mut cache, counting_renderer(&calls, sr, len)).unwrap();

    assert_eq!(out.sample_rate, sr);
    // unit0 audio (value 0.1) at offset 0; unit1 audio (value 0.2) at 20.
    assert_eq!(out.mono.len(), 20 + len);
    assert!((out.mono[0] - 0.1).abs() < 1e-6, "unit 0 at offset 0");
    assert!((out.mono[4] - 0.1).abs() < 1e-6);
    assert!((out.mono[10] - 0.0).abs() < 1e-6, "silence in the gap");
    assert!((out.mono[20] - 0.2).abs() < 1e-6, "unit 1 at offset 20");
    assert!((out.mono[24] - 0.2).abs() < 1e-6);
}
