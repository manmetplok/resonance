//! Performance mode — full-screen, distraction-free live chord
//! teleprompter (epic #11, design doc #151).
//!
//! This module owns the full-bleed Performance surface routed to from
//! [`crate::Resonance::view`] when `view_mode == ViewMode::Performance`.
//! It lays out the four horizontal bands from design #151:
//!
//! 1. **Status bar** (56px) — `PERFORMANCE` pill + project name; centre
//!    transport state (recording / rehearsal / stopped); right-aligned mono
//!    telemetry (bar·beat clock, BPM, time signature, key) + `Exit` button.
//! 2. **Center stage** (fill) — placeholder region; the huge chord symbol +
//!    fingering diagram + beat ring land in todos #308/#310.
//! 3. **Next-chords lane** (180px) — placeholder region; the look-ahead
//!    cards land in todo #309.
//! 4. **Footer strip** (44px) — instrument/tuning + capo placeholders
//!    (interactive controls land in todo #311) and the keyboard-hint line.
//!
//! Only the static chrome (bands 1 and 4 plus the band skeleton) is wired
//! here; the live Canvas content is added by the follow-up todos. Per the
//! view-performance rules the three non-live bands (centre stage, next-lane
//! skeleton, footer) are wrapped in `iced::widget::lazy` keyed on a
//! fingerprint of their inputs, so the status bar's per-frame telemetry tick
//! never rebuilds them — no per-frame churn during a take.

use crate::message::{Message, UiMessage};
use crate::theme;
use crate::Resonance;
use iced::widget::text::LineHeight;
use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};

/// Band heights, from design #151.
const STATUS_BAR_HEIGHT: f32 = 56.0;
const NEXT_LANE_HEIGHT: f32 = 180.0;
const FOOTER_HEIGHT: f32 = 44.0;
/// Horizontal lead-in/out for the status bar and footer (prototype: 28px).
const CHROME_HPAD: f32 = 28.0;
/// Horizontal lead-in/out for the wide stage + next-lane bands (80px).
const STAGE_HPAD: f32 = 80.0;

// -- Static-band inputs ------------------------------------------------------
//
// The centre stage, next-lane skeleton, and footer render no per-frame state
// in this scaffold, so the values they show are pinned here and fed to both
// the band builders and their lazy-cache fingerprints below. Each const is the
// single seam a follow-up todo swaps for real state — at which point the
// matching lazy region begins invalidating on genuine input changes:
//   * #308 wires the chord under the playhead    -> `STAGE_HAS_CHORD`
//   * #309 wires the look-ahead cards            -> `NEXT_LANE_HAS_CHORDS`
//   * #311 wires the instrument + capo controls  -> `FOOTER_ACTIVE_TUNING` / `FOOTER_CAPO_FRETS`

/// Whether a chord sits under the playhead (centre stage shows the empty
/// state until #308 lands).
const STAGE_HAS_CHORD: bool = false;
/// Whether any upcoming chords exist (next-lane shows the empty card until
/// #309 lands).
const NEXT_LANE_HAS_CHORDS: bool = false;
/// Active instrument tuning cell (Guitar 6); the selector lands in #311.
const FOOTER_ACTIVE_TUNING: usize = 0;
/// Capo position in frets; the stepper lands in #311.
const FOOTER_CAPO_FRETS: u8 = 0;

/// Hash a static band's inputs into a stable fingerprint for its lazy cache.
/// Matches the `*_fingerprint` convention used by the mixer inspector and
/// track-header columns.
fn fingerprint(parts: &[u64]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Fingerprint for the centre-stage lazy region.
fn center_stage_fingerprint() -> u64 {
    fingerprint(&[STAGE_HAS_CHORD as u64])
}

/// Fingerprint for the next-chords-lane lazy region.
fn next_lane_fingerprint() -> u64 {
    fingerprint(&[NEXT_LANE_HAS_CHORDS as u64])
}

/// Fingerprint for the footer lazy region.
fn footer_fingerprint() -> u64 {
    fingerprint(&[FOOTER_ACTIVE_TUNING as u64, FOOTER_CAPO_FRETS as u64])
}

impl Resonance {
    /// Top-level Performance shell: a full-bleed BG_0 surface stacking the
    /// four design bands. The normal transport chrome is hidden in this
    /// mode (see [`crate::Resonance::view`]).
    pub(crate) fn view_performance_shell(&self) -> Element<'_, Message> {
        // Only the status bar carries live state (the bar·beat clock + transport
        // cluster tick every audio frame), so it is rebuilt each redraw. The
        // other three bands render no per-frame state in this scaffold, so their
        // content is cached behind `iced::widget::lazy` keyed on a fingerprint
        // of its inputs — during a take the telemetry clock can churn without
        // rebuilding the stage / next-lane / footer subtrees (view-performance
        // rule #2; see ui-work.md §11, and the reference impls in
        // mixer/inspector.rs and track_header/mod.rs). The next-lane + footer
        // are fixed-height, so the whole band wraps; the centre stage is
        // `Length::Fill` (which lazy doesn't forward), so it caches its content
        // internally and keeps its sizing container outside the cache.
        let next_lane_fp = next_lane_fingerprint();
        let footer_fp = footer_fingerprint();
        let body = column![
            self.performance_status_bar(),
            hairline(),
            self.performance_center_stage(),
            hairline(),
            iced::widget::lazy(next_lane_fp, move |_: &u64| -> Element<'static, Message> {
                self.performance_next_lane()
            }),
            hairline(),
            iced::widget::lazy(footer_fp, move |_: &u64| -> Element<'static, Message> {
                self.performance_footer()
            }),
        ]
        .spacing(0);

        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(performance_backdrop)
            .into()
    }

    // -- Band 1: status bar --------------------------------------------------

    /// Top status bar: identity (left), transport state (centre), telemetry
    /// + Exit (right). Recedes visually so the centre stage dominates.
    fn performance_status_bar(&self) -> Element<'_, Message> {
        // Left: PERFORMANCE pill + italic-serif project name.
        let project_name = self
            .io
            .project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Untitled".to_string());

        let left = row![
            performance_pill(),
            text(project_name)
                .size(19)
                .font(theme::SERIF_ITALIC_FONT)
                .color(theme::TEXT_2)
                .line_height(LineHeight::Relative(1.0)),
        ]
        .spacing(14)
        .align_y(alignment::Vertical::Center);

        let centre = self.performance_transport_state();
        let right = self.performance_telemetry();

        let inner = row![
            left,
            Space::new().width(Length::Fill),
            centre,
            Space::new().width(Length::Fill),
            right,
        ]
        .align_y(alignment::Vertical::Center);

        container(
            row![
                Space::new().width(CHROME_HPAD),
                inner,
                Space::new().width(CHROME_HPAD),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .width(Length::Fill)
        .height(STATUS_BAR_HEIGHT)
        .align_y(alignment::Vertical::Center)
        .style(band_bg(theme::BG_1))
        .into()
    }

    /// Centre cluster of the status bar: REC dot · "Recording", a play
    /// triangle · "Rehearsal", or just "Stopped" — uppercase, recessed.
    fn performance_transport_state(&self) -> Element<'_, Message> {
        // `glyph` carries its own colour; `None` means no leading icon
        // (the doc's STOPPED state has no symbol, just the label).
        let (glyph, label, label_color) = if self.transport.recording {
            (Some((theme::fa::CIRCLE, theme::BAD)), "Recording", theme::BAD)
        } else if self.transport.playing {
            (
                Some((theme::fa::PLAY, theme::TEXT_2)),
                "Rehearsal",
                theme::TEXT_2,
            )
        } else {
            (None, "Stopped", theme::TEXT_2)
        };

        let mut cluster = row![].spacing(10).align_y(alignment::Vertical::Center);
        if let Some((g, color)) = glyph {
            cluster = cluster.push(
                theme::icon(g)
                    .size(11)
                    .color(color)
                    .line_height(LineHeight::Relative(1.0)),
            );
        }
        cluster
            .push(
                text(label.to_uppercase())
                    .size(13)
                    .font(theme::UI_FONT_SEMIBOLD)
                    .color(label_color)
                    .line_height(LineHeight::Relative(1.0)),
            )
            .into()
    }

    /// Right cluster: bar·beat clock (WARM), BPM, time signature, key, and
    /// the `Exit` ghost button. All read straight from the cached transport
    /// labels — no per-frame allocation.
    fn performance_telemetry(&self) -> Element<'_, Message> {
        let labels = &self.transport_labels;

        // Clock shows just "bar . beat" (drop the sub-division segment that
        // the transport's POSITION block carries).
        let clock = bar_beat(labels.position.as_str());
        let clock_el = text(clock)
            .size(18)
            .font(theme::MONO_FONT)
            .color(theme::WARM)
            .line_height(LineHeight::Relative(1.0));

        let bpm = format_bpm(self.transport.bpm);
        let bpm_el = telemetry_value(bpm, theme::TEXT_1);
        let bpm_lab = telemetry_label("BPM");

        let sig_el = telemetry_value(labels.sig.clone(), theme::TEXT_1);

        // Key is "Root mode" (e.g. "B min") or "—". Split so the root reads
        // bright and the mode recedes, matching the design.
        let key_cluster: Element<'_, Message> = match labels.key.split_once(' ') {
            Some((root, mode)) => row![
                telemetry_value(root.to_string(), theme::TEXT_1),
                telemetry_label(mode),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .into(),
            None => telemetry_value(labels.key.clone(), theme::TEXT_1),
        };

        let exit_btn = button(
            text("Exit \u{238b}")
                .size(12)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_2),
        )
        .on_press(Message::Ui(UiMessage::ExitPerformanceMode))
        .padding([6, 12])
        .style(|_theme, status| theme::ghost_button_style(status));

        row![
            clock_el,
            row![bpm_el, bpm_lab]
                .spacing(6)
                .align_y(alignment::Vertical::Center),
            sig_el,
            key_cluster,
            exit_btn,
        ]
        .spacing(18)
        .align_y(alignment::Vertical::Center)
        .into()
    }

    // -- Band 2: center stage (placeholder) ----------------------------------

    /// Center stage region. The huge current-chord symbol + fingering
    /// diagram + beat ring land in todos #308/#310; until then this shows
    /// the design's empty state (an em-dash with guidance), which is also
    /// the genuine "no chord under the playhead" fallback.
    ///
    /// The band's `Length::Fill` container stays *outside* the lazy cache —
    /// `iced::widget::lazy` doesn't forward a `Fill` size hint, so wrapping
    /// the whole band collapses the layout. The container is cheap structural
    /// chrome; the (static) empty-state content it centres is what gets
    /// cached, keyed on [`center_stage_fingerprint`].
    fn performance_center_stage(&self) -> Element<'_, Message> {
        let content = iced::widget::lazy(
            center_stage_fingerprint(),
            |_: &u64| -> Element<'static, Message> { center_stage_content() },
        );

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([0, STAGE_HPAD as u16])
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
            .into()
    }

    // -- Band 3: next-chords lane (placeholder) ------------------------------

    /// Next-chords look-ahead lane. The current + upcoming cards with mini
    /// fingering diagrams land in todo #309; this is the band skeleton with
    /// its label and an empty-state card. Returns owned (`'static`) content
    /// so it can live behind the lazy cache in
    /// [`Resonance::view_performance_shell`].
    fn performance_next_lane(&self) -> Element<'static, Message> {
        let label = text("UP NEXT")
            .size(12)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3)
            .line_height(LineHeight::Relative(1.0));

        let empty_card = container(
            column![
                text("\u{2014}")
                    .size(40)
                    .font(theme::SERIF_ITALIC_FONT)
                    .color(theme::TEXT_4)
                    .line_height(LineHeight::Relative(1.0)),
                Space::new().height(6),
                text("no upcoming chords")
                    .size(11)
                    .font(theme::MONO_FONT)
                    .color(theme::TEXT_3),
            ]
            .align_x(alignment::Horizontal::Center),
        )
        .padding([14, 22]);

        let inner = row![label, Space::new().width(22), empty_card]
            .align_y(alignment::Vertical::Center);

        container(
            row![
                Space::new().width(STAGE_HPAD),
                inner,
                Space::new().width(Length::Fill),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .width(Length::Fill)
        .height(NEXT_LANE_HEIGHT)
        .align_y(alignment::Vertical::Center)
        .style(band_bg(theme::BG_1))
        .into()
    }

    // -- Band 4: footer strip ------------------------------------------------

    /// Footer: instrument/tuning + capo placeholders on the left (the
    /// interactive controls land in todo #311) and the keyboard-hint line
    /// on the right. Returns owned (`'static`) content so it can live behind
    /// the lazy cache in [`Resonance::view_performance_shell`]; the active
    /// tuning + capo are read from the placeholder consts that also feed
    /// [`footer_fingerprint`].
    fn performance_footer(&self) -> Element<'static, Message> {
        // Instrument segmented placeholder (Guitar 6 active by default).
        let tunings = ["Guitar 6", "Guitar 8", "Bass 4", "Bass 5"];
        let mut seg = row![].spacing(0).align_y(alignment::Vertical::Center);
        for (i, t) in tunings.iter().enumerate() {
            seg = seg.push(segmented_cell(
                t,
                i == FOOTER_ACTIVE_TUNING,
                i + 1 == tunings.len(),
            ));
        }
        let seg = container(seg).style(segmented_frame);

        let capo = row![
            footer_label("Capo"),
            container(
                row![
                    stepper_glyph("\u{2013}"),
                    text(FOOTER_CAPO_FRETS.to_string())
                        .size(12)
                        .font(theme::MONO_FONT)
                        .color(theme::TEXT_1),
                    stepper_glyph("+"),
                ]
                .spacing(12)
                .align_y(alignment::Vertical::Center)
            )
            .padding([4, 12])
            .style(segmented_frame),
        ]
        .spacing(16)
        .align_y(alignment::Vertical::Center);

        let left = row![footer_label("Instrument"), seg, capo]
            .spacing(16)
            .align_y(alignment::Vertical::Center);

        let hints = key_hint_line();

        let inner = row![left, Space::new().width(Length::Fill), hints]
            .align_y(alignment::Vertical::Center);

        container(
            row![
                Space::new().width(CHROME_HPAD),
                inner,
                Space::new().width(CHROME_HPAD),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .width(Length::Fill)
        .height(FOOTER_HEIGHT)
        .align_y(alignment::Vertical::Center)
        .style(band_bg(theme::BG_0))
        .into()
    }
}

// -- Small stateless pieces --------------------------------------------------

/// The centred empty-state shown on the centre stage: a large em-dash with
/// guidance. Static (owns its content), so it sits behind the centre-stage
/// lazy cache; the live chord symbol + fingering diagram replace it in
/// todos #308/#310.
fn center_stage_content() -> Element<'static, Message> {
    column![
        text("\u{2014}")
            .size(200)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_4)
            .line_height(LineHeight::Relative(0.8)),
        Space::new().height(18),
        text("No chord under the playhead")
            .size(20)
            .color(theme::TEXT_2),
        Space::new().height(10),
        text("Place sections with a progression in Compose, then roll the transport")
            .size(13)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3),
    ]
    .align_x(alignment::Horizontal::Center)
    .into()
}

/// The lavender `PERFORMANCE` identity pill.
fn performance_pill<'a>() -> Element<'a, Message> {
    container(
        text("PERFORMANCE")
            .size(11)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::ACCENT_SOFT)
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([4, 11])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::ACCENT_DIM)),
        border: iced::Border {
            color: theme::ACCENT_LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// A bright telemetry value token (mono).
fn telemetry_value<'a>(value: String, color: iced::Color) -> Element<'a, Message> {
    text(value)
        .size(14)
        .font(theme::MONO_FONT)
        .color(color)
        .line_height(LineHeight::Relative(1.0))
        .into()
}

/// A recessed telemetry unit label (mono).
fn telemetry_label<'a>(label: &str) -> Element<'a, Message> {
    text(label.to_string())
        .size(13)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0))
        .into()
}

/// Footer group label (uppercase mono, recessed).
fn footer_label<'a>(label: &str) -> Element<'a, Message> {
    text(label.to_uppercase())
        .size(11)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0))
        .into()
}

/// One cell of the instrument segmented placeholder. `on` marks the active
/// tuning; `last` drops the trailing divider.
fn segmented_cell<'a>(label: &str, on: bool, last: bool) -> Element<'a, Message> {
    let color = if on { theme::TEXT_1 } else { theme::TEXT_2 };
    let bg = if on { theme::BG_3 } else { iced::Color::TRANSPARENT };
    container(
        text(label.to_string())
            .size(12)
            .font(theme::UI_FONT)
            .color(color)
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([6, 13])
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            color: theme::LINE_2,
            width: if last { 0.0 } else { 1.0 },
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// A static +/− glyph for the capo stepper placeholder.
fn stepper_glyph<'a>(glyph: &str) -> Element<'a, Message> {
    text(glyph.to_string())
        .size(14)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_2)
        .line_height(LineHeight::Relative(1.0))
        .into()
}

/// The keyboard-hint line on the footer's right edge.
fn key_hint_line<'a>() -> Element<'a, Message> {
    let mut hints = row![].spacing(8).align_y(alignment::Vertical::Center);
    let pairs = [
        ("Space", "play"),
        ("R", "record"),
        ("F", "fullscreen"),
        ("\u{238b}", "exit"),
    ];
    for (i, (key, action)) in pairs.iter().enumerate() {
        if i > 0 {
            hints = hints.push(
                text("\u{00b7}")
                    .size(11)
                    .font(theme::MONO_FONT)
                    .color(theme::TEXT_4),
            );
        }
        hints = hints.push(
            row![
                text(key.to_string())
                    .size(11)
                    .font(theme::MONO_FONT)
                    .color(theme::TEXT_2),
                text(action.to_string())
                    .size(11)
                    .font(theme::MONO_FONT)
                    .color(theme::TEXT_3),
            ]
            .spacing(4)
            .align_y(alignment::Vertical::Center),
        );
    }
    hints.into()
}

/// A 1px hairline separator between bands.
fn hairline<'a>() -> Element<'a, Message> {
    container(Space::new().height(1).width(Length::Fill))
        .width(Length::Fill)
        .height(1)
        .style(theme::separator_bg)
        .into()
}

// -- Container styles --------------------------------------------------------

/// Full-bleed Performance backdrop (the darkest app surface).
fn performance_backdrop(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(theme::BG_0)),
        ..Default::default()
    }
}

/// A flat band background of the given colour. The design prototype shows
/// subtle vertical gradients on the status/next bands, but the rest of the
/// app's chrome (e.g. the transport bar) is flat, so the bands stay flat
/// for visual consistency — the prototype is a design artifact, not the
/// shipped convention (see arch doc #152).
fn band_bg(color: iced::Color) -> impl Fn(&iced::Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(iced::Background::Color(color)),
        ..Default::default()
    }
}

/// Outline-only frame used by the footer's segmented + stepper placeholders.
fn segmented_frame(_theme: &iced::Theme) -> container::Style {
    container::Style {
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

// -- Formatting helpers ------------------------------------------------------

/// Render the status-bar clock as `bar . beat`, taking the first two
/// dot-separated segments of the transport's `bar.beat.subdiv` position
/// string. Falls back to the raw string if it isn't shaped as expected.
fn bar_beat(position: &str) -> String {
    let mut parts = position.split('.');
    match (parts.next(), parts.next()) {
        (Some(bar), Some(beat)) => format!("{bar} . {beat}"),
        _ => position.to_string(),
    }
}

/// Format the BPM for the telemetry readout: drop the decimal when whole
/// (90.0 -> "90"), otherwise keep one place (92.5 -> "92.5").
fn format_bpm(bpm: f32) -> String {
    if (bpm.fract()).abs() < f32::EPSILON {
        format!("{}", bpm.round() as i64)
    } else {
        format!("{bpm:.1}")
    }
}
