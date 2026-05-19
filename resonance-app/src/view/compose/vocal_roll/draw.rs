//! Per-frame orchestrator for [`VocalRollCanvas`]. [`draw_into`] composes
//! the strips, the note grid, the keyboard, and the velocity lane in one
//! cached pass; the actual paint routines live in [`super::grid`],
//! [`super::notes`], and [`super::keyboard`]. This file also owns the
//! small coordinate-conversion helpers shared with the event dispatcher
//! in [`super::canvas_program`].

use iced::widget::canvas::{self, Frame};
use iced::{Point, Rectangle, Size};

use resonance_music_theory::g2p;

use crate::theme;

use super::{
    note_name, VocalRollCanvas, VocalRollFingerprint, VocalRollState, HEADER_TOTAL_HEIGHT,
    VR_CHORD_STRIP_HEIGHT, VR_KEYBOARD_WIDTH, VR_VELOCITY_LANE_HEIGHT,
};

impl VocalRollCanvas<'_> {
    pub(super) fn fingerprint(&self, state: &VocalRollState) -> VocalRollFingerprint {
        use std::hash::{Hash, Hasher};
        let mut nh = std::collections::hash_map::DefaultHasher::new();
        for n in &self.clip.notes {
            n.note.hash(&mut nh);
            n.start_tick.hash(&mut nh);
            n.duration_ticks.hash(&mut nh);
            n.velocity.to_bits().hash(&mut nh);
        }
        let mut ch = std::collections::hash_map::DefaultHasher::new();
        for c in self.chords {
            c.start_beat.hash(&mut ch);
            c.duration_beats.hash(&mut ch);
            c.chord.root.to_semitone().hash(&mut ch);
        }
        let mut dh = std::collections::hash_map::DefaultHasher::new();
        for l in &self.params.draft {
            l.text.hash(&mut dh);
            l.syllables.hash(&mut dh);
        }
        let mut lh = std::collections::hash_map::DefaultHasher::new();
        for l in self.lyrics {
            l.hash(&mut lh);
        }
        let (lo, hi) = self.params.range;
        VocalRollFingerprint {
            clip_id: self.clip.id,
            notes_len: self.clip.notes.len(),
            notes_hash: nh.finish(),
            scroll_y_bits: self.scroll_y.to_bits(),
            zoom_x_bits: self.zoom_x.to_bits(),
            zoom_y_bits: self.zoom_y.to_bits(),
            snap_ticks: self.snap_ticks,
            selected_note: self.selected_note,
            time_sig_num: self.time_sig_num,
            drag_active: state.drag.is_some(),
            preview_note: state.previewing_note,
            range_lo: lo,
            range_hi: hi,
            chords_hash: ch.finish(),
            draft_hash: dh.finish(),
            lyrics_hash: lh.finish(),
            bpm_bits: self.bpm.to_bits(),
            portamento_ms_bits: self.params.portamento_ms.to_bits(),
            vibrato_bits: self.params.vibrato.to_bits(),
            vibrato_rate_bits: self.params.vibrato_rate.to_bits(),
        }
    }

    pub(super) fn draw_into(&self, frame: &mut Frame, bounds: Rectangle) {
        let grid_x = VR_KEYBOARD_WIDTH;
        let grid_w = bounds.width - VR_KEYBOARD_WIDTH;
        let grid_top = HEADER_TOTAL_HEIGHT;
        let grid_h = bounds.height - HEADER_TOTAL_HEIGHT - VR_VELOCITY_LANE_HEIGHT;

        // Resolve every note's (label, phonemes, is_slur) once. Same
        // helper drives the SVS pipeline, so the lyric labels on
        // note bodies and the phoneme strip are guaranteed to match
        // what the engine will sing.
        let resolved = g2p::resolve_draft(self.params.draft.as_slice());
        let assigned =
            g2p::assign_syllables_to_notes(&resolved, self.lyrics, self.clip.notes.len());

        // Background
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        // Top-left corner — voice label
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(grid_x, HEADER_TOTAL_HEIGHT),
            theme::BG_2,
        );
        frame.fill_text(canvas::Text {
            content: self.voice_label.to_uppercase(),
            position: Point::new(8.0, 6.0),
            color: theme::WARM,
            size: 9.0.into(),
            font: theme::UI_FONT_SEMIBOLD,
            ..canvas::Text::default()
        });
        frame.fill_text(canvas::Text {
            content: format!(
                "{}-{}",
                note_name(self.params.range.0),
                note_name(self.params.range.1)
            ),
            position: Point::new(8.0, 20.0),
            color: theme::TEXT_3,
            size: 9.0.into(),
            font: theme::MONO_FONT,
            ..canvas::Text::default()
        });

        // Chord strip + phoneme strip across the grid width.
        self.draw_chord_strip(frame, grid_x, grid_w);
        self.draw_phoneme_strip(frame, grid_x, grid_w, &assigned);

        // Note row backgrounds
        self.draw_note_rows(frame, grid_x, grid_w, grid_top, grid_h);

        // Bar / beat lines
        self.draw_grid_lines(frame, grid_x, grid_w, grid_top, grid_h);

        // Notes — drawn before slurs/pitch curve so those overlay on top.
        self.draw_notes(frame, grid_x, grid_top, grid_h, &assigned);

        // Slur arcs between adjacent flowing notes.
        self.draw_slurs(frame, grid_x, grid_top, grid_h, &assigned);

        // Rendered f0 contour overlay (portamento + vibrato).
        self.draw_pitch_curve(frame, grid_x, grid_w, grid_top, grid_h);

        // Lexical-stress overlay — step curve floating above each note
        // showing CMU's stress mark for that syllable. Same visual idea
        // as the pitch curve but in ACCENT so the two can sit on top of
        // each other without confusion.
        self.draw_stress_curve(frame, grid_x, grid_w, grid_top, grid_h, &assigned);

        // Piano keyboard
        self.draw_keyboard(frame, grid_top, grid_h);

        // Velocity lane
        self.draw_velocity_lane(frame, grid_x, grid_w, bounds.height);

        // Separator lines
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(1.0, bounds.height - VR_VELOCITY_LANE_HEIGHT),
            theme::SEPARATOR,
        );
        frame.fill_rectangle(
            Point::new(0.0, grid_top + grid_h),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
        frame.fill_rectangle(
            Point::new(0.0, VR_CHORD_STRIP_HEIGHT),
            Size::new(bounds.width, 1.0),
            theme::LINE_2,
        );
        frame.fill_rectangle(
            Point::new(0.0, HEADER_TOTAL_HEIGHT),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
    }
}

impl VocalRollCanvas<'_> {
    pub(super) fn tick_to_x(&self, tick: u64) -> f32 {
        tick as f32 * self.zoom_x
    }

    pub(super) fn x_to_tick(&self, x: f32) -> u64 {
        if x <= 0.0 {
            0
        } else {
            (x / self.zoom_x) as u64
        }
    }

    pub(super) fn duration_to_width(&self, ticks: u64) -> f32 {
        ticks as f32 * self.zoom_x
    }

    /// Convert MIDI note number to pixel y inside the *grid band*
    /// (relative to grid_top). Returns the top of the row.
    /// Notes outside `params.range` map outside the band — callers
    /// clip them.
    pub(super) fn note_to_y(&self, note: u8, _grid_h: f32) -> Option<f32> {
        let (lo, hi) = self.params.range;
        if note < lo || note > hi {
            return None;
        }
        let row = (hi - note) as f32; // top row is hi
        Some(row * self.zoom_y - self.scroll_y)
    }

    /// Inverse — pixel y inside the grid band → MIDI note number, or
    /// `None` if outside the visible row range. Allows scroll past the
    /// top/bottom edges.
    pub(super) fn y_to_note(&self, y: f32, _grid_h: f32) -> Option<u8> {
        let (lo, hi) = self.params.range;
        let row = ((y + self.scroll_y) / self.zoom_y).floor() as i32;
        if row < 0 {
            return None;
        }
        let note = hi as i32 - row;
        if note < lo as i32 || note > hi as i32 {
            return None;
        }
        Some(note as u8)
    }

    pub(super) fn snap(&self, tick: u64) -> u64 {
        if self.snap_ticks == 0 {
            return tick;
        }
        let half = self.snap_ticks / 2;
        ((tick + half) / self.snap_ticks) * self.snap_ticks
    }
}
