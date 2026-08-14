//! Unit tests for the vocal render *planner* (ba todo #1259).
//!
//! `enqueue_vocal_render` used to decide what would be sung, where the
//! audio lands and which render epoch it carries inside one ~290-line
//! body, so none of it could be asserted without booting a `Resonance`
//! and an audio engine. Those decisions are pure now, and this suite
//! pins them — most importantly the epoch counter, because a wrong
//! epoch silently drops a finished render (or installs a stale one) on
//! the user's timeline.

use std::collections::HashMap;

use resonance_app::compose::vocal_svs::{
    vocal_audio_start, DictionaryEntry, DictionaryScope, InvalidPhonemeReason, InvalidSyllable,
    SyllableOverride,
};
use resonance_app::update::compose::vocal_render_plan::{
    describe_invalid, lead_ticks, next_render_epoch, placement_audio_starts, plan_vocal_render,
    VocalRenderPlanInputs,
};
use resonance_audio::types::{MidiNote, TempoMap, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::{VocalParams, VocalVoicebank};

const SR: u32 = 48_000;
const BPM: f32 = 120.0;
/// One beat at 120 BPM / 48 kHz.
const BEAT_SAMPLES: u64 = 24_000;

fn flat_tempo_map() -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = BPM;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.rebuild_bar_table(SR);
    tm
}

fn beats(n: u64) -> u64 {
    n * TICKS_PER_QUARTER_NOTE
}

fn note_at(start_tick: u64) -> MidiNote {
    MidiNote {
        note: 60,
        velocity: 0.8,
        start_tick,
        duration_ticks: TICKS_PER_QUARTER_NOTE,
    }
}

/// A one-line, one-syllable draft.
fn params(text: &str, syllables: u8) -> VocalParams {
    VocalParams {
        voicebank: VocalVoicebank::Tiger,
        draft: vec![LyricLine {
            n: 1,
            rhyme: 'A',
            syllables,
            text: text.to_string(),
            locked: false,
        }],
        ..VocalParams::default()
    }
}

// ---------------------------------------------------------------------------
// Lead-in + placement maths
// ---------------------------------------------------------------------------

#[test]
fn an_empty_lane_has_no_lead_in() {
    assert_eq!(lead_ticks(&[]), 0);
}

#[test]
fn the_lead_in_is_the_first_notes_tick() {
    // Not the minimum tick, not the clip start: the first note in the
    // list, matching what the render cache uses as its base tick.
    assert_eq!(lead_ticks(&[note_at(beats(3)), note_at(beats(1))]), beats(3));
}

#[test]
fn every_placement_is_advanced_by_the_lead_in() {
    let tm = flat_tempo_map();
    let starts = vec![(7, tm.bar_to_sample(0)), (9, tm.bar_to_sample(16))];

    let placed = placement_audio_starts(&tm, &starts, beats(8), SR);

    assert_eq!(placed.len(), 2);
    // Placement ids are carried through untouched — the install step
    // keys the engine clips off them.
    assert_eq!(placed[0].0, 7);
    assert_eq!(placed[1].0, 9);
    assert_eq!(placed[0].1 - starts[0].1, 8 * BEAT_SAMPLES);
    assert_eq!(placed[1].1 - starts[1].1, 8 * BEAT_SAMPLES);
}

#[test]
fn a_downbeat_lane_is_placed_at_the_section_start() {
    let tm = flat_tempo_map();
    let starts = vec![(1, tm.bar_to_sample(4))];
    assert_eq!(placement_audio_starts(&tm, &starts, 0, SR), starts);
}

#[test]
fn placement_maths_agrees_with_the_shared_helper() {
    // The planner must not grow its own arithmetic: it delegates to the
    // same `vocal_audio_start` the alignment suite pins (ba doc #272).
    let tm = flat_tempo_map();
    let section_start = tm.bar_to_sample(12);
    let placed = placement_audio_starts(&tm, &[(3, section_start)], beats(5), SR);
    assert_eq!(
        placed[0].1,
        vocal_audio_start(&tm, section_start, beats(5), SR)
    );
}

#[test]
fn no_placements_plans_no_audio_starts() {
    let tm = flat_tempo_map();
    assert!(placement_audio_starts(&tm, &[], beats(2), SR).is_empty());
}

// ---------------------------------------------------------------------------
// Render epoch — stale-result protection
// ---------------------------------------------------------------------------

#[test]
fn a_lanes_first_render_carries_epoch_one() {
    // `handle_vocal_audio_ready` compares against a missing entry as 0,
    // so the first queued render must be 1 or its own result would be
    // discarded as stale.
    assert_eq!(next_render_epoch(None), 1);
    assert_eq!(next_render_epoch(Some(0)), 1);
}

#[test]
fn each_render_takes_the_next_epoch() {
    assert_eq!(next_render_epoch(Some(1)), 2);
    assert_eq!(next_render_epoch(Some(41)), 42);
}

#[test]
fn the_epoch_wraps_rather_than_overflowing() {
    assert_eq!(next_render_epoch(Some(u64::MAX)), 0);
}

// ---------------------------------------------------------------------------
// Pronunciation gate
// ---------------------------------------------------------------------------

#[test]
fn a_singable_draft_plans_one_syllable_per_note() {
    let tm = flat_tempo_map();
    let notes = vec![note_at(beats(2)), note_at(beats(3))];
    let starts = vec![(1, tm.bar_to_sample(8))];

    let plan = plan_vocal_render(VocalRenderPlanInputs {
        tempo_map: &tm,
        engine_sample_rate: SR,
        params: &params("hello there", 2),
        annotations: &[],
        midi_notes: &notes,
        placement_starts: &starts,
        overrides: &HashMap::new(),
        project_dictionary: &[],
    })
    .expect("an ordinary English draft is singable");

    assert_eq!(plan.assigned.len(), notes.len());
    assert_eq!(plan.lead_ticks, beats(2));
    assert_eq!(plan.audio_starts.len(), 1);
    assert_eq!(plan.audio_starts[0].1 - starts[0].1, 2 * BEAT_SAMPLES);
}

#[test]
fn the_project_dictionary_reaches_the_planned_syllables() {
    let tm = flat_tempo_map();
    let plan = plan_vocal_render(VocalRenderPlanInputs {
        tempo_map: &tm,
        engine_sample_rate: SR,
        params: &params("vee", 1),
        annotations: &[],
        midi_notes: &[note_at(0)],
        placement_starts: &[(1, 0)],
        overrides: &HashMap::new(),
        project_dictionary: &[DictionaryEntry::new(
            "vee",
            &["v", "iy"],
            DictionaryScope::Project,
        )],
    })
    .expect("TIGER sings `v` directly");
    assert_eq!(plan.assigned[0].phonemes, vec!["v", "iy"]);
}

#[test]
fn an_unsingable_phoneme_blocks_the_plan() {
    // The handler returns early on `Err` and leaves the lane's existing
    // audio in place, so nothing downstream may run: the error is the
    // whole result.
    let tm = flat_tempo_map();
    let mut overrides = HashMap::new();
    overrides.insert(
        0,
        SyllableOverride {
            phonemes: vec!["zzz"],
            variant_idx: None,
        },
    );

    let err = plan_vocal_render(VocalRenderPlanInputs {
        tempo_map: &tm,
        engine_sample_rate: SR,
        params: &params("hello", 1),
        annotations: &[],
        midi_notes: &[note_at(0)],
        placement_starts: &[(1, 0)],
        overrides: &overrides,
        project_dictionary: &[],
    })
    .expect_err("a non-ARPAbet override must not reach the model");

    assert!(err.contains("zzz"), "{err}");
    assert!(err.contains("1 phoneme(s)"), "{err}");
}

// ---------------------------------------------------------------------------
// Blocked-phoneme report
// ---------------------------------------------------------------------------

fn invalid(note_index: usize, label: &str, phoneme: &str) -> InvalidSyllable {
    InvalidSyllable {
        note_index,
        label: label.to_string(),
        phoneme: phoneme.to_string(),
        reason: InvalidPhonemeReason::NotArpabet,
    }
}

#[test]
fn the_report_names_the_note_one_based() {
    let line = describe_invalid(&[invalid(0, "hi", "zzz")]);
    assert!(line.contains("note 1"), "{line}");
    assert!(line.contains("zzz"), "{line}");
    assert!(line.contains("not ARPAbet"), "{line}");
}

#[test]
fn a_label_less_syllable_is_shown_as_a_question_mark() {
    let line = describe_invalid(&[invalid(3, "", "qq")]);
    assert!(line.contains("note 4"), "{line}");
    assert!(line.contains('?'), "{line}");
}

#[test]
fn a_wholesale_bad_draft_is_summarised_not_spelled_out() {
    // Every offender is counted, but at most six are named — the report
    // goes into a one-line status bar.
    let all: Vec<InvalidSyllable> = (0..9).map(|i| invalid(i, "x", "zz")).collect();
    let line = describe_invalid(&all);
    assert!(line.contains("9 phoneme(s)"), "{line}");
    assert!(line.contains("+3 more"), "{line}");
    assert_eq!(line.matches("note ").count(), 6, "{line}");
}
