//! Per-track channel strip (and the collapsed-parent meters and the
//! output-destination picker that hangs off each strip).
//!
//! Each strip is split into a **live** part — the fader/meter block,
//! whose `StereoMeterCanvas` levels tick per frame — and a **lazy** body
//! (head, chips, buttons, FX switch, slot lines, pan) that is
//! cached across redraws behind `iced::widget::lazy`, keyed on
//! [`super::strip_fingerprint`]. The 16 ms redraw tick repaints the app
//! at ~60 Hz; without the split every strip re-ran its `format!`s,
//! button builds and pick_lists on every frame.

use iced::widget::{button, column, container, mouse_area, row, text, text_input, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::*;

use crate::message::*;
use crate::state::*;
use crate::theme::{self, fa};
use crate::view::controls::{
    fader_section, meter_v, monitor_button, mute_button, record_arm_button, solo_button,
};
use crate::view::knob::pan_knob;

use super::strip_parts::InstrumentSlot;

/// Characters of a track name the strip head shows before it ellipsises
/// (size-12 text beside the colour band and glyph in a 160 px strip).
const STRIP_NAME_CHARS: usize = 15;

/// Height of a strip's head (band, glyph, name).
pub(super) const STRIP_HEAD_HEIGHT: f32 = 36.0;

/// Height of a strip's button row.
pub(super) const STRIP_BUTTON_ROW_HEIGHT: f32 = 26.0;

/// Characters of a sub-track's port name its 104 px strip shows.
const SUB_STRIP_NAME_CHARS: usize = 12;

impl crate::Resonance {
    pub(super) fn view_channel_strip<'a>(
        &'a self,
        track: &'a TrackState,
        available_plugins: &'a [ScannedPlugin],
    ) -> Element<'a, Message> {
        // Sub-tracks never reach this function — view_mixer skips them
        // in its outer loop and renders them via view_sub_channel_strip
        // (the slimmer variant) inside their parent's cluster instead.
        debug_assert!(
            track.sub_track.is_none(),
            "view_channel_strip called with a sub-track; use view_sub_channel_strip"
        );

        // Live fader + meter block. `fader_section` embeds the per-tick
        // `StereoMeterCanvas` levels (and the live automated-gain tint),
        // so it is built fresh every frame, OUTSIDE the lazy body below:
        // a lazy subtree captures the levels it was built with and would
        // freeze the meter — the same live/lazy split as the inspector's
        // SIGNAL group (ui-work §11.2).
        let track_id_for_fader = track.id;
        let gain_live = super::automation::live_value(
            &self.automation,
            resonance_common::AutomationTarget::TrackGain(track.id),
        )
        .map(|v| resonance_common::lane_value_to_real(
            &resonance_common::AutomationTarget::TrackGain(track.id),
            v,
        ));
        let fader_block =
            fader_section(track.level_l, track.level_r, track.volume, gain_live, move |v| {
                Message::Track(TrackMessage::SetTrackVolume(track_id_for_fader, v))
            });

        // Everything above the fader only changes on user edits and
        // engine echoes, never per audio tick — lazy-cache it so the
        // ~60 Hz redraw loop reuses the cached subtree instead of
        // rebuilding every strip every frame. The fingerprint must hash
        // everything the body renders (and nothing live) — see
        // `strip_fingerprint.rs`.
        let _ = available_plugins;
        let fp = super::strip_fingerprint::track_strip_fingerprint(self, track);
        let body = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
            self.channel_strip_body(track)
        });

        let has_sub_tracks = self
            .registry
            .tracks
            .iter()
            .any(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id));
        let is_collapsed =
            has_sub_tracks && !self.ui.mixer.expanded_sub_track_parents.contains(&track.id);

        let ext_state = self.devices.external_instruments.get(&track.id);
        let is_selected = self.ui.interaction.selected_track == Some(track.id);
        // A configured external device that's gone offline gives the strip a
        // BAD-pink inset glow so the outage reads at a glance from the mixer
        // (doc #169, todo #459). The route itself is preserved.
        let ext_offline = ext_state
            .map(|e| e.midi_out_offline || e.return_input_offline)
            .unwrap_or(false);
        let bg = if track.record_armed {
            theme::PANEL_ARMED
        } else {
            theme::BG_2
        };
        let border_color = if track.record_armed || ext_offline {
            theme::BAD
        } else if is_selected {
            theme::ACCENT_LINE
        } else {
            theme::LINE_2
        };
        let border_w = if is_selected || track.record_armed || ext_offline {
            1.0
        } else {
            0.5
        };

        let strip_style = move |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                color: border_color,
                width: border_w,
                radius: theme::RADIUS_XL.into(),
            },
            ..Default::default()
        };

        let track_id_for_select = track.id;
        let strip_height = Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32);
        if is_collapsed {
            // Two-column layout: normal controls on the left, compact
            // subtrack meters on the right. The strip widens by 30px per
            // sub-track output so a kit with N pads has every meter
            // visible at a glance instead of crammed into a fixed area.
            let subtrack_count = self
                .registry
                .tracks
                .iter()
                .filter(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id))
                .count() as u32;
            let right_col_w: f32 = ((subtrack_count.max(1) * 30) + 8) as f32;

            let left_col = column![body, fader_block]
                .spacing(6)
                .width(theme::MIXER_STRIP_WIDTH - 20.0)
                .height(Length::Fill);

            let v_sep = container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);

            // The sub-track meters are live level readouts — like the
            // fader block, they render outside the lazy body.
            let right_col = container(self.view_collapsed_subtrack_meters(track.id))
                .width(Length::Fixed(right_col_w))
                .height(Length::Fill)
                .padding([0, 4]);

            let strip_content = row![left_col, v_sep, right_col]
                .height(Length::Fill)
                .padding([12, 10])
                .width(theme::MIXER_STRIP_WIDTH + right_col_w + 20.0);

            mouse_area(
                container(strip_content)
                    .height(strip_height)
                    .style(strip_style),
            )
            .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id_for_select))))
            .into()
        } else {
            let strip_content = column![body, fader_block]
                .spacing(6)
                .padding([12, 10])
                .width(theme::MIXER_STRIP_WIDTH)
                .height(Length::Fill);

            mouse_area(
                container(strip_content)
                    .height(strip_height)
                    .style(strip_style),
            )
            .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id_for_select))))
            .into()
        }
    }

    /// The non-live upper region of a track strip — everything except
    /// the fader/meter block (and, on a collapsed parent, the sub-track
    /// meter column). Built inside the strip's `lazy` region, so it
    /// returns an owned (`'static`) tree and must only read state that
    /// [`super::strip_fingerprint::track_strip_fingerprint`] hashes.
    ///
    /// Anatomy (mixer-cleanup.md §2): head (colour band, glyph, one-line
    /// name), one button row (M / S / ● / 🎧), the FX header switch, the
    /// slot lines, the centred pan knob. Structural edits live in the
    /// inspector.
    fn channel_strip_body(&self, track: &TrackState) -> Element<'static, Message> {
        let has_sub_tracks = self
            .registry
            .tracks
            .iter()
            .any(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id));
        let is_collapsed =
            has_sub_tracks && !self.ui.mixer.expanded_sub_track_parents.contains(&track.id);
        let indent_pixels =
            self.track_groups.indent_depth(track.id) as f32 * theme::GROUP_MEMBER_INDENT;

        // ---- Head: colour band, glyph, one-line name ----
        let glyph_char = match track.track_type {
            TrackType::Audio => fa::MICROPHONE,
            TrackType::Instrument => track.instrument_icon.glyph(),
            TrackType::Vocal => fa::MICROPHONE,
        };
        let head_glyph: Element<'static, Message> = container(
            theme::icon(glyph_char)
                .size(11)
                .color(theme::ACCENT_SOFT),
        )
        .center_x(22)
        .center_y(22)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                radius: theme::RADIUS_SM.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

        let mut head_row = row![]
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .height(28);
        if has_sub_tracks {
            let glyph_caret = if is_collapsed {
                fa::CARET_RIGHT
            } else {
                fa::CARET_DOWN
            };
            let toggle = button(theme::icon(glyph_caret).size(10).color(theme::TEXT_3))
                .on_press(Message::Track(TrackMessage::ToggleSubTracksVisible(
                    track.id,
                )))
                .padding([2, 4])
                .style(|_theme, status| theme::small_button_style(status));
            head_row = head_row.push(toggle);
        }
        head_row = head_row
            .push(super::strip_parts::color_band(theme::track_color(track.color)))
            .push(head_glyph)
            .push(self.strip_name(track));
        // External-instrument tracks carry a lavender `Ext` pill — the
        // at-a-glance "this strip drives outboard gear" cue from design
        // doc #169 — and an `offline` flag while the device is
        // unreachable (todo #459). The route itself is preserved.
        let ext_state = self.devices.external_instruments.get(&track.id);
        if let Some(ext) = ext_state {
            head_row = head_row.push(ext_pill());
            if ext.midi_out_offline || ext.return_input_offline {
                head_row = head_row.push(offline_flag());
            }
        }
        // Fixed heights on the head and button row: inside a Fill-height
        // strip column an unpinned row shares the slack with the slot
        // list instead of hugging its content.
        let head: Element<'static, Message> = container(head_row)
            .width(Length::Fill)
            .height(STRIP_HEAD_HEIGHT)
            .padding(iced::Padding {
                top: 4.0,
                right: 0.0,
                bottom: 4.0,
                left: indent_pixels,
            })
            .into();

        // ---- One button row: M / S / ● / 🎧 ----
        let button_row: Element<'static, Message> = container(
            row![
                mute_button(
                    track.muted,
                    Message::Track(TrackMessage::ToggleMute(track.id)),
                    12
                ),
                solo_button(
                    track.soloed,
                    Message::Track(TrackMessage::ToggleSolo(track.id)),
                    12
                ),
                record_arm_button(track.record_armed, track.id, 12),
                monitor_button(track.monitor_enabled, track.id, 12),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center),
        )
        .width(Length::Fill)
        .height(STRIP_BUTTON_ROW_HEIGHT)
        .center_x(Length::Fill)
        .center_y(STRIP_BUTTON_ROW_HEIGHT)
        .into();

        // ---- FX header switch + slot lines ----
        let fx_header = super::strip_parts::fx_header(
            track.fx_bypassed,
            Message::Track(TrackMessage::ToggleTrackFxBypass(track.id)),
        );
        // Slot 0 of a (non-external) instrument track is the instrument:
        // fixed above the scrolling effects, accent with a hairline under
        // it. External instruments run every plugin as an insert over the
        // audio return, so their whole chain is effects — the same rule
        // the inspector's CHAIN group applies.
        let is_instrument_chain =
            track.track_type == TrackType::Instrument && ext_state.is_none();
        let (instrument, effects) = if is_instrument_chain {
            match track.plugins.split_first() {
                Some((first, rest)) => (InstrumentSlot::Filled(first), rest),
                None => (InstrumentSlot::Empty(track.id), &track.plugins[..]),
            }
        } else {
            (InstrumentSlot::None, &track.plugins[..])
        };
        let slots = super::strip_parts::slot_list(
            instrument,
            effects,
            track.fx_bypassed,
            self.ui.mixer.focused_slot,
        );

        // ---- Centred pan: knob, value under it ----
        // Tinted with the live automated pan while a Read-enabled pan
        // lane drives it during playback. The tint value is hashed into
        // the strip fingerprint, so the body rebuilds exactly when it
        // moves.
        let id = track.id;
        let pan_live = super::automation::live_value(
            &self.automation,
            resonance_common::AutomationTarget::TrackPan(track.id),
        )
        .map(|v| resonance_common::lane_value_to_real(
            &resonance_common::AutomationTarget::TrackPan(track.id),
            v,
        ));
        let pan_ctrl = crate::view::knob::pan_knob_automated(track.pan, pan_live, move |v| {
            Message::Track(TrackMessage::SetTrackPan(id, v))
        });
        let pan = super::strip_parts::pan_block(pan_ctrl, track.pan);

        let mut body_col = column![head]
            .spacing(6)
            .width(Length::Fill)
            .height(Length::Fill);
        if let Some(ext) = ext_state {
            body_col = body_col.push(ext_summary_chips(track, ext));
        }
        body_col
            .push(button_row)
            .push(fx_header)
            .push(slots)
            .push(pan)
            .into()
    }

    /// The strip head's name: one line, ellipsised and clipped (never
    /// wraps). A double-click swaps it for the inline rename field
    /// (mixer-cleanup.md §2.3); while that is open for this track, the
    /// field is drawn instead.
    fn strip_name(&self, track: &TrackState) -> Element<'static, Message> {
        if let Some((_, buffer)) = self
            .ui
            .mixer
            .renaming
            .as_ref()
            .filter(|(id, _)| *id == track.id)
        {
            return text_input("Track name", buffer)
                .id(crate::update::strip_rename::input_id())
                .on_input(|s| Message::Ui(UiMessage::StripRenameInput(s)))
                .on_submit(Message::Ui(UiMessage::CommitStripRename))
                .size(12)
                .padding([2, 4])
                .width(Length::Fill)
                .into();
        }
        // Truncate first, then clip in a width-Fill container:
        // `Wrapping::None` alone isn't enough when the parent has a
        // finite width.
        let name = container(
            text(crate::util::short(&track.name, STRIP_NAME_CHARS))
                .size(12)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .width(Length::Fill)
        .clip(true);
        // The name's own mouse area takes the press (it has to, to see a
        // double-click), so it selects the track itself, as the strip
        // around it would.
        mouse_area(name)
            .on_press(Message::Ui(UiMessage::SelectTrack(Some(track.id))))
            .on_double_click(Message::Ui(UiMessage::BeginStripRename(track.id)))
            .into()
    }

    /// Dedicated narrow strip for an expanded sub-track. Mixer-mod.rs
    /// emits one of these immediately after the parent strip for each
    /// sub-track of an expanded parent, so the cluster (parent + its
    /// sub-strips) reads as a coherent group.
    ///
    /// Visual contract:
    /// - `MIXER_SUB_STRIP_WIDTH`-wide (narrower than the parent strip)
    ///   and `MIXER_STRIP_HEIGHT` tall (same as parent — fader bottoms
    ///   line up).
    /// - Background = `MIXER_SUB_STRIP_BG` (one step darker than the
    ///   parent strip's `BG_2`) so the recessed shade signals "child".
    /// - 2 px lavender left-edge rail (`MIXER_SUB_STRIP_RAIL`,
    ///   saturating to `_SELECTED` when the sub-track is the
    ///   selected track) — the at-a-glance parent → child cue.
    /// - Slimmer control set (mixer-cleanup.md §2.4): one-line name,
    ///   M / S, the FX switch, centred pan, fader. No record-arm or
    ///   monitor (sub-tracks are fed from the parent plugin's fan-out,
    ///   never from a hardware input) and no slot lines.
    pub(super) fn view_sub_channel_strip<'a>(
        &'a self,
        track: &'a TrackState,
        _available_plugins: &'a [ScannedPlugin],
    ) -> Element<'a, Message> {
        debug_assert!(
            track.sub_track.is_some(),
            "view_sub_channel_strip called with a non-sub-track"
        );

        let track_id_for_fader = track.id;
        // Sub-track strips are slim children fed from the parent plugin's
        // fan-out; they carry no automation lane header. Live gain tint
        // still applies so a Read-enabled lane on the sub-track shows.
        // Live meter levels tick per frame, so the fader/meter block is
        // built outside the lazy body below.
        let gain_live = super::automation::live_value(
            &self.automation,
            resonance_common::AutomationTarget::TrackGain(track.id),
        )
        .map(|v| resonance_common::lane_value_to_real(
            &resonance_common::AutomationTarget::TrackGain(track.id),
            v,
        ));
        let fader_block =
            fader_section(track.level_l, track.level_r, track.volume, gain_live, move |v| {
                Message::Track(TrackMessage::SetTrackVolume(track_id_for_fader, v))
            });

        // The head / M-S / FX switch / pan block are non-live — cache
        // them behind `lazy` keyed on the slim sub-strip fingerprint.
        let fp = super::strip_fingerprint::sub_strip_fingerprint(track);
        let body_top = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
            sub_channel_strip_body(track)
        });

        let is_selected = self.ui.interaction.selected_track == Some(track.id);

        // Left-edge accent rail. A thin colored column the full strip
        // height — sits flush against the left edge so the eye reads a
        // visual tie to the parent strip on its left. Saturates to the
        // full lavender when the sub-track is selected.
        let rail_color = if is_selected {
            theme::ACCENT
        } else {
            theme::MIXER_SUB_STRIP_RAIL
        };
        let rail = container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fixed(theme::MIXER_SUB_STRIP_RAIL_WIDTH))
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(rail_color)),
                ..Default::default()
            });

        // Body content sits to the right of the rail. Spacer pads the
        // top a bit so the head label aligns roughly with the parent
        // strip's name row.
        // Same vertical rhythm as the parent strip (12 px top/bottom,
        // 6 px between the body and the fader), so the heads, button
        // rows, pan knobs and faders of a cluster line up.
        let body = column![body_top, fader_block]
            .spacing(6)
            .padding([12, 0])
            .width(Length::Fill)
            .height(Length::Fill);

        let border_color = if is_selected {
            theme::ACCENT_LINE
        } else {
            theme::LINE_2
        };
        let border_w = if is_selected { 1.0 } else { 0.5 };
        let strip_style = move |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(theme::MIXER_SUB_STRIP_BG)),
            border: iced::Border {
                color: border_color,
                width: border_w,
                // Slightly tighter corner radius than the parent strip
                // so the recessed shape reads as nested rather than as
                // a peer card.
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        };

        let inner = row![rail, body]
            .spacing(0)
            .height(Length::Fill)
            .width(Length::Fill);

        let track_id_for_select = track.id;
        let strip_height = Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32);
        mouse_area(
            container(inner)
                .width(Length::Fixed(theme::MIXER_SUB_STRIP_WIDTH))
                .height(strip_height)
                .style(strip_style),
        )
        .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id_for_select))))
        .into()
    }

    /// Compact subtrack meters shown in the right half of a collapsed
    /// parent strip. Each subtrack gets a name label and stereo meter bars.
    fn view_collapsed_subtrack_meters(&self, parent_id: TrackId) -> Element<'_, Message> {
        let mut subtracks: Vec<&TrackState> = self
            .registry
            .tracks
            .iter()
            .filter(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == parent_id))
            .collect();
        subtracks.sort_by_key(|t| t.order);

        // Each meter column gets a fixed width so labels never wrap into
        // their glyph cells. 28px fits "Snare" / "Hi-Hat" abbreviated to
        // four characters at size 9.
        const COL_W: u16 = 28;
        let mut meters_row = row![].spacing(2);
        for sub in &subtracks {
            // Show the port label (after "→") rather than the full name,
            // so "Instrument 2 → Kick" displays as "Kick" instead of "Inst.".
            let short_name = sub.name.split(" \u{2192} ").nth(1).unwrap_or(&sub.name);
            let label = crate::util::short(short_name, 5);
            let name_label = text(label)
                .size(9)
                .color(theme::TEXT_3)
                .wrapping(iced::widget::text::Wrapping::None);
            let meter = meter_v(sub.level_l, sub.level_r, theme::FADER_HEIGHT);
            let col = column![
                container(name_label)
                    .width(Length::Fill)
                    .center_x(Length::Fill)
                    .clip(true),
                container(meter).width(Length::Fill).center_x(Length::Fill),
            ]
            .spacing(2)
            .width(Length::Fixed(COL_W as f32))
            .align_x(alignment::Horizontal::Center);
            meters_row = meters_row.push(col);
        }

        let title = text(format!("{} outs", subtracks.len()))
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3);

        // The meters sit at the bottom of the column so they line up with
        // the parent strip's fader. Bottom offset matches the fader-label
        // band so both columns end on the same baseline.
        column![
            title,
            Space::new().height(Length::Fill),
            meters_row,
            Space::new().height(20),
        ]
        .spacing(2)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

}

/// The non-live upper region of a sub-track strip (mixer-cleanup.md
/// §2.4): one-line name, M / S, the FX switch, the centred pan. Built
/// inside the sub-strip's `lazy` region, so it returns an owned
/// (`'static`) tree and must only read state that
/// [`super::strip_fingerprint::sub_strip_fingerprint`] hashes.
fn sub_channel_strip_body(track: &TrackState) -> Element<'static, Message> {
    // Show the port label (after "→") rather than the full name —
    // "Drums → Kick" becomes "Kick", which fits the narrower strip.
    let short_name = track.name.split(" \u{2192} ").nth(1).unwrap_or(&track.name);
    let name_text = container(
        text(crate::util::short(short_name, SUB_STRIP_NAME_CHARS))
            .size(11)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_2)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .width(Length::Fill)
    .clip(true);
    let head: Element<'static, Message> = container(
        row![name_text]
            .align_y(alignment::Vertical::Center)
            .height(28),
    )
    .width(Length::Fill)
    .height(STRIP_HEAD_HEIGHT)
    .padding([4, 8])
    .into();

    // M / S — record-arm and monitor are intentionally omitted:
    // sub-tracks are fed from the parent plugin's fan-out, never from an
    // input.
    let button_row = container(
        row![
            mute_button(
                track.muted,
                Message::Track(TrackMessage::ToggleMute(track.id)),
                11
            ),
            solo_button(
                track.soloed,
                Message::Track(TrackMessage::ToggleSolo(track.id)),
                11
            ),
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .height(STRIP_BUTTON_ROW_HEIGHT)
    .center_x(Length::Fill)
    .center_y(STRIP_BUTTON_ROW_HEIGHT);

    let fx_header = container(super::strip_parts::fx_header(
        track.fx_bypassed,
        Message::Track(TrackMessage::ToggleTrackFxBypass(track.id)),
    ))
    .padding([0, 8]);

    let id = track.id;
    let pan_ctrl = pan_knob(track.pan, move |v| {
        Message::Track(TrackMessage::SetTrackPan(id, v))
    });
    let pan = super::strip_parts::pan_block(pan_ctrl, track.pan);

    // The Fill spacer lives INSIDE this lazy body so the body column is
    // Fill-height and the pan knob sits on the fader, as on the parent.
    column![
        head,
        button_row,
        fx_header,
        Space::new().height(Length::Fill),
        pan,
    ]
    .spacing(6)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Lavender `Ext` pill shown in the strip head of an external-instrument
/// track — the MIDI-domain accent tag from design doc #169 (mirrors the
/// inspector's `External` badge).
fn ext_pill() -> Element<'static, Message> {
    container(
        text("Ext")
            .size(8)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::ACCENT_SOFT),
    )
    .padding([2, 5])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::ACCENT_DIM)),
        border: iced::Border {
            color: theme::ACCENT_LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Small BAD-pink `offline` flag shown in the strip head when a configured
/// external device is unreachable (doc #169, todo #459). The route is kept;
/// this is the at-a-glance outage marker beside the `Ext` pill.
fn offline_flag() -> Element<'static, Message> {
    container(
        text("offline")
            .size(8)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::BAD),
    )
    .padding([2, 5])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BAD_DIM)),
        border: iced::Border {
            color: theme::BAD_LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// The three external-instrument summary chips under the strip head:
/// **MIDI** (device · channel + activity dot), **Return** (input device ·
/// channels) and **Patch** (bank/program). Pure function of `TrackState`
/// (MIDI out / audio return live there) plus the external config — the
/// same single source of truth the inspector reads, so the two surfaces
/// can never disagree.
fn ext_summary_chips(track: &TrackState, ext: &ExternalInstrumentState) -> Element<'static, Message> {
    // MIDI — device · channel. While the device is offline the route is
    // preserved (still shown) but flagged, mirroring the inspector.
    let midi_value = match track.midi_output_device.as_deref() {
        Some(dev) if ext.midi_out_offline => {
            format!("{} \u{b7} offline", crate::util::short(dev, 9))
        }
        Some(dev) => {
            let ch = u16::from(track.midi_output_channel.unwrap_or(0)) + 1;
            format!("{} \u{b7} Ch {}", crate::util::short(dev, 9), ch)
        }
        None => "\u{2014}".to_string(),
    };
    // Activity dot — the design's pulsing MIDI indicator. #454 carries no
    // transient MIDI-activity state, so the dot reflects the derived
    // lifecycle status statically (bright when live, faint when idle,
    // BAD-pink when the device is offline) rather than animating.
    let dot_color = match ext.status(track) {
        ExternalInstrumentStatus::Offline => theme::BAD,
        ExternalInstrumentStatus::Live => theme::ACCENT,
        _ => iced::Color {
            a: 0.35,
            ..theme::ACCENT
        },
    };
    let midi_chip = strip_chip("MIDI", midi_value, false, Some(dot_color));

    // Return — input device · port label ("In N/N+1"), reusing the
    // inspector's `PortChoice` formatting.
    let return_value = match track.input_device_name.as_deref() {
        Some(dev) => {
            let port = super::picks::PortChoice {
                index: track.input_port_index,
                mono: track.mono,
            };
            format!("{} {}", crate::util::short(dev, 8), port)
        }
        None => "\u{2014}".to_string(),
    };
    let return_chip = strip_chip("Return", return_value, false, None);

    // Patch — bank/program by number (named patches arrive with the
    // device-preset epic #40).
    let patch_value = match (ext.bank, ext.program) {
        (Some(bank), Some(program)) => format!("Bank {} \u{b7} Prog {}", bank, program),
        (Some(bank), None) => format!("Bank {}", bank),
        (None, Some(program)) => format!("Prog {}", program),
        (None, None) => "not set".to_string(),
    };
    let patch_chip = strip_chip("Patch", patch_value, true, None);

    column![midi_chip, return_chip, patch_chip]
        .spacing(5)
        .width(Length::Fill)
        .into()
}

/// One summary chip: a fixed-width uppercase key, an ellipsised value, and
/// an optional trailing dot (the MIDI-activity indicator). `value_accent`
/// tints the value lavender for the Patch chip.
fn strip_chip(
    key: &'static str,
    value: String,
    value_accent: bool,
    dot: Option<iced::Color>,
) -> Element<'static, Message> {
    let value_color = if value_accent {
        theme::ACCENT_SOFT
    } else {
        theme::TEXT_1
    };
    let mut inner = row![
        container(
            text(key)
                .size(8)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
        )
        .width(Length::Fixed(30.0)),
        container(
            text(value)
                .size(10)
                .color(value_color)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);
    if let Some(color) = dot {
        inner = inner.push(
            container(Space::new().width(7).height(7)).style(move |_theme| container::Style {
                background: Some(iced::Background::Color(color)),
                border: iced::Border {
                    radius: 999.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
    }
    container(inner)
        .width(Length::Fill)
        .padding([5, 7])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_XS.into(),
            },
            ..Default::default()
        })
        .into()
}
