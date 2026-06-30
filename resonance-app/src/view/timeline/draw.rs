//! Canvas drawing for the timeline. These are the pure-draw methods
//! that take a `&mut Frame` and render bar/beat grids, rulers, audio
//! clips, and MIDI clips. They're in a separate impl block (and file)
//! so `mod.rs` can stay focused on canvas event handling and state.
use iced::widget::canvas;
use iced::widget::text::Alignment as TextAlignment;
use iced::{Color, Point, Size};

use crate::state::{self, ClipState, MidiClipState, TrackState};
use crate::theme;
use super::TimelineCanvas;
use resonance_audio::types::{avg_bpm_for_bar, FadeCurve, TrackId};

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

    /// Walk bars left-to-right across the visible range, calling `f`
    /// once per bar that should be drawn. Bar positions follow per-bar
    /// tempo and time-signature values from the tempo map, so spacing
    /// correctly follows tempo changes.
    ///
    /// `min_bar_px` controls decimation at low zoom: when a bar is
    /// narrower than this, bars are skipped so the surviving lines stay
    /// at least `min_bar_px` apart (grid uses 20 px, ruler 40 px).
    fn for_each_visible_bar(&self, width: f32, min_bar_px: f32, mut f: impl FnMut(&VisibleBar)) {
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
            let bar_step = if bar_pixel_width < min_bar_px {
                (min_bar_px / bar_pixel_width).ceil() as u32
            } else {
                1
            };
            let draw_this = bar_step <= 1 || bar % bar_step == 0;

            if draw_this && x >= -1.0 {
                f(&VisibleBar {
                    bar,
                    x,
                    pixel_width: bar_pixel_width,
                    numerator: cur_num,
                    sample_pos,
                    samples_per_beat,
                    sr,
                    zoom: self.zoom,
                    scroll_offset: self.scroll_offset,
                });
            }

            sample_pos += samples_per_bar;
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

        self.for_each_visible_bar(width, 20.0, |bar| {
            frame.fill_rectangle(
                Point::new(bar.x, ruler_height),
                Size::new(1.0, line_height),
                theme::BAR_LINE,
            );

            // Beat lines within this bar.
            if bar.pixel_width >= 40.0 {
                for beat in 1..bar.numerator {
                    let bx = bar.beat_x(beat);
                    if bx >= 0.0 && bx <= width {
                        frame.fill_rectangle(
                            Point::new(bx, ruler_height),
                            Size::new(1.0, line_height),
                            theme::BEAT_LINE,
                        );
                    }
                }
            }
        });
    }

    /// Render arrangement markers in the ruler band: point markers as a
    /// colour-tinted flag (pole + swallow-tailed pennant + name label),
    /// ranged markers as a translucent labelled span with start/end edge
    /// lines. Deliberately distinct from the amber loop range (which fills
    /// the ruler with centred triangle handles) and the rounded Compose
    /// section pills (which sit in their own band below the ruler).
    ///
    /// The selected marker (if any) gets the stronger accent: a full-opacity
    /// pole, an outlined flag, and a bright `TEXT_1` label.
    pub(super) fn draw_markers(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
    ) {
        const FLAG_W: f32 = 11.0;
        const FLAG_H: f32 = 9.0;

        for marker in self.markers {
            let start_x = self.sample_to_x(marker.start_sample);
            let end_x = marker.end_sample.map(|e| self.sample_to_x(e));

            // Cull markers wholly off either edge (a region's right edge or
            // a point's own x).
            let right_edge = end_x.unwrap_or(start_x);
            if right_edge < 0.0 || start_x > width {
                continue;
            }

            let is_selected = self.selected_marker_id == Some(marker.id);
            let color = Color::from_rgb(
                marker.color[0] as f32 / 255.0,
                marker.color[1] as f32 / 255.0,
                marker.color[2] as f32 / 255.0,
            );

            // Ranged region: translucent fill across the ruler + an edge
            // line at the end so the span's extent reads clearly.
            if let Some(end_x) = end_x {
                let span_x = start_x.max(0.0);
                let span_w = (end_x.min(width) - span_x).max(0.0);
                if span_w > 0.0 {
                    frame.fill_rectangle(
                        Point::new(span_x, 0.0),
                        Size::new(span_w, ruler_height),
                        Color {
                            a: if is_selected { 0.26 } else { 0.16 },
                            ..color
                        },
                    );
                }
                if end_x >= 0.0 && end_x <= width {
                    frame.fill_rectangle(
                        Point::new(end_x - 0.5, 0.0),
                        Size::new(1.0, ruler_height),
                        Color { a: 0.7, ..color },
                    );
                }
            }

            // Start pole — the flag's mast, shared by point and ranged
            // markers so a region also gets a clear start handle.
            if start_x >= 0.0 && start_x <= width {
                frame.fill_rectangle(
                    Point::new(start_x - 0.5, 0.0),
                    Size::new(1.0, ruler_height),
                    if is_selected {
                        color
                    } else {
                        Color { a: 0.8, ..color }
                    },
                );
            }

            // Flag pennant at the top of the pole.
            if start_x <= width && start_x + FLAG_W >= 0.0 {
                let fx = start_x;
                let flag = canvas::Path::new(|b| {
                    b.move_to(Point::new(fx, 0.0));
                    b.line_to(Point::new(fx + FLAG_W, 0.0));
                    b.line_to(Point::new(fx + FLAG_W - 3.0, FLAG_H * 0.5));
                    b.line_to(Point::new(fx + FLAG_W, FLAG_H));
                    b.line_to(Point::new(fx, FLAG_H));
                    b.close();
                });
                frame.fill(&flag, color);
                if is_selected {
                    frame.stroke(
                        &flag,
                        canvas::Stroke::default()
                            .with_width(1.0)
                            .with_color(theme::TEXT_1),
                    );
                }
            }

            // Name label, just right of the flag near the top of the ruler.
            let label_x = start_x.max(0.0) + FLAG_W + 4.0;
            if label_x < width - 6.0 {
                frame.fill_text(canvas::Text {
                    content: crate::util::short_with(&marker.name, 18, "..."),
                    position: Point::new(label_x, 1.0),
                    color: if is_selected { theme::TEXT_1 } else { color },
                    size: 10.0.into(),
                    font: theme::UI_FONT_SEMIBOLD,
                    ..canvas::Text::default()
                });
            }
        }
    }

    /// Draw the bar/beat ruler at the top.
    /// Uses per-bar tempo and time-signature values so bar numbers are
    /// positioned correctly when tempo changes.
    pub(super) fn draw_ruler(&self, frame: &mut canvas::Frame, width: f32, ruler_height: f32) {
        self.for_each_visible_bar(width, 40.0, |bar| {
            let bar_number = bar.bar as i64 + 1; // 1-based

            // Major tick (bar)
            frame.fill_rectangle(
                Point::new(bar.x, ruler_height - 12.0),
                Size::new(1.0, 12.0),
                theme::TEXT_DIM,
            );

            // Bar number label
            frame.fill_text(canvas::Text {
                content: format!("{}", bar_number),
                position: Point::new(bar.x + 3.0, ruler_height - 24.0),
                color: theme::TEXT_DIM,
                size: 11.0.into(),
                ..canvas::Text::default()
            });

            // Beat ticks within bar (only if enough space)
            if bar.pixel_width >= 40.0 {
                for beat in 1..bar.numerator {
                    let bx = bar.beat_x(beat);
                    if bx >= 0.0 && bx <= width {
                        frame.fill_rectangle(
                            Point::new(bx, ruler_height - 6.0),
                            Size::new(1.0, 6.0),
                            Color::from_rgb(0.25, 0.25, 0.25),
                        );
                    }
                }
            }
        });

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
        let Some((y, clip_height)) = clip_lane_rect(
            clip.track_id,
            sorted_tracks,
            ruler_height,
            y_off,
            visible_height,
        ) else {
            return;
        };

        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = clip.duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;
        if w <= 0.0 {
            return;
        }

        // Audio clips: warm/amber wash + tinted border. Name text in
        // WARM so the kind reads at a glance. A frozen / rendered track
        // has no editable sample source — its clips are "unsupported":
        // diagonal hatch, no fade handles (gain still applies). Design #153.
        let is_selected = self.selected_clip == Some(clip.id);
        let fadeable = !self.frozen_tracks.contains(&clip.track_id);
        let chrome = ClipChrome {
            x,
            y,
            w,
            h: clip_height,
            // Clip-gain tint: louder brightens the warm wash, quieter
            // darkens it, so level reads at a glance (design #153).
            body_color: gain_tinted_body(clip.gain_db),
            border_color: if is_selected {
                theme::ACCENT
            } else {
                Color {
                    a: 0.32,
                    ..theme::WARM
                }
            },
            is_selected,
            name: &clip.name,
            name_color: theme::WARM,
            show_name: x + 6.0 < x + w,
        };

        chrome.draw(frame, |frame| {
            self.draw_clip_waveform(frame, clip, x, y, w, clip_height);
            if fadeable {
                self.draw_clip_fades(frame, clip, x, y, w, clip_height);
            } else {
                draw_clip_hatch(frame, x, y, w, clip_height);
            }
        });

        // Overlays drawn on top of the border: the fade-handle / gain
        // beads and the mono dB header tag.
        self.draw_clip_handles(frame, clip, x, y, w, clip_height, fadeable, is_selected);
        draw_clip_gain_tag(frame, clip, x, y, w);
    }

    /// Fade-in / fade-out ramps: a darkened wedge over the attenuated
    /// region (so the waveform under it reads as faded) plus a warm ramp
    /// line tracing the chosen curve. Drawn inside the clip body, over
    /// the waveform and under the name / border.
    fn draw_clip_fades(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        let sr = self.sample_rate as f32;
        let fade_in_w = ((clip.fade_in_frames as f32 / sr) * self.zoom).clamp(0.0, w);
        let fade_out_w = ((clip.fade_out_frames as f32 / sr) * self.zoom).clamp(0.0, w);

        if fade_in_w > 0.5 {
            let env = fade_envelope(clip.fade_in_curve, x, fade_in_w, y, h, true);
            frame.fill(&fade_wedge_path(&env, x, fade_in_w, y), FADE_WEDGE_COLOR);
            stroke_polyline(frame, &env, theme::WARM, 1.5);
        }
        if fade_out_w > 0.5 {
            let x0 = x + w - fade_out_w;
            let env = fade_envelope(clip.fade_out_curve, x0, fade_out_w, y, h, false);
            frame.fill(&fade_wedge_path(&env, x0, fade_out_w, y), FADE_WEDGE_COLOR);
            stroke_polyline(frame, &env, theme::WARM, 1.5);
        }
    }

    /// Circular fade-handle beads on the two top corners (warm) and the
    /// clip-gain bead at top-centre (lavender). Fade beads ride inward
    /// as the fade grows (`handle x = ramp end`); they stay hidden on a
    /// clean clip until it is selected, but are always shown once a fade
    /// exists. Gain is available even on frozen clips. Design #153.
    #[allow(clippy::too_many_arguments)]
    fn draw_clip_handles(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        x: f32,
        y: f32,
        w: f32,
        _h: f32,
        fadeable: bool,
        is_selected: bool,
    ) {
        let sr = self.sample_rate as f32;
        let right = x + w;

        if fadeable {
            if clip.fade_in_frames > 0 || is_selected {
                let bx = (x + (clip.fade_in_frames as f32 / sr) * self.zoom).clamp(x, right);
                draw_bead(frame, bx, y, theme::WARM);
            }
            if clip.fade_out_frames > 0 || is_selected {
                let bx = (right - (clip.fade_out_frames as f32 / sr) * self.zoom).clamp(x, right);
                draw_bead(frame, bx, y, theme::WARM);
            }
        }

        // Gain bead at top-centre. Shown once gain departs unity, or on
        // selection so an untouched clip stays clean but discoverable.
        if clip.gain_db.abs() > 0.05 || is_selected {
            draw_bead(frame, x + w / 2.0, y, theme::ACCENT);
        }
    }

    /// Waveform — warm-tinted bars on top of the wash.
    fn draw_clip_waveform(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        x: f32,
        y: f32,
        w: f32,
        clip_height: f32,
    ) {
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
    }

    /// Automatic crossfades: wherever two audio clips on the same track
    /// overlap, the seam gets a lavender overlap wash, two crossing
    /// equal-power curves (left clip fading out, right clip fading in),
    /// and an `⤬` badge. Crossfade is derived, never stored — overlap
    /// implies crossfade regardless of the clips' manual fades (design
    /// #153 / arch #156).
    pub(super) fn draw_crossfades(
        &self,
        frame: &mut canvas::Frame,
        sorted_tracks: &[&TrackState],
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        let n = self.clips.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let a = &self.clips[i];
                let b = &self.clips[j];
                if a.track_id != b.track_id {
                    continue;
                }
                let Some((ov_start, ov_end)) = overlap_range(
                    a.start_sample,
                    a.duration_samples,
                    b.start_sample,
                    b.duration_samples,
                ) else {
                    continue;
                };
                let Some((y, h)) = clip_lane_rect(
                    a.track_id,
                    sorted_tracks,
                    ruler_height,
                    y_off,
                    visible_height,
                ) else {
                    continue;
                };

                let x0 = self.sample_to_x(ov_start);
                let x1 = self.sample_to_x(ov_end);
                let ow = x1 - x0;
                if ow <= 0.5 {
                    continue;
                }

                // Lavender overlap wash.
                frame.fill_rectangle(
                    Point::new(x0, y),
                    Size::new(ow, h),
                    Color {
                        a: 0.16,
                        ..theme::ACCENT
                    },
                );

                // Crossing equal-power curves: the earlier clip fades out
                // across the overlap, the later clip fades in. Their sum
                // is constant power — a click-free seam. Equal-power is
                // symmetric, so the same pair of curves serves either
                // ordering of the overlapping clips.
                let fade_out = fade_envelope(FadeCurve::EqualPower, x0, ow, y, h, false);
                let fade_in = fade_envelope(FadeCurve::EqualPower, x0, ow, y, h, true);
                stroke_polyline(frame, &fade_out, theme::ACCENT_SOFT, 1.5);
                stroke_polyline(frame, &fade_in, theme::ACCENT_SOFT, 1.5);

                // `⤬` badge centred at the top of the overlap.
                draw_crossfade_badge(frame, x0 + ow / 2.0, y + 9.0);
            }
        }
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
        let Some((y, clip_height)) = clip_lane_rect(
            clip.track_id,
            sorted_tracks,
            ruler_height,
            y_off,
            visible_height,
        ) else {
            return;
        };

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

        // MIDI clips: lavender wash + lavender border, name in lavender
        // accent.
        let is_selected = self.selected_midi_clip == Some(clip.id);
        let chrome = ClipChrome {
            x,
            y,
            w,
            h: clip_height,
            body_color: Color {
                a: 0.10,
                ..theme::ACCENT
            },
            border_color: if is_selected {
                theme::ACCENT
            } else {
                theme::ACCENT_LINE
            },
            is_selected,
            name: &clip.name,
            name_color: theme::ACCENT_SOFT,
            show_name: true,
        };

        chrome.draw(frame, |frame| {
            self.draw_midi_clip_notes(frame, clip, x, y, w, clip_height)
        });
    }

    /// Note preview — small lavender rects mapped to the clip's note
    /// range. Drawn dimmed so the wash still reads as lavender.
    fn draw_midi_clip_notes(
        &self,
        frame: &mut canvas::Frame,
        clip: &MidiClipState,
        x: f32,
        y: f32,
        w: f32,
        clip_height: f32,
    ) {
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
    }
}

/// One bar yielded by [`TimelineCanvas::for_each_visible_bar`]: its
/// index, on-screen x position, pixel width, and the timing context
/// needed to place beat subdivisions within the bar.
struct VisibleBar {
    /// Zero-based bar index.
    bar: u32,
    /// X position of the bar start, in canvas pixels.
    x: f32,
    /// Width of this bar in pixels at the current zoom.
    pixel_width: f32,
    /// Time-signature numerator (beats per bar) active in this bar.
    numerator: u8,
    sample_pos: f64,
    samples_per_beat: f64,
    sr: f64,
    zoom: f32,
    scroll_offset: f32,
}

impl VisibleBar {
    /// X position of `beat` (1-based within the bar), in canvas pixels.
    fn beat_x(&self, beat: u8) -> f32 {
        let beat_sample = self.sample_pos + beat as f64 * self.samples_per_beat;
        (beat_sample / self.sr) as f32 * self.zoom - self.scroll_offset
    }
}

/// Lane-relative rect for a clip on `track_id`: returns `(y, height)`,
/// or `None` when the track is unknown or the lane is scrolled out of
/// view. The `CLIP_LANE_INSET` top/bottom inset matches the design.
fn clip_lane_rect(
    track_id: TrackId,
    sorted_tracks: &[&TrackState],
    ruler_height: f32,
    y_off: f32,
    visible_height: f32,
) -> Option<(f32, f32)> {
    let track_index = sorted_tracks.iter().position(|t| t.id == track_id)?;

    let lane_y = ruler_height + track_index as f32 * theme::TRACK_HEIGHT - y_off;
    let y = lane_y + theme::CLIP_LANE_INSET;
    let clip_height = theme::TRACK_HEIGHT - 2.0 * theme::CLIP_LANE_INSET;

    if y + clip_height < ruler_height || y > visible_height {
        return None;
    }

    Some((y, clip_height))
}

/// Shared chrome for audio and MIDI clips on the timeline: rounded
/// body wash, truncated name label, and selection-aware border. The
/// kind-specific interior (waveform or note preview) is drawn by the
/// `content` closure between the body fill and the name/border, so
/// layering matches the per-kind draw order.
struct ClipChrome<'a> {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    body_color: Color,
    border_color: Color,
    is_selected: bool,
    name: &'a str,
    name_color: Color,
    /// Audio clips gate the label on clip width; MIDI clips always
    /// draw it.
    show_name: bool,
}

impl ClipChrome<'_> {
    fn draw(self, frame: &mut canvas::Frame, content: impl FnOnce(&mut canvas::Frame)) {
        let body = canvas::Path::rounded_rectangle(
            Point::new(self.x, self.y),
            Size::new(self.w, self.h),
            8.0.into(),
        );
        frame.fill(&body, self.body_color);

        content(frame);

        // Clip name in the header row, truncated for long names.
        // ASCII "..." suffix (not '…') — keeps the canvas text metrics
        // identical to the pre-refactor rendering.
        let display_name = crate::util::short_with(self.name, 20, "...");
        if self.show_name {
            frame.fill_text(canvas::Text {
                content: display_name,
                position: Point::new(self.x + 9.0, self.y + 4.0),
                color: self.name_color,
                size: 10.5.into(),
                ..canvas::Text::default()
            });
        }

        // Border. Selection wins over normal hairline.
        let border_w = if self.is_selected { 1.5 } else { 1.0 };
        let stroke_path = canvas::Path::rounded_rectangle(
            Point::new(self.x, self.y),
            Size::new(self.w, self.h),
            8.0.into(),
        );
        frame.stroke(
            &stroke_path,
            canvas::Stroke::default()
                .with_color(self.border_color)
                .with_width(border_w),
        );
    }
}

/// Darkening applied over the attenuated part of a fade ramp so the
/// waveform under it reads as faded out.
const FADE_WEDGE_COLOR: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.22,
};

/// Visual radius of the circular fade / gain handle beads. Smaller than
/// the 10px hit radius in [`super::hit_test`] so the bead reads as a neat
/// dot while staying easy to grab.
const BEAD_RADIUS: f32 = 4.5;

/// Map a clip's gain (dB) to its body wash colour. Unity sits at the
/// existing warm wash; louder raises the alpha (brighter), quieter lowers
/// it (darker) so a clip's level is legible without opening anything.
/// Clamped over ±18 dB so extreme gains stay within a readable band.
pub fn gain_tinted_body(gain_db: f32) -> Color {
    let norm = (gain_db / 18.0).clamp(-1.0, 1.0);
    let a = (0.10 + norm * 0.08).clamp(0.03, 0.20);
    Color { a, ..theme::WARM }
}

/// Format a clip-gain value as a signed mono dB tag, e.g. `+3.0 dB`,
/// `-6.0 dB`. Values within 0.05 dB of unity collapse to `+0.0 dB` so a
/// `-0.0` never prints.
pub fn format_gain_db(gain_db: f32) -> String {
    let g = if gain_db.abs() < 0.05 { 0.0 } else { gain_db };
    format!("{g:+.1} dB")
}

/// Sample-overlap of two clips (in samples), or `None` if they don't
/// overlap. Used to derive the automatic crossfade region.
pub fn overlap_range(
    a_start: u64,
    a_dur: u64,
    b_start: u64,
    b_dur: u64,
) -> Option<(u64, u64)> {
    let a_end = a_start.saturating_add(a_dur);
    let b_end = b_start.saturating_add(b_dur);
    let start = a_start.max(b_start);
    let end = a_end.min(b_end);
    if end > start {
        Some((start, end))
    } else {
        None
    }
}

/// Screen-space points tracing a fade ramp envelope across `[x0, x0+rw]`.
/// `fade_in` runs silence→unity left→right; otherwise unity→silence. The
/// curve's [`FadeCurve::coefficient`] maps progress to amplitude, and
/// amplitude maps to height (unity at the clip top, silence at the
/// bottom), so the polyline is the ramp line and the area above it is the
/// attenuated wedge.
pub fn fade_envelope(
    curve: FadeCurve,
    x0: f32,
    rw: f32,
    top_y: f32,
    h: f32,
    fade_in: bool,
) -> Vec<Point> {
    const SEGMENTS: usize = 16;
    (0..=SEGMENTS)
        .map(|i| {
            let t = i as f32 / SEGMENTS as f32;
            let amp = if fade_in {
                curve.coefficient(t)
            } else {
                curve.coefficient(1.0 - t)
            };
            Point::new(x0 + t * rw, top_y + h * (1.0 - amp))
        })
        .collect()
}

/// Polygon for the darkened part of a fade ramp: the region between the
/// clip's top edge (`[x0, x0+rw]`) and the envelope below it.
fn fade_wedge_path(env: &[Point], x0: f32, rw: f32, top_y: f32) -> canvas::Path {
    canvas::Path::new(|b| {
        if let Some(first) = env.first() {
            b.move_to(*first);
            for p in &env[1..] {
                b.line_to(*p);
            }
            b.line_to(Point::new(x0 + rw, top_y));
            b.line_to(Point::new(x0, top_y));
            b.close();
        }
    })
}

/// Stroke a polyline through `pts`.
fn stroke_polyline(frame: &mut canvas::Frame, pts: &[Point], color: Color, width: f32) {
    if pts.len() < 2 {
        return;
    }
    let path = canvas::Path::new(|b| {
        b.move_to(pts[0]);
        for p in &pts[1..] {
            b.line_to(*p);
        }
    });
    frame.stroke(
        &path,
        canvas::Stroke::default().with_color(color).with_width(width),
    );
}

/// A circular handle bead centred at `(cx, cy)`: a filled disc in `color`
/// with a thin dark ring for contrast against the clip wash.
fn draw_bead(frame: &mut canvas::Frame, cx: f32, cy: f32, color: Color) {
    let disc = canvas::Path::circle(Point::new(cx, cy), BEAD_RADIUS);
    frame.fill(&disc, color);
    frame.stroke(
        &disc,
        canvas::Stroke::default()
            .with_color(Color {
                a: 0.9,
                ..theme::BG_1
            })
            .with_width(1.0),
    );
}

/// Diagonal hatch over an "unsupported" (frozen / rendered) clip — the
/// degradation surface for clips with no editable sample source. Each
/// 45° line is clamped to the clip rect by hand (a nested `with_clip`
/// renders nothing inside the cached timeline frame), so the strokes
/// never bleed past the body.
fn draw_clip_hatch(frame: &mut canvas::Frame, x: f32, y: f32, w: f32, h: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let color = Color {
        a: 0.16,
        ..theme::TEXT_1
    };
    const SPACING: f32 = 7.0;
    // Each line runs from the bottom edge at `sx` up to the top edge at
    // `sx + h` (slope -1): point(t) = (sx + t·h, (y+h) − t·h), t ∈ [0, 1].
    // Clamp t so x stays within [x, x+w]; y then stays within [y, y+h].
    let mut sx = x - h;
    while sx < x + w {
        let t_lo = ((x - sx) / h).clamp(0.0, 1.0);
        let t_hi = ((x + w - sx) / h).clamp(0.0, 1.0);
        if t_hi > t_lo {
            let p = |t: f32| Point::new(sx + t * h, (y + h) - t * h);
            let line = canvas::Path::new(|b| {
                b.move_to(p(t_lo));
                b.line_to(p(t_hi));
            });
            frame.stroke(
                &line,
                canvas::Stroke::default().with_color(color).with_width(1.0),
            );
        }
        sx += SPACING;
    }
}

/// The mono `±N.N dB` gain tag in the clip header, right-aligned. Hidden
/// at unity and on clips too narrow to fit it, so untouched clips stay
/// clean (design #153).
fn draw_clip_gain_tag(frame: &mut canvas::Frame, clip: &ClipState, x: f32, y: f32, w: f32) {
    if clip.gain_db.abs() <= 0.05 || w < 64.0 {
        return;
    }
    frame.fill_text(canvas::Text {
        content: format_gain_db(clip.gain_db),
        position: Point::new(x + w - 7.0, y + 4.0),
        color: theme::TEXT_2,
        size: 9.5.into(),
        font: theme::MONO_FONT,
        align_x: TextAlignment::Right,
        ..canvas::Text::default()
    });
}

/// The `⤬` crossfade badge: a small lavender disc with a white cross,
/// centred at `(cx, cy)`. Drawn with strokes rather than a glyph so it
/// renders identically regardless of font coverage.
fn draw_crossfade_badge(frame: &mut canvas::Frame, cx: f32, cy: f32) {
    const R: f32 = 7.0;
    let disc = canvas::Path::circle(Point::new(cx, cy), R);
    frame.fill(&disc, theme::ACCENT);
    let arm = R * 0.5;
    let cross = canvas::Path::new(|b| {
        b.move_to(Point::new(cx - arm, cy - arm));
        b.line_to(Point::new(cx + arm, cy + arm));
        b.move_to(Point::new(cx + arm, cy - arm));
        b.line_to(Point::new(cx - arm, cy + arm));
    });
    frame.stroke(
        &cross,
        canvas::Stroke::default()
            .with_color(theme::TEXT_1)
            .with_width(1.5),
    );
}
