//! The voicebank library overlay (vocal-mcp-control-and-quality.md §5,
//! D15), opened from a vocal lane's Library… button: tabs `Installed |
//! plok.org`, search and ★ over the shared `BrowserModel`, a list, and a
//! detail pane with the bank's credit, terms and actions.
//!
//! A download never starts from one click: Download (and Update) shows the
//! bank's terms and credit line in the detail pane, and only "Accept &
//! download" starts it. Delete asks first. Everything shown is the rows
//! `update::voicebank_library` built; the list is a `lazy` region keyed on
//! their generation and the selection, and the detail pane (which carries
//! the live progress) sits outside it.

use iced::widget::{
    button, column, container, mouse_area, opaque, progress_bar, row, scrollable, stack, text,
    text_input, Space,
};
use iced::{alignment, Element, Length};
use resonance_plugin::library_view::{BrowserModel, Notice};

use crate::message::{Message, VoicebankLibraryMessage as M};
use crate::state::voicebank_library::{LibraryOverlay, LibraryTab, RowStatus, VoicebankRow};
use crate::theme;
use crate::Resonance;

const PANEL_WIDTH: f32 = 760.0;
const LIST_WIDTH: f32 = 330.0;
const BODY_HEIGHT: f32 = 380.0;

fn msg(m: M) -> Message {
    Message::VoicebankLibrary(m)
}

fn star_glyph<'a>(favorite: bool, size: f32) -> iced::widget::Text<'a> {
    text(String::from(theme::fa::STAR))
        .font(theme::ICON_FONT)
        .size(size)
        .color(if favorite { theme::WARM } else { theme::TEXT_3 })
}

pub(crate) fn view_voicebank_library_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let Some(o) = r.voicebank_library.overlay.as_ref() else {
        return Space::new().into();
    };
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.4,
                ))),
                ..Default::default()
            }),
    )
    .on_press(msg(M::Close));

    let header = row![
        text("VOICEBANKS")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        text("Library").size(12).color(theme::ACCENT),
        Space::new().width(Length::Fill),
        button(text("Refresh").size(11).color(theme::TEXT))
            .on_press(msg(M::Refresh))
            .style(|_theme, status| theme::ghost_button_style(status))
            .padding([2, 8]),
        button(text("\u{00d7}").size(14).color(theme::TEXT_DIM))
            .on_press(msg(M::Close))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let tab = |label: &'a str, which: LibraryTab| {
        let active = o.tab == which;
        button(text(label).size(11))
            .on_press(msg(M::SetTab(which)))
            .style(move |_theme, status| theme::tab_button_style(active, status))
            .padding([4, 12])
    };
    let index_note = index_note(r);
    let tabs = row![
        tab(&o.tab_labels.0, LibraryTab::Installed),
        tab(&o.tab_labels.1, LibraryTab::Plok),
        Space::new().width(Length::Fill),
        text(index_note).size(10).color(theme::TEXT_3),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    let (browser, _) = o.current();
    let body = row![
        container(list(o)).width(LIST_WIDTH).height(BODY_HEIGHT),
        container(detail(o))
            .width(Length::Fill)
            .height(BODY_HEIGHT)
            .padding([8, 12])
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG_3)),
                border: iced::Border {
                    color: theme::LINE_2,
                    width: 1.0,
                    radius: theme::RADIUS_MD.into(),
                },
                ..Default::default()
            }),
    ]
    .spacing(10);

    let panel = container(opaque(
        column![
            header,
            tabs,
            search_row(browser),
            body,
            notice_line(browser.notice()),
        ]
        .spacing(10)
        .padding(14),
    ))
    .width(PANEL_WIDTH)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_LG.into(),
        },
        ..Default::default()
    });

    let centered = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center);
    stack![backdrop, centered].into()
}

/// Where the plok.org list stands, for the tab row.
fn index_note(r: &Resonance) -> &'static str {
    let s = r.voicebank_library.lib.state();
    match (s.fetching_index, s.index.is_some(), s.index_error.is_some()) {
        (true, _, _) => "Reading the plok.org index\u{2026}",
        (false, false, true) => "The plok.org index is unavailable",
        _ => "",
    }
}

fn search_row(browser: &BrowserModel) -> Element<'_, Message> {
    let only = browser.favorites_only();
    row![
        text_input("Search voicebanks (lang:en, is:fav)\u{2026}", browser.query())
            .on_input(|s| msg(M::Search(s)))
            .size(12)
            .padding([4, 6])
            .style(theme::borderless_text_input_style),
        button(
            row![
                star_glyph(only, 10.0),
                text("only")
                    .size(10)
                    .color(if only { theme::WARM } else { theme::TEXT_2 }),
            ]
            .spacing(4)
            .align_y(alignment::Vertical::Center),
        )
        .on_press(msg(M::FavoritesOnly(!only)))
        .style(move |_theme, status| theme::tab_button_style(only, status))
        .padding([3, 8]),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// The always-present notice slot (an empty line when there is nothing to
/// say, so the panel keeps its shape).
fn notice_line(notice: Option<&Notice>) -> Element<'_, Message> {
    let (body, color) = match notice {
        Some(n) if n.is_error() => (n.text(), theme::BAD),
        Some(n) => (n.text(), theme::TEXT_2),
        None => ("", theme::TEXT_3),
    };
    text(body).size(11).color(color).into()
}

/// The current tab's rows: a `lazy` region keyed on the rows' generation,
/// the tab, the view and the selection.
fn list(o: &LibraryOverlay) -> Element<'_, Message> {
    let (browser, rows) = o.current();
    // Hashed rather than copied: the key is built every frame.
    let key = (
        o.rows_generation,
        o.tab == LibraryTab::Installed,
        hash_of(browser.view()),
        hash_of(&browser.selected()),
    );
    let empty = match o.tab {
        LibraryTab::Installed => "No voicebank is installed. The plok.org tab lists downloads.",
        LibraryTab::Plok => "The plok.org index lists no voicebanks yet.",
    };
    let content = iced::widget::lazy(key, move |_| -> Element<'static, Message> {
        let view = browser.view();
        let selected = browser.selected();
        let mut col = column![].spacing(2);
        if view.is_empty() {
            col = col.push(text(empty).size(11).color(theme::TEXT_3));
        }
        for &i in view {
            let r = &rows.rows[i];
            col = col.push(list_row(r, selected == Some(r.key.as_str())));
        }
        scrollable(col).height(Length::Fill).into()
    });
    content.into()
}

fn hash_of<T: std::hash::Hash + ?Sized>(value: &T) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

fn list_row(r: &VoicebankRow, selected: bool) -> Element<'static, Message> {
    let mut label = column![text(r.name.clone()).size(12).color(theme::TEXT)].spacing(1);
    if !r.subtitle.is_empty() {
        label = label.push(text(r.subtitle.clone()).size(10).color(theme::TEXT_3));
    }
    let (chip, chip_color) = status_chip(&r.status);
    let body = container(
        row![
            label,
            Space::new().width(Length::Fill),
            text(chip).size(9).font(theme::UI_FONT_SEMIBOLD).color(chip_color),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .padding([4, 8])
    .style(move |_theme| row_style(selected));
    let body = mouse_area(body)
        .on_press(msg(M::Select(r.key.clone())))
        .interaction(iced::mouse::Interaction::Pointer);
    let favorite = r.marks.as_ref().is_some_and(|m| m.favorite);
    let star = button(star_glyph(favorite, 12.0))
        .on_press(msg(M::ToggleFavorite(r.key.clone())))
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding([2, 4]);
    row![body, star]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
        .into()
}

fn status_chip(status: &RowStatus) -> (String, iced::Color) {
    match status {
        RowStatus::Installed => ("INSTALLED".into(), theme::GOOD),
        RowStatus::NotInstalled => (String::new(), theme::TEXT_3),
        RowStatus::UpdateAvailable => ("UPDATE".into(), theme::WARM),
        RowStatus::Installing {
            fraction: Some(f), ..
        } => (format!("{:.0}%", f * 100.0), theme::ACCENT),
        RowStatus::Installing { .. } => ("\u{2026}".into(), theme::ACCENT),
    }
}

fn row_style(selected: bool) -> container::Style {
    let (bg, border) = if selected {
        (theme::WARM_DIM, theme::WARM_LINE)
    } else {
        (theme::BG_3, theme::LINE_2)
    };
    container::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Some(theme::TEXT_1),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// The selected bank: what it is, whose it is, its terms, and what can be
/// done with it — or the terms to accept, or the delete to confirm.
fn detail(o: &LibraryOverlay) -> Element<'_, Message> {
    let (browser, rows) = o.current();
    let Some(r) = browser.selected().and_then(|k| rows.row(k)) else {
        return text("Select a voicebank to see its terms and actions.")
            .size(11)
            .color(theme::TEXT_3)
            .into();
    };
    let mut col = column![
        text(&r.name).size(14).color(theme::TEXT),
        text(&r.subtitle).size(10).color(theme::TEXT_3),
    ]
    .spacing(4);
    col = col.push(Space::new().height(4));
    col = col.push(fact("Credit", r.credit.as_deref().unwrap_or("\u{2014}")));
    col = col.push(fact("Terms", r.terms_url.as_deref().unwrap_or("\u{2014}")));
    col = col.push(fact(
        "Commercial use",
        r.commercial_text().unwrap_or("Not stated \u{2014} check the terms"),
    ));
    col = col.push(fact("Vocoder", &r.vocoder_line));
    if !r.english {
        col = col.push(
            text("Lyrics are English; this bank cannot sing them.")
                .size(10)
                .color(theme::WARM),
        );
    }
    col = col.push(Space::new().height(Length::Fill));

    if let RowStatus::Installing { label, fraction } = &r.status {
        col = col.push(text(label).size(10).color(theme::TEXT_2));
        col = col.push(progress_bar(0.0..=1.0, fraction.unwrap_or(0.0)).girth(6));
    }
    if o.accept.as_deref() == Some(r.id.as_str()) {
        col = col.push(accept_terms(r));
    } else if o.confirm_delete.as_deref() == Some(r.id.as_str()) {
        col = col.push(confirm_delete(r));
    } else {
        col = col.push(actions(r));
    }
    col.into()
}

fn fact<'a>(label: &'static str, value: &'a str) -> Element<'a, Message> {
    row![
        container(text(label).size(10).color(theme::TEXT_3)).width(100),
        // A terms URL is one long word: break it anywhere rather than
        // run past the pane.
        text(value)
            .size(11)
            .color(theme::TEXT_1)
            .width(Length::Fill)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
    ]
    .spacing(6)
    .into()
}

fn ghost(label: &'static str, m: Option<M>) -> button::Button<'static, Message> {
    let b = button(text(label).size(11).color(theme::TEXT))
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding([4, 12]);
    match m {
        Some(m) => b.on_press(msg(m)),
        None => b,
    }
}

fn primary(label: &'static str, m: M) -> button::Button<'static, Message> {
    button(text(label).size(11))
        .on_press(msg(m))
        .style(|_theme, status| theme::primary_button_style(status))
        .padding([4, 12])
}

fn actions(r: &VoicebankRow) -> Element<'static, Message> {
    let id = r.id.clone();
    let mut buttons = row![Space::new().width(Length::Fill)]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
    match &r.status {
        RowStatus::Installing { .. } => {
            buttons = buttons.push(ghost("Cancel", Some(M::Cancel(id))));
        }
        RowStatus::NotInstalled => {
            buttons = buttons.push(primary("Download\u{2026}", M::Download(id)));
        }
        RowStatus::UpdateAvailable | RowStatus::Installed => {
            if r.user_dir.is_some() {
                buttons = buttons.push(
                    button(text("Delete\u{2026}").size(11))
                        .on_press(msg(M::Delete(id.clone())))
                        .style(|_theme, status| theme::destructive_button_style(status))
                        .padding([4, 12]),
                );
            }
            if r.status == RowStatus::UpdateAvailable {
                buttons = buttons.push(ghost("Update\u{2026}", Some(M::Download(id.clone()))));
            }
            let usable = r.english && (r.vocoder_installed || r.source == "plok");
            buttons = buttons.push(
                button(text("Use on this lane").size(11))
                    .on_press_maybe(usable.then(|| msg(M::Use(id))))
                    .style(|_theme, status| theme::primary_button_style(status))
                    .padding([4, 12]),
            );
        }
    }
    buttons.into()
}

/// The terms a download accepts, with the explicit accept.
fn accept_terms(r: &VoicebankRow) -> Element<'static, Message> {
    let credit = r
        .credit
        .clone()
        .map(|c| format!("Credit it as \u{201c}{c}\u{201d} wherever you publish its singing."))
        .unwrap_or_else(|| "Its terms name no credit line.".to_owned());
    let terms = r
        .terms_url
        .clone()
        .map(|t| format!("Terms of use: {t}"))
        .unwrap_or_else(|| "Its creator publishes no terms URL.".to_owned());
    let size = r
        .size
        .clone()
        .map(|s| format!(" ({s})"))
        .unwrap_or_default();
    container(
        column![
            text("BEFORE YOU DOWNLOAD")
                .size(9)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::WARM),
            text(format!(
                "{}{size} comes from its creator's own download; plok.org only lists it. \
                 Downloading it accepts its terms.",
                r.name
            ))
            .size(11)
            .color(theme::TEXT_1),
            text(terms)
                .size(11)
                .color(theme::TEXT_1)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            text(credit).size(11).color(theme::TEXT_1),
            text(r.commercial_text().unwrap_or("Commercial use: check the terms."))
                .size(11)
                .color(theme::TEXT_1),
            row![
                Space::new().width(Length::Fill),
                ghost("Decline", Some(M::DeclineTerms)),
                primary("Accept & download", M::AcceptTerms),
            ]
            .spacing(8),
        ]
        .spacing(6),
    )
    .padding(8)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::WARM_DIM)),
        border: iced::Border {
            color: theme::WARM_LINE,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    })
    .into()
}

fn confirm_delete(r: &VoicebankRow) -> Element<'static, Message> {
    column![
        text(format!(
            "Delete {} from your voicebank folder? Lanes that use it will not render until it \
             is installed again.",
            r.name
        ))
        .size(11)
        .color(theme::TEXT_1),
        row![
            Space::new().width(Length::Fill),
            ghost("Keep", Some(M::CancelDelete)),
            button(text("Delete").size(11))
                .on_press(msg(M::ConfirmDelete))
                .style(|_theme, status| theme::destructive_button_style(status))
                .padding([4, 12]),
        ]
        .spacing(8),
    ]
    .spacing(6)
    .into()
}
