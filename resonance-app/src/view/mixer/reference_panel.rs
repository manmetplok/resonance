//! Mixer **Reference & A/B** right-rail (design doc #184/#198). A 360px
//! panel that auditions external mastered tracks against the mix. This
//! module lands the panel *container* and its state routing — the shell
//! header, the fully-built **Empty** drop-zone and **Analyzing** checklist
//! states, the **Error / Missing** cards (full-panel for a load that
//! created no entry, inline per-entry for a missing/errored reference),
//! and the populated **A/B controls** for *loaded* references — the
//! reference list, the Mix/Reference A/B switch, the waveform overview
//! (with playhead, marker ticks and click-to-scrub), loudness-match, and
//! the level trim.
//!
//! The reference plays through the monitor path only and is excluded from
//! every bounce / stem export; the panel is purely a monitoring surface.

use std::cell::Cell;

use iced::widget::{button, canvas, column, container, row, slider, text, Space};
use iced::{alignment, mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{ABSource, ReferenceAnalysisStage, ReferenceId};
use resonance_metering::MeterSnapshot;

use crate::message::*;
use crate::reference::{
    ReferenceEntry, ReferenceMarkerState, ReferenceMessage, ReferenceState, ReferenceStatus,
};
use crate::theme::{self, fa};
use crate::update::reference::REFERENCE_AUDIO_EXTENSIONS;
use crate::util::format_db;

/// Which body the panel container routes to, derived from
/// [`ReferenceState`]. The populated A/B controls for *loaded* references
/// are filled in by a later todo; missing/errored entries already render
/// inline within the populated body so the rest of the panel stays usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelState {
    /// No references loaded and no pending load.
    Empty,
    /// At least one reference is mid-analysis.
    Analyzing,
    /// One or more references exist (loaded, missing, or errored).
    Populated,
    /// A load failed with no entry to attach the notice to — surfaced from
    /// `last_error` while the slot is otherwise empty.
    Error,
}

fn classify(state: &ReferenceState) -> PanelState {
    // A load failure that created no entry takes the whole panel: there is
    // nothing else to show it against. A missing/errored *entry*, by
    // contrast, renders inline in `Populated` so any loaded references
    // alongside it stay usable (design doc #198).
    if state.entries.is_empty() {
        if state.last_error.is_some() {
            return PanelState::Error;
        }
        if state.pending_loads.is_empty() {
            return PanelState::Empty;
        }
        // A load is dispatched but no analysis event has arrived yet.
        return PanelState::Populated;
    }
    if state
        .entries
        .iter()
        .any(|e| matches!(e.status, ReferenceStatus::Analyzing(_)))
    {
        return PanelState::Analyzing;
    }
    PanelState::Populated
}

pub(super) fn view(r: &crate::Resonance) -> Element<'_, Message> {
    let body: Element<'_, Message> = match classify(&r.reference) {
        PanelState::Empty => empty_body(),
        PanelState::Analyzing => analyzing_body(&r.reference),
        PanelState::Populated => populated_body(&r.reference),
        PanelState::Error => error_body(&r.reference),
    };

    let content = column![header(), Space::new().height(18), body].spacing(0);

    container(content)
        .width(Length::Fixed(theme::REFERENCE_PANEL_WIDTH))
        .height(Length::Fill)
        .padding(theme::RAIL_PADDING)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            ..Default::default()
        })
        .into()
}

/// Panel title row: label + a close (×) button that re-toggles the rail.
fn header() -> Element<'static, Message> {
    let close = button(text("\u{00d7}").size(15).color(theme::TEXT_3))
        .on_press(Message::Ui(UiMessage::ToggleReferencePanel))
        .padding([1, 7])
        .style(|_theme, status| theme::small_button_style(status));

    row![
        column![
            text("REFERENCE & A/B")
                .size(10)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::new().height(2),
            text("Monitor only — not in exports")
                .size(11)
                .color(theme::TEXT_2),
        ]
        .spacing(0),
        Space::new().width(Length::Fill),
        close,
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}

// ---------------------------------------------------------------------------
// Empty — dashed drop zone, format chips, "Add reference…", exclusion badge.
// ---------------------------------------------------------------------------

fn empty_body() -> Element<'static, Message> {
    let drop_zone = container(
        column![
            text(fa::MUSIC.to_string())
                .font(theme::ICON_FONT)
                .size(22)
                .color(theme::TEXT_4),
            Space::new().height(12),
            text("Drop an audio file to compare")
                .size(13)
                .color(theme::TEXT_2),
            Space::new().height(4),
            text("or pick one below")
                .size(11)
                .color(theme::TEXT_3),
        ]
        .spacing(0)
        .align_x(alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .padding([34, 16])
    .center_x(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    });

    // Format chips, one per accepted container extension.
    let mut chips = row![].spacing(6);
    for ext in REFERENCE_AUDIO_EXTENSIONS {
        chips = chips.push(format_chip(ext));
    }
    let chips = container(chips).center_x(Length::Fill);

    // Primary CTA for the Empty state — a filled lavender action button,
    // not a toggle. (The chrome REF button is the genuine toggle.)
    let add_btn = button(
        text("Add reference\u{2026}")
            .size(12)
            .font(theme::UI_FONT_MEDIUM),
    )
    .on_press(Message::Reference(crate::reference::ReferenceMessage::PickFile))
    .width(Length::Fill)
    .padding([9, 12])
    .style(|_theme, status| theme::primary_button_style(status));

    column![
        drop_zone,
        Space::new().height(14),
        chips,
        Space::new().height(16),
        add_btn,
        Space::new().height(14),
        container(exclusion_badge()).center_x(Length::Fill),
    ]
    .spacing(0)
    .into()
}

/// The "not in exports" reassurance pill. A reference is a monitoring-only
/// surface, never bounced or stem-exported; this badge says so wherever the
/// panel needs to reassure the user (Empty state today, per design doc #198).
fn exclusion_badge() -> Element<'static, Message> {
    container(
        row![
            text(fa::EYE.to_string())
                .font(theme::ICON_FONT)
                .size(9)
                .color(theme::GOOD),
            text("Not included in exports")
                .size(10)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_2),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
    )
    .padding([4, 10])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    })
    .into()
}

fn format_chip(label: &str) -> Element<'static, Message> {
    container(
        text(label.to_uppercase())
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
    )
    .padding([3, 8])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Analyzing — the 4-stage offline-analysis checklist + determinate progress
// bar + Cancel, driven by `ReferenceAnalysisProgress` events. Populated /
// Error remain placeholder bodies whose detail UI lands in later todos.
// ---------------------------------------------------------------------------

/// The four offline-analysis stages, in the order the engine reports them,
/// paired with the user-facing label shown in the checklist.
const ANALYSIS_STAGES: [(ReferenceAnalysisStage, &str); 4] = [
    (ReferenceAnalysisStage::Decoding, "Decoding audio"),
    (ReferenceAnalysisStage::MeasuringLufs, "Measuring loudness"),
    (ReferenceAnalysisStage::BuildingPeaks, "Building waveform"),
    (ReferenceAnalysisStage::ComputingOffset, "Matching loudness"),
];

/// Position of `stage` within [`ANALYSIS_STAGES`] (0-based). Stages strictly
/// before the current one are complete; the current one is in progress; later
/// ones are pending.
fn stage_index(stage: ReferenceAnalysisStage) -> usize {
    ANALYSIS_STAGES
        .iter()
        .position(|(s, _)| *s == stage)
        .unwrap_or(0)
}

fn analyzing_body(state: &ReferenceState) -> Element<'_, Message> {
    // The first analysing entry drives the panel — the Analyzing state is
    // the first-load experience, and `classify` only routes here when one
    // exists. Fall back gracefully if it has just flipped to Loaded.
    let Some(entry) = state
        .entries
        .iter()
        .find(|e| matches!(e.status, ReferenceStatus::Analyzing(_)))
    else {
        return placeholder_card("Analyzing reference\u{2026}", theme::TEXT_2);
    };
    let ReferenceStatus::Analyzing(stage) = entry.status else {
        return placeholder_card("Analyzing reference\u{2026}", theme::TEXT_2);
    };
    let current = stage_index(stage);

    let title = if entry.name.is_empty() {
        "Analyzing reference\u{2026}".to_string()
    } else {
        format!("Analyzing {}\u{2026}", entry.name)
    };

    let mut checklist = column![].spacing(10);
    for (i, (_, label)) in ANALYSIS_STAGES.iter().enumerate() {
        checklist = checklist.push(stage_row(label, i, current));
    }

    let cancel = button(text("Cancel").size(12).font(theme::UI_FONT_MEDIUM))
        .on_press(Message::Reference(
            crate::reference::ReferenceMessage::Remove(entry.id),
        ))
        .padding([7, 14])
        .style(|_theme, status| theme::small_button_style(status));

    container(
        column![
            text(title)
                .size(13)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_1),
            Space::new().height(14),
            progress_bar(current),
            Space::new().height(16),
            checklist,
            Space::new().height(18),
            row![Space::new().width(Length::Fill), cancel],
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding([16, 16])
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

/// One checklist row: a status glyph (done / in-progress / pending) and the
/// stage label, coloured to match its state.
fn stage_row(label: &str, index: usize, current: usize) -> Element<'static, Message> {
    let (glyph, glyph_color, text_color) = if index < current {
        (fa::CIRCLE, theme::GOOD, theme::TEXT_2)
    } else if index == current {
        (fa::BULLSEYE, theme::ACCENT, theme::TEXT_1)
    } else {
        (fa::CIRCLE_HOLLOW, theme::TEXT_4, theme::TEXT_3)
    };

    row![
        text(glyph.to_string())
            .font(theme::ICON_FONT)
            .size(11)
            .color(glyph_color),
        text(label.to_string()).size(12).color(text_color),
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// Determinate progress track filled to the current stage. The fill grows a
/// quarter per stage (Decoding → ¼ … ComputingOffset → 4/4), giving the user
/// a sense of forward motion through the four-step analysis.
fn progress_bar(current: usize) -> Element<'static, Message> {
    let done = (current + 1) as u16;
    let remaining = ANALYSIS_STAGES.len() as u16 - done;

    let fill = container(Space::new())
        .width(Length::FillPortion(done))
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::ACCENT)),
            border: iced::Border {
                radius: theme::RADIUS_XS.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    let track = row![fill, Space::new().width(Length::FillPortion(remaining))];

    container(track)
        .width(Length::Fill)
        .height(Length::Fixed(5.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                radius: theme::RADIUS_XS.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

// ---------------------------------------------------------------------------
// Populated — the loaded-reference experience: a selectable reference list,
// then (for the active reference) the A/B switch, waveform, loudness-match
// and trim controls. Missing / errored entries render inline as BAD cards so
// the rest of the panel stays usable.
// ---------------------------------------------------------------------------

fn populated_body(state: &ReferenceState) -> Element<'_, Message> {
    let mut col = column![reference_list(state)].spacing(14);

    // The A/B detail controls operate on the *active* reference, and only
    // once it has finished analysing (a waveform + loudness to show).
    if let Some(entry) = state
        .active_id
        .and_then(|id| state.entries.iter().find(|e| e.id == id))
        .filter(|e| matches!(e.status, ReferenceStatus::Loaded))
    {
        col = col.push(ab_controls(state, entry));
    } else if state
        .entries
        .iter()
        .any(|e| matches!(e.status, ReferenceStatus::Loaded))
    {
        col = col.push(
            text("Select a reference above to compare")
                .size(11)
                .color(theme::TEXT_3),
        );
    }

    // The comparative loudness readout lives at the bottom of every
    // populated state (design doc #198), driven by the latest A/B meter
    // snapshot. The reference column reads "—" until a reference is active
    // and metered.
    if state.entries.iter().any(|e| matches!(e.status, ReferenceStatus::Loaded)) {
        col = col.push(loudness_readout(state));
    }

    col.into()
}

// ---------------------------------------------------------------------------
// Comparative loudness readout — a dual mix/reference loudness-bar pair on a
// shared LUFS scale with a target line, above a Mix / Ref / Δ table of the
// integrated, short-term and momentary LUFS, the true-peak max and the LRA.
// Driven by the `PollABMeters` snapshot (`ReferenceState::ab_meter`); the
// reference column shows "—" whenever no reference is metered.
// ---------------------------------------------------------------------------

/// Loudness-scale floor / ceiling (LUFS) the bar pair maps onto, and the
/// loudness target the reference line marks. -14 LUFS is the common
/// streaming integrated target the user is mixing toward.
const LUFS_FLOOR: f32 = -36.0;
const LUFS_CEIL: f32 = 0.0;
const LUFS_TARGET: f32 = -14.0;

/// A Δ at or beyond this magnitude (LU / dB) is "meaningful" and lights the
/// Δ cell amber so a real divergence between mix and reference stands out.
const DELTA_HIGHLIGHT: f32 = 1.0;

fn loudness_readout(state: &ReferenceState) -> Element<'static, Message> {
    let mix = state.ab_meter.map(|m| m.mix);
    let reference = state.ab_meter.and_then(|m| m.reference);

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

/// Fixed width of each numeric column, and the gap between them, so the
/// header captions line up exactly over the Mix / Ref / Δ values below.
const VALUE_COL_WIDTH: f32 = 52.0;
const VALUE_COL_SPACING: f32 = 6.0;

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
fn format_lufs(value: Option<f32>) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{v:.1}"),
        _ => "\u{2014}".to_string(),
    }
}

/// The dual mix/reference loudness-bar Canvas: two horizontal bars on a
/// shared LUFS scale (mix lavender, reference amber) with a target line at
/// [`LUFS_TARGET`]. A live visual, so a `canvas::Cache` keeps the geometry
/// across hover/resize repaints and only re-rasterises when a level moves.
fn loudness_bars(mix: Option<MeterSnapshot>, reference: Option<MeterSnapshot>) -> Element<'static, Message> {
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

/// The loaded-reference list. Each loaded entry is a selectable row (name +
/// integrated loudness + a remove ×); the active one is lavender-lit.
/// Missing / errored entries keep their inline BAD card so they stay
/// actionable without hiding the references that did load.
fn reference_list(state: &ReferenceState) -> Element<'_, Message> {
    let mut list = column![].spacing(6);
    for entry in &state.entries {
        let row = match &entry.status {
            ReferenceStatus::Missing => error_card(entry, None),
            ReferenceStatus::Error(reason) => error_card(entry, Some(reason)),
            _ => reference_row(entry, state.active_id == Some(entry.id)),
        };
        list = list.push(row);
    }
    list.into()
}

/// One selectable row for a loaded reference: the clickable name + loudness
/// block (selects it active) sits beside a remove (×) button.
fn reference_row(entry: &ReferenceEntry, active: bool) -> Element<'_, Message> {
    let lufs = if entry.integrated_lufs.is_finite() {
        format!("{:.1} LUFS", entry.integrated_lufs)
    } else {
        "— LUFS".to_string()
    };

    let info = button(
        column![
            text(entry.name.clone())
                .size(12)
                .font(theme::UI_FONT_MEDIUM)
                .color(if active { theme::ACCENT_SOFT } else { theme::TEXT_1 }),
            Space::new().height(1),
            text(lufs).size(10).color(theme::TEXT_3),
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding([7, 10])
    .on_press(Message::Reference(ReferenceMessage::SetActive(entry.id)))
    .style(move |_theme, status| select_row_style(active, status));

    let remove = button(text("\u{00d7}").size(14).color(theme::TEXT_3))
        .on_press(Message::Reference(ReferenceMessage::Remove(entry.id)))
        .padding([1, 8])
        .style(|_theme, status| theme::small_button_style(status));

    row![info, remove]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
        .into()
}

/// Selected-row chrome: a lavender wash + border when active, a hairline
/// card otherwise (hover-lit).
fn select_row_style(active: bool, status: button::Status) -> button::Style {
    let (bg, border) = if active {
        (theme::ACCENT_DIM, theme::ACCENT_LINE)
    } else {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => theme::BG_3,
            _ => theme::BG_2,
        };
        (bg, theme::LINE_2)
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::TEXT_1,
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

/// The A/B control stack for the active reference: the Mix/Reference
/// switch, the waveform overview, the marker + loop row, the
/// loudness-match toggle and the level trim.
fn ab_controls<'a>(state: &'a ReferenceState, entry: &'a ReferenceEntry) -> Element<'a, Message> {
    container(
        column![
            ab_switch(state.ab_source),
            Space::new().height(14),
            waveform(entry),
            Space::new().height(10),
            marker_row(state, entry),
            Space::new().height(14),
            loudness_row(state),
            Space::new().height(12),
            trim_row(state.trim_db),
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding([16, 16])
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

/// Two-segment A/B switch. **A · Mix** lights lavender (ACCENT) and
/// **B · Reference** lights amber (WARM); pressing a segment selects that
/// source outright. A "hold B" hint nods to the momentary key.
fn ab_switch(source: ABSource) -> Element<'static, Message> {
    let seg = |label: &str, on: bool, color: Color, set: ABSource| {
        button(
            text(label.to_string())
                .size(12)
                .font(theme::UI_FONT_MEDIUM),
        )
        .width(Length::Fill)
        .padding([8, 12])
        .on_press(Message::Reference(ReferenceMessage::SetAbSource(set)))
        .style(move |_theme, status| ab_segment_style(on, color, status))
    };

    column![
        row![
            seg("A \u{00b7} Mix", source == ABSource::Mix, theme::ACCENT, ABSource::Mix),
            seg(
                "B \u{00b7} Reference",
                source == ABSource::Reference,
                theme::WARM,
                ABSource::Reference,
            ),
        ]
        .spacing(8),
        Space::new().height(6),
        text("Hold B to monitor the reference")
            .size(10)
            .color(theme::TEXT_3),
    ]
    .spacing(0)
    .into()
}

/// Style for one A/B segment. Active lights with a tint of its source
/// colour (lavender Mix / amber Reference) like [`theme::toggle_button_style`];
/// inactive keeps a visible BG_2 + hairline card so the control always reads
/// as two segments rather than one floating button.
fn ab_segment_style(active: bool, color: Color, status: button::Status) -> button::Style {
    let (bg, text_color, border_color) = if active {
        let a = match status {
            button::Status::Hovered => 0.22,
            button::Status::Pressed => 0.30,
            _ => 0.16,
        };
        (Color { a, ..color }, color, color)
    } else {
        let bg = match status {
            button::Status::Hovered | button::Status::Pressed => theme::BG_3,
            _ => theme::BG_2,
        };
        (bg, theme::TEXT_2, theme::LINE_2)
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color,
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// The waveform overview Canvas (peaks + playhead + marker ticks +
/// click-to-scrub) at a fixed height.
fn waveform(entry: &ReferenceEntry) -> Element<'_, Message> {
    canvas(ReferenceWaveform {
        peaks: &entry.waveform_peaks,
        position_samples: entry.position_samples,
        length_samples: entry.length_samples,
        markers: &entry.markers,
        ref_id: entry.id,
    })
    .width(Length::Fill)
    .height(Length::Fixed(72.0))
    .into()
}

/// The marker + loop affordances under the waveform: an "Add marker"
/// button (drops one at the current cursor), the loop-to-mix chip, then a
/// wrap of removable marker chips.
fn marker_row<'a>(state: &'a ReferenceState, entry: &'a ReferenceEntry) -> Element<'a, Message> {
    let add = button(text("Add marker").size(11).font(theme::UI_FONT_MEDIUM))
        .padding([6, 10])
        .on_press(Message::Reference(ReferenceMessage::AddMarker {
            ref_id: entry.id,
            position_samples: entry.position_samples,
            label: format!("Marker {}", entry.markers.len() + 1),
        }))
        .style(|_theme, status| theme::small_button_style(status));

    let loop_chip = button(text("Loop to mix").size(11).font(theme::UI_FONT_MEDIUM))
        .padding([6, 10])
        .on_press(Message::Reference(ReferenceMessage::ToggleLoopToMix))
        .style(move |_theme, status| {
            theme::toggle_button_style(state.loop_to_mix, theme::ACCENT, true, status)
        });

    let mut chips = row![].spacing(6).align_y(alignment::Vertical::Center);
    for mk in &entry.markers {
        chips = chips.push(marker_chip(entry.id, mk));
    }

    column![
        row![add, loop_chip, Space::new().width(Length::Fill)].spacing(8),
        Space::new().height(if entry.markers.is_empty() { 0 } else { 8 }),
        chips,
    ]
    .spacing(0)
    .into()
}

/// One removable marker chip: its label and a × that removes it.
fn marker_chip(ref_id: ReferenceId, mk: &ReferenceMarkerState) -> Element<'static, Message> {
    let remove = button(text("\u{00d7}").size(12).color(theme::TEXT_2))
        .on_press(Message::Reference(ReferenceMessage::RemoveMarker {
            ref_id,
            marker_id: mk.id,
        }))
        .padding([1, 5])
        .style(|_theme, status| theme::small_button_style(status));

    container(
        row![
            text(mk.label.clone()).size(10).color(theme::TEXT_2),
            remove,
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center),
    )
    .padding([2, 4])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_3)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    })
    .into()
}

/// The loudness-match toggle, with the engine-reported gain offset shown
/// alongside once matching is engaged.
fn loudness_row(state: &ReferenceState) -> Element<'static, Message> {
    let matched = state.loudness_match;
    let offset_db = state.offset_db;

    let toggle = button(
        text("Match loudness")
            .size(12)
            .font(theme::UI_FONT_MEDIUM),
    )
    .padding([7, 12])
    .on_press(Message::Reference(ReferenceMessage::ToggleLoudnessMatch))
    .style(move |_theme, status| {
        theme::toggle_button_style(matched, theme::GOOD, true, status)
    });

    let offset = if matched {
        text(format!("{:+.1} dB", offset_db))
            .size(11)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_2)
    } else {
        text("off").size(11).color(theme::TEXT_3)
    };

    row![toggle, Space::new().width(Length::Fill), offset]
        .align_y(alignment::Vertical::Center)
        .into()
}

/// The reference level trim: a ±12 dB slider with a monospace readout.
fn trim_row(trim_db: f32) -> Element<'static, Message> {
    let trim = slider(-12.0..=12.0f32, trim_db, |v| {
        Message::Reference(ReferenceMessage::TrimChanged(v))
    })
    .step(0.1);

    column![
        row![
            text("Trim").size(11).color(theme::TEXT_3),
            Space::new().width(Length::Fill),
            text(format_db(trim_db))
                .size(11)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_2),
        ]
        .align_y(alignment::Vertical::Center),
        Space::new().height(6),
        trim,
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// Waveform Canvas — the reference's downsampled overview with a playhead, the
// comparison-marker ticks, and click-to-scrub. A live visual per the view
// performance rules: a `canvas::Cache` keeps the geometry across hover /
// resize redraws and only re-rasterises when the inputs actually change.
// ---------------------------------------------------------------------------

struct ReferenceWaveform<'a> {
    peaks: &'a [(f32, f32)],
    position_samples: u64,
    length_samples: u64,
    markers: &'a [ReferenceMarkerState],
    ref_id: ReferenceId,
}

#[derive(Default)]
struct WaveformState {
    cache: canvas::Cache,
    fingerprint: Cell<u64>,
}

impl ReferenceWaveform<'_> {
    /// Fraction `[0, 1]` of a sample position along the overview. Returns
    /// `0` when the length is unknown so nothing is drawn off-canvas.
    fn fraction(&self, sample: u64) -> f32 {
        if self.length_samples == 0 {
            0.0
        } else {
            (sample as f32 / self.length_samples as f32).clamp(0.0, 1.0)
        }
    }

    /// A cheap order-sensitive hash of everything that affects the drawn
    /// pixels, so the cache invalidates on a real change but survives a
    /// pure hover / resize repaint.
    fn fingerprint(&self) -> u64 {
        let mut h: u64 = 1469598103934665603; // FNV-1a offset basis
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(1099511628211);
        };
        mix(self.peaks.len() as u64);
        mix(self.position_samples);
        mix(self.length_samples);
        mix(self.ref_id.0 as u64);
        for mk in self.markers {
            mix(mk.position_samples);
        }
        h
    }
}

impl canvas::Program<Message> for ReferenceWaveform<'_> {
    type State = WaveformState;

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        // Click anywhere on the overview scrubs the reference cursor to the
        // matching sample. Needs a known length to map x → samples.
        if self.length_samples == 0 {
            return None;
        }
        if let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            if let Some(pos) = cursor.position_in(bounds) {
                let fraction = (pos.x / bounds.width).clamp(0.0, 1.0);
                let position_samples = (fraction * self.length_samples as f32) as u64;
                return Some(
                    canvas::Action::publish(Message::Reference(ReferenceMessage::Scrub {
                        ref_id: self.ref_id,
                        position_samples,
                    }))
                    .and_capture(),
                );
            }
        }
        None
    }

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
            let mid = h / 2.0;

            // Backdrop.
            frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), theme::BG_3);
            // Zero-amplitude centre line.
            frame.fill_rectangle(Point::new(0.0, mid - 0.5), Size::new(w, 1.0), theme::LINE_2);

            // Peak columns over the mono (min, max) overview.
            if !self.peaks.is_empty() {
                let col_w = w / self.peaks.len() as f32;
                let bar_w = col_w.max(1.0);
                for (i, (min, max)) in self.peaks.iter().enumerate() {
                    let x = i as f32 * col_w;
                    let top = mid - max.clamp(-1.0, 1.0) * mid;
                    let bottom = mid - min.clamp(-1.0, 1.0) * mid;
                    let bar_h = (bottom - top).max(1.0);
                    frame.fill_rectangle(
                        Point::new(x, top),
                        Size::new(bar_w, bar_h),
                        theme::TEXT_2,
                    );
                }
            }

            // Marker ticks — a thin amber line per comparison marker.
            for mk in self.markers {
                let x = self.fraction(mk.position_samples) * w;
                frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, h), theme::WARM_LINE);
            }

            // Playhead — the reference's own cursor.
            let px = self.fraction(self.position_samples) * w;
            frame.fill_rectangle(Point::new(px - 0.75, 0.0), Size::new(1.5, h), theme::ACCENT);
        });
        vec![geometry]
    }
}

/// Full-panel error body for a load that failed before any entry existed
/// (so the notice lives in `last_error`, not on an entry). Reached only
/// while the slot is otherwise empty; a missing/errored entry alongside
/// loaded references renders inline via [`error_card`] instead.
fn error_body(state: &ReferenceState) -> Element<'_, Message> {
    let reason = state
        .last_error
        .clone()
        .unwrap_or_else(|| "Reference failed to load".to_string());

    bad_card(
        column![
            error_heading("Couldn\u{2019}t load reference"),
            Space::new().height(6),
            text(reason).size(12).color(theme::TEXT_2),
            Space::new().height(14),
            // No entry to drop, so Dismiss just clears the notice.
            error_actions(Message::Reference(
                crate::reference::ReferenceMessage::DismissError,
            )),
        ]
        .spacing(0),
    )
}

/// A BAD-tinted card for one missing or errored reference entry. `reason`
/// is `Some` for an [`ReferenceStatus::Error`] (the analysis failure text)
/// and `None` for an [`ReferenceStatus::Missing`] entry (file gone since
/// the project was saved). Dismiss drops just this entry; Choose another
/// re-opens the picker. Other entries are untouched.
fn error_card<'a>(entry: &'a ReferenceEntry, reason: Option<&'a str>) -> Element<'a, Message> {
    let name = if entry.name.is_empty() {
        std::path::Path::new(&entry.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Reference")
            .to_string()
    } else {
        entry.name.clone()
    };

    let detail = reason
        .map(str::to_string)
        .unwrap_or_else(|| "File not found".to_string());

    let mut body = column![
        error_heading(&name),
        Space::new().height(4),
        text(detail).size(11).color(theme::TEXT_2),
    ]
    .spacing(0);

    // For a missing file, show the path so the user can tell which one.
    if reason.is_none() && !entry.path.is_empty() {
        body = body.push(Space::new().height(2));
        body = body.push(text(entry.path.clone()).size(10).color(theme::TEXT_3));
    }

    body = body.push(Space::new().height(14));
    body = body.push(error_actions(Message::Reference(
        crate::reference::ReferenceMessage::Remove(entry.id),
    )));

    bad_card(body)
}

/// Heading row for an error card: a BAD-tinted info glyph + the title.
fn error_heading(title: &str) -> Element<'static, Message> {
    row![
        text(fa::CIRCLE_INFO.to_string())
            .font(theme::ICON_FONT)
            .size(12)
            .color(theme::BAD),
        text(title.to_string())
            .size(13)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// The two error actions: a `dismiss` button (whose message drops the
/// failed entry or clears the notice) and a "Choose another…" button that
/// re-opens the file picker.
fn error_actions(dismiss_msg: Message) -> Element<'static, Message> {
    let dismiss = button(text("Dismiss").size(12).font(theme::UI_FONT_MEDIUM))
        .on_press(dismiss_msg)
        .padding([7, 14])
        .style(|_theme, status| theme::small_button_style(status));

    let choose = button(
        text("Choose another\u{2026}")
            .size(12)
            .font(theme::UI_FONT_MEDIUM),
    )
    .on_press(Message::Reference(
        crate::reference::ReferenceMessage::PickFile,
    ))
    .padding([7, 14])
    .style(|_theme, status| theme::small_button_style(status));

    row![dismiss, choose].spacing(8).into()
}

/// Wrap `content` in the shared BAD-tinted card chrome (faint red wash +
/// BAD border) used by both the full-panel error body and per-entry cards.
fn bad_card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding([16, 16])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(iced::Color {
                a: 0.08,
                ..theme::BAD
            })),
            border: iced::Border {
                color: iced::Color {
                    a: 0.5,
                    ..theme::BAD
                },
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

fn placeholder_card(label: &str, color: iced::Color) -> Element<'static, Message> {
    container(text(label.to_string()).size(12).color(color))
        .width(Length::Fill)
        .padding([12, 14])
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
