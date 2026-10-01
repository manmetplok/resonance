//! Master channel strip rendering, on the shared strip anatomy
//! (mixer-cleanup.md §5): head, the FX switch, slot lines, the fader. No
//! instrument, no input/arm, no per-channel routing, and no pan. Adding
//! effects, automation and Bounce live in the master inspector; a click
//! on the strip selects the master.
//!
//! Split like the other strips: the fader/meter block (live per-tick
//! levels) rebuilds every frame, while the body above the fader is
//! cached behind `iced::widget::lazy`, keyed on
//! [`super::strip_fingerprint::master_strip_fingerprint`].

use iced::widget::{column, container, row, text};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme;
use crate::view::controls::fader_section;

use super::strip_parts::InstrumentSlot;

impl crate::Resonance {
    pub(super) fn view_master_strip(&self) -> Element<'_, Message> {
        // Master automation live gain tint + the fader/meter block. The
        // per-tick levels render outside the lazy body, like every strip.
        let gain_live = super::automation::live_value(
            &self.automation,
            resonance_common::AutomationTarget::MasterGain,
        )
        .map(|v| resonance_common::lane_value_to_real(
            &resonance_common::AutomationTarget::MasterGain,
            v,
        ));
        let fader_block = fader_section(
            self.master.level_l,
            self.master.level_r,
            self.master.volume,
            gain_live,
            |v| Message::Track(TrackMessage::SetMasterVolume(v)),
        );

        // Head, FX switch and slot lines — non-live, cached across
        // redraw ticks.
        let fp = super::strip_fingerprint::master_strip_fingerprint(self);
        let body = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
            self.master_strip_body()
        });

        let strip_content = column![body, fader_block]
            .spacing(6)
            .padding([12, 10])
            .width(theme::MASTER_STRIP_WIDTH)
            .height(Length::Fill);

        // Selected reads like a selected track strip: the lavender
        // hairline saturates and thickens. Unselected, the master sits
        // in the strips' resting card.
        let (border_color, border_width) = master_strip_border(self.ui.mixer.selected_master);
        let style = move |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: border_color,
                width: border_width,
                radius: theme::RADIUS_XL.into(),
            },
            ..Default::default()
        };

        // Clicking the strip (anywhere a control doesn't take the press)
        // selects the master, so the inspector describes it — the same
        // outside-the-container `mouse_area` the track and bus strips use.
        iced::widget::mouse_area(
            container(strip_content)
                .height(Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32))
                .style(style),
        )
        .on_press(Message::Ui(UiMessage::SelectMaster))
        .into()
    }

    /// The non-live upper region of the master strip — everything above
    /// the fader/meter block. Built inside the strip's `lazy` region, so
    /// it returns an owned (`'static`) tree and must only read state
    /// that [`super::strip_fingerprint::master_strip_fingerprint`]
    /// hashes.
    fn master_strip_body(&self) -> Element<'static, Message> {
        let head = container(
            row![
                super::strip_parts::color_band(theme::TEXT_2),
                text("MASTER")
                    .size(11)
                    .font(theme::UI_FONT_SEMIBOLD)
                    .color(theme::TEXT_1),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .height(28),
        )
        .width(Length::Fill)
        .height(super::track_strip::STRIP_HEAD_HEIGHT)
        .padding([4, 0]);

        let fx_header = super::strip_parts::fx_header(
            self.master.fx_bypassed,
            Message::Master(MasterMessage::ToggleMasterFxBypass),
        );
        let slots = super::strip_parts::slot_list(
            InstrumentSlot::None,
            &self.master.plugins,
            self.master.fx_bypassed,
            self.ui.mixer.focused_slot,
            theme::MIXER_SLOT_LINE_CHARS,
        );

        column![head, fx_header, slots]
            .spacing(6)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// The master strip card's border: the selected-track treatment (a 1 px
/// lavender hairline) while the master is selected, the strips' resting
/// 0.5 px hairline otherwise.
pub(crate) fn master_strip_border(selected: bool) -> (iced::Color, f32) {
    if selected {
        (theme::ACCENT_LINE, 1.0)
    } else {
        (theme::LINE_2, 0.5)
    }
}
