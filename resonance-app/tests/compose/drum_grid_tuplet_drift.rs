//! Drum groups on grids that don't divide a quarter note evenly keep their
//! steps on the beat grid (code review VIEW-24).
//!
//! `step_ticks = 480 / 7 = 68` and `start_tick = step * 68` lost 4 ticks
//! per beat: across an 8-bar 4/4 section the last hit landed 128 ticks
//! early, and every bar was audibly further ahead than the last.

use resonance_app::compose::{ArrangementSpan, DrumGroup, DrumGroupPad};
use resonance_app::update::compose::build_drum_notes;
use resonance_audio::types::TICKS_PER_QUARTER_NOTE;

const TS: u8 = 4;
const TPQ: u64 = TICKS_PER_QUARTER_NOTE;
const BARS: u32 = 8;

/// A single-pad group that fires on every step of `grid` steps per beat.
fn every_step(grid: u8) -> DrumGroup {
    DrumGroup {
        id: 1,
        name: "tuplets".into(),
        color: [0, 0, 0],
        grid,
        cycle: 1,
        phase: 0,
        pads: vec![DrumGroupPad {
            name: "hat".into(),
            note: 42,
            weight: 100,
            pattern: vec![1],
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

fn notes_for(grid: u8) -> Vec<resonance_audio::types::MidiNote> {
    let span = ArrangementSpan {
        bar_start: 0,
        bar_end: BARS,
        pattern_id: 1,
        is_fill: false,
    };
    build_drum_notes(&[(span, vec![every_step(grid)])], TS)
}

#[test]
fn grid_7_steps_stay_on_every_beat() {
    let notes = notes_for(7);
    assert_eq!(notes.len(), (BARS as usize) * TS as usize * 7);
    // Every 7th step is a beat start and must land exactly on the beat.
    for (i, n) in notes.iter().enumerate().step_by(7) {
        let beat = (i / 7) as u64;
        assert_eq!(n.start_tick, beat * TPQ, "beat {beat} drifted");
    }
    // In particular the first step of the last bar is on its downbeat.
    let last_bar_first = &notes[(BARS as usize - 1) * TS as usize * 7];
    assert_eq!(last_bar_first.start_tick, (BARS as u64 - 1) * TS as u64 * TPQ);
}

#[test]
fn grid_7_steps_tile_the_section_without_gaps() {
    let notes = notes_for(7);
    for pair in notes.windows(2) {
        assert_eq!(
            pair[0].start_tick + pair[0].duration_ticks,
            pair[1].start_tick,
            "a step's duration must reach the next onset"
        );
    }
    let last = notes.last().expect("notes");
    assert_eq!(last.start_tick + last.duration_ticks, BARS as u64 * TS as u64 * TPQ);
}
