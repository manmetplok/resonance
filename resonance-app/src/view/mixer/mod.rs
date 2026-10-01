//! Mixer view: top-level layout and the small "+ Bus" strip. The strips
//! live in submodules — `track_strip.rs`, `bus_strip.rs`,
//! `master_strip.rs` — built from the shared pieces in `strip_parts.rs`
//! (slot lines, FX switch, centred pan). A plugin's parameters open in the
//! floating generic window (`view/plugin_window.rs`), not in the mixer.

pub(crate) mod automation;
mod bus_strip;
mod group_strip;
pub(crate) mod inspector;
mod master_strip;
pub(crate) mod picks;
mod reference_panel;
pub(crate) mod reorder;
mod strip_fingerprint;
mod strip_parts;
mod track_strip;

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Color, Element, Length};

use resonance_audio::types::ScannedPlugin;

use crate::message::*;
use crate::state::*;
use crate::theme;


pub(crate) use group_strip::MixerTopItem;
pub(crate) use strip_parts::slot_line_label;

impl crate::Resonance {
    pub(crate) fn view_mixer(&self) -> Element<'_, Message> {
        let sorted_tracks = self.sorted_tracks();
        let sorted_busses = self.sorted_busses();
        let available_plugins = &self.plugin_catalog.available_plugins;

        // -- Top row: track strips + master strip on the right. --
        // The lane is built in two clustering layers so related strips
        // stay visually attached instead of scattering to their `.order`
        // positions:
        //   1. Top-level order (`mixer_top_level_items`): ungrouped
        //      tracks keep their sorted position; a track group folds
        //      into one coloured cluster at the slot of its first member
        //      (`view_mixer_group_cluster`).
        //   2. Per track (`view_track_cluster`): a parent strip is
        //      immediately followed by its expanded sub-tracks (in
        //      `output_port_index` order — how the engine fans them out)
        //      in a tight inner row (0 px spacing).
        // The outer row keeps `MIXER_STRIP_GAP` between unrelated
        // strips/clusters, and the lane gets a `MIXER_LANE_HPAD` lead-in
        // so the first strip doesn't sit flush against the window edge.
        let mut track_strip_row = row![]
            .spacing(theme::MIXER_STRIP_GAP)
            .padding([0.0, theme::MIXER_LANE_HPAD]);
        for item in self.mixer_top_level_items() {
            match item {
                group_strip::MixerTopItem::Track(track_id) => {
                    if let Some(track) = sorted_tracks.iter().find(|t| t.id == track_id) {
                        track_strip_row = track_strip_row.push(self.view_track_cluster(
                            track,
                            sorted_tracks,
                            available_plugins,
                        ));
                    }
                }
                group_strip::MixerTopItem::Group(group_id) => {
                    if let Some(group) = self.track_groups.get_group(group_id) {
                        track_strip_row = track_strip_row.push(self.view_mixer_group_cluster(
                            group,
                            sorted_tracks,
                            available_plugins,
                        ));
                    }
                }
            }
        }
        // Construct the scrollable with its horizontal direction up
        // front. `scrollable(content)` would default to Vertical and run
        // its `validate()` debug assertion before the chained
        // `.direction(...)` has a chance to change it — and
        // `track_strip_row`'s size hint is Fill-height now that each
        // strip claims Length::Fill vertically.
        let scrollable_tracks = iced::widget::Scrollable::with_direction(
            track_strip_row,
            scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
        )
        .width(Length::Fill);
        let master_strip = self.view_master_strip(available_plugins);
        let v_separator_tracks = container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);
        let tracks_area = row![scrollable_tracks, v_separator_tracks, master_strip]
            .height(Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32));

        // -- Bottom row: bus strips + "+ Bus" button on the right. --
        let mut bus_strip_row = row![]
            .spacing(theme::MIXER_STRIP_GAP)
            .padding([0.0, theme::MIXER_LANE_HPAD]);
        for bus in sorted_busses {
            bus_strip_row = bus_strip_row.push(self.view_bus_strip(bus, available_plugins));
        }
        let scrollable_busses = iced::widget::Scrollable::with_direction(
            bus_strip_row,
            scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
        )
        .width(Length::Fill);
        let add_bus_strip = self.view_add_bus_strip();
        let v_separator_busses = container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);
        let busses_area = row![scrollable_busses, v_separator_busses, add_bus_strip]
            .height(Length::Fixed(theme::BUS_STRIP_HEIGHT as f32));

        let h_sep_mid = container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg);

        let mut mixer_col = column![].spacing(0);
        mixer_col = mixer_col.push(tracks_area);
        mixer_col = mixer_col.push(h_sep_mid);
        mixer_col = mixer_col.push(busses_area);

        // Inspector sits to the right of the strips; a hairline separates
        // it from the strips column.
        let inspector_panel = inspector::view(self);
        let v_sep_inspector =
            container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);

        let mut body = row![
            container(mixer_col).width(Length::Fill).height(Length::Fill),
            v_sep_inspector,
            inspector_panel,
        ]
        .height(Length::Fill);

        // The Reference & A/B rail is the outermost right rail, shown only
        // when the chrome "REF" toggle is on. A hairline separates it from
        // the inspector.
        if self.ui.mixer.reference_panel_open {
            let v_sep_reference =
                container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);
            body = body
                .push(v_sep_reference)
                .push(reference_panel::view(self));
        }

        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::base_bg)
            .into()
    }

    /// Render a top-level track strip plus its expanded sub-track strips as
    /// a tight (0 px) cluster — the parent + sub-track grouping lifted out
    /// of `view_mixer` so it is reused both standalone and inside a group
    /// cluster (`view_mixer_group_cluster`). Sub-tracks sort by their plugin
    /// `output_port_index` (stable across save/load) and render through the
    /// recessed `view_sub_channel_strip`; an unexpanded or sub-less parent
    /// just returns its own strip.
    pub(super) fn view_track_cluster<'a>(
        &'a self,
        track: &'a TrackState,
        sorted_tracks: &'a [TrackState],
        available_plugins: &'a [ScannedPlugin],
    ) -> Element<'a, Message> {
        let parent_strip = self.view_channel_strip(track, available_plugins);

        let parent_expanded = self.ui.mixer.expanded_sub_track_parents.contains(&track.id);
        if !parent_expanded {
            return parent_strip;
        }
        let mut subs: Vec<&TrackState> = sorted_tracks
            .iter()
            .filter(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id))
            .collect();
        if subs.is_empty() {
            return parent_strip;
        }
        subs.sort_by_key(|t| t.sub_track.map(|l| l.output_port_index).unwrap_or(0));

        // Cluster: parent + sub-strips with no internal gap, so the recessed
        // sub-strip backgrounds visually butt up against the parent strip.
        let mut cluster = row![parent_strip].spacing(0);
        for sub in subs {
            cluster = cluster.push(self.view_sub_channel_strip(sub, available_plugins));
        }
        cluster.into()
    }

    /// Small "+ Bus" strip that lives in the same slot the master strip
    /// occupies in the top row, but in the bus row. Clicking it dispatches
    /// `Message::Bus(BusMessage::AddBus)`.
    fn view_add_bus_strip(&self) -> Element<'_, Message> {
        let label = container(text("Busses").size(11).color(theme::TEXT_DIM))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([6, 4]);

        let add_btn = button(text("+ Bus").size(11).color(theme::TEXT))
            .on_press(Message::Bus(BusMessage::AddBus))
            .style(|_theme, status| theme::small_button_style(status))
            .padding([4, 10]);

        let content = column![
            label,
            container(add_btn)
                .width(Length::Fill)
                .center_x(Length::Fill),
            Space::new().height(Length::Fill),
        ]
        .spacing(4)
        .padding(8)
        .width(theme::MASTER_STRIP_WIDTH);

        container(content)
            .height(Length::Fill)
            .style(theme::panel_dark_outlined)
            .into()
    }
}

/// What a slot's floating-editor toggle carries and how it is tinted:
/// the message a press raises, and the glyph colour (ba todo #1306).
///
/// `None` for a plugin that declares no GUI: it has no editor to toggle
/// (its `↗` follows the generic window instead). Drawn by the inspector
/// CHAIN row's `↗` (`inspector::chain::open_toggle_spec`), and readable
/// by a test (`test_chain_open_toggle`) — the tint is not observable
/// through the widget tree, and "the glyph lights up while the window is
/// open" is the only feedback a press gives, since the engine reports
/// neither success nor failure per instance (ba todo #1347).
pub(crate) fn editor_toggle_spec(plugin: &PluginSlotState) -> Option<(Message, Color)> {
    if !plugin.has_gui {
        return None;
    }
    let pid = plugin.instance_id;
    let open = plugin.editor_open;
    let message = if open {
        Message::Plugin(PluginMessage::ClosePluginEditor(pid))
    } else {
        Message::Plugin(PluginMessage::OpenPluginEditor(pid))
    };
    let color = if open { theme::ACCENT } else { theme::TEXT_DIM };
    Some((message, color))
}
