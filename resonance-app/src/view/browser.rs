//! Docked media-browser panel for the Arrange view (design doc #175,
//! epic #35).
//!
//! The container scaffold landed in todo #601: a fixed-width left column
//! ([`theme::BROWSER_WIDTH`], `BG_2` fill, `LINE` right border), its header,
//! and the Files / Pool tab switcher. This file now also holds the **Pool tab
//! body** (todo #603): a scrollable list of the project's imported audio
//! assets, each annotated with a usage badge (`used ×N` / `unused`) and —
//! for assets whose backing file is missing — an inline `relink` chip.
//!
//! Pool tab row anatomy (left → right):
//!
//! * **Type glyph** — ♪ (WARM) or ⚠ (BAD for missing).
//! * **Mini waveform thumbnail** — a 48 × 26 px Canvas bar chart of the
//!   asset's downsampled `thumbnail_peaks`.
//! * **Name** — the original filename without its directory path.
//! * **Format chip** — `wav` / `flac` / `mp3` / `ogg` etc., WARM border pill.
//! * **Duration** — `M:SS` derived from `duration_frames / sample_rate`.
//! * **Usage badge** — `used ×N` (ACCENT_SOFT) or `unused` (TEXT_3).
//! * *For missing rows only*: an inline `relink` chip (BAD-outlined, fires
//!   [`RelinkMessage::ShowModal`](crate::message::RelinkMessage::ShowModal)).
//!
//! The breadcrumb strip is intentionally absent from the Pool tab — the
//! pool is a project-level list, not a filesystem path.

use std::path::Path;

use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::text::LineHeight;
use iced::widget::{button, column, container, row, scrollable, text, Canvas, Space};
use iced::{alignment, mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::message::*;
use crate::state::pool::PoolAsset;
use crate::state::BrowserTab;
use crate::theme;
use crate::view::controls::collapse_caret;
use crate::view::relink_dialog::relink_chip;
use crate::Resonance;

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

/// Build the docked media-browser panel: a `BROWSER_WIDTH` column with a
/// `LINE` right border. Returned only when `browser.visible`; the caller
/// (`view_main_area`) prepends it to the arrange row so it sits flush
/// against the left edge, a peer of the track headers + timeline.
pub(crate) fn view_browser_panel(r: &Resonance) -> Element<'_, Message> {
    let tab = r.browser.tab;

    let body: Element<'_, Message> = column![
        header(),
        Space::new().height(14),
        tab_switcher(tab),
        Space::new().height(14),
        tab_body(r, tab),
    ]
    .spacing(0)
    .height(Length::Fill)
    .into();

    let panel = container(body)
        .width(Length::Fixed(theme::BROWSER_WIDTH))
        .height(Length::Fill)
        .padding(18)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            ..Default::default()
        });

    // `LINE` right border as a 1px hairline — the same separator the mixer
    // uses between its columns. Keeping it a sibling (rather than a uniform
    // `Border` on the container) gives a right-edge-only rule.
    let right_border =
        container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);

    row![panel, right_border].spacing(0).into()
}

// ---------------------------------------------------------------------------
// Panel chrome
// ---------------------------------------------------------------------------

/// Panel title row: a "MEDIA" section label plus the shared collapse caret,
/// which closes the panel (`ToggleVisible`) — the same affordance as the
/// "Media" chrome toggle.
fn header<'a>() -> Element<'a, Message> {
    let title = text("MEDIA")
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    let caret = button(collapse_caret(true))
        .on_press(Message::Browser(BrowserMessage::ToggleVisible))
        .padding(0)
        .style(|_theme, status| theme::ghost_button_style(status));

    row![
        title,
        Space::new().width(Length::Fill),
        caret,
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}

/// Files / Pool segmented switcher, styled like the chrome view tabs.
fn tab_switcher<'a>(current: BrowserTab) -> Element<'a, Message> {
    container(
        row![
            browser_tab_button("Files", BrowserTab::Files, current),
            browser_tab_button("Pool", BrowserTab::Pool, current),
        ]
        .spacing(3)
        .padding(4),
    )
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_LG.into(),
        },
        ..Default::default()
    })
    .into()
}

fn browser_tab_button<'a>(
    label: &'a str,
    tab: BrowserTab,
    current: BrowserTab,
) -> iced::widget::Button<'a, Message> {
    let active = current == tab;
    button(
        text(label)
            .size(12)
            .font(theme::UI_FONT_MEDIUM)
            .line_height(LineHeight::Relative(1.0)),
    )
    .on_press(Message::Browser(BrowserMessage::SelectTab(tab)))
    .style(move |_theme, status| theme::tab_button_style(active, status))
    .padding([6, 16])
}

// ---------------------------------------------------------------------------
// Per-tab bodies
// ---------------------------------------------------------------------------

/// Per-tab body. The Files tab leads with the filesystem breadcrumb; the
/// Pool tab hides it (a project-level list has no path).
fn tab_body<'a>(r: &'a Resonance, tab: BrowserTab) -> Element<'a, Message> {
    match tab {
        BrowserTab::Files => column![
            breadcrumb(r),
            Space::new().height(12),
            placeholder("Browse folders, audition, and drag audio onto the timeline."),
        ]
        .spacing(0)
        .into(),
        BrowserTab::Pool => pool_body(r),
    }
}

/// Filesystem breadcrumb strip for the Files tab. Renders the current
/// folder's path segments (root → current) as a `/`-joined label, or a
/// muted hint before the user has navigated anywhere. The clickable,
/// per-crumb `OpenFolder` affordance lands with the Files-tab body (#602).
fn breadcrumb<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let crumbs = r.browser.breadcrumb();
    let label = if crumbs.is_empty() {
        "No folder open".to_string()
    } else {
        crumbs
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| p.to_string_lossy().into_owned())
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" / ")
    };

    container(
        text(label)
            .size(11)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3)
            .line_height(LineHeight::Relative(1.0)),
    )
    .width(Length::Fill)
    .into()
}

fn placeholder<'a>(body: &'a str) -> Element<'a, Message> {
    text(body).size(12).color(theme::TEXT_3).into()
}

// ---------------------------------------------------------------------------
// Pool tab body (todo #603)
// ---------------------------------------------------------------------------

/// The Pool tab body: a scrollable list of the project's imported audio
/// assets. Each row shows a type glyph, a mini waveform thumbnail, the
/// filename, a format chip, the duration, and a usage badge. Missing rows
/// carry an inline `relink` chip instead of (or alongside) the usage badge.
fn pool_body<'a>(r: &'a Resonance) -> Element<'a, Message> {
    if r.pool.assets.is_empty() {
        return placeholder("No assets imported yet. Drag audio onto the timeline or use Import audio\u{2026}");
    }

    let sample_rate = r.sample_rate.max(1);
    let mut list = column![].spacing(4);
    for asset in &r.pool.assets {
        let usage = r.pool.usage_count(asset.id);
        list = list.push(pool_asset_row(asset, usage, sample_rate));
    }

    scrollable(list)
        .height(Length::Fill)
        .into()
}

/// One pool-asset row. Layout (left → right):
///
/// ```text
/// [♪] [~~waveform~~] [filename     ] [wav] [0:04] [used ×2]
///                                                  [relink ]  ← only if missing
/// ```
fn pool_asset_row<'a>(asset: &'a PoolAsset, usage: u32, sample_rate: u32) -> Element<'a, Message> {
    // --- Type glyph -------------------------------------------------------
    let (glyph_char, glyph_color) = if asset.missing {
        (theme::fa::TRIANGLE_EXCLAMATION, theme::BAD)
    } else {
        (theme::fa::MUSIC, theme::WARM)
    };
    let glyph = text(String::from(glyph_char))
        .font(theme::ICON_FONT)
        .size(11)
        .color(glyph_color)
        .line_height(LineHeight::Relative(1.0));

    // --- Mini waveform thumbnail ------------------------------------------
    // Fixed 48 × 26 px Canvas bar-chart of thumbnail_peaks (WARM amber).
    let thumbnail = Canvas::new(PoolThumbnail {
        peaks: &asset.thumbnail_peaks,
        missing: asset.missing,
    })
    .width(Length::Fixed(THUMB_W))
    .height(Length::Fixed(THUMB_H));

    // --- Filename ---------------------------------------------------------
    let fname = asset_filename(asset).into_owned();
    let name_label = text(fname)
        .size(11)
        .font(theme::UI_FONT_MEDIUM)
        .color(if asset.missing { theme::TEXT_3 } else { theme::TEXT_1 })
        .line_height(LineHeight::Relative(1.0));

    // --- Format chip ------------------------------------------------------
    let fmt_str = format_label(asset.format);
    let fmt_chip = container(
        text(fmt_str)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(if asset.missing { theme::TEXT_3 } else { theme::WARM })
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([2, 5])
    .style(move |_theme| {
        let border_color = if asset.missing {
            theme::LINE
        } else {
            theme::WARM_LINE
        };
        container::Style {
            border: iced::Border {
                color: border_color,
                width: 1.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        }
    });

    // --- Duration ---------------------------------------------------------
    let dur_secs = asset.duration_frames as f64 / sample_rate as f64;
    let dur_label = format_duration(dur_secs);
    let duration = text(dur_label)
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    // --- Name + chips row -------------------------------------------------
    let meta_row = row![name_label, Space::new().width(4), fmt_chip, Space::new().width(4), duration]
        .align_y(alignment::Vertical::Center)
        .spacing(0);

    // --- Usage badge / relink chip ----------------------------------------
    let badge: Element<'_, Message> = if asset.missing {
        relink_chip()
    } else if usage > 0 {
        text(format!("used \u{d7}{usage}"))
            .size(10)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::ACCENT_SOFT)
            .line_height(LineHeight::Relative(1.0))
            .into()
    } else {
        text("unused")
            .size(10)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_3)
            .line_height(LineHeight::Relative(1.0))
            .into()
    };

    // --- Compose the full row ---------------------------------------------
    let inner = column![meta_row, Space::new().height(3), badge].spacing(0);

    let content = row![
        glyph,
        Space::new().width(6),
        thumbnail,
        Space::new().width(8),
        inner,
    ]
    .align_y(alignment::Vertical::Center)
    .spacing(0);

    container(content)
        .width(Length::Fill)
        .padding([5, 6])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

// ---------------------------------------------------------------------------
// Pool thumbnail Canvas
// ---------------------------------------------------------------------------

/// Fixed size of the mini waveform thumbnail in pool rows.
const THUMB_W: f32 = 48.0;
const THUMB_H: f32 = 26.0;

/// A simple stateless [`canvas::Program`] that draws a waveform bar-chart
/// from `(min, max)` peak pairs. Used for the pool asset thumbnail; not a
/// live visual so no cache is needed.
struct PoolThumbnail<'a> {
    peaks: &'a [(f32, f32)],
    missing: bool,
}

impl canvas::Program<Message> for PoolThumbnail<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let h = bounds.height;
        let mid = h / 2.0;

        // Backdrop
        frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), theme::BG_3);
        // Centre zero line
        frame.fill_rectangle(Point::new(0.0, mid - 0.5), Size::new(w, 1.0), theme::LINE_2);

        if !self.peaks.is_empty() {
            let bar_color: Color = if self.missing {
                Color { a: 0.45, ..theme::BAD }
            } else {
                Color { a: 0.75, ..theme::WARM }
            };
            let col_w = w / self.peaks.len() as f32;
            let bar_w = col_w.max(1.0);
            for (i, &(min_v, max_v)) in self.peaks.iter().enumerate() {
                let x = i as f32 * col_w;
                let top = mid - max_v.clamp(-1.0, 1.0) * mid;
                let bot = mid - min_v.clamp(-1.0, 1.0) * mid;
                let bar_h = (bot - top).max(1.0);
                frame.fill_rectangle(Point::new(x, top), Size::new(bar_w, bar_h), bar_color);
            }
        }

        vec![frame.into_geometry()]
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The filename (final path component) of an asset's original source path,
/// falling back to the whole path string when there is no component.
fn asset_filename(asset: &PoolAsset) -> std::borrow::Cow<'_, str> {
    Path::new(&asset.original_path)
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or(std::borrow::Cow::Borrowed(&asset.original_path))
}

/// Short label for the container/codec family shown in the format chip.
fn format_label(fmt: resonance_common::AudioFormat) -> &'static str {
    use resonance_common::AudioFormat;
    match fmt {
        AudioFormat::Wav => "wav",
        AudioFormat::Flac => "flac",
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Ogg => "ogg",
        AudioFormat::Aac => "aac",
        AudioFormat::Mp4 => "m4a",
        AudioFormat::Other => "audio",
    }
}

/// Format a duration in seconds as `M:SS` (e.g. `"2:07"`).
fn format_duration(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{m}:{s:02}")
}
