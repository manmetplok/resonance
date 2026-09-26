//! Audio-import transcode-progress modal (design doc #175, ba todo #606).
//!
//! Shown during a multi-file audio import while the engine copies, decodes,
//! resamples, and channel-mixes each source file into the project's
//! `*.rproj/audio/` folder. The overlay lists every file in the current
//! batch with a live status indicator:
//!
//! * **Queued** — file is waiting in the engine's import queue.
//! * **Working** — the engine is actively transcoding / resampling this file.
//! * **Done** — the file was imported successfully; the asset is now in the
//!   pool.
//! * **Failed** — an error prevented the import (shown with a `BAD` glyph and
//!   an inline reason).
//!
//! An explanatory note beneath the list reminds the user that files are
//! **copied** into the project folder (`*.rproj/audio/`) so the project
//! stays self-contained and relocatable.
//!
//! The modal shares the import/bounce/relink modal scaffold — dimmed backdrop,
//! centered `BG_2` card with a `LINE` border and `RADIUS_XL` corners,
//! serif-italic title — so it reads as one family. The backdrop blocks clicks
//! while imports are in flight; once every file reaches a terminal state a
//! "Done" button appears.
//!
//! Status is driven by the [`crate::state::ImportProgressTracker`] populated
//! via `ImportProgress` / `ImportFailed` engine events (ba todo #597); the
//! modal opens when an import is kicked off (ba todo #598) and is dismissed
//! via [`crate::message::UiMessage::DismissImportProgress`].

use std::path::Path;

use iced::widget::{button, column, container, mouse_area, opaque, row, scrollable, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::{Message, UiMessage};
use crate::state::{FileImportProgress, FileImportStatus};
use crate::theme;
use crate::Resonance;

/// Fixed width of the modal card — wide enough for a filename plus a short
/// reason string, matching the relink / bounce modal proportions.
const CARD_WIDTH: f32 = 500.0;

/// Cap on the scrollable file-list height so a large batch still leaves the
/// header, note, and footer visible on screen.
const LIST_MAX_HEIGHT: f32 = 280.0;

/// The import-progress modal overlay. Returns a zero-sized element when the
/// modal isn't open, so the caller can stack it unconditionally.
pub(crate) fn view_import_progress_overlay(r: &Resonance) -> Element<'_, Message> {
    if !r.media.import_progress_modal_open {
        return Space::new()
            .width(Length::Fixed(0.0))
            .height(Length::Fixed(0.0))
            .into();
    }

    let statuses = r.media.import_progress.statuses();
    // Guard: an empty tracker (before the first ImportProgress event lands) must
    // NOT be treated as "complete" — that would flash a 0-file "Done" state right
    // after the modal opens.  Require at least one entry before declaring the
    // batch finished.
    let all_done = !statuses.is_empty() && r.media.import_progress.is_complete();

    // Backdrop: blocks clicks while imports are in flight (so the user can't
    // start editing while the engine is mid-transcode); clicking the backdrop
    // dismisses once the batch is complete — same pattern as bounce progress.
    let backdrop_base = container(Space::new().width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgba(
                0.0, 0.0, 0.0, 0.65,
            ))),
            ..Default::default()
        });

    let backdrop: Element<'_, Message> = if all_done {
        mouse_area(backdrop_base)
            .on_press(Message::Ui(UiMessage::DismissImportProgress))
            .into()
    } else {
        // No on_press while in flight — import can't be cancelled.
        mouse_area(backdrop_base).into()
    };

    let title_str = if all_done {
        modal_title(statuses, true)
    } else {
        "Importing audio\u{2026}".to_string()
    };

    let title = text(title_str)
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    // Per-file status rows.
    let mut file_list = column![].spacing(6);
    if statuses.is_empty() {
        // Batch queued but no `ImportProgress` events have landed yet — show a
        // placeholder so the modal isn't confusingly blank.
        file_list = file_list.push(
            text("Preparing import\u{2026}")
                .size(12)
                .color(theme::TEXT_3),
        );
    } else {
        for status in statuses {
            file_list = file_list.push(file_row(status));
        }
    }

    let file_list = container(
        scrollable(file_list.width(Length::Fill))
            .height(Length::Shrink)
            .width(Length::Fill),
    )
    .max_height(LIST_MAX_HEIGHT);

    // Explanatory note: files are copied into the project folder.
    let note = row![
        theme::icon(theme::fa::CIRCLE_INFO)
            .size(11)
            .color(theme::TEXT_3),
        text(
            "Files are copied and converted into the project folder \
             (*.rproj/audio/) so the project stays self-contained \
             and can be moved or shared without broken links.",
        )
        .size(11)
        .color(theme::TEXT_3),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Top);

    let body = column![
        file_list,
        Space::new().height(12),
        note,
    ]
    .spacing(0);

    // Footer: a "Done" button that's only enabled once all files are settled.
    let footer = if all_done {
        let done_btn = button(text("Done").size(13))
            .on_press(Message::Ui(UiMessage::DismissImportProgress))
            .padding([8, 24])
            .style(|_theme, status| theme::primary_button_style(status));

        row![Space::new().width(Length::Fill), done_btn]
            .align_y(alignment::Vertical::Center)
    } else {
        // Show a subtle "Importing…" label while the batch runs.
        let label = text("Importing\u{2026}")
            .size(12)
            .color(theme::TEXT_3);
        row![Space::new().width(Length::Fill), label]
            .align_y(alignment::Vertical::Center)
    };

    let card_content = column![
        title,
        Space::new().height(12),
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

/// One file row: a status glyph, the file name, and (for failures) the
/// inline reason.
fn file_row(status: &FileImportStatus) -> Element<'_, Message> {
    let (glyph, glyph_color, label_color) = match &status.progress {
        FileImportProgress::Queued => (theme::fa::CIRCLE, theme::TEXT_3, theme::TEXT_3),
        FileImportProgress::Working => (theme::fa::CIRCLE, theme::WARM, theme::TEXT_2),
        FileImportProgress::Done => (theme::fa::CIRCLE_CHECK, theme::GOOD, theme::TEXT_2),
        FileImportProgress::Failed { .. } => {
            (theme::fa::TRIANGLE_EXCLAMATION, theme::BAD, theme::TEXT_2)
        }
    };

    let status_label = match &status.progress {
        FileImportProgress::Queued => "queued",
        FileImportProgress::Working => "working\u{2026}",
        FileImportProgress::Done => "done",
        FileImportProgress::Failed { .. } => "failed",
    };

    let name = file_name(&status.path).into_owned();

    let reason_row: Element<'_, Message> =
        if let FileImportProgress::Failed { reason } = &status.progress {
            text(reason.as_str()).size(10).color(theme::BAD).into()
        } else {
            Space::new().height(0).into()
        };

    let name_col = column![
        text(name)
            .size(12)
            .color(label_color),
        reason_row,
    ]
    .spacing(2)
    .width(Length::Fill);

    let status_text = text(status_label)
        .size(11)
        .color(glyph_color);

    let inner = row![
        theme::icon(glyph).size(12).color(glyph_color),
        name_col,
        status_text,
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .padding([7, 12]);

    let border_color = match &status.progress {
        FileImportProgress::Done => theme::GOOD_LINE,
        FileImportProgress::Failed { .. } => theme::BAD_LINE,
        FileImportProgress::Working => theme::WARM_LINE,
        FileImportProgress::Queued => theme::LINE,
    };

    container(inner)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: border_color,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// The modal title, contextual once complete.
fn modal_title(statuses: &[FileImportStatus], complete: bool) -> String {
    if !complete {
        return "Importing audio\u{2026}".to_string();
    }
    let total = statuses.len();
    let failed = statuses
        .iter()
        .filter(|s| matches!(s.progress, FileImportProgress::Failed { .. }))
        .count();
    if failed == 0 {
        if total == 1 {
            "1 file imported".to_string()
        } else {
            format!("{total} files imported")
        }
    } else if failed == total {
        if total == 1 {
            "Import failed".to_string()
        } else {
            format!("All {total} files failed to import")
        }
    } else {
        let done = total - failed;
        format!("{done} of {total} imported ({failed} failed)")
    }
}

/// The filename (final path component) of a source path, falling back to the
/// whole path if it has no filename component.
fn file_name(path: &str) -> std::borrow::Cow<'_, str> {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or(std::borrow::Cow::Borrowed(path))
}
