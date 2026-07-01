//! Docked media-browser panel for the Arrange view (design doc #175,
//! epic #35).
//!
//! This is the **container scaffold** only: a fixed-width left column
//! ([`theme::BROWSER_WIDTH`], `BG_2` fill, `LINE` right border) that mirrors
//! the Compose right-rail / Mixer inspector pattern, its header (title +
//! [`crate::view::controls::collapse_caret`] that closes the panel), and the
//! Files / Pool tab switcher. The per-tab bodies — the filesystem browse
//! (breadcrumb + shelf + rows), the pool asset list, and the audition
//! transport — land in follow-up todos (#602–#604); until then each tab
//! shows a short placeholder.
//!
//! Everything the panel drives is transient UI state routed through
//! [`crate::message::BrowserMessage`] (classified `UndoAction::Skip`), so
//! toggling the panel, switching tabs, and — later — navigating folders
//! never touches the project file or the undo stack.

use iced::widget::text::LineHeight;
use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::BrowserTab;
use crate::theme;
use crate::view::controls::collapse_caret;
use crate::Resonance;

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

/// Per-tab body. The Files tab leads with the filesystem breadcrumb; the
/// Pool tab hides it (a project-level list has no path). Both bodies are
/// placeholders in this scaffold todo — the real rows arrive in #602/#603.
fn tab_body<'a>(r: &'a Resonance, tab: BrowserTab) -> Element<'a, Message> {
    match tab {
        BrowserTab::Files => column![breadcrumb(r), Space::new().height(12), placeholder(
            "Browse folders, audition, and drag audio onto the timeline.",
        )]
        .spacing(0)
        .into(),
        BrowserTab::Pool => placeholder(
            "Imported assets in this project appear here with usage counts.",
        ),
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
