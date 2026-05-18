//! Pure-draw helpers and small coordinate-conversion methods for
//! [`VocalRollCanvas`]. The draw entry [`draw_into`] composes the strips,
//! the note grid, the keyboard, and the velocity lane in one cached pass.

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Size};

use resonance_audio::types::TICKS_PER_QUARTER_NOTE;
use resonance_music_theory::g2p;

use crate::theme;

use super::{
    chord_label, is_black_key, note_name, VocalRollCanvas, VocalRollFingerprint, VocalRollState,
    HEADER_TOTAL_HEIGHT, VR_CHORD_STRIP_HEIGHT, VR_KEYBOARD_WIDTH, VR_PHONEME_STRIP_HEIGHT,
    VR_VELOCITY_LANE_HEIGHT,
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

    fn draw_note_rows(&self, frame: &mut Frame, grid_x: f32, grid_w: f32, grid_top: f32, grid_h: f32) {
        // Backdrop
        frame.fill_rectangle(
            Point::new(grid_x, grid_top),
            Size::new(grid_w, grid_h),
            theme::BG_2,
        );
        let (lo, hi) = self.params.range;
        for note in lo..=hi {
            let Some(y_local) = self.note_to_y(note, grid_h) else {
                continue;
            };
            let y = grid_top + y_local;
            let h = self.zoom_y;
            if y + h < grid_top || y > grid_top + grid_h {
                continue;
            }
            if is_black_key(note) {
                frame.fill_rectangle(Point::new(grid_x, y), Size::new(grid_w, h), theme::BG_1);
            }
            if note % 12 == 0 {
                frame.fill_rectangle(
                    Point::new(grid_x, y + h - 1.0),
                    Size::new(grid_w, 1.0),
                    theme::LINE_2,
                );
            }
        }
    }

    fn draw_grid_lines(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        grid_top: f32,
        grid_h: f32,
    ) {
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let ticks_per_bar = TICKS_PER_QUARTER_NOTE * self.time_sig_num as u64;
        let pixels_per_beat = ticks_per_beat as f32 * self.zoom_x;
        if pixels_per_beat < 4.0 {
            return;
        }
        let max_tick = ((grid_w / self.zoom_x) as u64) + ticks_per_beat;
        let mut tick = 0u64;
        while tick <= max_tick {
            let x = grid_x + self.tick_to_x(tick);
            if x > grid_x + grid_w {
                break;
            }
            let is_bar = tick.is_multiple_of(ticks_per_bar);
            let color = if is_bar { theme::BAR_LINE } else { theme::BEAT_LINE };
            frame.fill_rectangle(
                Point::new(x, grid_top),
                Size::new(1.0, grid_h),
                color,
            );
            tick = tick.saturating_add(ticks_per_beat);
        }

        // 16th sub-divisions when zoomed in.
        let snap_px = self.snap_ticks as f32 * self.zoom_x;
        if snap_px >= 8.0 && self.snap_ticks < ticks_per_beat {
            let mut tick = 0u64;
            while tick <= max_tick {
                if !tick.is_multiple_of(ticks_per_beat) {
                    let x = grid_x + self.tick_to_x(tick);
                    if x > grid_x + grid_w {
                        break;
                    }
                    frame.fill_rectangle(
                        Point::new(x, grid_top),
                        Size::new(1.0, grid_h),
                        Color { a: 0.5, ..theme::LINE_2 },
                    );
                }
                tick = tick.saturating_add(self.snap_ticks);
            }
        }
    }

    fn draw_notes(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_top: f32,
        grid_h: f32,
        assigned: &[g2p::AssignedSyllable],
    ) {
        for (i, n) in self.clip.notes.iter().enumerate() {
            let Some(y_local) = self.note_to_y(n.note, grid_h) else {
                continue;
            };
            let x = grid_x + self.tick_to_x(n.start_tick);
            let w = self.duration_to_width(n.duration_ticks);
            let y = grid_top + y_local;
            let h = self.zoom_y;
            if y + h < grid_top || y > grid_top + grid_h {
                continue;
            }
            let v = n.velocity.clamp(0.0, 1.0);
            let is_slur = assigned.get(i).map(|a| a.is_slur).unwrap_or(false);
            // Slur notes paint thinner and more transparent — visually
            // says "this isn't a new attack, just a pitch change inside
            // the previous syllable". Matches the engraving convention
            // where slurred notes share a beam / common phrase.
            let body_color = if is_slur {
                Color {
                    a: 0.30 + 0.25 * v,
                    ..theme::WARM
                }
            } else {
                Color {
                    a: 0.55 + 0.40 * v,
                    ..theme::WARM
                }
            };
            let body = if w >= 4.0 && h >= 4.0 {
                Path::rounded_rectangle(Point::new(x, y), Size::new(w, h), 2.0.into())
            } else {
                Path::rectangle(Point::new(x, y), Size::new(w, h))
            };
            frame.fill(&body, body_color);
            let selected = self.selected_note == Some(i);
            let stroke_color = if selected {
                theme::WARM
            } else if is_slur {
                Color { a: 0.60, ..theme::WARM }
            } else {
                theme::WARM_LINE
            };
            let stroke_w = if selected { 1.5 } else { 1.0 };
            frame.stroke(
                &body,
                Stroke::default().with_color(stroke_color).with_width(stroke_w),
            );
            // Selected: outer glow ring.
            if selected {
                frame.stroke(
                    &Path::rounded_rectangle(
                        Point::new(x - 1.5, y - 1.5),
                        Size::new(w + 3.0, h + 3.0),
                        3.0.into(),
                    ),
                    Stroke::default()
                        .with_color(Color { a: 0.45, ..theme::WARM })
                        .with_width(1.0),
                );
            }

            // Slur notes get a thin dashed top edge — visual "tie"
            // affordance that reads even when the slur arc above is
            // outside the viewport.
            if is_slur && w >= 6.0 {
                let dash_y = y + 1.0;
                let dash_count = ((w / 4.0) as usize).max(1);
                let dash_step = w / dash_count as f32;
                for d in 0..dash_count {
                    let dx = x + d as f32 * dash_step + dash_step * 0.2;
                    let dw = (dash_step * 0.5).max(1.0);
                    frame.fill_rectangle(
                        Point::new(dx, dash_y),
                        Size::new(dw, 1.0),
                        theme::WARM,
                    );
                }
            }

            // Syllable / slur marker on the note body — only when the
            // note is wide enough for the text to fit. Italic serif
            // matches the rest of the vocal UI; ink colour is dark
            // against the warm body for legibility. Slur notes show
            // the `+` marker centred (smaller) so the visual reads as
            // a continuation rather than a syllable.
            let label = assigned.get(i).map(|a| a.label.clone()).unwrap_or_default();
            if !label.is_empty() && w >= 12.0 && h >= 10.0 {
                let (size, dx) = if is_slur {
                    ((h * 0.7).min(11.0), w * 0.5 - 3.0)
                } else {
                    ((h * 0.85).min(13.0), 4.0)
                };
                frame.fill_text(canvas::Text {
                    content: label,
                    position: Point::new(x + dx, y - 1.0),
                    color: if is_slur {
                        Color::from_rgba(0.08, 0.06, 0.04, 0.7)
                    } else {
                        Color::from_rgba(0.08, 0.06, 0.04, 0.92)
                    },
                    size: size.into(),
                    font: theme::SERIF_ITALIC_FONT,
                    ..canvas::Text::default()
                });
            }
        }
    }

    /// Slur arcs between adjacent notes. Driven by
    /// `AssignedSyllable::is_slur` — the same flag the SVS pipeline
    /// reads — so a visible arc always corresponds to a melisma the
    /// engine will actually sing. The arc rises above the higher of
    /// the two notes (standard engraving for legato).
    fn draw_slurs(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_top: f32,
        grid_h: f32,
        assigned: &[g2p::AssignedSyllable],
    ) {
        if self.clip.notes.len() < 2 {
            return;
        }
        for (i, win) in self.clip.notes.windows(2).enumerate() {
            let (a, b) = (&win[0], &win[1]);
            let b_index = i + 1;
            let connected = assigned.get(b_index).map(|x| x.is_slur).unwrap_or(false);
            if !connected {
                continue;
            }
            let Some(ay) = self.note_to_y(a.note, grid_h) else { continue };
            let Some(by) = self.note_to_y(b.note, grid_h) else { continue };
            let ax = grid_x + self.tick_to_x(a.start_tick + a.duration_ticks);
            let bx = grid_x + self.tick_to_x(b.start_tick);
            let top_y_local = ay.min(by) - 4.0;
            let mid_x = (ax + bx) * 0.5;
            let p_a = Point::new(ax, grid_top + ay);
            let p_b = Point::new(bx, grid_top + by);
            let arc = Path::new(|p| {
                p.move_to(p_a);
                p.quadratic_curve_to(Point::new(mid_x, grid_top + top_y_local - 6.0), p_b);
            });
            frame.stroke(
                &arc,
                Stroke::default()
                    .with_color(Color { a: 0.85, ..theme::WARM })
                    .with_width(1.4),
            );
        }
    }

    /// Synthesise the f0 path the SVS engine will sing and draw it as a
    /// thin overlay on the note grid. Walks the notes back-to-back,
    /// interpolating pitch across `portamento_ms` between adjacent notes
    /// and applying a sinusoidal vibrato wobble in the sustain tail of
    /// long notes. This isn't the engine's exact f0 (which depends on
    /// model behaviour) but it's the same formula the pipeline uses to
    /// generate its starting f0_seq, so what the user sees is what
    /// they'll hear modulo the model's own micro-variations.
    fn draw_pitch_curve(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        grid_top: f32,
        _grid_h: f32,
    ) {
        if self.clip.notes.is_empty() {
            return;
        }
        // Sample density — one sample per 2 px. The curve gets resampled
        // anyway by Iced; this just controls how many control points
        // the polyline has before the renderer takes over.
        let px_per_sample: f32 = 2.0;
        let total_w = grid_w.max(1.0);
        let samples = (total_w / px_per_sample) as usize;
        if samples < 4 {
            return;
        }
        // Pre-compute the portamento radius in ticks. Uses the section's
        // real BPM (plumbed through from `app.transport.bpm`) so the
        // preview curve and the SVS-rendered audio agree on the
        // absolute portamento duration.
        let bpm = self.bpm.max(1.0);
        let portamento_ticks =
            ((self.params.portamento_ms / 1000.0) * (bpm / 60.0) * TICKS_PER_QUARTER_NOTE as f32)
                .max(0.0) as u64;
        let vibrato_depth_st = (self.params.vibrato * 0.45).clamp(0.0, 0.45);
        let vibrato_rate_hz = self.params.vibrato_rate.clamp(2.0, 9.0);

        // The view-space y for a (possibly fractional) MIDI pitch.
        let pitch_y_view = |midi: f32| -> Option<f32> {
            let (lo, hi) = self.params.range;
            if midi < lo as f32 - 1.0 || midi > hi as f32 + 1.0 {
                return None;
            }
            let row = hi as f32 - midi;
            Some(grid_top + (row * self.zoom_y - self.scroll_y) + self.zoom_y * 0.5)
        };

        // Find the rendered pitch at tick `t` — piecewise constant note
        // pitch, with linear portamento ramps that *finish* at each
        // note start tick (matches the engine's portamento_frames
        // back-fill).
        let pitch_at = |t: u64| -> Option<f32> {
            // Find the note covering t (or the previous note for gaps).
            let mut prev: Option<&resonance_audio::types::MidiNote> = None;
            let mut cur: Option<&resonance_audio::types::MidiNote> = None;
            for n in &self.clip.notes {
                if n.start_tick <= t {
                    prev = cur;
                    cur = Some(n);
                } else {
                    break;
                }
            }
            let cur = cur?;
            // Portamento ramp: the last `portamento_ticks` before the
            // current note's start tick are blended from prev → cur.
            if let Some(p) = prev {
                let ramp_start = cur.start_tick.saturating_sub(portamento_ticks);
                if t >= ramp_start && t < cur.start_tick && portamento_ticks > 0 {
                    let span = (cur.start_tick - ramp_start) as f32;
                    let local = (t - ramp_start) as f32;
                    let alpha = (local / span).clamp(0.0, 1.0);
                    return Some(p.note as f32 * (1.0 - alpha) + cur.note as f32 * alpha);
                }
            }
            Some(cur.note as f32)
        };

        // Walk samples and build a path. Sustains add vibrato in the
        // back half of the note.
        let beats_per_sec = bpm / 60.0;
        let ticks_per_sec = beats_per_sec * TICKS_PER_QUARTER_NOTE as f32;
        let path = Path::new(|p| {
            let mut started = false;
            for i in 0..=samples {
                let x_local = i as f32 * px_per_sample;
                let tick = self.x_to_tick(x_local);
                let Some(mut midi) = pitch_at(tick) else { continue };
                // Vibrato: only when we're inside a sustained note,
                // 60 ms into its duration (matches the engine's onset
                // gate so onsets read clean).
                if let Some(n) = self
                    .clip
                    .notes
                    .iter()
                    .find(|n| n.start_tick <= tick && tick < n.start_tick + n.duration_ticks)
                {
                    let onset_ticks =
                        (0.06 * ticks_per_sec) as u64;
                    let elapsed = tick.saturating_sub(n.start_tick);
                    if elapsed > onset_ticks && n.duration_ticks > onset_ticks + 1 {
                        let t_sec = (elapsed - onset_ticks) as f32 / ticks_per_sec;
                        let wobble = (t_sec * vibrato_rate_hz * std::f32::consts::TAU).sin();
                        midi += vibrato_depth_st * wobble;
                    }
                }
                let Some(y) = pitch_y_view(midi) else { continue };
                let x = grid_x + x_local;
                if x > grid_x + grid_w {
                    break;
                }
                if !started {
                    p.move_to(Point::new(x, y));
                    started = true;
                } else {
                    p.line_to(Point::new(x, y));
                }
            }
        });
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(Color::from_rgba(1.0, 0.95, 0.78, 0.85))
                .with_width(1.2),
        );
    }

    /// Lexical-stress contour overlay. One horizontal step segment per
    /// note, lifted above the note body by an amount proportional to
    /// the syllable's CMU stress (primary > secondary > none). Drawn in
    /// ACCENT so it reads distinctly from the warm-cream pitch curve.
    /// Slurs inherit their parent syllable's stress, so the overlay
    /// reads as a single continuous level across a melisma.
    fn draw_stress_curve(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        grid_top: f32,
        grid_h: f32,
        assigned: &[g2p::AssignedSyllable],
    ) {
        if self.clip.notes.is_empty() {
            return;
        }
        // Vertical lift per stress level, in pixels above the note top.
        // Tight band so the overlay never collides with the slur arcs
        // (which sit ~10 px above the higher of two notes).
        let stress_lift = |s: g2p::SyllableStress| -> f32 {
            match s {
                g2p::SyllableStress::Primary => 8.0,
                g2p::SyllableStress::Secondary => 4.0,
                g2p::SyllableStress::None => 1.0,
            }
        };
        let color_for = |s: g2p::SyllableStress| -> Color {
            match s {
                g2p::SyllableStress::Primary => Color { a: 0.90, ..theme::ACCENT },
                g2p::SyllableStress::Secondary => Color { a: 0.65, ..theme::ACCENT_SOFT },
                g2p::SyllableStress::None => Color { a: 0.30, ..theme::ACCENT_SOFT },
            }
        };
        let mut prev_anchor: Option<(f32, f32, g2p::SyllableStress)> = None;
        for (i, n) in self.clip.notes.iter().enumerate() {
            let Some(a) = assigned.get(i) else { break };
            let Some(y_local) = self.note_to_y(n.note, grid_h) else {
                prev_anchor = None;
                continue;
            };
            let x0 = grid_x + self.tick_to_x(n.start_tick);
            let x1 = grid_x + self.tick_to_x(n.start_tick + n.duration_ticks);
            if x0 > grid_x + grid_w {
                break;
            }
            let y_top = grid_top + y_local;
            // Clamp the lift so we never draw outside the grid band.
            let y_seg = (y_top - stress_lift(a.stress)).max(grid_top + 1.0);
            // Step connector: vertical line from the previous note's
            // segment to this one's, if they share the same horizontal
            // tick range (back-to-back). Skips when there's a gap or
            // we wrapped to a different row.
            if let Some((px, py, _)) = prev_anchor {
                if (x0 - px).abs() < 1.5 && (y_seg - py).abs() > 0.5 {
                    let v = Path::line(Point::new(x0, py), Point::new(x0, y_seg));
                    frame.stroke(
                        &v,
                        Stroke::default()
                            .with_color(Color { a: 0.40, ..theme::ACCENT_SOFT })
                            .with_width(1.0),
                    );
                }
            }
            let seg = Path::line(Point::new(x0, y_seg), Point::new(x1, y_seg));
            frame.stroke(
                &seg,
                Stroke::default().with_color(color_for(a.stress)).with_width(1.4),
            );
            // Tick mark at the segment's start so primary-stress notes
            // get an extra visual punch (matches how scores mark
            // accented beats with a wedge).
            if a.stress == g2p::SyllableStress::Primary && !a.is_slur {
                let tick = Path::line(
                    Point::new(x0, y_seg - 2.0),
                    Point::new(x0, y_seg + 2.0),
                );
                frame.stroke(
                    &tick,
                    Stroke::default()
                        .with_color(Color { a: 0.95, ..theme::ACCENT })
                        .with_width(1.2),
                );
            }
            prev_anchor = Some((x1, y_seg, a.stress));
        }
    }

    fn draw_keyboard(&self, frame: &mut Frame, grid_top: f32, grid_h: f32) {
        frame.fill_rectangle(
            Point::new(0.0, grid_top),
            Size::new(VR_KEYBOARD_WIDTH, grid_h),
            theme::BG_2,
        );
        let (lo, hi) = self.params.range;
        for note in lo..=hi {
            let Some(y_local) = self.note_to_y(note, grid_h) else {
                continue;
            };
            let y = grid_top + y_local;
            let h = self.zoom_y;
            if y + h < grid_top || y > grid_top + grid_h {
                continue;
            }
            let black = is_black_key(note);
            let key_color = if black { theme::BG_0 } else { theme::BG_3 };
            let key_w = if black {
                VR_KEYBOARD_WIDTH * 0.6
            } else {
                VR_KEYBOARD_WIDTH - 1.0
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(key_w, h - 1.0),
                key_color,
            );
            // Label only on C rows when there's headroom.
            if note % 12 == 0 && h >= 8.0 {
                frame.fill_text(canvas::Text {
                    content: note_name(note),
                    position: Point::new(3.0, y + 1.0),
                    color: theme::TEXT_3,
                    size: (h * 0.7).min(10.0).into(),
                    font: theme::MONO_FONT,
                    ..canvas::Text::default()
                });
            }
        }
        // Right edge separator
        frame.fill_rectangle(
            Point::new(VR_KEYBOARD_WIDTH - 1.0, grid_top),
            Size::new(1.0, grid_h),
            theme::LINE_2,
        );
    }

    fn draw_velocity_lane(&self, frame: &mut Frame, grid_x: f32, grid_w: f32, total_h: f32) {
        let lane_y = total_h - VR_VELOCITY_LANE_HEIGHT;
        // Left label column
        frame.fill_rectangle(
            Point::new(0.0, lane_y),
            Size::new(grid_x + grid_w, VR_VELOCITY_LANE_HEIGHT),
            theme::PANEL_DARK,
        );
        frame.fill_text(canvas::Text {
            content: "VEL".to_string(),
            position: Point::new(8.0, lane_y + 4.0),
            color: theme::TEXT_3,
            size: 9.0.into(),
            font: theme::UI_FONT_SEMIBOLD,
            ..canvas::Text::default()
        });

        // Baseline
        frame.fill_rectangle(
            Point::new(grid_x, lane_y + VR_VELOCITY_LANE_HEIGHT - 2.0),
            Size::new(grid_w, 1.0),
            theme::LINE_2,
        );

        for (i, n) in self.clip.notes.iter().enumerate() {
            let x = grid_x + self.tick_to_x(n.start_tick);
            if x > grid_x + grid_w {
                break;
            }
            let w = self.duration_to_width(n.duration_ticks).clamp(2.0, 6.0);
            let bar_h = n.velocity.clamp(0.0, 1.0) * (VR_VELOCITY_LANE_HEIGHT - 8.0);
            let bar_y = lane_y + VR_VELOCITY_LANE_HEIGHT - bar_h - 4.0;
            let color = if self.selected_note == Some(i) {
                theme::WARM
            } else {
                Color { a: 0.65, ..theme::WARM }
            };
            frame.fill_rectangle(Point::new(x, bar_y), Size::new(w, bar_h), color);
        }
    }

    /// Read-only chord context strip aligned to the section's beat
    /// timeline so chord boundaries land on bar lines.
    fn draw_chord_strip(&self, frame: &mut Frame, grid_x: f32, grid_w: f32) {
        // Background — warm tint.
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, VR_CHORD_STRIP_HEIGHT),
            Color { a: 0.06, ..theme::WARM },
        );
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let section_ticks = (self.section_beats as u64) * ticks_per_beat;
        if section_ticks == 0 {
            return;
        }
        for c in self.chords {
            let start_tick = c.start_beat as u64 * ticks_per_beat;
            let dur_tick = c.duration_beats as u64 * ticks_per_beat;
            let x0 = grid_x + self.tick_to_x(start_tick);
            let w = self.duration_to_width(dur_tick);
            if w < 1.0 {
                continue;
            }
            // Cell border
            frame.stroke(
                &Path::rounded_rectangle(
                    Point::new(x0 + 1.0, 3.0),
                    Size::new((w - 2.0).max(2.0), VR_CHORD_STRIP_HEIGHT - 6.0),
                    3.0.into(),
                ),
                Stroke::default()
                    .with_color(Color { a: 0.45, ..theme::WARM })
                    .with_width(1.0),
            );
            // Symbol — root + quality. Italic serif for the symbol.
            let label = chord_label(&c.chord);
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x0 + 8.0, 6.0),
                color: theme::WARM,
                size: 13.0.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
        }
    }

    /// Phoneme strip — per-note ARPAbet breakdown. Reads phonemes
    /// directly from `AssignedSyllable::phonemes`, so the labels here
    /// are guaranteed to match what `build_segment` feeds the SVS
    /// model. Slur notes show the held vowel from the previous
    /// syllable; non-slur notes show their full phoneme list.
    fn draw_phoneme_strip(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        assigned: &[g2p::AssignedSyllable],
    ) {
        frame.fill_rectangle(
            Point::new(grid_x, VR_CHORD_STRIP_HEIGHT),
            Size::new(grid_w, VR_PHONEME_STRIP_HEIGHT),
            theme::BG_1,
        );
        // Section label on the left edge.
        frame.fill_text(canvas::Text {
            content: "PHN".to_string(),
            position: Point::new(6.0, VR_CHORD_STRIP_HEIGHT + 5.0),
            color: theme::TEXT_3,
            size: 8.5.into(),
            font: theme::UI_FONT_SEMIBOLD,
            ..canvas::Text::default()
        });
        let strip_y = VR_CHORD_STRIP_HEIGHT + 4.0;

        if self.clip.notes.is_empty() || assigned.iter().all(|a| a.phonemes.is_empty()) {
            frame.fill_text(canvas::Text {
                content: "(no phonemes \u{2014} generate from the right rail)".to_string(),
                position: Point::new(grid_x + 8.0, strip_y + 1.0),
                color: theme::TEXT_3,
                size: 10.5.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
            return;
        }

        for (i, n) in self.clip.notes.iter().enumerate() {
            let Some(a) = assigned.get(i) else { break };
            if a.phonemes.is_empty() {
                continue;
            }
            let display = a.phonemes.join(" ");
            let x = grid_x + self.tick_to_x(n.start_tick);
            let nw = self.duration_to_width(n.duration_ticks).max(8.0);
            if x > grid_x + grid_w {
                break;
            }
            let pill_w = nw.clamp(14.0, 72.0);
            let pill_alpha = if a.is_slur { 0.06 } else { 0.10 };
            frame.fill(
                &Path::rounded_rectangle(
                    Point::new(x + 1.0, strip_y - 1.0),
                    Size::new(pill_w - 2.0, VR_PHONEME_STRIP_HEIGHT - 6.0),
                    3.0.into(),
                ),
                Color { a: pill_alpha, ..theme::WARM },
            );
            let text_color = if a.is_slur {
                Color { a: 0.65, ..theme::WARM }
            } else {
                theme::WARM
            };
            frame.fill_text(canvas::Text {
                content: display,
                position: Point::new(x + 4.0, strip_y + 1.0),
                color: text_color,
                size: 9.5.into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });
        }
    }
}
