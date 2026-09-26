//! The bar-length contract: every consumer of a bar's length must get the
//! same answer for the same time signature.
//!
//! Bar length used to be computed twice — `TempoMap::rebuild_bar_table`
//! said a 7/8 bar was `7 * TICKS_PER_QUARTER_NOTE` (3360 ticks, which is a
//! 7/4 bar) while the quantize `BarRuler` said 1680. Both were tested,
//! both passed, and nothing made them meet, so the disagreement survived
//! for as long as every project stayed in a /4 meter (ba todo #1389).
//!
//! These tests are that meeting point. They deliberately assert *across*
//! consumers rather than within one, so breaking the shared
//! [`bar_len_ticks`] fails here even though each consumer remains
//! internally consistent with itself.

use resonance_audio::midi_io::{parse_midi_file, write_midi_project, MidiTrackSource};
use resonance_audio::quantize::BarRuler;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BPM: f32 = 120.0;

/// Every signature the app can produce, with its bar length in ticks
/// worked out by hand: the denominator pick-list is [2, 4, 8, 16] and the
/// control API accepts 1..=32. A quarter note is 480 ticks, so one beat is
/// `1920 / denominator` ticks and a bar is `numerator` of them.
///
/// These are literals on purpose. A test that only checks the consumers
/// against each other — or against `bar_len_ticks` itself — still passes
/// when the shared function returns the wrong number, since everything
/// then agrees on the wrong bar.
const SIGNATURES: &[(u8, u8, u64)] = &[
    (4, 4, 1920), // 4 * 480
    (3, 4, 1440), // 3 * 480
    (5, 4, 2400), // 5 * 480
    (2, 4, 960), // 2 * 480
    (6, 8, 1440), // 6 * 240
    (7, 8, 1680), // 7 * 240
    (12, 8, 2880), // 12 * 240
    (3, 8, 720), // 3 * 240
    (9, 16, 1080), // 9 * 120
    (5, 16, 600), // 5 * 120
    (2, 2, 1920), // 2 * 960
    (4, 1, 7680), // 4 * 1920
];

fn map_sig(num: u8, den: u8) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = BPM;
    tm.numerator = num;
    tm.denominator = den;
    tm.tempo_points = vec![TempoPoint { bar: 0, bpm: BPM }];
    tm.signature_points = vec![SignaturePoint {
        bar: 0,
        numerator: num,
        denominator: den,
    }];
    tm.rebuild_bar_table(SR);
    tm
}

// =====================================================================
// The tick side: tempo map ↔ quantize grid
// =====================================================================

/// The headline. For every signature, the bar the tempo map hands the
/// engine and the bar the quantiser snaps notes to are the same bar.
#[test]
fn tempo_map_and_quantize_ruler_agree_on_bar_length() {
    for &(num, den, bar_ticks) in SIGNATURES {
        let tm = map_sig(num, den);
        let ruler = BarRuler::new(&tm);

        let mut tick = 0u64;
        for bar in 0..8u32 {
            let from_tempo_map = tm.bar_len_ticks_at(bar);
            let (ruler_start, ruler_len) = ruler.bar_at(tick);
            assert_eq!(
                from_tempo_map, ruler_len,
                "{num}/{den} bar {bar}: tempo map says {from_tempo_map} ticks, \
                 quantize ruler says {ruler_len}"
            );
            assert_eq!(
                ruler_start, tick,
                "{num}/{den} bar {bar}: ruler bar starts at {ruler_start}, \
                 tempo map bar table starts it at {tick}"
            );
            // Both consumers against the hand-computed literal, so a
            // wrong-but-shared answer still fails here.
            assert_eq!(
                from_tempo_map, bar_ticks,
                "{num}/{den} bar {bar}: tempo map bar table"
            );
            assert_eq!(ruler_len, bar_ticks, "{num}/{den} bar {bar}: quantize ruler");
            tick += from_tempo_map;
        }
    }
}

/// The same agreement across a mid-project signature change, where the
/// two implementations accumulate bar starts independently.
#[test]
fn tempo_map_and_quantize_ruler_agree_across_a_signature_change() {
    let mut tm = TempoMap::default();
    tm.bpm = BPM;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.tempo_points = vec![TempoPoint { bar: 0, bpm: BPM }];
    tm.signature_points = vec![
        SignaturePoint {
            bar: 0,
            numerator: 4,
            denominator: 4,
        },
        SignaturePoint {
            bar: 4,
            numerator: 7,
            denominator: 8,
        },
        SignaturePoint {
            bar: 9,
            numerator: 6,
            denominator: 8,
        },
    ];
    tm.rebuild_bar_table(SR);
    let ruler = BarRuler::new(&tm);

    let mut tick = 0u64;
    for bar in 0..14u32 {
        let (ruler_start, ruler_len) = ruler.bar_at(tick);
        assert_eq!(ruler_start, tick, "bar {bar} start");
        assert_eq!(ruler_len, tm.bar_len_ticks_at(bar), "bar {bar} length");
        tick += ruler_len;
    }

    // Bar 4 is where 7/8 takes over: 4 bars of 4/4 = 4 * 1920.
    assert_eq!(ruler.bar_at(4 * 1920).1, 1680);
}

/// A 7/8 bar is seven eighth-notes. Spelled out so the intended number is
/// in the suite in plain sight, not only as a cross-check.
#[test]
fn odd_meter_bar_lengths_are_musically_correct() {
    assert_eq!(bar_len_ticks(4, 4), 1920, "4/4 = one whole note");
    assert_eq!(bar_len_ticks(7, 8), 1680, "7/8 = seven eighth notes");
    assert_eq!(bar_len_ticks(6, 8), 1440, "6/8 = six eighth notes");
    assert_eq!(bar_len_ticks(12, 8), 2880, "12/8 = twelve eighth notes");
    assert_eq!(bar_len_ticks(3, 4), 1440, "3/4 = three quarter notes");
    assert_eq!(bar_len_ticks(9, 16), 1080, "9/16 = nine sixteenth notes");

    // A 7/8 bar is exactly half a 7/4 bar — the shape of the old bug.
    assert_eq!(bar_len_ticks(7, 8) * 2, bar_len_ticks(7, 4));

    assert_eq!(map_sig(7, 8).bar_len_ticks_at(0), 1680);
}

// =====================================================================
// The sample side: it must move with the tick side, not separately
// =====================================================================

/// Ticks and samples in the bar table describe the same bar.
///
/// Fixing only the tick side would leave the table saying a 7/8 bar is
/// 1680 ticks long *and* 7 quarter-notes' worth of samples — worse than
/// the original bug, because the two halves of one entry would disagree.
#[test]
fn bar_table_sample_positions_match_its_tick_positions() {
    let samples_per_quarter = SR as f64 * 60.0 / BPM as f64;
    for &(num, den, bar_ticks) in SIGNATURES {
        let tm = map_sig(num, den);
        let mut tick = 0u64;
        for bar in 0..8u32 {
            assert_eq!(
                tm.bar_len_ticks_at(bar),
                bar_ticks,
                "{num}/{den} bar {bar} tick length"
            );
            let expected = (tick as f64 / TICKS_PER_QUARTER_NOTE as f64) * samples_per_quarter;
            let actual = tm.bar_to_sample(bar) as f64;
            assert!(
                (actual - expected).abs() <= 1.0,
                "{num}/{den} bar {bar}: table sample {actual} but its tick {tick} \
                 is {expected} samples in"
            );
            tick += tm.bar_len_ticks_at(bar);
        }
    }
}

/// The tick↔sample *ratio* is a constant of the tempo, never of the
/// meter: a quarter note lasts the same time in 4/4 and in 7/8.
///
/// This is what keeps existing MIDI clips sounding identical across this
/// change — only the bar *numbering* moves, not the notes.
#[test]
fn a_quarter_note_lasts_the_same_in_every_meter() {
    let samples_per_quarter = (SR as f64 * 60.0 / BPM as f64) as u64;
    for &(num, den, bar_ticks) in SIGNATURES {
        let tm = map_sig(num, den);
        assert_eq!(tm.bar_len_ticks_at(0), bar_ticks, "{num}/{den} bar length");
        let one_quarter = tm.tick_to_abs_sample(0, TICKS_PER_QUARTER_NOTE, SR);
        assert!(
            one_quarter.abs_diff(samples_per_quarter) <= 1,
            "{num}/{den}: a quarter note took {one_quarter} samples, expected \
             {samples_per_quarter}"
        );
        // And the inverse agrees.
        let back = tm.sample_to_abs_tick(samples_per_quarter, SR);
        assert!(
            back.abs_diff(TICKS_PER_QUARTER_NOTE) <= 1,
            "{num}/{den}: {samples_per_quarter} samples read back as {back} ticks"
        );
    }
}

/// `samples_per_bar` (the count-in length) agrees with the bar table.
#[test]
fn samples_per_bar_agrees_with_the_bar_table() {
    for &(num, den, bar_ticks) in SIGNATURES {
        let tm = map_sig(num, den);
        let from_table = tm.bar_to_sample(1) as f64;
        let literal = bar_ticks as f64 / TICKS_PER_QUARTER_NOTE as f64
            * (SR as f64 * 60.0 / BPM as f64);
        assert!(
            (from_table - literal).abs() <= 1.0,
            "{num}/{den}: bar 1 at sample {from_table}, a {bar_ticks}-tick bar is \
             {literal} samples"
        );
        let scalar = tm.samples_per_bar(SR);
        assert!(
            (scalar - from_table).abs() <= 1.0,
            "{num}/{den}: samples_per_bar {scalar} vs bar table {from_table}"
        );
    }
}

// =====================================================================
// Beats
// =====================================================================

/// A beat is a note of value `1/denominator`, so 7/8 has seven of them
/// and the metronome clicks seven times per bar.
#[test]
fn beats_in_bar_counts_the_numerator_not_the_quarter_notes() {
    for &(num, den, bar_ticks) in SIGNATURES {
        let tm = map_sig(num, den);
        assert_eq!(
            tm.beats_in_bar(0),
            num as u32,
            "{num}/{den} should have {num} beats"
        );
        // Every beat resolves, the last one lands inside the bar, and one
        // past the end does not.
        assert_eq!(tm.bar_len_ticks_at(0), bar_ticks, "{num}/{den} bar length");
        let bar_end = tm.bar_to_sample(1);
        for beat in 0..num as u32 {
            let s = tm
                .beat_sample_in_bar(0, beat, SR)
                .unwrap_or_else(|| panic!("{num}/{den} beat {beat} missing"));
            assert!(s < bar_end, "{num}/{den} beat {beat} at {s} past bar end");
        }
        assert!(tm.beat_sample_in_bar(0, num as u32, SR).is_none());
    }
}

/// Beats are evenly spaced and span exactly the bar.
#[test]
fn beat_spacing_fills_the_bar_exactly() {
    for &(num, den, bar_ticks) in SIGNATURES {
        let tm = map_sig(num, den);
        let bar_samples = tm.bar_to_sample(1) as f64;
        let step = bar_samples / num as f64;
        // One beat is the bar's ticks divided by its numerator.
        assert_eq!(bar_ticks % num as u64, 0, "{num}/{den} bar is whole beats");
        assert_eq!(
            bar_ticks / num as u64,
            beat_len_ticks(den),
            "{num}/{den} beat length"
        );
        for beat in 0..num as u32 {
            let s = tm.beat_sample_in_bar(0, beat, SR).unwrap() as f64;
            assert!(
                (s - beat as f64 * step).abs() <= 2.0,
                "{num}/{den} beat {beat}: {s}, expected {}",
                beat as f64 * step
            );
        }
    }
}

// =====================================================================
// MIDI export/import — the third consumer
// =====================================================================

/// A signature event written into an exported conductor track lands at
/// the tick the bar table puts that bar at, and reading the file back
/// recovers the same bar.
#[test]
fn midi_export_places_signature_events_at_the_bar_table_tick() {
    let dir = std::env::temp_dir().join(format!(
        "resonance-bar-length-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("meter.mid");

    let mut tm = TempoMap::default();
    tm.bpm = BPM;
    tm.numerator = 6;
    tm.denominator = 8;
    tm.tempo_points = vec![TempoPoint { bar: 0, bpm: BPM }];
    tm.signature_points = vec![
        SignaturePoint {
            bar: 0,
            numerator: 6,
            denominator: 8,
        },
        SignaturePoint {
            bar: 4,
            numerator: 7,
            denominator: 8,
        },
    ];
    tm.rebuild_bar_table(SR);

    // Bar 4 in 6/8 is four 1440-tick bars in, not four 2880-tick ones.
    let expected_tick = tm.bar_len_ticks_at(0) * 4;
    assert_eq!(expected_tick, 5760);

    let notes = [MidiNote {
        note: 60,
        start_tick: expected_tick,
        duration_ticks: 240,
        velocity: 0.8,
    }];
    write_midi_project(
        &path,
        &tm,
        &[MidiTrackSource {
            name: "t",
            notes: &notes,
        }],
    )
    .unwrap();

    let imported = parse_midi_file(&path).unwrap();
    let sig = imported
        .signature_events
        .iter()
        .find(|e| e.numerator == 7)
        .expect("7/8 event round-tripped");
    assert_eq!(
        sig.tick, expected_tick,
        "7/8 event written at tick {} but the bar table starts bar 4 at {expected_tick}",
        sig.tick
    );
    // Import maps that tick back to bar 4 with its own bar walk, so the
    // signature survives a whole export → import round trip on the bar it
    // was authored on.
    let point = imported
        .signature_points
        .iter()
        .find(|p| p.numerator == 7)
        .expect("7/8 recovered as a bar-indexed point");
    assert_eq!(
        (point.bar, point.denominator),
        (4, 8),
        "7/8 authored at bar 4 came back at bar {}",
        point.bar
    );

    let _ = std::fs::remove_dir_all(&dir);
}
