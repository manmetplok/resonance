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
//! The **Files tab body** (todo #602) browses the filesystem:
//!
//! * a **breadcrumb** row of clickable path crumbs plus a WARM star toggle
//!   that favourites / unfavourites the current folder;
//! * a **favourites / recent shelf** of pill chips (favourite = WARM star,
//!   recent = clock) — the entry point before any folder is open;
//! * a **per-folder filter** field (case-insensitive file-name substring);
//! * the **subfolder rows** (folder glyph + name) and **audio rows** (type
//!   glyph, mini waveform thumbnail, lightly colour-coded format chip, and
//!   duration); and
//! * the **empty-folder state** — copy plus a "Choose files…" button that
//!   opens the import picker — shown when an open folder holds no audio.
//!
//! The mini waveform thumbnails come from the `resonance_common`
//! waveform-thumbnail peaks decoded off-thread during the folder scan
//! ([`crate::state::FolderScan::thumbnails`]) and cached per navigation, per
//! the view-performance rules.
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
//!
//! # Module layout
//!
//! The tab bodies and shared helpers live in sibling submodules to keep each
//! file under ~500 lines and every concern in its own reason-to-change:
//!
//! * [`files_tab`] — filesystem breadcrumb, shelf, filter field, listing
//! * [`pool_tab`] — pool asset list + audition transport
//! * [`style`] — `WaveThumbnail` canvas, format helpers, button styles

mod files_tab;
mod pool_tab;
mod style;

use iced::widget::text::LineHeight;
use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::BrowserTab;
use crate::theme;
use crate::view::controls::collapse_caret;
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
        // The tab body fills the remaining height, pushing the audition
        // transport flush against the panel's bottom edge.
        container(tab_body(r, tab)).height(Length::Fill),
        pool_tab::audition_transport(r),
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

    row![title, Space::new().width(Length::Fill), caret,]
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
// Per-tab dispatch
// ---------------------------------------------------------------------------

/// Per-tab body. The Files tab leads with the filesystem breadcrumb; the
/// Pool tab hides it (a project-level list has no path).
fn tab_body<'a>(r: &'a Resonance, tab: BrowserTab) -> Element<'a, Message> {
    match tab {
        BrowserTab::Files => files_tab::files_body(r),
        BrowserTab::Pool => pool_tab::pool_body(r),
    }
}
