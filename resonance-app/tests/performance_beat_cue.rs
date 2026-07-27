//! Headless tests for the Performance-mode beat ring + count-in cue state
//! derivation (epic #11, todo #310). [`BeatCueState::derive`] is a pure
//! function over a [`ChordReadout`] + transport context, so the ring
//! tracking, the WARM time-until-next arc, and the mint count-in countdown
//! are all verified without booting the GUI.

use std::collections::HashMap;

use resonance_app::compose::{
    ChordState, GenerateParams, SectionDefinitionState, SectionPlacementState,
};
use resonance_app::engine_events::performance::{chord_readout, ChordQuery, ChordReadout};
use resonance_app::view::performance::beat_cue::BeatCueState;
use resonance_audio::types::TempoMap;
use resonance_music_theory::{Chord, ChordQuality, MotifSource, PitchClass};

const SR: u32 = 48_000;
/// 120 bpm, 4/4 → 24_000 samples per beat, 96_000 per bar.
const SPB: u64 = 24_000;

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
        arrangement: Vec::new(),
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

/// One 4-bar section at bar 0 with four 1-bar (4-beat) chords: C, G, Am, F.
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
fn beat_ring_tracks_current_beat() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    // Half-way through bar 2 (the G chord) → beat 3 of 4.
    let playhead = 96_000 + 48_000;
    let readout = chord_readout(&placements, &defs, &map, ChordQuery::at(playhead, SR));
    let cue = BeatCueState::derive(&readout, true, &map, SR, playhead, None);

    assert_eq!(cue.beats_per_bar, 4, "6/8 would give 6; 4/4 gives 4 pips");
    assert_eq!(cue.beat_in_bar, 3, "lit pip tracks the current beat");
    assert!(cue.rolling);
    assert!(cue.count_in_beats.is_none(), "no count-in while playing live");
    // G spans beats 4..8; playhead is at beat 6 → 2 beats until Am at beat 8.
    // arc = 2 / 4 (chord length) = 0.5 of the WARM ring still to play.
    let arc = cue.arc_remaining.expect("a time-until-next arc");
    assert!((arc - 0.5).abs() < 1e-3, "arc = {arc}");
}

#[test]
fn parked_transport_lights_no_beat() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    let playhead = 96_000 + 48_000;
    let readout = chord_readout(&placements, &defs, &map, ChordQuery::at(playhead, SR));
    // rolling = false: the ring rests, no beat is lit.
    let cue = BeatCueState::derive(&readout, false, &map, SR, playhead, None);

    assert_eq!(cue.beat_in_bar, 0);
    assert!(!cue.rolling);
    assert_eq!(cue.beats_per_bar, 4, "pip count still reflects the meter");
}

#[test]
fn arc_steps_per_whole_beat() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    // Beat 5.5 overall = 1.5 beats into bar 2 → beat 2; 2.5 beats until the
    // Am change at beat 8. The arc steps to whole beats: ceil(2.5)=3 → 3/4.
    let playhead = (5.5 * SPB as f64) as u64;
    let readout = chord_readout(&placements, &defs, &map, ChordQuery::at(playhead, SR));
    let cue = BeatCueState::derive(&readout, true, &map, SR, playhead, None);

    assert_eq!(cue.beat_in_bar, 2);
    let arc = cue.arc_remaining.expect("arc present");
    assert!((arc - 0.75).abs() < 1e-3, "stepped arc = {arc}");
}

#[test]
fn no_arc_without_a_current_chord() {
    // Empty project → no chord under the playhead, so no WARM arc.
    let map = tempo_44();
    let readout = chord_readout(&[], &[], &map, ChordQuery::at(48_000, SR));
    assert!(readout.current.is_none(), "sanity: no current chord");
    let cue = BeatCueState::derive(&readout, true, &map, SR, 48_000, None);

    assert!(cue.arc_remaining.is_none(), "no arc with nothing to count down to");
}

#[test]
fn count_in_counts_down_to_primed_chord() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();

    // Pre-count: audio will start at bar 3 (sample 192_000 = beat 8 = Am),
    // the playhead is a bar earlier (sample 96_000 = beat 4).
    let playhead = 96_000;
    let primed = 192_000;
    let query = ChordQuery {
        playhead,
        sample_rate: SR,
        primed_position: Some(primed),
        loop_region: None,
    };
    let readout = chord_readout(&placements, &defs, &map, query);
    assert!(readout.priming, "core flags the readout as priming");
    assert_eq!(
        readout.current.as_ref().map(|s| s.chord_id),
        Some(12),
        "primed onto the first chord that will sound (Am)"
    );

    let cue = BeatCueState::derive(&readout, true, &map, SR, playhead, Some(primed));
    // 192_000 - 96_000 = 96_000 samples = 4 beats of count-in.
    assert_eq!(cue.count_in_beats, Some(4));
}

#[test]
fn count_in_decrements_as_playhead_advances() {
    let (placements, defs) = four_chord_project();
    let map = tempo_44();
    let primed = 192_000; // beat 8

    // 4 beats out → "4".
    let cue4 = derive_count_in(&placements, &defs, &map, 96_000, primed); // beat 4
    assert_eq!(cue4.count_in_beats, Some(4));

    // 1 beat out → "1".
    let cue1 = derive_count_in(&placements, &defs, &map, 168_000, primed); // beat 7
    assert_eq!(cue1.count_in_beats, Some(1));
}

#[test]
fn meter_pip_count_is_capped() {
    // A pathological meter can't blow the ring up past the pip cap.
    let readout = ChordReadout {
        beats_per_bar: 64,
        ..ChordReadout::default()
    };
    let cue = BeatCueState::derive(&readout, true, &tempo_44(), SR, 0, None);
    assert!(cue.beats_per_bar <= 12, "pips capped, got {}", cue.beats_per_bar);
}

// -- helpers -----------------------------------------------------------------

fn derive_count_in(
    placements: &[SectionPlacementState],
    defs: &[SectionDefinitionState],
    map: &TempoMap,
    playhead: u64,
    primed: u64,
) -> BeatCueState {
    let query = ChordQuery {
        playhead,
        sample_rate: SR,
        primed_position: Some(primed),
        loop_region: None,
    };
    let readout = chord_readout(placements, defs, map, query);
    BeatCueState::derive(&readout, true, map, SR, playhead, Some(primed))
}
