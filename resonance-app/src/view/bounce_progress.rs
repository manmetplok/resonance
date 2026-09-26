//! Progress modal shown while a bounce-in-place run is in flight.
//! Blocks every other UI surface so the user can't disturb the engine
//! mid-render (transport, track edits, plugin tweaks all gate on
//! `Resonance::bounce_in_progress`). A Cancel button sends
//! `AudioCommand::CancelBounce`; for the offline path the engine
//! aborts cooperatively between chunks, for the realtime path it
//! pauses the transport, restores the mute snapshot, and removes the
//! freshly-added empty target track.
//!
//! The **freeze** progress modal (design doc #181, ba todo #582) reuses
//! this exact overlay — freeze *is* a bounce-in-place run — relabelled
//! with the snowflake title and, for freeze-all / freeze-selected
//! batches, a "track N / M" counter fed from the sequential
//! [`FreezeQueue`](crate::state::FreezeQueue). Input gating happens in
//! `update/gates.rs` (`freeze_blocks_message`), mirroring the bounce
//! gate; the Cancel button dispatches `FreezeMessage::CancelFreeze`,
//! the one whitelisted carve-out.
//!
//! The **WAV mixdown** (master strip "Bounce", or control
//! `render.mixdown`) gates the same traffic (`io.bouncing`) and shows the
//! same overlay too (code review FU-F1c), fed by the legacy bounce path's
//! whole-percent `BounceProgress` events; its Cancel dispatches
//! `ProjectIoMessage::CancelBounce` (the `ProjectIo` family passes the
//! gate), which flips the export's cooperative cancel token.

use iced::widget::{
    button, column, container, mouse_area, opaque, progress_bar, row, stack, text, Space,
};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::{BounceMode, FreezeStatus};
use crate::theme::{self, fa};
use crate::Resonance;

pub(crate) fn view_bounce_progress_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let Some(state) = r.bounce_in_progress.as_ref() else {
        return Space::new().width(Length::Fixed(0.0)).height(Length::Fixed(0.0)).into();
    };

    let title_str = match state.mode {
        BounceMode::Offline => format!("Bouncing \"{}\"", state.source_name),
        BounceMode::Realtime => format!("Recording \"{}\"", state.source_name),
    };
    let title = text(title_str)
        .size(18)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1)
        .into();

    let detail = match state.mode {
        BounceMode::Offline => "Rendering through the instrument and effect chain offline.",
        BounceMode::Realtime => {
            "Playing the timeline and capturing the external instrument's audio return."
        }
    };

    let pct = (state.fraction * 100.0).round() as u32;
    progress_dialog(
        title,
        detail,
        state.fraction,
        format!("{pct}%"),
        Message::Track(TrackMessage::Bounce(BounceMessage::CancelInProgress)),
    )
}

/// The freeze progress modal (design doc #181, ba todo #582): the bounce
/// overlay relabelled for the freeze render. Shows the snowflake +
/// serif-italic "Freezing "name"" title, the live progress bar fed from
/// the engine's `FreezeProgress` fractions (mirrored by ba todo #575),
/// and — during a freeze-all / freeze-selected batch — a mono
/// "track N / M" counter from the queue. Cancel dispatches
/// [`FreezeMessage::CancelFreeze`], the one message the freeze gate lets
/// through, aborting the render cooperatively and abandoning the batch.
pub(crate) fn view_freeze_progress_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    // The track currently rendering and its progress fraction. Between
    // batch items (completion handled, next not yet started) no status is
    // `Freezing`; fall back to the queue's current entry at 0%.
    let freezing = r
        .freeze
        .statuses
        .iter()
        .find_map(|(id, s)| match s {
            FreezeStatus::Freezing { fraction } => Some((*id, *fraction)),
            _ => None,
        })
        .or_else(|| {
            r.freeze
                .queue
                .as_ref()
                .and_then(|q| q.current)
                .map(|id| (id, 0.0))
        });
    let Some((track_id, fraction)) = freezing else {
        return Space::new().width(Length::Fixed(0.0)).height(Length::Fixed(0.0)).into();
    };

    let name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.as_str())
        .unwrap_or("track");

    let title = row![
        theme::icon(fa::SNOWFLAKE).size(16).color(theme::FROST_ICON),
        Space::new().width(9),
        text(format!("Freezing \"{name}\""))
            .size(18)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_1),
    ]
    .align_y(alignment::Vertical::Center)
    .into();

    let detail = "Rendering through the instrument and effect chain into the freeze cache.";

    // Mono caption: percent, plus the batch counter ("track N / M") when a
    // freeze-all / freeze-selected queue is driving this run. `completed`
    // counts finished tracks, so the one rendering now is `completed + 1`.
    let pct = (fraction * 100.0).round() as u32;
    let caption = match r.freeze.queue.as_ref() {
        Some(q) if q.total > 1 => {
            let n = (q.completed + 1).min(q.total);
            format!("{pct}% \u{00b7} track {n} / {}", q.total)
        }
        _ => format!("{pct}%"),
    };

    progress_dialog(
        title,
        detail,
        fraction,
        caption,
        Message::Freeze(FreezeMessage::CancelFreeze),
    )
}

/// The WAV mixdown progress modal (FU-F1c): the bounce overlay titled
/// with the target file name, fed by `io.bounce_fraction`. Cancel flips
/// the export's cancel token; once pressed the caption says so until the
/// engine confirms, and the button's message is a no-op repeat.
pub(crate) fn view_mixdown_progress_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    if !r.io.bouncing {
        return Space::new().width(Length::Fixed(0.0)).height(Length::Fixed(0.0)).into();
    }
    let title = text(format!("Bouncing \"{}\"", r.io.bounce_target))
        .size(18)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1)
        .into();
    let pct = (r.io.bounce_fraction * 100.0).round() as u32;
    let caption = if r.io.bounce_cancel_requested {
        format!("{pct}% \u{00b7} cancelling")
    } else {
        format!("{pct}%")
    };
    progress_dialog(
        title,
        "Rendering the full mix offline to a WAV file.",
        r.io.bounce_fraction,
        caption,
        Message::ProjectIo(ProjectIoMessage::CancelBounce),
    )
}

/// The shared blocking progress dialog: dimmed click-swallowing backdrop,
/// centered panel with title / detail / 14 px progress bar / mono caption,
/// and a ghost Cancel that dispatches `cancel_msg`. Bounce and freeze
/// (ba todo #582) render the identical scaffold — only the labels, the
/// progress source, and the cancel message differ.
fn progress_dialog<'a>(
    title: Element<'a, Message>,
    detail: &'a str,
    fraction: f32,
    caption: String,
    cancel_msg: Message,
) -> Element<'a, Message> {
    // Backdrop: an opaque mouse_area that swallows clicks so nothing
    // behind it can be interacted with. No on_press — clicking the
    // backdrop must NOT close the modal (cancel is intentional).
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.7,
                ))),
                ..Default::default()
            }),
    );

    let bar = progress_bar(0.0..=1.0, fraction).girth(Length::Fixed(14.0));

    let cancel = button(text("Cancel").size(13).color(theme::TEXT_1))
        .on_press(cancel_msg)
        .padding([8, 18])
        .style(|_t, status| theme::ghost_button_style(status));

    let dialog_content = column![
        title,
        Space::new().height(8),
        text(detail).size(13).color(theme::TEXT_2),
        Space::new().height(16),
        bar,
        Space::new().height(6),
        text(caption)
            .size(12)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3),
        Space::new().height(20),
        row![Space::new().width(Length::Fill), cancel]
            .align_y(alignment::Vertical::Center),
    ]
    .padding(24)
    .width(420);

    let dialog = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(dialog))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}
