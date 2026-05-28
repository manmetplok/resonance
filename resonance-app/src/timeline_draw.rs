//! Canvas drawing for the timeline. These are the pure-draw methods
//! that take a `&mut Frame` and render bar/beat grids, rulers, audio
//! clips, and MIDI clips. They're in a separate impl block (and file)
//! so `timeline.rs` can stay focused on canvas event handling and state.
use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::state::{self, ClipState, MidiClipState, TrackState};
use crate::theme;
use crate::timeline::TimelineCanvas;
use resonance_audio::types::avg_bpm_for_bar;

impl TimelineCanvas<'_> {
    /// Render the compose-section pills above the lanes. Each placement
    /// becomes a colored pill spanning its bars. Selected placement gets
    /// the lavender-wash accent; unselected placements use a softer wash
    /// derived from the section's color.
    pub(super) fn draw_section_band(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        band_top: f32,
        band_height: f32,
    ) {
        // Tinted backdrop so the band reads as a continuous strip even
        // when the placements are sparse.
        frame.fill_rectangle(
            Point::new(0.0, band_top),
            Size::new(width, band_height),
            theme::BG_1,
        );
        // Bottom hairline.
        frame.fill_rectangle(
            Point::new(0.0, band_top + band_height - 1.0),
            Size::new(width, 1.0),
            theme::LINE_2,
        );

        for placement in self.section_placements {
            let Some(definition) = self
                .section_definitions
                .iter()
                .find(|d| d.id == placement.definition_id)
            else {
                continue;
            };
            let start_sample = self.tempo_map.bar_to_sample(placement.start_bar);
            let end_sample = self
                .tempo_map
                .bar_to_sample(placement.start_bar + definition.length_bars);
            let x = self.sample_to_x(start_sample);
            let next_x = self.sample_to_x(end_sample);
            if next_x < 0.0 || x > width {
                continue;
            }

            let is_selected = self.selected_placement_id == Some(placement.id);
            let base = Color::from_rgba(
                definition.color[0] as f32 / 255.0,
                definition.color[1] as f32 / 255.0,
                definition.color[2] as f32 / 255.0,
                if is_selected { 0.32 } else { 0.18 },
            );
            let border = Color::from_rgba(
                definition.color[0] as f32 / 255.0,
                definition.color[1] as f32 / 255.0,
                definition.color[2] as f32 / 255.0,
                if is_selected { 0.85 } else { 0.45 },
            );

            let pill_x = x.max(0.0);
            let pill_visible = (next_x.min(width) - pill_x).max(0.0);
            let pill_y = band_top + 4.0;
            let pill_h = band_height - 8.0;

            let pill = canvas::Path::rounded_rectangle(
                Point::new(pill_x, pill_y),
                Size::new(pill_visible, pill_h),
                4.0.into(),
            );
            frame.fill(&pill, base);
            frame.stroke(
                &pill,
                canvas::Stroke::default()
                    .with_width(if is_selected { 1.5 } else { 1.0 })
                    .with_color(border),
            );

            // Label "Name · NbBars" — only render if there's room.
            if pill_visible > 50.0 {
                let bpm = self.tempo_map.bpm;
                let num = self.tempo_map.numerator;
                let den = self.tempo_map.denominator;
                let label = format!(
                    "{} · {}/{}{}",
                    definition.name,
                    num,
                    den,
                    if pill_visible > 110.0 {
                        format!(" · {} bpm", bpm.round() as u32)
                    } else {
                        String::new()
                    }
                );
                frame.fill_text(canvas::Text {
                    content: label,
                    position: Point::new(pill_x + 8.0, pill_y + 3.0),
                    color: if is_selected {
                        theme::ACCENT_SOFT
                    } else {
                        theme::TEXT_2
                    },
                    size: 10.0.into(),
                    font: theme::UI_FONT_SEMIBOLD,
                    ..canvas::Text::default()
                });
            }
        }
    }

    /// Draw the global-tracks shelf — a collapsible strip sitting between
    /// the ruler/section-band and the regular track lanes. The shelf has
    /// three parts:
    ///
    /// 1. **Header strip** (`GLOBAL_SHELF_HEADER_HEIGHT`, always visible)
    ///    — backdrop + one-line summary `6/8 · 90 BPM · B min · N chords`.
    ///    The caret-toggle + `GLOBAL` tag live on the track-header
    ///    column side (see `view::track_header`); the canvas only paints
    ///    the summary text on the right of the header strip.
    /// 2. **Chord lane** (`GLOBAL_TRACK_CHORD_HEIGHT`) — flattened view of
    ///    every section's chord progression, rendered as section tabs
    ///    with chord blocks underneath. Only painted when the shelf is
    ///    expanded.
    /// 3. **Tempo lane** (`GLOBAL_TRACK_TEMPO_HEIGHT`) — automation curve
    ///    with anchor points + BPM labels. Only painted when expanded.
    /// 4. **Signature lane** (`GLOBAL_TRACK_SIG_HEIGHT`) — pill markers
    ///    for time-signature changes + downbeat ticks. Only when expanded.
    pub(super) fn draw_global_tracks(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
    ) {
        let shelf_top = ruler_height;
        let header_h = theme::GLOBAL_SHELF_HEADER_HEIGHT;

        // ---- Header strip (always visible) ----
        // Slightly elevated backdrop so the shelf reads as a distinct
        // sub-section between the ruler and the lanes. Matches the
        // design's `linear-gradient(--bg-1 0%, #131419 100%)` by tinting
        // the lower edge a notch darker than the ruler.
        frame.fill_rectangle(
            Point::new(0.0, shelf_top),
            Size::new(width, header_h),
            theme::BG_1,
        );
        // Bottom hairline of the header strip.
        frame.fill_rectangle(
            Point::new(0.0, shelf_top + header_h - 1.0),
            Size::new(width, 1.0),
            theme::LINE_2,
        );

        // Right-side summary line: `6/8 · 90 BPM · B min · N chords`.
        // The text sits on the canvas side of the shelf header — the
        // caret + GLOBAL tag live in the track-header column. Padding
        // on the left so the text aligns with the lane content below.
        let summary = self.global_shelf_summary();
        let summary_y = shelf_top + (header_h - 12.0) * 0.5;
        frame.fill_text(canvas::Text {
            content: summary,
            position: Point::new(12.0, summary_y),
            color: theme::TEXT_2,
            size: 11.5.into(),
            font: theme::UI_FONT_MEDIUM,
            ..canvas::Text::default()
        });

        if !self.global_tracks_expanded {
            return;
        }

        let chord_h = theme::GLOBAL_TRACK_CHORD_HEIGHT;
        let tempo_h = theme::GLOBAL_TRACK_TEMPO_HEIGHT;
        let sig_h = theme::GLOBAL_TRACK_SIG_HEIGHT;

        let chord_y = shelf_top + header_h;
        let tempo_y = chord_y + chord_h;
        let sig_y = tempo_y + tempo_h;

        // ---- Chord lane background ----
        frame.fill_rectangle(
            Point::new(0.0, chord_y),
            Size::new(width, chord_h),
            theme::GLOBAL_TRACK_BG,
        );
        frame.fill_rectangle(
            Point::new(0.0, chord_y + chord_h - 1.0),
            Size::new(width, 1.0),
            theme::LINE_2,
        );

        // ---- Tempo row background ----
        frame.fill_rectangle(
            Point::new(0.0, tempo_y),
            Size::new(width, tempo_h),
            theme::GLOBAL_TRACK_BG,
        );
        frame.fill_rectangle(
            Point::new(0.0, tempo_y + tempo_h - 1.0),
            Size::new(width, 1.0),
            theme::LINE_2,
        );

        // ---- Time signature row background ----
        frame.fill_rectangle(
            Point::new(0.0, sig_y),
            Size::new(width, sig_h),
            theme::GLOBAL_TRACK_BG,
        );
        frame.fill_rectangle(
            Point::new(0.0, sig_y + sig_h - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );

        // ---- Chord blocks per section ----
        self.draw_chord_lane(frame, width, chord_y, chord_h);

        // ---- Draw tempo line graph ----
        // (Geometry repurposed from the previous implementation; only the
        // row height shrank from 40 → 40, so the math is unchanged.)
        let row_h = tempo_h;
        if !self.tempo_map.tempo_points.is_empty() {
            // Determine BPM range for vertical mapping.
            let mut min_bpm = f32::MAX;
            let mut max_bpm = f32::MIN;
            for e in &self.tempo_map.tempo_points {
                min_bpm = min_bpm.min(e.bpm);
                max_bpm = max_bpm.max(e.bpm);
            }
            // Add padding so points aren't flush with edges; ensure a
            // minimum range so a flat tempo doesn't compress to zero.
            let range = (max_bpm - min_bpm).max(10.0);
            let pad = range * 0.15;
            let lo = min_bpm - pad;
            let hi = max_bpm + pad;

            let graph_top = tempo_y + 3.0;
            let graph_bot = tempo_y + row_h - 3.0;
            let graph_h = graph_bot - graph_top;

            // Map BPM to y within the tempo row (high BPM = top).
            let bpm_to_y = |bpm: f32| -> f32 { graph_bot - ((bpm - lo) / (hi - lo)) * graph_h };

            // Build (x, y, bpm, is_selected) for each event point.
            let points: Vec<(f32, f32, f32, bool)> = self
                .tempo_map
                .tempo_points
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let sample = self.tempo_map.bar_to_sample(e.bar);
                    let x = self.sample_to_x(sample);
                    let y = bpm_to_y(e.bpm);
                    let selected = self.selected_global_event
                        == Some(state::SelectedGlobalEvent {
                            kind: state::GlobalTrackKind::Tempo,
                            index: i,
                        });
                    (x, y, e.bpm, selected)
                })
                .collect();

            // Draw connecting lines and filled area. Tempo events read in
            // the warm/amber accent matching the rest of the redesign's
            // "playhead / time" semantic; the dim variant softens the
            // filled area underneath the line.
            let line_color = Color {
                a: 0.7,
                ..theme::WARM
            };
            let fill_color = Color {
                a: 0.10,
                ..theme::WARM
            };

            // Build the polyline vertices: every tempo point, plus a
            // horizontal extension out to the right edge from the last
            // point so the fill/line reach the end of the canvas.
            // Previously this geometry was rasterised as hundreds of
            // 1 px-wide `fill_rectangle` calls per segment; one Path
            // fill + one Path stroke does the same work in two draw
            // submissions.
            let mut polyline: Vec<Point> = points
                .iter()
                .map(|&(x, y, _, _)| Point::new(x, y))
                .collect();
            if let Some(&last) = polyline.last() {
                if last.x < width {
                    polyline.push(Point::new(width, last.y));
                }
            }

            if polyline.len() >= 2 {
                // Filled trapezoid under the polyline: trace the
                // polyline left-to-right, then close along the bottom
                // (graph_bot) back to the starting x.
                let fill_path = canvas::Path::new(|b| {
                    let first = polyline[0];
                    b.move_to(Point::new(first.x, graph_bot));
                    for p in &polyline {
                        b.line_to(*p);
                    }
                    let last = polyline[polyline.len() - 1];
                    b.line_to(Point::new(last.x, graph_bot));
                    b.close();
                });
                frame.fill(&fill_path, fill_color);

                // Line itself. Round joins so the 2 px stroke doesn't
                // spike at sharp tempo changes (the previous overlapping
                // 1 px rect stack had no visible miter artifacts).
                let line_path = canvas::Path::new(|b| {
                    b.move_to(polyline[0]);
                    for p in &polyline[1..] {
                        b.line_to(*p);
                    }
                });
                frame.stroke(
                    &line_path,
                    canvas::Stroke::default()
                        .with_width(2.0)
                        .with_color(line_color)
                        .with_line_join(canvas::LineJoin::Round),
                );
            }

            // Draw event points (dots) and BPM labels.
            for (i, &(x, y, bpm, selected)) in points.iter().enumerate() {
                if x > width + 50.0 || x < -50.0 {
                    continue;
                }
                // Dot.
                let dot_r = if selected { 4.0 } else { 3.0 };
                let dot_color = if selected { theme::ACCENT } else { theme::WARM };
                if x >= -dot_r && x <= width + dot_r {
                    frame.fill_rectangle(
                        Point::new(x - dot_r, y - dot_r),
                        Size::new(dot_r * 2.0, dot_r * 2.0),
                        dot_color,
                    );
                }
                // Vertical marker line.
                if i > 0 && x >= 0.0 {
                    let marker_color = if selected {
                        theme::ACCENT
                    } else {
                        Color {
                            a: 0.30,
                            ..theme::WARM
                        }
                    };
                    frame.fill_rectangle(
                        Point::new(x, tempo_y),
                        Size::new(1.0, row_h),
                        marker_color,
                    );
                }
                // BPM label.
                let label_x = x.max(2.0) + 5.0;
                if label_x < width - 10.0 {
                    frame.fill_text(canvas::Text {
                        content: format!("{:.0}", bpm),
                        position: Point::new(label_x, tempo_y + 2.0),
                        color: if selected {
                            theme::ACCENT
                        } else {
                            theme::TEXT_DIM
                        },
                        size: 10.0.into(),
                        ..canvas::Text::default()
                    });
                }
            }
        }

        // ---- Draw signature event markers ----
        for (i, event) in self.tempo_map.signature_points.iter().enumerate() {
            let sample = self.tempo_map.bar_to_sample(event.bar);
            let x = self.sample_to_x(sample);
            if x > width + 50.0 || x < -50.0 {
                continue;
            }
            let next_x = self
                .tempo_map
                .signature_points
                .get(i + 1)
                .map(|ne| self.sample_to_x(self.tempo_map.bar_to_sample(ne.bar)))
                .unwrap_or(width);
            let block_w = (next_x - x).max(2.0).min(width - x.max(0.0));

            let is_selected = self.selected_global_event
                == Some(state::SelectedGlobalEvent {
                    kind: state::GlobalTrackKind::Signature,
                    index: i,
                });

            // Signature change blocks use the lavender accent at low alpha
            // so they're visible but don't compete with clips for attention.
            let block_color = if is_selected {
                theme::ACCENT_DIM
            } else {
                Color {
                    a: 0.08,
                    ..theme::ACCENT
                }
            };
            frame.fill_rectangle(
                Point::new(x.max(0.0), sig_y + 1.0),
                Size::new(block_w, sig_h - 2.0),
                block_color,
            );

            if x >= 0.0 {
                let marker_color = if is_selected {
                    theme::ACCENT
                } else {
                    theme::TEXT_DIM
                };
                frame.fill_rectangle(Point::new(x, sig_y), Size::new(1.0, sig_h), marker_color);
            }

            // Pill-style label `{n}/{d}`.
            let label_x = x.max(2.0) + 5.0;
            if label_x < width - 10.0 {
                let label = format!("{}/{}", event.numerator, event.denominator);
                let label_y = sig_y + (sig_h - 11.0) * 0.5;
                frame.fill_text(canvas::Text {
                    content: label.clone(),
                    position: Point::new(label_x, label_y),
                    color: if is_selected {
                        theme::ACCENT
                    } else {
                        theme::TEXT_1
                    },
                    size: 10.5.into(),
                    font: theme::MONO_FONT,
                    ..canvas::Text::default()
                });

                // Optional "compound · N eighths" hint for compound meters
                // (numerator divisible by 3 and >= 6, e.g. 6/8, 9/8, 12/8).
                if !is_selected
                    && event.numerator >= 6
                    && event.numerator % 3 == 0
                    && event.denominator == 8
                {
                    let hint_x = label_x + (label.len() as f32) * 6.5 + 10.0;
                    if hint_x < width - 60.0 {
                        frame.fill_text(canvas::Text {
                            content: format!(
                                "compound · {} eighths",
                                event.numerator
                            ),
                            position: Point::new(hint_x, label_y + 1.0),
                            color: theme::TEXT_3,
                            size: 9.5.into(),
                            font: theme::MONO_FONT,
                            ..canvas::Text::default()
                        });
                    }
                }
            }
        }
    }

    /// Build the one-line "GLOBAL" summary text shown in the always-visible
    /// shelf header strip: `6/8 · 90 BPM · B min · N chords`.
    fn global_shelf_summary(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let num = self.tempo_map.numerator;
        let den = self.tempo_map.denominator;
        let _ = write!(out, "{}/{}", num, den);

        let bpm = self.tempo_map.bpm;
        let _ = write!(out, "  ·  {} BPM", bpm.round() as u32);

        // Key signature: read from the first section that has a scale.
        if let Some(scale) = self
            .section_definitions
            .iter()
            .find_map(|d| d.scale.as_ref())
        {
            let mode_label = match scale.mode {
                resonance_music_theory::Mode::Major => "maj".to_string(),
                resonance_music_theory::Mode::Minor => "min".to_string(),
                other => other.to_string(),
            };
            let _ = write!(out, "  ·  {} {}", scale.root, mode_label);
        }

        // Chord count: sum of every section's progression.
        let chord_total: usize = self
            .section_definitions
            .iter()
            .map(|d| d.chords.len())
            .sum();
        let _ = write!(out, "  ·  {} chords", chord_total);
        out
    }

    /// Draw the chord lane: for each placed section, render a small
    /// section tab at the top + chord blocks beneath, sized to the
    /// section's footprint on the timeline. Chord blocks are tinted by
    /// quality (minor = lavender, dom = warm, major = neutral) so the
    /// progression reads at a glance.
    fn draw_chord_lane(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        chord_y: f32,
        chord_h: f32,
    ) {
        // Top sub-strip holds the section name tab; the chord blocks
        // fill the remaining vertical space.
        let tab_h = 14.0;
        let blocks_y = chord_y + tab_h;
        let blocks_h = chord_h - tab_h - 4.0;

        for placement in self.section_placements {
            let Some(definition) = self
                .section_definitions
                .iter()
                .find(|d| d.id == placement.definition_id)
            else {
                continue;
            };

            let section_start_sample =
                self.tempo_map.bar_to_sample(placement.start_bar);
            let section_end_sample = self
                .tempo_map
                .bar_to_sample(placement.start_bar + definition.length_bars);
            let section_x = self.sample_to_x(section_start_sample);
            let section_end_x = self.sample_to_x(section_end_sample);
            if section_end_x < 0.0 || section_x > width {
                continue;
            }

            // Section dot + name tab — same color identity as the
            // section-pill band so the chord lane links back visually.
            let dot_color = Color::from_rgb(
                definition.color[0] as f32 / 255.0,
                definition.color[1] as f32 / 255.0,
                definition.color[2] as f32 / 255.0,
            );
            let tab_x = section_x.max(0.0) + 4.0;
            let dot_size = 5.0;
            if tab_x + dot_size < width {
                frame.fill_rectangle(
                    Point::new(tab_x, chord_y + (tab_h - dot_size) * 0.5),
                    Size::new(dot_size, dot_size),
                    dot_color,
                );
                frame.fill_text(canvas::Text {
                    content: definition.name.to_uppercase(),
                    position: Point::new(tab_x + dot_size + 5.0, chord_y + 2.0),
                    color: theme::TEXT_3,
                    size: 9.0.into(),
                    font: theme::UI_FONT_SEMIBOLD,
                    ..canvas::Text::default()
                });
            }

            // Chord blocks — laid out in the bottom sub-strip of the lane.
            // Each chord occupies its `start_beat..start_beat+duration_beats`
            // window within the section. Convert beat positions to bar
            // fractions, then to samples + screen-x.
            let beats_per_bar = self.tempo_map.numerator.max(1) as f32;
            let section_bars = definition.length_bars as f32;
            let section_pixel_width = section_end_x - section_x;
            for chord in &definition.chords {
                let chord_start_bars =
                    chord.start_beat as f32 / beats_per_bar;
                let chord_end_bars = (chord.start_beat + chord.duration_beats)
                    as f32
                    / beats_per_bar;
                if chord_start_bars >= section_bars {
                    continue;
                }
                let chord_end_bars = chord_end_bars.min(section_bars);

                let block_left = section_x
                    + (chord_start_bars / section_bars) * section_pixel_width;
                let block_right = section_x
                    + (chord_end_bars / section_bars) * section_pixel_width;
                let block_w = (block_right - block_left - 3.0).max(0.0);
                if block_w <= 0.0 || block_right < 0.0 || block_left > width {
                    continue;
                }

                // Tint by quality — minor uses the lavender accent, dom
                // uses warm/amber, every other quality reads as neutral.
                use resonance_music_theory::ChordQuality;
                let (body_color, border_color, text_color) = match chord
                    .chord
                    .quality
                {
                    ChordQuality::Min
                    | ChordQuality::Min7
                    | ChordQuality::Min6
                    | ChordQuality::MinMaj7
                    | ChordQuality::HalfDim7 => (
                        Color { a: 0.10, ..theme::ACCENT },
                        Color { a: 0.30, ..theme::ACCENT },
                        theme::ACCENT_SOFT,
                    ),
                    ChordQuality::Dom7 => (
                        Color { a: 0.10, ..theme::WARM },
                        Color { a: 0.32, ..theme::WARM },
                        theme::WARM,
                    ),
                    _ => (
                        Color { a: 0.04, ..theme::TEXT_1 },
                        Color { a: 0.10, ..theme::TEXT_1 },
                        theme::TEXT_1,
                    ),
                };

                let visible_x = block_left.max(0.0);
                let visible_w =
                    (block_left + block_w).min(width) - visible_x;
                if visible_w <= 0.0 {
                    continue;
                }
                let body = canvas::Path::rounded_rectangle(
                    Point::new(visible_x, blocks_y),
                    Size::new(visible_w, blocks_h),
                    6.0.into(),
                );
                frame.fill(&body, body_color);
                frame.stroke(
                    &body,
                    canvas::Stroke::default()
                        .with_color(border_color)
                        .with_width(1.0),
                );

                // Chord symbol: render root + quality suffix on one line.
                // Tiny — fits in the chord block height of ~38 px.
                if visible_w > 14.0 {
                    let root_label = chord.chord.root.as_str();
                    let suffix = chord.chord.quality.suffix();
                    frame.fill_text(canvas::Text {
                        content: format!("{}{}", root_label, suffix),
                        position: Point::new(
                            visible_x + 6.0,
                            blocks_y + 4.0,
                        ),
                        color: text_color,
                        size: 12.0.into(),
                        font: theme::UI_FONT_MEDIUM,
                        ..canvas::Text::default()
                    });
                }
                // Duration label "{N}b" in the bottom-right corner of the
                // block — mono, dim, so it doesn't compete with the chord
                // symbol but the user can still scan progression timing.
                if visible_w > 36.0 {
                    let beats_per_bar = self.tempo_map.numerator.max(1) as u32;
                    let dur_bars = chord.duration_beats / beats_per_bar.max(1);
                    let dur_label = if dur_bars > 0
                        && chord.duration_beats % beats_per_bar == 0
                    {
                        format!("{}b", dur_bars)
                    } else {
                        format!("{}·", chord.duration_beats)
                    };
                    let label_x = visible_x + visible_w - 22.0;
                    frame.fill_text(canvas::Text {
                        content: dur_label,
                        position: Point::new(
                            label_x.max(visible_x + 4.0),
                            blocks_y + blocks_h - 13.0,
                        ),
                        color: theme::TEXT_3,
                        size: 8.5.into(),
                        font: theme::MONO_FONT,
                        ..canvas::Text::default()
                    });
                }
            }
        }
    }

    /// Draw vertical bar and beat grid lines in the track area.
    /// Iterates bars using per-bar tempo and time-signature values so
    /// that grid spacing correctly follows tempo changes.
    pub(super) fn draw_grid_lines(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
        track_area_height: f32,
        _y_off: f32,
    ) {
        let line_height = track_area_height.max(600.0);
        let sr = self.sample_rate as f64;

        // Walk bars from 0, accumulating sample positions with interpolation.
        let mut sample_pos: f64 = 0.0;
        let mut cur_num = self
            .tempo_map
            .signature_points
            .first()
            .map(|e| e.numerator)
            .unwrap_or(4);
        let mut si: usize = if self.tempo_map.signature_points.first().map(|e| e.bar) == Some(0) {
            1
        } else {
            0
        };

        for bar in 0u32.. {
            while let Some(e) = self.tempo_map.signature_points.get(si) {
                if e.bar == bar {
                    cur_num = e.numerator;
                    si += 1;
                } else {
                    break;
                }
            }

            let cur_bpm = avg_bpm_for_bar(bar, &self.tempo_map.tempo_points);
            let samples_per_beat = sr * 60.0 / cur_bpm;
            let samples_per_bar = samples_per_beat * cur_num as f64;
            let bar_seconds = samples_per_bar / sr;
            let bar_pixel_width = bar_seconds as f32 * self.zoom;

            let x = (sample_pos / sr) as f32 * self.zoom - self.scroll_offset;

            // Past the right edge — done.
            if x > width + 1.0 {
                break;
            }
            // Safety limit.
            if bar > 20_000 {
                break;
            }

            // Bar step: skip bars for readability at low zoom.
            let bar_step = if bar_pixel_width < 20.0 {
                (20.0 / bar_pixel_width).ceil() as u32
            } else {
                1
            };
            let draw_this = bar_step <= 1 || bar % bar_step == 0;

            if draw_this && x >= -1.0 {
                frame.fill_rectangle(
                    Point::new(x, ruler_height),
                    Size::new(1.0, line_height),
                    theme::BAR_LINE,
                );

                // Beat lines within this bar.
                if bar_pixel_width >= 40.0 {
                    for beat in 1..cur_num {
                        let beat_sample = sample_pos + beat as f64 * samples_per_beat;
                        let bx = (beat_sample / sr) as f32 * self.zoom - self.scroll_offset;
                        if bx >= 0.0 && bx <= width {
                            frame.fill_rectangle(
                                Point::new(bx, ruler_height),
                                Size::new(1.0, line_height),
                                theme::BEAT_LINE,
                            );
                        }
                    }
                }
            }

            sample_pos += samples_per_bar;
        }
    }

    /// Draw the bar/beat ruler at the top.
    /// Uses per-bar tempo and time-signature values so bar numbers are
    /// positioned correctly when tempo changes.
    pub(super) fn draw_ruler(&self, frame: &mut canvas::Frame, width: f32, ruler_height: f32) {
        let sr = self.sample_rate as f64;

        let mut sample_pos: f64 = 0.0;
        let mut cur_num = self
            .tempo_map
            .signature_points
            .first()
            .map(|e| e.numerator)
            .unwrap_or(4);
        let mut si: usize = if self.tempo_map.signature_points.first().map(|e| e.bar) == Some(0) {
            1
        } else {
            0
        };

        for bar in 0u32.. {
            while let Some(e) = self.tempo_map.signature_points.get(si) {
                if e.bar == bar {
                    cur_num = e.numerator;
                    si += 1;
                } else {
                    break;
                }
            }

            let cur_bpm = avg_bpm_for_bar(bar, &self.tempo_map.tempo_points);
            let samples_per_beat = sr * 60.0 / cur_bpm;
            let samples_per_bar = samples_per_beat * cur_num as f64;
            let bar_seconds = samples_per_bar / sr;
            let bar_pixel_width = bar_seconds as f32 * self.zoom;

            let x = (sample_pos / sr) as f32 * self.zoom - self.scroll_offset;

            if x > width + 1.0 {
                break;
            }
            if bar > 20_000 {
                break;
            }

            let bar_step = if bar_pixel_width < 40.0 {
                (40.0 / bar_pixel_width).ceil() as u32
            } else {
                1
            };
            let draw_this = bar_step <= 1 || bar % bar_step == 0;

            if draw_this && x >= -1.0 {
                let bar_number = bar as i64 + 1; // 1-based

                // Major tick (bar)
                frame.fill_rectangle(
                    Point::new(x, ruler_height - 12.0),
                    Size::new(1.0, 12.0),
                    theme::TEXT_DIM,
                );

                // Bar number label
                frame.fill_text(canvas::Text {
                    content: format!("{}", bar_number),
                    position: Point::new(x + 3.0, ruler_height - 24.0),
                    color: theme::TEXT_DIM,
                    size: 11.0.into(),
                    ..canvas::Text::default()
                });

                // Beat ticks within bar (only if enough space)
                if bar_pixel_width >= 40.0 {
                    for beat in 1..cur_num {
                        let beat_sample = sample_pos + beat as f64 * samples_per_beat;
                        let bx = (beat_sample / sr) as f32 * self.zoom - self.scroll_offset;
                        if bx >= 0.0 && bx <= width {
                            frame.fill_rectangle(
                                Point::new(bx, ruler_height - 6.0),
                                Size::new(1.0, 6.0),
                                Color::from_rgb(0.25, 0.25, 0.25),
                            );
                        }
                    }
                }
            }

            sample_pos += samples_per_bar;
        }

        // Ruler bottom line
        frame.fill_rectangle(
            Point::new(0.0, ruler_height - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );
    }

    pub(super) fn draw_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        sorted_tracks: &[&TrackState],
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        let track_index = sorted_tracks.iter().position(|t| t.id == clip.track_id);

        let track_index = match track_index {
            Some(i) => i,
            None => return,
        };

        // 8px inset top/bottom matches the design.
        let lane_y = ruler_height + track_index as f32 * theme::TRACK_HEIGHT - y_off;
        let y = lane_y + 8.0;
        let clip_height = theme::TRACK_HEIGHT - 16.0;

        if y + clip_height < ruler_height || y > visible_height {
            return;
        }

        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = clip.duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;
        if w <= 0.0 {
            return;
        }

        // Audio clips: warm/amber wash + tinted border.
        let is_selected = self.selected_clip == Some(clip.id);
        let body_color = Color {
            a: 0.10,
            ..theme::WARM
        };
        let border_color = if is_selected {
            theme::ACCENT
        } else {
            Color {
                a: 0.32,
                ..theme::WARM
            }
        };

        let body = canvas::Path::rounded_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            8.0.into(),
        );
        frame.fill(&body, body_color);

        // Waveform — warm-tinted bars on top of the wash.
        let header_height = 18.0;
        if !clip.waveform_peaks.is_empty() {
            let wave_y = y + header_height;
            let wave_h = clip_height - header_height - 4.0;
            let wave_center = wave_y + wave_h * 0.5;

            let peak_frames = resonance_audio::types::WAVEFORM_PEAK_FRAMES as f32;
            let seconds_per_peak = peak_frames / self.sample_rate as f32;
            let pixels_per_peak = seconds_per_peak * self.zoom;

            let trim_start_peaks = clip.trim_start_frames as f32 / peak_frames;

            let waveform_color = Color {
                a: 0.7,
                ..theme::WARM
            };

            let start_px = (-x).max(0.0);
            let mut px = start_px;
            while px < w {
                let peak_idx_f = trim_start_peaks + px / pixels_per_peak;
                let peak_idx = peak_idx_f as usize;
                if peak_idx >= clip.waveform_peaks.len() {
                    break;
                }
                let (min_val, max_val) = clip.waveform_peaks[peak_idx];

                let draw_x = x + px;
                if draw_x + pixels_per_peak >= 0.0 && draw_x <= w + x {
                    let top = wave_center - max_val * wave_h * 0.5;
                    let bottom = wave_center - min_val * wave_h * 0.5;
                    let bar_h = (bottom - top).max(1.0);
                    frame.fill_rectangle(
                        Point::new(draw_x, top),
                        Size::new(pixels_per_peak.max(1.0), bar_h),
                        waveform_color,
                    );
                }
                px += pixels_per_peak.max(1.0);
            }
        }

        // Clip name + bar count footer in the header row. Text in WARM
        // for audio clips so the kind reads at a glance.
        let display_name: String = if clip.name.chars().count() > 20 {
            let mut truncated: String = clip.name.chars().take(17).collect();
            truncated.push_str("...");
            truncated
        } else {
            clip.name.clone()
        };
        if x + 6.0 < x + w {
            frame.fill_text(canvas::Text {
                content: display_name,
                position: Point::new(x + 9.0, y + 4.0),
                color: theme::WARM,
                size: 10.5.into(),
                ..canvas::Text::default()
            });
        }

        // Border. Selection wins over normal hairline.
        let border_w = if is_selected { 1.5 } else { 1.0 };
        let stroke_path = canvas::Path::rounded_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            8.0.into(),
        );
        frame.stroke(
            &stroke_path,
            canvas::Stroke::default()
                .with_color(border_color)
                .with_width(border_w),
        );
    }

    pub(super) fn draw_midi_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &MidiClipState,
        sorted_tracks: &[&TrackState],
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        let track_index = sorted_tracks.iter().position(|t| t.id == clip.track_id);

        let track_index = match track_index {
            Some(i) => i,
            None => return,
        };

        let lane_y = ruler_height + track_index as f32 * theme::TRACK_HEIGHT - y_off;
        let y = lane_y + 8.0;
        let clip_height = theme::TRACK_HEIGHT - 16.0;

        if y + clip_height < ruler_height || y > visible_height {
            return;
        }

        let clip_end_sample = self.tempo_map.tick_to_abs_sample(
            clip.start_sample,
            clip.duration_ticks,
            self.sample_rate,
        );
        let duration_samples = clip_end_sample.saturating_sub(clip.start_sample) as f64;
        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;
        if w <= 0.0 {
            return;
        }

        // MIDI clips: lavender wash + lavender border.
        let is_selected = self.selected_midi_clip == Some(clip.id);
        let body_color = Color {
            a: 0.10,
            ..theme::ACCENT
        };
        let border_color = if is_selected {
            theme::ACCENT
        } else {
            theme::ACCENT_LINE
        };

        let body = canvas::Path::rounded_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            8.0.into(),
        );
        frame.fill(&body, body_color);

        // Note preview — small lavender rects mapped to the clip's note
        // range. Drawn dimmed so the wash still reads as lavender.
        let header_height = 18.0;
        let note_area_y = y + header_height;
        let note_area_h = clip_height - header_height - 4.0;

        if !clip.notes.is_empty() && note_area_h > 2.0 && w > 2.0 {
            let mut min_note: u8 = 127;
            let mut max_note: u8 = 0;
            for note in &clip.notes {
                if note.note < min_note {
                    min_note = note.note;
                }
                if note.note > max_note {
                    max_note = note.note;
                }
            }
            let range_min = min_note.saturating_sub(2);
            let range_max = (max_note + 2).min(127);
            let note_range = (range_max - range_min).max(1) as f32;

            let total_ticks = clip.duration_ticks as f32;
            if total_ticks > 0.0 {
                let note_color = Color {
                    a: 0.85,
                    ..theme::ACCENT_SOFT
                };
                for note in &clip.notes {
                    let note_start_in_clip = note.start_tick as f32 - clip.trim_start_ticks as f32;
                    if note_start_in_clip + note.duration_ticks as f32 <= 0.0 {
                        continue;
                    }
                    if note_start_in_clip >= total_ticks {
                        continue;
                    }
                    let visible_start = note_start_in_clip.max(0.0);
                    let visible_end =
                        (note_start_in_clip + note.duration_ticks as f32).min(total_ticks);

                    let nx = x + (visible_start / total_ticks) * w;
                    let nw = ((visible_end - visible_start) / total_ticks) * w;

                    let ny = note_area_y
                        + (1.0 - (note.note as f32 - range_min as f32) / note_range)
                            * (note_area_h - 3.0);
                    let nh = (note_area_h / note_range).clamp(2.0, 6.0);

                    frame.fill_rectangle(
                        Point::new(nx, ny),
                        Size::new(nw.max(1.0), nh),
                        note_color,
                    );
                }
            }
        }

        // Clip name in lavender accent.
        let display_name: String = if clip.name.chars().count() > 20 {
            let mut truncated: String = clip.name.chars().take(17).collect();
            truncated.push_str("...");
            truncated
        } else {
            clip.name.clone()
        };
        frame.fill_text(canvas::Text {
            content: display_name,
            position: Point::new(x + 9.0, y + 4.0),
            color: theme::ACCENT_SOFT,
            size: 10.5.into(),
            ..canvas::Text::default()
        });

        let border_w = if is_selected { 1.5 } else { 1.0 };
        let stroke_path = canvas::Path::rounded_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            8.0.into(),
        );
        frame.stroke(
            &stroke_path,
            canvas::Stroke::default()
                .with_color(border_color)
                .with_width(border_w),
        );
    }
}
