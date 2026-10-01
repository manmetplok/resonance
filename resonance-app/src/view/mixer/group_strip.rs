//! Mixer reflection of track groups (epic #36, doc #200 — "Mixer
//! reflection"). A group reads in the mixer as a **coloured cluster**: a
//! leading group-header strip (caret · identity swatch · name · `N trk`
//! count · macro level / mute / solo) followed by its member channel
//! strips, all wrapped by the group's identity colour — a left identity
//! rail plus a faint colour wash — so the cluster reads the same way it
//! does in the Arrange view's coloured rail/swatch.
//!
//! This mirrors the existing parent + sub-track clustering
//! (`expanded_sub_track_parents`): members render flush (0 px gap) and the
//! whole cluster sits in the outer row's `MIXER_STRIP_GAP` rhythm. Group
//! fold state (`is_collapsed`, persisted in the project) hides the member
//! strips, leaving just the header — the mixer analogue of the Arrange
//! consolidated-overview lane.
//!
//! The macro controls dispatch the same [`GroupMessage`]s as the Arrange
//! group header (todo #680), so the two surfaces stay a single source of
//! truth for group mute / solo / level.

use iced::widget::{column, container, mouse_area, row, text, vertical_slider, Space};
use iced::{alignment, Element, Length, Padding};
use resonance_audio::types::TrackId;
use resonance_common::track_group::TrackGroup;

use crate::message::*;
use crate::state::TrackState;
use crate::theme;
use crate::view::controls::{collapse_caret, mute_button, solo_button};

/// One top-level slot in the mixer's track-strip lane: either a standalone
/// (ungrouped) track or a whole group cluster. Computing this order in one
/// pure pass keeps the displayed sequence testable without rendering pixels
/// (see `tests/mixer_group_clustering.rs`) and shared with the renderer so
/// the two can't drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MixerTopItem {
    /// An ungrouped top-level track. Its expanded sub-tracks cluster with
    /// it in the view (handled by `view_track_cluster`).
    Track(TrackId),
    /// A root group, rendered as a coloured cluster (header + members).
    Group(TrackId),
}

impl crate::Resonance {
    /// Plan the mixer's top-level strip order: walk `sorted_tracks` and,
    /// the first time a grouped track is met, emit its **root** group as a
    /// single cluster (consuming every member, including nested ones) so
    /// the cluster lands at the position of its first member and members
    /// never appear standalone elsewhere. Ungrouped tracks keep their
    /// sorted position. Sub-tracks are skipped here — they cluster with
    /// their parent inside `view_track_cluster`.
    pub(crate) fn mixer_top_level_items(&self) -> Vec<MixerTopItem> {
        let mut items = Vec::new();
        let mut consumed: std::collections::HashSet<TrackId> = std::collections::HashSet::new();
        for track in self.sorted_tracks() {
            if track.sub_track.is_some() || consumed.contains(&track.id) {
                continue;
            }
            if let Some(group) = self.mixer_root_group_of(track.id) {
                consumed.insert(track.id);
                for member in self.track_groups.get_all_member_ids(group.id) {
                    consumed.insert(member);
                }
                items.push(MixerTopItem::Group(group.id));
            } else {
                consumed.insert(track.id);
                items.push(MixerTopItem::Track(track.id));
            }
        }
        items
    }

    /// The outermost (root) group a track belongs to, walking up any one
    /// level of nesting. `None` for an ungrouped track.
    pub(crate) fn mixer_root_group_of(&self, track_id: TrackId) -> Option<&TrackGroup> {
        let direct = self.track_groups.group_of_member(track_id)?;
        let mut group = self.track_groups.get_group(direct)?;
        while let Some(parent_id) = group.nesting_parent {
            match self.track_groups.get_group(parent_id) {
                Some(parent) => group = parent,
                None => break,
            }
        }
        Some(group)
    }

    /// Render a whole group as a coloured cluster: the group-header strip,
    /// then each member's channel strip (nested groups render inline as
    /// their own sub-cluster). Collapsed groups render the header only.
    pub(super) fn view_mixer_group_cluster<'a>(
        &'a self,
        group: &'a TrackGroup,
        sorted_tracks: &'a [TrackState],
    ) -> Element<'a, Message> {
        let (base, wash, _line) = theme::group_identity_colors(group.identity_color);
        let member_count = self.track_groups.get_all_member_ids(group.id).len();

        let mut cluster = row![self.view_mixer_group_header(group, member_count)].spacing(0);
        if !group.is_collapsed {
            for &member_id in &group.ordered_members {
                if let Some(nested) = self.track_groups.get_group(member_id) {
                    // A nested group: render its own sub-cluster inline.
                    cluster = cluster.push(self.view_mixer_group_cluster(nested, sorted_tracks));
                } else if let Some(member) = sorted_tracks
                    .iter()
                    .find(|t| t.id == member_id && t.sub_track.is_none())
                {
                    cluster = cluster.push(self.view_track_cluster(member, sorted_tracks));
                }
            }
        }

        // Identity rail down the left edge + a faint colour wash behind the
        // cluster — the same identity cue the Arrange rail/swatch carry.
        let rail = container(Space::new().width(theme::GROUP_RAIL_WIDTH).height(Length::Fill))
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(base)),
                ..Default::default()
            });
        let body = row![rail, cluster].spacing(0);
        container(body)
            .height(Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32))
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(wash)),
                ..Default::default()
            })
            .into()
    }

    /// The group-header strip: caret · swatch · name · count badge stacked
    /// above the macro level fader and the macro M / S buttons, over the
    /// group-colour wash. The mixer analogue of the Arrange group-header row
    /// (todo #680) — same controls, vertical strip layout.
    fn view_mixer_group_header(
        &self,
        group: &TrackGroup,
        member_count: usize,
    ) -> Element<'_, Message> {
        let group_id = group.id;
        let (base, wash, line) = theme::group_identity_colors(group.identity_color);

        // Caret folds the cluster (▾ expanded / ▸ collapsed).
        let caret = mouse_area(collapse_caret(!group.is_collapsed))
            .on_press(Message::Group(GroupMessage::ToggleCollapse(group_id)));

        let swatch = container(Space::new())
            .width(theme::GROUP_SWATCH_SIZE)
            .height(theme::GROUP_SWATCH_SIZE)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(base)),
                border: iced::Border {
                    color: line,
                    width: 1.0,
                    radius: theme::RADIUS_XS.into(),
                },
                ..Default::default()
            });

        let name = container(
            text(group.name.clone())
                .size(13)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_1)
                .wrapping(text::Wrapping::None),
        )
        .clip(true)
        .width(Length::Fill);

        let title_row = row![caret, Space::new().width(5), swatch, Space::new().width(5), name]
            .align_y(alignment::Vertical::Center);

        let count_badge = container(
            text(format!("{member_count} trk"))
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_3),
        )
        .padding(Padding {
            top: 1.0,
            right: 5.0,
            bottom: 1.0,
            left: 5.0,
        })
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_PILL.into(),
            },
            ..Default::default()
        });

        // Macro level trim: a vertical fader (0.0..=2.0 around unity) + dB
        // readout, mirroring the channel-strip fader so it reads as a level
        // control. The trim scales members' contribution; it does not touch
        // their own faders.
        let level = group.macro_level;
        let level_fader = vertical_slider(0.0..=2.0f32, level, move |v| {
            Message::Group(GroupMessage::SetMacroLevel(group_id, v))
        })
        .height(theme::FADER_HEIGHT)
        .step(0.01f32);
        let db_text = if level <= 0.0001 {
            "\u{2212}\u{221e}".to_string() // −∞
        } else {
            format!("{:.1}", 20.0 * level.log10())
        };
        let level_block = column![
            container(level_fader).width(Length::Fill).center_x(Length::Fill),
            text(db_text).size(9).font(theme::MONO_FONT).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_x(alignment::Horizontal::Center);

        let ms_row = row![
            mute_button(
                group.macro_mute,
                Message::Group(GroupMessage::ToggleMacroMute(group_id)),
                12,
            ),
            solo_button(
                group.macro_solo,
                Message::Group(GroupMessage::ToggleMacroSolo(group_id)),
                12,
            ),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center);

        let content = column![
            title_row,
            container(count_badge).width(Length::Fill).center_x(Length::Fill),
            Space::new().height(Length::Fill),
            level_block,
            container(ms_row).width(Length::Fill).center_x(Length::Fill),
        ]
        .spacing(8)
        .padding([12, 10])
        .width(theme::MIXER_GROUP_HEADER_WIDTH)
        .height(Length::Fill);

        container(content)
            .height(Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32))
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(wash)),
                border: iced::Border {
                    color: line,
                    width: 1.0,
                    radius: theme::RADIUS_XL.into(),
                },
                ..Default::default()
            })
            .into()
    }
}
