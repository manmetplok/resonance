//! Unit tests for the ARRANGEMENT strip's pure display helpers (todo #488).
//!
//! These cover the deterministic label / stepper logic the strip view
//! renders — coverage pill text, per-entry bar-span readout, the inline
//! stepper value, and the `±1` stepper clamp — without booting a renderer.
//! The rendered strip itself is locked in by the golden-image snapshot
//! tests in `compose_arrangement_strip.rs`.

use resonance_app::compose::{
    entry_span_label, entry_stepper_label, entry_stepper_value, step_entry_length,
    ArrangementCoverage, EntryLength,
};

// ---- Coverage pill -------------------------------------------------------

#[test]
fn coverage_chip_exact_reads_n_of_n_bars() {
    assert_eq!(ArrangementCoverage::Exact.chip_label(8), "8 / 8 bars");
}

#[test]
fn coverage_chip_gap_reads_covered_slash_total_and_gap() {
    // 4 bars covered of an 8-bar section, 4 short.
    let cov = ArrangementCoverage::Gap { bars: 4 };
    assert_eq!(cov.covered_bars(8), 4);
    assert_eq!(cov.chip_label(8), "4 / 8 · gap 4");
}

#[test]
fn coverage_chip_overflow_reads_over_total_and_overflow() {
    // 10 bars of entries over an 8-bar section, 2 clipped.
    let cov = ArrangementCoverage::Overflow { bars: 2 };
    assert_eq!(cov.covered_bars(8), 10);
    assert_eq!(cov.chip_label(8), "10 / 8 · overflow 2");
}

#[test]
fn coverage_covered_bars_saturates_when_gap_exceeds_section() {
    // Degenerate gap larger than the section never underflows.
    assert_eq!(ArrangementCoverage::Gap { bars: 12 }.covered_bars(8), 0);
}

// ---- Entry bar-span readout ---------------------------------------------

#[test]
fn entry_span_repeatn_multiplies_pattern_length() {
    // 2-bar pattern repeated 3× = 6 bars.
    assert_eq!(
        entry_span_label(EntryLength::RepeatN(3), 2),
        "2b ×3 = 6b"
    );
}

#[test]
fn entry_span_repeatn_single_bar_pattern() {
    assert_eq!(
        entry_span_label(EntryLength::RepeatN(4), 1),
        "1b ×4 = 4b"
    );
}

#[test]
fn entry_span_fixed_bars_reads_fixed() {
    assert_eq!(entry_span_label(EntryLength::Bars(5), 2), "5b fixed");
}

#[test]
fn entry_span_guards_zero_length_pattern() {
    // A malformed 0-bar pattern is treated as at least 1 bar.
    assert_eq!(
        entry_span_label(EntryLength::RepeatN(3), 0),
        "1b ×3 = 3b"
    );
}

// ---- Inline stepper ------------------------------------------------------

#[test]
fn stepper_label_shows_times_n_for_repeat_and_bars_for_fixed() {
    assert_eq!(entry_stepper_label(EntryLength::RepeatN(3)), "×3");
    assert_eq!(entry_stepper_label(EntryLength::Bars(4)), "4b");
}

#[test]
fn stepper_value_extracts_raw_count() {
    assert_eq!(entry_stepper_value(EntryLength::RepeatN(3)), 3);
    assert_eq!(entry_stepper_value(EntryLength::Bars(7)), 7);
}

#[test]
fn stepper_preserves_mode_and_steps_up_and_down() {
    assert_eq!(
        step_entry_length(EntryLength::RepeatN(3), 1),
        EntryLength::RepeatN(4)
    );
    assert_eq!(
        step_entry_length(EntryLength::RepeatN(3), -1),
        EntryLength::RepeatN(2)
    );
    assert_eq!(
        step_entry_length(EntryLength::Bars(4), 1),
        EntryLength::Bars(5)
    );
}

#[test]
fn stepper_clamps_at_one_floor() {
    // Never steps below 1 in either mode — a zero-bar entry is invalid.
    assert_eq!(
        step_entry_length(EntryLength::RepeatN(1), -1),
        EntryLength::RepeatN(1)
    );
    assert_eq!(
        step_entry_length(EntryLength::Bars(1), -1),
        EntryLength::Bars(1)
    );
}
