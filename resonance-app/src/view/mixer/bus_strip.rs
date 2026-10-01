//! Bus channel strip rendering: the track strip's anatomy
//! (mixer-cleanup.md §5) minus the per-track parts — no solo, arm or
//! monitor, no instrument slot, no input or output picker (busses always
//! go to master).
//!
//! Like the track strip, the bus strip splits into a live fader/meter
//! block (rebuilt every frame) and a lazy body cached behind
//! `iced::widget::lazy`, keyed on
//! [`super::strip_fingerprint::bus_strip_fingerprint`].

use iced::widget::{column, container, row, text};
use iced::{alignment, Element, Length};
use resonance_audio::types::*;

use crate::message::*;
use crate::state::*;
use crate::theme;
use crate::view::controls::{fader_section, mute_button};

use super::strip_parts::InstrumentSlot;

/// Characters of a bus name the strip head shows before it ellipsises
/// (size-12 text between the band and the mute button).
const BUS_NAME_CHARS: usize = 16;

impl crate::Resonance {
    pub(super) fn view_bus_strip<'a>(
        &'a self,
        bus: &'a BusState,
        available_plugins: &'a [ScannedPlugin],
    ) -> Element<'a, Message> {
        let _ = available_plugins;
        let bus_id = bus.id;

        // Live fader + meter block — the per-tick levels (and the live
        // automated-gain tint) render outside the lazy body, exactly as
        // on the track strip: a lazy subtree would freeze the meter at
        // the levels it was built with.
        let gain_live = super::automation::live_value(
            &self.automation,
            resonance_common::AutomationTarget::BusGain(bus.id),
        )
        .map(|v| resonance_common::lane_value_to_real(
            &resonance_common::AutomationTarget::BusGain(bus.id),
            v,
        ));
        let fader_block =
            fader_section(bus.level_l, bus.level_r, bus.volume, gain_live, move |v| {
                Message::Bus(BusMessage::SetBusVolume(bus_id, v))
            });

        // Everything above the fader is non-live — cache it across
        // redraw ticks.
        let fp = super::strip_fingerprint::bus_strip_fingerprint(self, bus);
        let body = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
            self.bus_strip_body(bus)
        });

        let strip_content = column![body, fader_block]
            .spacing(6)
            .padding([12, 10])
            .width(theme::MIXER_STRIP_WIDTH)
            .height(Length::Fill);

        // Clicking anywhere on the strip that isn't already a control
        // selects the bus, exactly as a track strip does — that is what
        // gives the inspector something to describe. `mouse_area` sits
        // *outside* the container so the inner buttons, pickers, knob and
        // fader keep their own press handling.
        let is_selected = self.ui.mixer.selected_bus == Some(bus_id);
        iced::widget::mouse_area(
            container(strip_content)
                .height(Length::Fixed(theme::BUS_STRIP_HEIGHT as f32))
                // Selected saturates the strip's resting warm hairline to
                // the full warm accent — the same dim-to-saturated move a
                // selected sub-track's rail makes, in the bus palette
                // rather than the track one.
                .style(move |theme| {
                    let base = theme::card_warm(theme);
                    if is_selected {
                        container::Style {
                            border: iced::Border {
                                color: theme::WARM,
                                ..base.border
                            },
                            ..base
                        }
                    } else {
                        base
                    }
                }),
        )
        .on_press(Message::Ui(UiMessage::SelectBus(Some(bus_id))))
        .into()
    }

    /// The non-live upper region of a bus strip — everything except the
    /// fader/meter block. Built inside the strip's `lazy` region, so it
    /// returns an owned (`'static`) tree and must only read state that
    /// [`super::strip_fingerprint::bus_strip_fingerprint`] hashes.
    ///
    /// The track anatomy (mixer-cleanup.md §5): head (with mute), the FX
    /// switch, slot lines, the centred pan. Adding effects and deleting
    /// the bus live in the bus inspector. A double-click on the name
    /// renames the bus in place, as on a track strip.
    fn bus_strip_body(&self, bus: &BusState) -> Element<'static, Message> {
        let bus_id = bus.id;

        // Head: a warm band (busses carry the audio-domain amber, not a
        // per-track colour) and the one-line name — or, while a rename
        // is open on this strip, the rename field (mixer-cleanup.md §2.3).
        let target = RenameTarget::Bus(bus_id);
        let name: Element<'static, Message> =
            match self.ui.mixer.rename_buffer(target, RenameSurface::Strip) {
                Some(buffer) => super::strip_parts::rename_field("Bus name", buffer, 12.0),
                None => {
                    let name = container(
                        text(crate::util::short(&bus.name, BUS_NAME_CHARS))
                            .size(12)
                            .font(theme::UI_FONT_MEDIUM)
                            .color(theme::WARM)
                            .wrapping(iced::widget::text::Wrapping::None),
                    )
                    .width(Length::Fill)
                    .clip(true);
                    // The name's own mouse area takes the press (it has
                    // to, to see a double-click), so it selects the bus
                    // itself, as the strip around it would.
                    iced::widget::mouse_area(name)
                        .on_press(Message::Ui(UiMessage::SelectBus(Some(bus_id))))
                        .on_double_click(Message::Ui(UiMessage::BeginRename(
                            target,
                            RenameSurface::Strip,
                        )))
                        .into()
                }
            };
        // A bus has one live button (mute: no solo, arm or monitor), so
        // it rides at the right of the head instead of taking a row of
        // its own — the bus lane is 120 px shorter than the track lane,
        // and that row is what lets the slot lines breathe.
        let head = container(
            row![
                super::strip_parts::color_band(theme::WARM),
                name,
                // `icon_button` cells claim Fill width; pin this one so
                // the name keeps the rest of the head.
                container(mute_button(
                    bus.muted,
                    Message::Bus(BusMessage::ToggleBusMute(bus_id)),
                    12,
                ))
                .width(22),
            ]
            .spacing(4)
            .align_y(alignment::Vertical::Center)
            .height(28),
        )
        .width(Length::Fill)
        .height(super::track_strip::STRIP_HEAD_HEIGHT)
        .padding([4, 0]);

        let fx_header = super::strip_parts::fx_header(
            bus.fx_bypassed,
            Message::Bus(BusMessage::ToggleBusFxBypass(bus_id)),
        );
        let slots = super::strip_parts::slot_list(
            InstrumentSlot::None,
            &bus.plugins,
            bus.fx_bypassed,
            self.ui.mixer.focused_slot,
        );

        // Pan knob — vertical drag to change, double-click to reset. The
        // live automated-pan tint is hashed into the strip fingerprint,
        // so the body rebuilds exactly when it moves.
        let pan_live = super::automation::live_value(
            &self.automation,
            resonance_common::AutomationTarget::BusPan(bus.id),
        )
        .map(|v| resonance_common::lane_value_to_real(
            &resonance_common::AutomationTarget::BusPan(bus.id),
            v,
        ));
        let pan_ctrl = crate::view::knob::pan_knob_automated(bus.pan, pan_live, move |v| {
            Message::Bus(BusMessage::SetBusPan(bus_id, v))
        });
        let pan = super::strip_parts::pan_block(pan_ctrl, bus.pan);

        column![head, fx_header, slots, pan]
            .spacing(6)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
