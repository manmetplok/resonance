//! Rubber-band marquee + rectangle hit-testing for the MIDI piano roll
//! (todo #389). The geometry helpers are pure, so we drive them directly
//! with a fixed layout/viewport and assert which note indices a swept
//! rectangle captures.

use iced::{Point, Rectangle};
use resonance_app::view::piano_roll::{
    note_rect, notes_in_marquee, rect_from_points, rects_intersect, PianoRollLayout,
    PianoRollViewport,
};
use resonance_audio::types::MidiNote;

fn layout() -> PianoRollLayout {
    PianoRollLayout {
        keyboard_w: 50.0,
        grid_top: 0.0,
        grid_h: 400.0,
    }
}

fn viewport() -> PianoRollViewport {
    // 1 px per tick, 10 px per semitone, no scroll.
    PianoRollViewport {
        zoom_x: 1.0,
        zoom_y: 10.0,
        scroll_x: 0.0,
        scroll_y: 0.0,
    }
}

fn note(n: u8, start: u64, dur: u64) -> MidiNote {
    MidiNote {
        note: n,
        velocity: 0.8,
        start_tick: start,
        duration_ticks: dur,
    }
}

#[test]
fn rect_from_points_normalises_any_direction() {
    let r = rect_from_points(Point::new(120.0, 25.0), Point::new(40.0, 0.0));
    assert_eq!(r.x, 40.0);
    assert_eq!(r.y, 0.0);
    assert_eq!(r.width, 80.0);
    assert_eq!(r.height, 25.0);
}

#[test]
fn rects_intersect_is_strict_on_touching_edges() {
    let a = Rectangle { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
    let overlapping = Rectangle { x: 5.0, y: 5.0, width: 10.0, height: 10.0 };
    let touching = Rectangle { x: 10.0, y: 0.0, width: 10.0, height: 10.0 };
    assert!(rects_intersect(a, overlapping));
    assert!(!rects_intersect(a, touching));
}

#[test]
fn note_rect_places_note_in_canvas_space() {
    // note 127 sits in the top row (y = 0); x = keyboard + start tick.
    let r = note_rect(&layout(), &viewport(), &note(127, 0, 20));
    assert_eq!(r.x, 50.0);
    assert_eq!(r.y, 0.0);
    assert_eq!(r.width, 20.0);
    assert_eq!(r.height, 10.0);
}

#[test]
fn marquee_captures_only_intersecting_notes() {
    let notes = vec![
        note(127, 0, 20),   // rect x[50,70]  y[0,10]
        note(126, 40, 20),  // rect x[90,110] y[10,20]
        note(125, 200, 20), // rect x[250,270] y[20,30]
    ];
    // Sweep over the first two notes only.
    let marquee = rect_from_points(Point::new(40.0, 0.0), Point::new(120.0, 25.0));
    let hits = notes_in_marquee(&notes, &layout(), &viewport(), marquee);
    assert_eq!(hits, vec![0, 1]);

    // A marquee clear of every note selects nothing.
    let empty = rect_from_points(Point::new(0.0, 200.0), Point::new(10.0, 220.0));
    assert!(notes_in_marquee(&notes, &layout(), &viewport(), empty).is_empty());
}
