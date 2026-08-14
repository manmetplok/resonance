//! The tempo-division grid (ba todo #1265).
//!
//! `bpm + division -> ms` used to be re-derived inside drawing and
//! input code — three copies, each fabricating a `TempoInfo` just to
//! reach the table, none of them testable without egui. These tests
//! cover the pure helpers all four call sites (DSP, division ticks,
//! stepper readout, drag-to-snap) now share.

use resonance_granular_delay::sync::{
    delay_seconds, division_beats, division_ms, division_seconds, nearest_division,
    DIVISION_LABELS,
};
use resonance_plugin::TempoInfo;

const MAX_DELAY: f32 = 4.0;

fn index_of(label: &str) -> usize {
    DIVISION_LABELS
        .iter()
        .position(|l| *l == label)
        .unwrap_or_else(|| panic!("no division labelled {label}"))
}

#[test]
fn every_label_has_a_length() {
    for (i, label) in DIVISION_LABELS.iter().enumerate() {
        let beats = division_beats(i);
        assert!(beats > 0.0, "{label} resolves to {beats} beats");
    }
}

#[test]
fn dotted_and_triplet_lengths_are_musical() {
    let quarter = division_beats(index_of("1/4"));
    assert_eq!(quarter, 1.0, "a quarter note is one beat");
    assert!((division_beats(index_of("1/4D")) - quarter * 1.5).abs() < 1e-6);
    assert!((division_beats(index_of("1/4T")) - quarter * 2.0 / 3.0).abs() < 1e-6);
    assert!((division_beats(index_of("1/8")) - quarter * 0.5).abs() < 1e-6);
    assert!((division_beats(index_of("1/1")) - quarter * 4.0).abs() < 1e-6);
}

#[test]
fn division_ms_at_one_twenty_bpm() {
    // 120 BPM: one beat is 500 ms.
    for (label, expected) in [
        ("1/1", 2000.0),
        ("1/2", 1000.0),
        ("1/4", 500.0),
        ("1/4D", 750.0),
        ("1/8", 250.0),
        ("1/16", 125.0),
    ] {
        let ms = division_ms(120.0, index_of(label));
        assert!(
            (ms - expected).abs() < 1e-3,
            "{label} at 120 BPM is {ms} ms, expected {expected}"
        );
    }
    // Triplets: three of them fill the plain division.
    let eighth_t = division_ms(120.0, index_of("1/8T"));
    assert!((eighth_t * 3.0 - division_ms(120.0, index_of("1/4"))).abs() < 1e-3);
}

#[test]
fn tempo_is_floored_so_a_division_cannot_run_away() {
    // Below 20 BPM the grid is resolved at 20 BPM, matching the DSP.
    assert_eq!(division_ms(5.0, index_of("1/4")), division_ms(20.0, index_of("1/4")));
    assert_eq!(division_ms(0.0, index_of("1/4")), division_ms(20.0, index_of("1/4")));
}

#[test]
fn out_of_range_divisions_clamp_to_the_table() {
    let last = DIVISION_LABELS.len() - 1;
    assert_eq!(division_beats(last + 99), division_beats(last));
}

/// The audio path and the editor must agree: `delay_seconds` is the
/// clamped form of the same conversion the editor draws with.
#[test]
fn delay_seconds_is_the_clamped_division_length() {
    let tempo = |bpm, num, den| TempoInfo {
        bpm,
        time_sig_num: num,
        time_sig_den: den,
        playing: true,
        song_pos_beats: 12.5,
    };
    for (division, label) in DIVISION_LABELS.iter().enumerate() {
        let dsp = delay_seconds(true, division, 0.0, Some(tempo(120.0, 4, 4)), MAX_DELAY);
        let editor = division_seconds(120.0, division).clamp(0.001, MAX_DELAY);
        assert_eq!(
            dsp.to_bits(),
            editor.to_bits(),
            "{label} diverges: dsp {dsp} vs editor {editor}"
        );
    }
}

/// Divisions are note values, so they do not depend on the meter — the
/// transport's BPM counts quarter notes in every time signature. The
/// editor used to hardcode `4/4` in a fabricated transport; this pins
/// that the hardcoded value never mattered, and that a future meter
/// term would be a behaviour change, not a bug fix.
#[test]
fn division_length_does_not_depend_on_the_time_signature() {
    let at = |num, den| {
        delay_seconds(
            true,
            index_of("1/4"),
            0.0,
            Some(TempoInfo {
                bpm: 96.0,
                time_sig_num: num,
                time_sig_den: den,
                playing: false,
                song_pos_beats: 0.0,
            }),
            MAX_DELAY,
        )
    };
    let four_four = at(4, 4);
    for (num, den) in [(3u16, 4u16), (6, 8), (5, 4), (7, 8), (12, 8)] {
        assert_eq!(
            at(num, den).to_bits(),
            four_four.to_bits(),
            "a quarter note changed length in {num}/{den}"
        );
    }
}

/// Free-running (or synced with no host transport) falls back to the
/// millisecond time parameter.
#[test]
fn without_sync_or_tempo_the_time_param_wins() {
    assert!((delay_seconds(false, 0, 250.0, None, MAX_DELAY) - 0.25).abs() < 1e-6);
    assert!((delay_seconds(true, 0, 250.0, None, MAX_DELAY) - 0.25).abs() < 1e-6);
    // And the buffer length is the ceiling.
    assert_eq!(delay_seconds(false, 0, 99_000.0, None, MAX_DELAY), MAX_DELAY);
    assert_eq!(delay_seconds(false, 0, 0.0, None, MAX_DELAY), 0.001);
}

#[test]
fn nearest_division_finds_the_exact_grid_points() {
    for (division, label) in DIVISION_LABELS.iter().enumerate() {
        let ms = division_ms(132.0, division);
        assert_eq!(
            nearest_division(132.0, ms),
            division,
            "{label} did not round-trip"
        );
    }
}

#[test]
fn nearest_division_snaps_targets_between_grid_points() {
    let quarter = index_of("1/4");
    let eighth = index_of("1/8");
    // 120 BPM: 1/8 = 250 ms, 1/4 = 500 ms.
    assert_eq!(nearest_division(120.0, 260.0), eighth);
    assert_eq!(nearest_division(120.0, 480.0), quarter);
    // Far outside the table: snap to its ends.
    assert_eq!(nearest_division(120.0, 99_000.0), index_of("1/1"));
    assert_eq!(nearest_division(120.0, 0.0), index_of("1/16T"));
}
