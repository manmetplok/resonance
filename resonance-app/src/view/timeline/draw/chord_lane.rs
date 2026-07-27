//! Chord lane and section band drawing.
use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::theme;
use super::TimelineCanvas;

impl TimelineCanvas<'_> {
    /// Render the compose-section pills above the lanes. Each placement
    /// becomes a colored pill spanning its bars. Selected placement gets
    /// the lavender-wash accent; unselected placements use a softer wash
    /// derived from the section's color.
    pub(in crate::view::timeline) fn draw_section_band(
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

    /// Draw the chord lane: for each placed section, render a small
    /// section tab at the top + chord blocks beneath, sized to the
    /// section's footprint on the timeline. Chord blocks are tinted by
    /// quality (minor = lavender, dom = warm, major = neutral) so the
    /// progression reads at a glance.
    pub(super) fn draw_chord_lane(
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

            let section_start_sample = self.tempo_map.bar_to_sample(placement.start_bar);
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
                let chord_start_bars = chord.start_beat as f32 / beats_per_bar;
                let chord_end_bars =
                    (chord.start_beat + chord.duration_beats) as f32 / beats_per_bar;
                if chord_start_bars >= section_bars {
                    continue;
                }
                let chord_end_bars = chord_end_bars.min(section_bars);

                let block_left =
                    section_x + (chord_start_bars / section_bars) * section_pixel_width;
                let block_right =
                    section_x + (chord_end_bars / section_bars) * section_pixel_width;
                let block_w = (block_right - block_left - 3.0).max(0.0);
                if block_w <= 0.0 || block_right < 0.0 || block_left > width {
                    continue;
                }

                // Tint by quality — minor uses the lavender accent, dom
                // uses warm/amber, every other quality reads as neutral.
                use resonance_music_theory::ChordQuality;
                let (body_color, border_color, text_color) = match chord.chord.quality {
                    ChordQuality::Min
                    | ChordQuality::Min7
                    | ChordQuality::Min6
                    | ChordQuality::MinMaj7
                    | ChordQuality::HalfDim7 => (
                        Color {
                            a: 0.10,
                            ..theme::ACCENT
                        },
                        Color {
                            a: 0.30,
                            ..theme::ACCENT
                        },
                        theme::ACCENT_SOFT,
                    ),
                    ChordQuality::Dom7 => (
                        Color {
                            a: 0.10,
                            ..theme::WARM
                        },
                        Color {
                            a: 0.32,
                            ..theme::WARM
                        },
                        theme::WARM,
                    ),
                    _ => (
                        Color {
                            a: 0.04,
                            ..theme::TEXT_1
                        },
                        Color {
                            a: 0.10,
                            ..theme::TEXT_1
                        },
                        theme::TEXT_1,
                    ),
                };

                let visible_x = block_left.max(0.0);
                let visible_w = (block_left + block_w).min(width) - visible_x;
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
                        position: Point::new(visible_x + 6.0, blocks_y + 4.0),
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
}
