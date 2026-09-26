//! Comparative loudness readout — dual mix/reference loudness-bar pair on a
//! shared LUFS scale with a target line, above a Mix / Ref / Δ table of the
//! integrated, short-term and momentary LUFS, the true-peak max and the LRA.
//! Driven by the `PollABMeters` snapshot (`ReferenceState::ab_meter`); the
//! reference column shows "—" whenever no reference is metered.

use std::cell::Cell;

use iced::widget::{canvas, column, container, row, text, Space};
use iced::{alignment, mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_metering::MeterSnapshot;

use crate::message::*;
use crate::reference::ReferenceState;
use crate::theme;

/// Loudness-scale floor / ceiling (LUFS) the bar pair maps onto, and the
/// loudness target the reference line marks. -14 LUFS is the common
/// streaming integrated target the user is mixing toward.
pub(super) const LUFS_FLOOR: f32 = -36.0;
pub(super) const LUFS_CEIL: f32 = 0.0;
pub(super) const LUFS_TARGET: f32 = -14.0;

/// A Δ at or beyond this magnitude (LU / dB) is "meaningful" and lights the
/// Δ cell amber so a real divergence between mix and reference stands out.
const DELTA_HIGHLIGHT: f32 = 1.0;

/// Fixed width of each numeric column, and the gap between them, so the
/// header captions line up exactly over the Mix / Ref / Δ values below.
const VALUE_COL_WIDTH: f32 = 52.0;
const VALUE_COL_SPACING: f32 = 6.0;

pub(super) fn loudness_readout(state: &ReferenceState) -> Element<'static, Message> {
    let mix = state.monitor.ab_meter.map(|m| m.mix);
    let reference = state.monitor.ab_meter.and_then(|m| m.reference);

    container(
        column![
            text("LOUDNESS")
                .size(10)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::new().height(10),
            loudness_bars(mix, reference),
            Space::new().height(12),
            readout_header(),
            Space::new().height(6),
            metric_row("Integrated", mix.map(|m| m.integrated_lufs), reference.map(|m| m.integrated_lufs), false),
            metric_row("Short-term", mix.map(|m| m.short_term_lufs), reference.map(|m| m.short_term_lufs), false),
            metric_row("Momentary", mix.map(|m| m.momentary_lufs), reference.map(|m| m.momentary_lufs), false),
            metric_row("True-peak", mix.map(|m| m.true_peak_max_dbtp), reference.map(|m| m.true_peak_max_dbtp), true),
            metric_row("LRA", mix.map(|m| m.lra_lu), reference.map(|m| m.lra_lu), false),
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding([14, 16])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    })
    .into()
}

/// The column header: a spacer over the metric labels, then the Mix / Ref /
/// Δ column captions, each right-aligned over its value column.
fn readout_header() -> Element<'static, Message> {
    let cap = |label: &str, color: Color| {
        container(
            text(label.to_string())
                .size(9)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(color),
        )
        .width(Length::Fixed(VALUE_COL_WIDTH))
        .align_x(alignment::Horizontal::Right)
    };

    row![
        Space::new().width(Length::Fill),
        cap("MIX", theme::ACCENT_SOFT),
        cap("REF", theme::WARM),
        cap("\u{0394}", theme::TEXT_3),
    ]
    .spacing(VALUE_COL_SPACING)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// One metric row: a label, then the mix / reference / Δ values. `mix` and
/// `reference` are `None` when unmetered (rendered as "—"); `is_dbtp` flags
/// the true-peak row so values above 0 dBTP turn BAD pink and the Δ is read
/// in dB rather than LU. The Δ lights amber once it crosses
/// [`DELTA_HIGHLIGHT`].
fn metric_row(
    label: &str,
    mix: Option<f32>,
    reference: Option<f32>,
    is_dbtp: bool,
) -> Element<'static, Message> {
    let mix_color = if is_dbtp && mix.is_some_and(|v| v > 0.0) {
        theme::BAD
    } else {
        theme::TEXT_1
    };
    let ref_color = if is_dbtp && reference.is_some_and(|v| v > 0.0) {
        theme::BAD
    } else {
        theme::TEXT_2
    };

    let delta = match (mix, reference) {
        (Some(m), Some(r)) if m.is_finite() && r.is_finite() => Some(m - r),
        _ => None,
    };
    let (delta_text, delta_color) = match delta {
        Some(d) => {
            let color = if d.abs() >= DELTA_HIGHLIGHT {
                theme::WARM
            } else {
                theme::TEXT_3
            };
            (format!("{d:+.1}"), color)
        }
        None => ("\u{2014}".to_string(), theme::TEXT_4),
    };

    row![
        text(label.to_string()).size(11).color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        value_cell(format_lufs(mix), mix_color),
        value_cell(format_lufs(reference), ref_color),
        value_cell(delta_text, delta_color),
    ]
    .spacing(VALUE_COL_SPACING)
    .align_y(alignment::Vertical::Center)
    .padding([2, 0])
    .into()
}

/// One right-aligned monospace value cell of the readout table.
fn value_cell(value: String, color: Color) -> Element<'static, Message> {
    container(text(value).size(11).font(theme::MONO_FONT).color(color))
        .width(Length::Fixed(VALUE_COL_WIDTH))
        .align_x(alignment::Horizontal::Right)
        .into()
}

/// Format a metered value for a table cell: one decimal, or "—" when the
/// value is absent or non-finite (e.g. `NEG_INFINITY` LUFS before any audio
/// has been measured).
pub(super) fn format_lufs(value: Option<f32>) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{v:.1}"),
        _ => "\u{2014}".to_string(),
    }
}

/// The dual mix/reference loudness-bar Canvas: two horizontal bars on a
/// shared LUFS scale (mix lavender, reference amber) with a target line at
/// [`LUFS_TARGET`]. A live visual, so a `canvas::Cache` keeps the geometry
/// across hover/resize repaints and only re-rasterises when a level moves.
pub(super) fn loudness_bars(mix: Option<MeterSnapshot>, reference: Option<MeterSnapshot>) -> Element<'static, Message> {
    canvas(LoudnessBars {
        mix_lufs: mix.map(|m| m.integrated_lufs),
        ref_lufs: reference.map(|m| m.integrated_lufs),
    })
    .width(Length::Fill)
    .height(Length::Fixed(36.0))
    .into()
}

struct LoudnessBars {
    mix_lufs: Option<f32>,
    ref_lufs: Option<f32>,
}

#[derive(Default)]
struct LoudnessBarsState {
    cache: canvas::Cache,
    fingerprint: Cell<u64>,
}

impl LoudnessBars {
    /// Fraction `[0, 1]` of a LUFS value along the shared bar scale.
    /// Non-finite / absent values map to `0` so the bar simply reads empty.
    fn fraction(lufs: Option<f32>) -> f32 {
        match lufs {
            Some(v) if v.is_finite() => {
                ((v - LUFS_FLOOR) / (LUFS_CEIL - LUFS_FLOOR)).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    /// Order-sensitive hash of the drawn levels so the cache invalidates on
    /// a real move but survives a pure hover/resize repaint. Quantised to
    /// 0.1 LUFS — finer changes wouldn't move a pixel on this bar.
    fn fingerprint(&self) -> u64 {
        let mut h: u64 = 1469598103934665603; // FNV-1a offset basis
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(1099511628211);
        };
        let quant = |lufs: Option<f32>| match lufs {
            Some(v) if v.is_finite() => (v * 10.0) as i64 as u64,
            _ => u64::MAX,
        };
        mix(quant(self.mix_lufs));
        mix(quant(self.ref_lufs));
        h
    }
}

impl canvas::Program<Message> for LoudnessBars {
    type State = LoudnessBarsState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let fp = self.fingerprint();
        if state.fingerprint.get() != fp {
            state.cache.clear();
            state.fingerprint.set(fp);
        }
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let w = bounds.width;
            let h = bounds.height;
            let bar_h = 14.0;
            let gap = h - bar_h * 2.0;

            let mut bar = |y: f32, lufs: Option<f32>, color: Color| {
                // Track.
                frame.fill_rectangle(Point::new(0.0, y), Size::new(w, bar_h), theme::BG_3);
                // Fill to the metered level.
                let fw = Self::fraction(lufs) * w;
                if fw > 0.0 {
                    frame.fill_rectangle(Point::new(0.0, y), Size::new(fw, bar_h), color);
                }
            };
            bar(0.0, self.mix_lufs, theme::ACCENT);
            bar(bar_h + gap, self.ref_lufs, theme::WARM);

            // Shared target line across both bars.
            let tx = Self::fraction(Some(LUFS_TARGET)) * w;
            frame.fill_rectangle(Point::new(tx - 0.5, 0.0), Size::new(1.0, h), theme::TEXT_2);
        });
        vec![geometry]
    }
}
