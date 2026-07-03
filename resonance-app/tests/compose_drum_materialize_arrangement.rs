//! Arrangement-aware drum materialization: turning a section's resolved
//! arrangement spans into a concrete `MidiNote` sequence (playback + bounce
//! render the same materialized clips, so this is both paths).
//!
//! These drive the real pipeline `materialize_drum_clips` uses — the #483
//! resolver ([`resolve_arrangement`]) feeding
//! [`build_drum_notes`](resonance_app::update::compose::build_drum_notes) —
//! but without booting a whole `Resonance`, so the cases stay pure and
//! deterministic. Coverage mirrors the DoD: chained, repeat, fill-on-last-bar,
//! gap-is-silent, overflow-is-clipped, plus the documented phase/cycle
//! continuity across spans.

use std::collections::HashMap;

use resonance_app::compose::{resolve_arrangement, ArrangementSpan, EntryLength, PatternEntry};
use resonance_app::update::compose::build_drum_notes;
use resonance_audio::types::MidiNote;

// Import the drum-group model directly so we can hand-build patterns.
use resonance_app::compose::{DrumGroup, DrumPattern};

// 4/4 throughout these cases.
const TS: u8 = 4;
const TPQN: u64 = 480;

// Pattern ids. Bar lengths come from `PATTERNS` below.
const GROOVE_A: u64 = 1;
const GROOVE_B: u64 = 2;
const FILL: u64 = 9;

// One MIDI note per pattern so a note's number identifies which pattern
// produced it in the emitted sequence.
const NOTE_A: u8 = 36;
const NOTE_B: u8 = 38;
const NOTE_FILL: u8 = 42;

/// Build a single-pad drum group. `grid` = steps per beat, `cycle` = steps
/// before the pad pattern repeats, `phase` rotates it. The pad fires on every
/// non-zero slot of `pattern` (indexed 0..cycle).
fn group(note: u8, grid: u8, cycle: u32, phase: u32, pattern: Vec<u8>) -> DrumGroup {
    DrumGroup {
        id: note as u64,
        name: format!("g{note}"),
        color: [0, 0, 0],
        grid,
        cycle,
        phase,
        pads: vec![resonance_app::compose::DrumGroupPad {
            name: format!("p{note}"),
            note,
            weight: 100,
            pattern,
        }],
        density: 0.5,
        swing: 0.0,
        accent: 0.0,
        humanize: 0.0,
        fills: 0.0,
        style: "Custom".into(),
        seed: 1,
    }
}

/// A one-bar pattern whose single group hits once per beat (grid 1, cycle 1,
/// pad pattern `[1]`) on `note`. Four hits per 4/4 bar.
fn steady_pattern(id: u64, note: u8) -> DrumPattern {
    DrumPattern {
        id,
        name: format!("pat{id}"),
        color: [0, 0, 0],
        groups: vec![group(note, 1, 1, 0, vec![1])],
        length_bars: 1,
    }
}

/// The pattern bank the cases resolve against.
fn bank() -> HashMap<u64, DrumPattern> {
    let mut m = HashMap::new();
    m.insert(GROOVE_A, steady_pattern(GROOVE_A, NOTE_A));
    m.insert(GROOVE_B, steady_pattern(GROOVE_B, NOTE_B));
    m.insert(FILL, steady_pattern(FILL, NOTE_FILL));
    m
}

fn entry(pattern_id: u64, length: EntryLength, fill: Option<u64>) -> PatternEntry {
    PatternEntry {
        pattern_id,
        length,
        fill,
    }
}

/// Resolve `arrangement` over a `section_bars`-bar section against `bank`,
/// pair each resolved span with its pattern's groups, then materialize — the
/// exact chain `materialize_drum_clips` runs, minus the engine send.
fn materialize(
    arrangement: &[PatternEntry],
    section_bars: u32,
    bank: &HashMap<u64, DrumPattern>,
) -> Vec<MidiNote> {
    let resolved = resolve_arrangement(arrangement, section_bars, |id| {
        bank.get(&id).map(|p| p.length_bars.max(1)).unwrap_or(1)
    });
    let spans: Vec<(ArrangementSpan, Vec<DrumGroup>)> = resolved
        .spans
        .into_iter()
        .map(|s| {
            let groups = bank
                .get(&s.pattern_id)
                .map(|p| p.groups.clone())
                .unwrap_or_default();
            (s, groups)
        })
        .collect();
    build_drum_notes(&spans, TS)
}

/// Compact `(note, start_tick)` view so sequence asserts don't fight floats.
fn seq(notes: &[MidiNote]) -> Vec<(u8, u64)> {
    notes.iter().map(|n| (n.note, n.start_tick)).collect()
}

/// The four beat ticks of bar `b` in 4/4 with a grid-1 steady pattern.
fn bar_ticks(b: u64) -> [u64; 4] {
    let base = b * TS as u64 * TPQN; // 1920 ticks per bar
    [base, base + TPQN, base + 2 * TPQN, base + 3 * TPQN]
}

/// Every steady-pattern hit is a beat start (grid 1), so all notes carry the
/// same velocity; assert the concrete value once.
#[test]
fn steady_hits_are_beat_start_velocity() {
    let notes = materialize(&[entry(GROOVE_A, EntryLength::RepeatN(1), None)], 1, &bank());
    assert_eq!(notes.len(), 4);
    // base = 0.70 + min(100/200, 0.25) = 0.95; accent 0 adds nothing.
    for n in &notes {
        assert!((n.velocity - 0.95).abs() < 1e-6, "velocity {}", n.velocity);
        assert_eq!(n.duration_ticks, TPQN); // step_ticks = 480/grid(1)
    }
}

/// Chained: Groove A then Groove B, one bar each. Bar 0 is all A-notes, bar 1
/// all B-notes, in tick order.
#[test]
fn chained_two_patterns_emit_per_bar() {
    let arr = [
        entry(GROOVE_A, EntryLength::RepeatN(1), None),
        entry(GROOVE_B, EntryLength::RepeatN(1), None),
    ];
    let notes = materialize(&arr, 2, &bank());
    let expected: Vec<(u8, u64)> = bar_ticks(0)
        .iter()
        .map(|&t| (NOTE_A, t))
        .chain(bar_ticks(1).iter().map(|&t| (NOTE_B, t)))
        .collect();
    assert_eq!(seq(&notes), expected);
}

/// Repeat: Groove A x3 tiles across three bars — 12 A-hits, nothing else.
#[test]
fn repeat_tiles_pattern_across_bars() {
    let notes = materialize(&[entry(GROOVE_A, EntryLength::RepeatN(3), None)], 3, &bank());
    let expected: Vec<(u8, u64)> = (0..3)
        .flat_map(|b| bar_ticks(b).into_iter().map(|t| (NOTE_A, t)))
        .collect();
    assert_eq!(seq(&notes), expected);
    assert!(notes.iter().all(|n| n.note == NOTE_A));
}

/// Fill on the last bar: Groove A x3 with a fill swaps the final bar to the
/// fill pattern. Bars 0,1 = A; bar 2 = fill.
#[test]
fn fill_replaces_last_bar_of_entry() {
    let notes = materialize(
        &[entry(GROOVE_A, EntryLength::RepeatN(3), Some(FILL))],
        3,
        &bank(),
    );
    let mut expected: Vec<(u8, u64)> = (0..2)
        .flat_map(|b| bar_ticks(b).into_iter().map(|t| (NOTE_A, t)))
        .collect();
    expected.extend(bar_ticks(2).into_iter().map(|t| (NOTE_FILL, t)));
    assert_eq!(seq(&notes), expected);
}

/// Gap: a one-bar entry in a three-bar section. Only bar 0 emits; bars 1-2
/// are silent (no notes at or past their tick range).
#[test]
fn trailing_gap_is_silent() {
    let notes = materialize(&[entry(GROOVE_A, EntryLength::RepeatN(1), None)], 3, &bank());
    assert_eq!(seq(&notes), bar_ticks(0).map(|t| (NOTE_A, t)).to_vec());
    // Nothing at or beyond bar 1's first tick.
    assert!(notes.iter().all(|n| n.start_tick < bar_ticks(1)[0]));
}

/// Interior gap: A at bar 0, gap at bar 1 (a zero-length entry contributes
/// nothing), B at bar 2. Middle bar stays silent.
#[test]
fn interior_gap_between_entries_is_silent() {
    // A zero-bar entry drops out, leaving bar 1 uncovered; B then starts at
    // bar 1 (entries are laid head-to-tail), so to force an interior *gap*
    // we instead give A a fixed 1-bar span, skip a bar with a Bars(0) filler
    // that the resolver ignores, then B. The resolver lays B right after A,
    // so this exercises the resolver's "no zero-length span" rule and the
    // materializer emitting only covered bars.
    let arr = [
        entry(GROOVE_A, EntryLength::Bars(1), None),
        entry(GROOVE_B, EntryLength::Bars(0), None), // ignored
        entry(GROOVE_B, EntryLength::Bars(1), None),
    ];
    let notes = materialize(&arr, 3, &bank());
    // A at bar 0, B at bar 1 (head-to-tail), bar 2 uncovered → silent.
    let expected: Vec<(u8, u64)> = bar_ticks(0)
        .iter()
        .map(|&t| (NOTE_A, t))
        .chain(bar_ticks(1).iter().map(|&t| (NOTE_B, t)))
        .collect();
    assert_eq!(seq(&notes), expected);
    assert!(notes.iter().all(|n| n.start_tick < bar_ticks(2)[0]));
}

/// Overflow: Groove A x4 in a two-bar section. Only bars 0-1 render; nothing
/// plays or bounces past `section_bars`.
#[test]
fn overflow_is_clipped_at_section_boundary() {
    let notes = materialize(&[entry(GROOVE_A, EntryLength::RepeatN(4), None)], 2, &bank());
    let expected: Vec<(u8, u64)> = (0..2)
        .flat_map(|b| bar_ticks(b).into_iter().map(|t| (NOTE_A, t)))
        .collect();
    assert_eq!(seq(&notes), expected);
    // Section is 2 bars → nothing at or past tick 2*1920 = 3840.
    let section_end = 2 * TS as u64 * TPQN;
    assert!(notes.iter().all(|n| n.start_tick < section_end));
}

/// Phase/cycle continuity across spans: a group whose `cycle` (3) does not
/// divide the bar's step count (grid 1 → 4 steps/bar) fires only when the
/// **absolute** section step is a multiple of 3. When Groove A is interrupted
/// by a different pattern at bar 1 and resumes at bar 2, its cycle must land
/// where an uninterrupted walk would — i.e. indexed by absolute bar, not
/// reset per span. This locks in the documented "phase is continuous across
/// spans" semantic (a per-span reset would put the bar-2 hits elsewhere).
#[test]
fn cycle_phase_is_continuous_across_spans() {
    // Groove A: grid 1, cycle 3, pad pattern [1,0,0] → hit when idx == 0.
    let mut bank = bank();
    bank.insert(
        GROOVE_A,
        DrumPattern {
            id: GROOVE_A,
            name: "A3".into(),
            color: [0, 0, 0],
            groups: vec![group(NOTE_A, 1, 3, 0, vec![1, 0, 0])],
            length_bars: 1,
        },
    );

    let arr = [
        entry(GROOVE_A, EntryLength::RepeatN(1), None), // bar 0
        entry(GROOVE_B, EntryLength::RepeatN(1), None), // bar 1 (interrupts)
        entry(GROOVE_A, EntryLength::RepeatN(1), None), // bar 2 (resumes)
    ];
    let notes = materialize(&arr, 3, &bank);

    // Absolute steps that are multiples of 3 and fall in an A-span:
    //   bar 0 (steps 0..4): step 0, step 3  → ticks 0, 1440
    //   bar 2 (steps 8..12): step 9         → tick 4320
    // A per-span reset would instead fire bar 2 at local steps 0 and 3
    // → absolute steps 8 and 11 → ticks 3840 and 5280. Assert continuity.
    let a_ticks: Vec<u64> = notes
        .iter()
        .filter(|n| n.note == NOTE_A)
        .map(|n| n.start_tick)
        .collect();
    assert_eq!(a_ticks, vec![0, 3 * TPQN, 9 * TPQN]);

    // Bar 1 is Groove B (steady) → four B-hits across bar 1.
    let b_ticks: Vec<u64> = notes
        .iter()
        .filter(|n| n.note == NOTE_B)
        .map(|n| n.start_tick)
        .collect();
    assert_eq!(b_ticks, bar_ticks(1).to_vec());
}

/// Determinism: identical inputs produce byte-identical output across runs.
#[test]
fn output_is_deterministic() {
    let arr = [
        entry(GROOVE_A, EntryLength::RepeatN(2), Some(FILL)),
        entry(GROOVE_B, EntryLength::RepeatN(1), None),
    ];
    let a = materialize(&arr, 3, &bank());
    let b = materialize(&arr, 3, &bank());
    // MidiNote has no PartialEq; compare the full field projection.
    let proj = |ns: &[MidiNote]| -> Vec<(u8, u64, u64, u32)> {
        ns.iter()
            .map(|n| (n.note, n.start_tick, n.duration_ticks, n.velocity.to_bits()))
            .collect()
    };
    assert_eq!(proj(&a), proj(&b));
    assert!(!a.is_empty());
}

/// Empty arrangement is handled by the state-level fallback (whole-section
/// default), not here — with no spans, `build_drum_notes` emits nothing.
#[test]
fn no_spans_emits_no_notes() {
    let notes = materialize(&[], 4, &bank());
    assert!(notes.is_empty());
}
