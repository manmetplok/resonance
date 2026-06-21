//! Headless tests for the Performance-mode chord-derivation core
//! (epic #11, todo #304): mapping the transport playhead onto placed
//! sections + chord progressions through the tempo map.
//!
//! Covers current/next/gap/loop/count-in cases plus an empty project and
//! a tempo/time-signature change mid-take, per the acceptance criteria.

use std::collections::HashMap;

use resonance_app::compose::{
    ChordState, GenerateParams, SectionDefinitionState, SectionPlacementState,
};
use resonance_app::engine_events::performance::{chord_readout, ChordQuery, UPCOMING_COUNT};
use resonance_audio::types::{SignaturePoint, TempoMap, TempoPoint};
use resonance_music_theory::{Chord, ChordQuality, MotifSource, PitchClass};

const SR: u32 = 48_000;

/// 120 bpm, 4/4. samples_per_beat = 24_000, samples_per_bar = 96_000.
fn tempo_44() -> TempoMap {
    let mut map = TempoMap::default();
    map.rebuild_bar_table(SR);
    map
}

fn chord(id: u64, start_beat: u32, duration_beats: u32, c: Chord) -> ChordState {
    ChordState {
        id,
        start_beat,
        duration_beats,
        chord: c,
    }
}

/// Build a minimal section definition with the given chords. The
/// generator/motif fields are defaulted — the derivation never reads them.
fn def(id: u64, length_bars: u32, chords: Vec<ChordState>) -> SectionDefinitionState {
    SectionDefinitionState {
        id,
        name: format!("S{id}"),
        color: [0, 0, 0],
        length_bars,
        chords,
        scale: None,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        drum_pattern_id: None,
    }
}

fn placement(id: u64, definition_id: u64, start_bar: u32) -> SectionPlacementState {
    SectionPlacementState {
        id,
        definition_id,
        start_bar,
    }
}

fn c(root: PitchClass, q: ChordQuality) -> Chord {
    Chord::new(root, q)
}

/// One 4-bar section at bar 0 with four 1-bar chords: C, G, Am, F.
fn four_chord_project() -> (Vec<SectionPlacementState>, Vec<SectionDefinitionState>) {
    let chords = vec![
        chord(10, 0, 4, c(PitchClass::C, ChordQuality::Maj)),
        chord(11, 4, 4, c(PitchClass::G, ChordQuality::Maj)),
        chord(12, 8, 4, c(PitchClass::A, ChordQuality::Min)),
        chord(13, 12, 4, c(PitchClass::F, ChordQuality::Maj)),
    ];
    (vec![placement(1, 100, 0)], vec![def(100, 4, chords)])
}

#[test]
fn current_chord_under_playhead() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    // Middle of bar 2 (the G chord): bar index 1, half-way → sample 96_000 + 48_000.
    let q = ChordQuery::at(96_000 + 48_000, SR);
    let r = chord_readout(&placements, &defs, &map, q);

    let cur = r.current.expect("a chord under the playhead");
    assert_eq!(cur.chord_id, 11);
    assert_eq!(cur.chord, c(PitchClass::G, ChordQuality::Maj));
    assert_eq!(cur.start_bar, 1);
    // 4/4 → beats_per_bar 4; half-way through bar 2 is beat 3.
    assert_eq!(r.beats_per_bar, 4);
    assert_eq!(r.beat_in_bar, 3);
    assert!((r.beat_phase - 0.0).abs() < 1e-6);
}

#[test]
fn upcoming_lists_next_chords_in_order() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    // Start of the first chord (C). Next up: G, Am, F.
    let r = chord_readout(&placements, &defs, &map, ChordQuery::at(0, SR));
    assert_eq!(r.current.as_ref().map(|s| s.chord_id), Some(10));
    let ids: Vec<u64> = r.upcoming.iter().map(|s| s.chord_id).collect();
    assert_eq!(ids, vec![11, 12, 13]);
    assert_eq!(r.upcoming.len(), UPCOMING_COUNT);

    // Distance to the next change: a full bar = 4 grid beats = 1 bar.
    assert!((r.beats_until_next.unwrap() - 4.0).abs() < 1e-6);
    assert_eq!(r.bars_until_next, Some(1));
}

#[test]
fn upcoming_truncates_near_the_end() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    // Inside the last chord (F, bar 4). Nothing comes after it.
    let r = chord_readout(&placements, &defs, &map, ChordQuery::at(96_000 * 3 + 100, SR));
    assert_eq!(r.current.as_ref().map(|s| s.chord_id), Some(13));
    assert!(r.upcoming.is_empty());
    assert_eq!(r.beats_until_next, None);
    assert_eq!(r.bars_until_next, None);
}

#[test]
fn gap_with_no_section_returns_none_current_but_next_upcoming() {
    // Section placed at bar 2 (start_bar = 2), so bars 0–1 are an empty gap.
    let chords = vec![
        chord(20, 0, 4, c(PitchClass::D, ChordQuality::Min)),
        chord(21, 4, 4, c(PitchClass::A, ChordQuality::Maj)),
    ];
    let placements = vec![placement(1, 200, 2)];
    let defs = vec![def(200, 2, chords)];
    let map = tempo_44();

    // Playhead in bar 1 (the gap before the section).
    let r = chord_readout(&placements, &defs, &map, ChordQuery::at(96_000 + 1_000, SR));
    assert!(r.current.is_none(), "no chord under the playhead in a gap");
    assert_eq!(
        r.upcoming.first().map(|s| s.chord_id),
        Some(20),
        "the next upcoming chord is still reported in a gap"
    );
    // Next change starts at bar 2 = global beat 8; playhead ~ bar1 + a hair.
    let beats = r.beats_until_next.unwrap();
    assert!(beats > 3.9 && beats < 4.1, "≈4 beats until the section, got {beats}");
}

#[test]
fn empty_project_has_no_chords() {
    let map = tempo_44();
    let r = chord_readout(&[], &[], &map, ChordQuery::at(50_000, SR));
    assert!(r.current.is_none());
    assert!(r.upcoming.is_empty());
    assert_eq!(r.beats_until_next, None);
    assert_eq!(r.bars_until_next, None);
    // Beat telemetry is still valid for an empty project.
    assert_eq!(r.beats_per_bar, 4);
}

#[test]
fn loop_region_wraps_upcoming_back_to_loop_in() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    // Loop the whole 4-bar section: loop_in = 0, loop_out = 4 bars.
    let loop_out = 96_000 * 4;
    // Sit inside the last chord (F). Without wrap there are no upcoming
    // chords; with the loop, the look-ahead wraps to C, G, Am.
    let q = ChordQuery {
        playhead: 96_000 * 3 + 100,
        sample_rate: SR,
        primed_position: None,
        loop_region: Some((0, loop_out)),
    };
    let r = chord_readout(&placements, &defs, &map, q);
    assert_eq!(r.current.as_ref().map(|s| s.chord_id), Some(13));
    let ids: Vec<u64> = r.upcoming.iter().map(|s| s.chord_id).collect();
    assert_eq!(ids, vec![10, 11, 12], "upcoming wraps through the loop seam");

    // Time until the next change crosses the loop seam: from just after the
    // start of bar 4 to loop_out (~4 beats) is the wrap distance to C.
    let beats = r.beats_until_next.unwrap();
    assert!(beats > 3.9 && beats < 4.0, "wrap distance ≈4 beats, got {beats}");
}

#[test]
fn primed_position_reads_first_chord_during_count_in() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    // Count-in: the live playhead sits before the take (in the pre-count),
    // but the primed position points at where the first chord (C) sounds.
    let q = ChordQuery {
        playhead: 0,
        sample_rate: SR,
        // Two-bar pre-count means the take begins at sample 0; model the
        // primed position as the first chord's onset directly.
        primed_position: Some(1_000),
        loop_region: None,
    };
    let r = chord_readout(&placements, &defs, &map, q);
    assert!(r.priming, "priming flag set when a primed position is given");
    assert_eq!(
        r.current.as_ref().map(|s| s.chord_id),
        Some(10),
        "the first chord is primed during the count-in"
    );
    assert_eq!(r.upcoming.first().map(|s| s.chord_id), Some(11));
}

#[test]
fn tempo_and_signature_change_mid_take() {
    // Bars 0–1 are 4/4; from bar 2 the meter switches to 3/4 and the tempo
    // steps to 90 bpm. The derivation must place chords through the changed
    // bar table.
    let mut map = TempoMap::default();
    // Declare the bar-0 meter explicitly: `rebuild_bar_table` seeds the
    // initial numerator from the first signature point, so the opening 4/4
    // span needs its own point before the bar-2 switch to 3/4.
    map.signature_points = vec![
        SignaturePoint {
            bar: 0,
            numerator: 4,
            denominator: 4,
        },
        SignaturePoint {
            bar: 2,
            numerator: 3,
            denominator: 4,
        },
    ];
    map.tempo_points = vec![TempoPoint { bar: 2, bpm: 90.0 }];
    map.rebuild_bar_table(SR);

    // Section spans bars 0–3 with a chord per bar. Bars 0,1 hold 4 grid
    // beats; bars 2,3 hold 3 grid beats.
    //   bar 0: beats 0..4   -> C
    //   bar 1: beats 4..8   -> G
    //   bar 2: beats 8..11  -> Am   (3/4)
    //   bar 3: beats 11..14 -> F    (3/4)
    let chords = vec![
        chord(30, 0, 4, c(PitchClass::C, ChordQuality::Maj)),
        chord(31, 4, 4, c(PitchClass::G, ChordQuality::Maj)),
        chord(32, 8, 3, c(PitchClass::A, ChordQuality::Min)),
        chord(33, 11, 3, c(PitchClass::F, ChordQuality::Maj)),
    ];
    let placements = vec![placement(1, 300, 0)];
    let defs = vec![def(300, 4, chords)];

    // Sample at the start of bar 3 (the second 3/4 bar) → the F chord.
    let bar3_sample = map.bar_to_sample(3);
    let r = chord_readout(&placements, &defs, &map, ChordQuery::at(bar3_sample + 10, SR));
    let cur = r.current.expect("a chord at bar 3");
    assert_eq!(cur.chord_id, 33, "F chord under the playhead in the 3/4 region");
    assert_eq!(cur.start_bar, 3);
    assert_eq!(r.beats_per_bar, 3, "3/4 meter active from bar 2");
    assert_eq!(r.beat_in_bar, 1);

    // And the Am chord lives in bar 2 with a 3-beat span.
    let bar2_sample = map.bar_to_sample(2);
    let r2 = chord_readout(&placements, &defs, &map, ChordQuery::at(bar2_sample + 10, SR));
    assert_eq!(r2.current.as_ref().map(|s| s.chord_id), Some(32));
    assert_eq!(r2.current.as_ref().unwrap().start_bar, 2);
    assert_eq!(r2.upcoming.first().map(|s| s.chord_id), Some(33));
}
