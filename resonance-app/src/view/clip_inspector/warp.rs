//! The clip inspector's WARP section: warp on/off, the source tempo
//! (typed, or found with "Detect"), the stretch algorithm, transpose and
//! the marker count.
//!
//! Every row is always rendered — whether warp is on, a detection is
//! running or the clip has markers only changes what a row says — so the
//! flyout's tree keeps its shape and the tempo field keeps focus across
//! edits (ux-guidelines.md, View Performance rule 4).

use iced::widget::{button, column, row, text, text_input, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::{ClipId, WarpAlgorithm};

use super::{curve_segment_style, numeric_field_style, section_label, unit};
use crate::message::{ClipMessage, ClipWarpMessage, Message};
use crate::state::{
    format_warp_bpm as format_bpm, ClipState, ClipWarpState, TempoDetectStatus,
    MAX_TRANSPOSE_SEMITONES,
};
use crate::theme;

/// One transpose stepper press, in semitones.
const TRANSPOSE_STEP: f32 = 1.0;

fn warp_msg(m: ClipWarpMessage) -> Message {
    Message::Clip(ClipMessage::Warp(m))
}

/// `SetWarp` with the clip's current scalars and `change` applied.
fn set_warp(clip_id: ClipId, warp: &ClipWarpState, change: impl FnOnce(&mut ClipWarpState)) -> Message {
    let mut w = warp.clone();
    change(&mut w);
    warp_msg(ClipWarpMessage::SetWarp {
        clip_id,
        enabled: w.enabled,
        original_bpm: w.original_bpm,
        transpose_semitones: w.transpose_semitones,
        algorithm: w.algorithm,
    })
}

/// A two-way segmented control in the curve picker's style.
fn segments<'a>(items: [(&'a str, bool, Message); 2]) -> Element<'a, Message> {
    let [a, b] = items.map(|(label, active, msg)| -> Element<'a, Message> {
        button(
            text(label)
                .size(10)
                .font(theme::UI_FONT_MEDIUM)
                .align_x(alignment::Horizontal::Center)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding([4, 4])
        .on_press(msg)
        .style(move |_theme, status| curve_segment_style(active, status))
        .into()
    });
    row![a, b].spacing(4).into()
}

impl crate::Resonance {
    /// The WARP section of the editable clip flyout.
    pub(super) fn warp_section<'a>(&'a self, clip: &'a ClipState) -> Element<'a, Message> {
        let id = clip.id;
        let warp = &clip.warp;

        let toggle = segments([
            ("Off", !warp.enabled, set_warp(id, warp, |w| w.enabled = false)),
            ("Warp on", warp.enabled, set_warp(id, warp, |w| w.enabled = true)),
        ]);

        let detect = self.ui.interaction.tempo_detect.get(&id).copied();
        let running = detect == Some(TempoDetectStatus::Running);

        // The tempo field shows the draft while the user types in it, the
        // clip's tempo otherwise; Enter commits.
        let value = match &self.ui.interaction.warp_bpm_draft {
            Some(d) if d.clip_id == id => d.text.clone(),
            _ => warp.original_bpm.map(format_bpm).unwrap_or_default(),
        };
        let field = text_input("—", &value)
            .on_input(move |text| warp_msg(ClipWarpMessage::BpmDraftChanged { clip_id: id, text }))
            .on_submit(warp_msg(ClipWarpMessage::CommitBpmDraft { clip_id: id }))
            .width(Length::Fixed(60.0))
            .size(13)
            .font(theme::MONO_FONT)
            .align_x(alignment::Horizontal::Right)
            .padding([4, 6])
            .style(numeric_field_style);
        let detect_button = button(
            text(if running { "Detecting…" } else { "Detect" })
                .size(11)
                .font(theme::UI_FONT_MEDIUM),
        )
        .padding([3, 7])
        .on_press_maybe((!running).then(|| warp_msg(ClipWarpMessage::DetectTempo { clip_id: id })))
        .style(|_theme, status| theme::small_button_style(status));
        let tempo_row = row![
            text("Tempo").size(12).color(theme::TEXT_2),
            Space::new().width(Length::Fill),
            field,
            unit("BPM"),
            detect_button,
        ]
        .spacing(5)
        .align_y(alignment::Vertical::Center);

        let algorithm = segments([
            (
                "Transient",
                warp.algorithm == WarpAlgorithm::Transient,
                set_warp(id, warp, |w| w.algorithm = WarpAlgorithm::Transient),
            ),
            (
                "Tonal",
                warp.algorithm == WarpAlgorithm::Tonal,
                set_warp(id, warp, |w| w.algorithm = WarpAlgorithm::Tonal),
            ),
        ]);

        let st = warp.transpose_semitones;
        let stepper = |label: &'a str, to: f32| {
            button(text(label).size(11).font(theme::MONO_FONT))
                .padding([3, 6])
                .on_press(set_warp(id, warp, move |w| {
                    w.transpose_semitones = to.clamp(-MAX_TRANSPOSE_SEMITONES, MAX_TRANSPOSE_SEMITONES)
                }))
                .style(|_theme, status| theme::small_button_style(status))
        };
        let transpose_row = row![
            text("Transpose").size(12).color(theme::TEXT_2),
            Space::new().width(Length::Fill),
            stepper("\u{2212}1", st - TRANSPOSE_STEP), // U+2212 MINUS SIGN
            text(format_semitones(st))
                .size(13)
                .font(theme::MONO_FONT)
                .color(theme::WARM)
                .width(Length::Fixed(44.0))
                .align_x(alignment::Horizontal::Center),
            stepper("+1", st + TRANSPOSE_STEP),
            unit("st"),
        ]
        .spacing(5)
        .align_y(alignment::Vertical::Center);

        let marker_count = warp.markers.len();
        let clear = button(text("Clear").size(11).font(theme::UI_FONT_MEDIUM))
            .padding([3, 7])
            .on_press_maybe((marker_count > 0).then(|| {
                warp_msg(ClipWarpMessage::SetWarpMarkers {
                    clip_id: id,
                    markers: Vec::new(),
                })
            }))
            .style(|_theme, status| theme::small_button_style(status));
        let markers_row = row![
            text("Markers").size(12).color(theme::TEXT_2),
            Space::new().width(Length::Fill),
            text(marker_count.to_string())
                .size(13)
                .font(theme::MONO_FONT)
                .color(theme::WARM),
            clear,
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center);

        column![
            section_label("WARP"),
            toggle,
            tempo_row,
            detect_status_row(id, warp, detect),
            algorithm,
            transpose_row,
            markers_row,
            text(
                "Double-click a warped clip's lower edge to add a marker; right-click one to \
                 remove it. Saved with the clip — playback does not stretch yet."
            )
            .size(10)
            .color(theme::TEXT_3),
        ]
        .spacing(7)
        .into()
    }
}

/// The detection status line under the tempo row: empty when nothing was
/// asked, progress while it runs, the result with a "Use" action, or the
/// detector's "no tempo" in `BAD`.
fn detect_status_row<'a>(
    clip_id: ClipId,
    warp: &ClipWarpState,
    status: Option<TempoDetectStatus>,
) -> Element<'a, Message> {
    let (label, color, apply): (String, iced::Color, Option<Message>) = match status {
        None => (String::new(), theme::TEXT_3, None),
        Some(TempoDetectStatus::Running) => {
            ("Listening for the beat…".to_string(), theme::TEXT_3, None)
        }
        Some(TempoDetectStatus::Detected { bpm, confidence }) => (
            format!(
                "Found {} BPM · {:.0}% sure",
                format_bpm(bpm),
                (confidence.clamp(0.0, 1.0) * 100.0)
            ),
            theme::GOOD,
            (warp.original_bpm != Some(bpm)).then(|| set_warp(clip_id, warp, |w| w.original_bpm = Some(bpm))),
        ),
        Some(TempoDetectStatus::NotFound) => {
            ("No steady tempo found".to_string(), theme::BAD, None)
        }
    };
    let use_button = button(text("Use").size(11).font(theme::UI_FONT_MEDIUM))
        .padding([2, 7])
        .on_press_maybe(apply.clone())
        .style(|_theme, status| theme::small_button_style(status));
    let action: Element<'a, Message> = if apply.is_some() {
        use_button.into()
    } else {
        Space::new().width(Length::Shrink).into()
    };
    row![
        text(label).size(11).color(color),
        Space::new().width(Length::Fill),
        action,
    ]
    .spacing(6)
    .height(Length::Fixed(20.0))
    .align_y(alignment::Vertical::Center)
    .into()
}

/// A transpose for display: signed, one decimal only when fractional.
fn format_semitones(st: f32) -> String {
    if st.fract() == 0.0 {
        format!("{st:+.0}")
    } else {
        format!("{st:+.1}")
    }
}
