//! Pure drum-arrangement resolution: turning a section's ordered
//! [`PatternEntry`] list plus its bar length into concrete per-bar spans
//! and a coverage status.
//!
//! The resolver ([`resolve_arrangement`]) is deterministic and free of
//! `ComposeState`, so these tests drive it directly with a `pattern_len`
//! closure that maps a pattern id to its intrinsic bar length. Cases
//! mirror the data model's DoD: single-entry, chained, repeat-vs-fixed
//! bars, fill-on-last-repeat, gap, and overflow.

use resonance_app::compose::{
    resolve_arrangement, ArrangementCoverage, ArrangementSpan, EntryLength, PatternEntry,
};

/// Pattern ids used across the cases. Bar lengths are assigned by
/// `pattern_len` below so the same id means the same intrinsic length.
const P1: u64 = 1; // 1-bar pattern
const P2: u64 = 2; // 2-bar pattern
const FILL: u64 = 9; // 1-bar fill pattern

/// Intrinsic bar length per pattern id. Unknown ids resolve to 1 bar,
/// matching `ComposeState::resolve_arrangement_for`'s fallback.
fn pattern_len(id: u64) -> u32 {
    match id {
        P2 => 2,
        _ => 1,
    }
}

fn entry(pattern_id: u64, length: EntryLength, fill: Option<u64>) -> PatternEntry {
    PatternEntry {
        pattern_id,
        length,
        fill,
    }
}

fn span(bar_start: u32, bar_end: u32, pattern_id: u64, is_fill: bool) -> ArrangementSpan {
    ArrangementSpan {
        bar_start,
        bar_end,
        pattern_id,
        is_fill,
    }
}

#[test]
fn single_entry_fills_section_exactly() {
    let arr = vec![entry(P1, EntryLength::RepeatN(4), None)];
    let resolved = resolve_arrangement(&arr, 4, pattern_len);

    assert_eq!(resolved.spans, vec![span(0, 4, P1, false)]);
    assert_eq!(resolved.coverage, ArrangementCoverage::Exact);
}

#[test]
fn empty_arrangement_is_a_full_section_gap() {
    let resolved = resolve_arrangement(&[], 8, pattern_len);

    assert!(resolved.spans.is_empty());
    assert_eq!(resolved.coverage, ArrangementCoverage::Gap { bars: 8 });
    // No span covers any bar — callers fall back to the default pattern.
    assert!(resolved.span_at(0).is_none());
}

#[test]
fn chained_entries_lay_head_to_tail() {
    let arr = vec![
        entry(P1, EntryLength::RepeatN(2), None),
        entry(P2, EntryLength::RepeatN(1), None),
        entry(P1, EntryLength::RepeatN(2), None),
    ];
    // 2 + (1 * 2) + 2 = 6 bars.
    let resolved = resolve_arrangement(&arr, 6, pattern_len);

    assert_eq!(
        resolved.spans,
        vec![
            span(0, 2, P1, false),
            span(2, 4, P2, false),
            span(4, 6, P1, false),
        ]
    );
    assert_eq!(resolved.coverage, ArrangementCoverage::Exact);
    // span_at lands in the right run either side of a boundary.
    assert_eq!(resolved.span_at(1).unwrap().pattern_id, P1);
    assert_eq!(resolved.span_at(2).unwrap().pattern_id, P2);
    assert_eq!(resolved.span_at(3).unwrap().pattern_id, P2);
    assert_eq!(resolved.span_at(4).unwrap().pattern_id, P1);
}

#[test]
fn repeat_n_scales_by_pattern_bar_length_unlike_fixed_bars() {
    // RepeatN(3) on a 2-bar pattern spans 3 * 2 = 6 bars...
    let repeat = vec![entry(P2, EntryLength::RepeatN(3), None)];
    let resolved = resolve_arrangement(&repeat, 6, pattern_len);
    assert_eq!(resolved.spans, vec![span(0, 6, P2, false)]);
    assert_eq!(resolved.coverage, ArrangementCoverage::Exact);

    // ...whereas Bars(3) on the same pattern spans exactly 3 bars,
    // ignoring the pattern's intrinsic length (it tiles to fill).
    let fixed = vec![entry(P2, EntryLength::Bars(3), None)];
    let resolved = resolve_arrangement(&fixed, 3, pattern_len);
    assert_eq!(resolved.spans, vec![span(0, 3, P2, false)]);
    assert_eq!(resolved.coverage, ArrangementCoverage::Exact);
}

#[test]
fn fill_replaces_the_last_bar_of_a_repeated_entry() {
    // 1-bar pattern repeated 4×, with a fill capping the last bar:
    // bars 0..3 play P1, bar 3 plays the fill.
    let arr = vec![entry(P1, EntryLength::RepeatN(4), Some(FILL))];
    let resolved = resolve_arrangement(&arr, 4, pattern_len);

    assert_eq!(
        resolved.spans,
        vec![span(0, 3, P1, false), span(3, 4, FILL, true)]
    );
    assert_eq!(resolved.coverage, ArrangementCoverage::Exact);
    assert!(resolved.span_at(3).unwrap().is_fill);
    assert!(!resolved.span_at(2).unwrap().is_fill);
}

#[test]
fn single_bar_entry_with_fill_is_all_fill() {
    let arr = vec![entry(P1, EntryLength::RepeatN(1), Some(FILL))];
    let resolved = resolve_arrangement(&arr, 1, pattern_len);

    // No room for a main span — the whole one-bar entry is the fill.
    assert_eq!(resolved.spans, vec![span(0, 1, FILL, true)]);
    assert_eq!(resolved.coverage, ArrangementCoverage::Exact);
}

#[test]
fn short_arrangement_reports_a_trailing_gap() {
    // 4 bars of content in an 8-bar section.
    let arr = vec![entry(P1, EntryLength::RepeatN(4), None)];
    let resolved = resolve_arrangement(&arr, 8, pattern_len);

    assert_eq!(resolved.spans, vec![span(0, 4, P1, false)]);
    assert_eq!(resolved.coverage, ArrangementCoverage::Gap { bars: 4 });
    // The gap bars have no span.
    assert!(resolved.span_at(4).is_none());
    assert!(resolved.span_at(7).is_none());
}

#[test]
fn overlong_arrangement_overflows_and_clips_at_section_end() {
    // 2 + 6 = 8 bars of content clipped into a 5-bar section.
    let arr = vec![
        entry(P1, EntryLength::RepeatN(2), None),
        entry(P2, EntryLength::RepeatN(3), None),
    ];
    let resolved = resolve_arrangement(&arr, 5, pattern_len);

    assert_eq!(
        resolved.spans,
        vec![span(0, 2, P1, false), span(2, 5, P2, false)]
    );
    assert_eq!(resolved.coverage, ArrangementCoverage::Overflow { bars: 3 });
    // Nothing past the clip boundary.
    assert!(resolved.span_at(5).is_none());
}

#[test]
fn overflow_can_drop_a_whole_entry_past_the_boundary() {
    // First entry already fills the 2-bar section; the second is wholly
    // past the end and contributes no span, but still counts as overflow.
    let arr = vec![
        entry(P1, EntryLength::RepeatN(2), None),
        entry(P2, EntryLength::RepeatN(1), None),
    ];
    let resolved = resolve_arrangement(&arr, 2, pattern_len);

    assert_eq!(resolved.spans, vec![span(0, 2, P1, false)]);
    assert_eq!(resolved.coverage, ArrangementCoverage::Overflow { bars: 2 });
}

#[test]
fn zero_length_entries_contribute_nothing() {
    let arr = vec![
        entry(P1, EntryLength::RepeatN(0), None),
        entry(P2, EntryLength::Bars(0), None),
        entry(P1, EntryLength::RepeatN(2), None),
    ];
    let resolved = resolve_arrangement(&arr, 2, pattern_len);

    // Only the third, real entry produces a span — starting at bar 0.
    assert_eq!(resolved.spans, vec![span(0, 2, P1, false)]);
    assert_eq!(resolved.coverage, ArrangementCoverage::Exact);
}

#[test]
fn unknown_pattern_id_defaults_to_one_bar_per_repeat() {
    // Pattern 42 has no entry in `pattern_len`, so it resolves to 1 bar:
    // RepeatN(3) -> 3 bars.
    let arr = vec![entry(42, EntryLength::RepeatN(3), None)];
    let resolved = resolve_arrangement(&arr, 3, pattern_len);

    assert_eq!(resolved.spans, vec![span(0, 3, 42, false)]);
    assert_eq!(resolved.coverage, ArrangementCoverage::Exact);
}

#[test]
fn resolution_is_deterministic() {
    let arr = vec![
        entry(P1, EntryLength::RepeatN(2), Some(FILL)),
        entry(P2, EntryLength::Bars(3), None),
    ];
    let a = resolve_arrangement(&arr, 6, pattern_len);
    let b = resolve_arrangement(&arr, 6, pattern_len);
    assert_eq!(a, b);
}
