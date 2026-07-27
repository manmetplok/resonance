//! Missing-files **relink modal** (design doc #175, todo #607).
//!
//! Surfaced on load when a project's media pool references audio whose
//! backing WAV isn't on disk (moved project, opened on another machine).
//! The clips are kept offline; this modal is the recovery surface:
//!
//! * a **list of every missing file** — filename + last-known path, one
//!   row each, with a per-file **`Locate…`** action that opens an OS file
//!   picker for that single asset ([`RelinkMessage::Locate`]);
//! * a one-shot **`Search a folder…`** primary action that picks a folder
//!   and resolves *every* missing file inside it by name
//!   ([`RelinkMessage::SearchFolder`]); and
//! * a **`Leave offline`** dismiss ([`RelinkMessage::DismissModal`]).
//!
//! Rows update live from the pool as relinks land: an in-flight asset
//! shows a `Relinking…` label, a resolved one flips to a green
//! `Relinked` check, and the footer tallies `N of M relinked`. The modal
//! shares the export/import/bounce modal scaffold — dimmed backdrop,
//! centered `BG_2` card with a `LINE` border and `RADIUS_XL` corners,
//! serif-italic title — so it reads as one family.
//!
//! Audio is the WARM domain, but a *missing file* is an error state, so
//! the warning glyph and count use `BAD`; the accent stays reserved for
//! the actionable `Locate…` / `Search a folder…` affordances.
//!
//! [`RelinkMessage::Locate`]: crate::message::RelinkMessage::Locate
//! [`RelinkMessage::SearchFolder`]: crate::message::RelinkMessage::SearchFolder
//! [`RelinkMessage::DismissModal`]: crate::message::RelinkMessage::DismissModal

use std::path::Path;

use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, Space,
};
use iced::{alignment, Element, Length};

use crate::message::{Message, RelinkMessage};
use crate::state::pool::PoolAsset;
use crate::theme;
use crate::Resonance;

/// Fixed width of the modal card — wide enough for a filename plus its
/// path on one row, matching the export/import modals' proportions.
const CARD_WIDTH: f32 = 520.0;
/// Cap on the scrollable file-list height so a project missing dozens of
/// files still leaves the header, note, and footer on screen.
const LIST_MAX_HEIGHT: f32 = 260.0;

/// The relink modal overlay. Returns an empty (zero-sized) element when
/// the modal isn't open or has nothing to track, so the caller can stack
/// it unconditionally.
pub(crate) fn view_relink_dialog_overlay(r: &Resonance) -> Element<'_, Message> {
    if !r.relink.modal_open || r.relink.modal_targets.is_empty() {
        return Space::new()
            .width(Length::Fixed(0.0))
            .height(Length::Fixed(0.0))
            .into();
    }

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Relink(RelinkMessage::DismissModal));

    // Resolve the tracked ids against the live pool. An id whose asset was
    // removed from the pool while the modal was open is dropped from the
    // list (nothing left to relink onto).
    let rows: Vec<(&PoolAsset, bool)> = r
        .relink
        .modal_targets
        .iter()
        .filter_map(|&id| r.pool.asset(id).map(|a| (a, r.relink.is_in_flight(id))))
        .collect();

    let total = rows.len();
    let relinked = rows.iter().filter(|(a, _)| !a.missing).count();

    let title = text(headline(total))
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    let subtitle = text(
        "These clips reference audio that isn't in the project folder or its \
         original location — likely moved, or opened on another machine. The \
         timeline keeps the clips; relink to restore audio.",
    )
    .size(12)
    .color(theme::TEXT_2);

    let mut list = column![].spacing(6);
    for (asset, in_flight) in &rows {
        list = list.push(relink_row(asset, *in_flight));
    }
    let list = scrollable(list)
        .height(Length::Shrink)
        .width(Length::Fill);
    let list = container(list).max_height(LIST_MAX_HEIGHT);

    let note = row![
        theme::icon(theme::fa::CIRCLE_INFO)
            .size(12)
            .color(theme::TEXT_3),
        text(
            "Pick a folder and Resonance searches it for every missing file at \
             once, by name. Relinked audio is copied back into the project folder.",
        )
        .size(11)
        .color(theme::TEXT_3),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let mut body = column![
        subtitle,
        Space::new().height(12),
        list,
        Space::new().height(12),
        note,
    ]
    .spacing(0);

    // Surface the most recent relink failure, if any, under the note.
    if let Some(err) = r.relink.last_error.as_deref() {
        body = body.push(Space::new().height(8));
        body = body.push(
            text(err)
                .size(11)
                .color(theme::BAD),
        );
    }

    let count_label = text(format!("{relinked} of {total} relinked"))
        .size(12)
        .color(if relinked == total {
            theme::GOOD
        } else {
            theme::TEXT_2
        });

    let leave_btn = button(text("Leave offline").size(13).color(theme::TEXT_1))
        .on_press(Message::Relink(RelinkMessage::DismissModal))
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));

    let search_btn = button(
        row![
            theme::icon(theme::fa::FOLDER_OPEN).size(12),
            text("Search a folder\u{2026}").size(13),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Relink(RelinkMessage::SearchFolder))
    .padding([8, 18])
    .style(|_theme, status| theme::primary_button_style(status));

    let footer = row![
        count_label,
        Space::new().width(Length::Fill),
        leave_btn,
        search_btn,
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let card_content = column![
        title,
        Space::new().height(8),
        body,
        Space::new().height(20),
        footer,
    ]
    .spacing(0)
    .padding(24)
    .width(CARD_WIDTH);

    let card = container(card_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(card))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

/// One file row in the relink list: a status glyph, the filename + its
/// last-known path, and the per-row action (Locate / Relinking… / a
/// resolved check).
fn relink_row(asset: &PoolAsset, in_flight: bool) -> Element<'_, Message> {
    let resolved = !asset.missing;

    let (glyph, glyph_color) = if resolved {
        (theme::fa::CIRCLE_CHECK, theme::GOOD)
    } else {
        (theme::fa::TRIANGLE_EXCLAMATION, theme::BAD)
    };

    let name_col = column![
        text(file_name(asset).to_string())
            .size(12)
            .color(theme::TEXT_1),
        text(asset.original_path.clone())
            .size(10)
            .color(theme::TEXT_4),
    ]
    .spacing(2)
    .width(Length::Fill);

    // Action cell: an in-flight import wins (spinner label), then a
    // resolved asset (green check), otherwise the actionable Locate button.
    let action: Element<'_, Message> = if in_flight {
        text("Relinking\u{2026}").size(11).color(theme::WARM).into()
    } else if resolved {
        row![
            theme::icon(theme::fa::CIRCLE_CHECK).size(11).color(theme::GOOD),
            text("Relinked").size(11).color(theme::GOOD),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .into()
    } else {
        button(text("Locate\u{2026}").size(11).color(theme::ACCENT_SOFT))
            .on_press(Message::Relink(RelinkMessage::Locate(asset.id)))
            .padding([4, 12])
            .style(|_theme, status| locate_button_style(status))
            .into()
    };

    let inner = row![
        theme::icon(glyph).size(13).color(glyph_color),
        name_col,
        action,
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .padding([8, 12]);

    container(inner)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: if resolved { theme::GOOD } else { theme::LINE },
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// The modal headline, singular/plural aware.
fn headline(count: usize) -> String {
    if count == 1 {
        "1 media file couldn't be found".to_string()
    } else {
        format!("{count} media files couldn't be found")
    }
}

/// The filename (final path component) of an asset's original source,
/// falling back to the whole path if it has no filename component.
fn file_name(asset: &PoolAsset) -> std::borrow::Cow<'_, str> {
    Path::new(&asset.original_path)
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or(std::borrow::Cow::Borrowed(&asset.original_path))
}

/// Outlined accent button used for the per-row `Locate…` action —
/// lighter than the primary `Search a folder…`, so the batch action stays
/// the visual default.
fn locate_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => theme::ACCENT_DIM,
        button::Status::Pressed => theme::ACCENT_LINE,
        _ => iced::Color::TRANSPARENT,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::ACCENT_SOFT,
        border: iced::Border {
            color: theme::ACCENT_LINE,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// The inline **`relink` chip** for a Pool tab `missing` row (design doc
/// #175). A small BAD-outlined pill that opens the relink modal
/// ([`RelinkMessage::ShowModal`]). Exposed here so the Pool tab renderer
/// (todo #603) can drop it into a missing asset's row without duplicating
/// the styling or the entry-point wiring.
///
/// [`RelinkMessage::ShowModal`]: crate::message::RelinkMessage::ShowModal
#[allow(dead_code)] // consumed by the Pool tab rendering, todo #603.
pub(crate) fn relink_chip<'a>() -> Element<'a, Message> {
    button(text("relink").size(9).color(theme::BAD))
        .on_press(Message::Relink(RelinkMessage::ShowModal))
        .padding([1, 8])
        .style(|_theme, status| {
            let bg = match status {
                button::Status::Hovered => iced::Color { a: 0.12, ..theme::BAD },
                _ => iced::Color::TRANSPARENT,
            };
            button::Style {
                background: Some(iced::Background::Color(bg)),
                text_color: theme::BAD,
                border: iced::Border {
                    color: theme::BAD,
                    width: 1.0,
                    radius: 999.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}
