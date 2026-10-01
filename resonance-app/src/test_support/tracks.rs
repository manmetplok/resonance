//! Track registry, grouping and selection test hooks, plus the
//! standalone track/group header and selection-bar render helpers.

use crate::state;
use crate::Resonance;

impl Resonance {
    /// Test-only: borrow the track registry to walk `sorted_tracks()` /
    /// inspect sub-track links from an integration test (which doesn't
    /// see `pub(crate)` fields). Used by
    /// `tests/mixer_sub_track_grouping.rs` to assert the displayed
    /// strip order without parsing the rendered widget tree.
    #[doc(hidden)]
    pub fn test_registry(&self) -> &state::TrackRegistry {
        &self.registry
    }

    /// Test-only: mutable registry, for staging states the handlers can't
    /// reach (a lane generator whose track is missing, code review VIEW-12).
    #[doc(hidden)]
    pub fn test_registry_mut(&mut self) -> &mut state::TrackRegistry {
        &mut self.registry
    }

    /// Test-only: read the mixer-side expanded-sub-track-parents set,
    /// also driven from `tests/mixer_sub_track_grouping.rs`.
    #[doc(hidden)]
    pub fn test_expanded_sub_track_parents(
        &self,
    ) -> &std::collections::HashSet<resonance_audio::types::TrackId> {
        &self.ui.mixer.expanded_sub_track_parents
    }

    /// Test-only: forcibly clear an expanded-sub-track-parent flag so
    /// the test can flip between expanded / collapsed without dragging
    /// in the full `Message` plumbing.
    #[doc(hidden)]
    pub fn test_collapse_sub_track_parent(
        &mut self,
        parent_id: resonance_audio::types::TrackId,
    ) {
        self.ui.mixer.expanded_sub_track_parents.remove(&parent_id);
    }

    /// Test-only: the mixer's top-level strip order as `(is_group, id)`
    /// pairs — exactly the sequence `view_mixer` renders. A group cluster
    /// reports `(true, group_id)` at its first member's slot; ungrouped
    /// tracks report `(false, track_id)`. Drives
    /// `tests/mixer_group_clustering.rs` so the group-clustering order is
    /// asserted without parsing the rendered widget tree.
    #[doc(hidden)]
    pub fn test_mixer_top_level(&self) -> Vec<(bool, resonance_audio::types::TrackId)> {
        use crate::view::mixer::MixerTopItem;
        self.mixer_top_level_items()
            .into_iter()
            .map(|item| match item {
                MixerTopItem::Track(id) => (false, id),
                MixerTopItem::Group(id) => (true, id),
            })
            .collect()
    }

    /// Test-only: push a track straight into the registry, bypassing the
    /// engine round-trip, and refresh the compose track-count cache the
    /// engine handlers would normally keep fresh. Used by the vocal
    /// placeholder snapshot tests to add a `TrackType::Vocal` track that
    /// has no lane-generator config.
    #[doc(hidden)]
    pub fn test_push_track(&mut self, track: state::TrackState) {
        self.registry.next_track_order = self.registry.next_track_order.max(track.order + 1);
        self.registry.tracks.push(track);
        self.compose.refresh_track_count(&self.registry.tracks);
    }

    /// Test-only: select a track so `FreezeSelectedTracks` has a target.
    #[doc(hidden)]
    pub fn test_select_track(&mut self, track_id: resonance_audio::types::TrackId) {
        self.ui.interaction.selected_track = Some(track_id);
    }

    /// Test-only: append a track of the given type so freeze tests have a
    /// registry to operate on without an engine round-trip.
    #[doc(hidden)]
    pub fn test_add_track(
        &mut self,
        track_id: resonance_audio::types::TrackId,
        track_type: resonance_audio::types::TrackType,
    ) {
        use resonance_audio::types::TrackType;
        let order = self.registry.tracks.len();
        let track = match track_type {
            TrackType::Audio => crate::state::TrackState::new_audio(track_id, order),
            TrackType::Instrument => crate::state::TrackState::new_instrument(track_id, order),
            TrackType::Vocal => crate::state::TrackState::new_vocal(track_id, order),
        };
        self.registry.tracks.push(track);
        self.registry.resort_tracks();
    }

    /// Test-only: the primary (single) track selection the mixer
    /// inspector reads.
    #[doc(hidden)]
    pub fn test_selected_track(&self) -> Option<resonance_audio::types::TrackId> {
        self.ui.interaction.selected_track
    }

    /// Test-only: the selected BUS strip, if any. Drives
    /// `tests/mixer_inspector_bus.rs` — the bus counterpart of
    /// `test_selected_track`, separate because bus ids and track ids are
    /// separate id spaces.
    #[doc(hidden)]
    pub fn test_selected_bus(&self) -> Option<resonance_audio::types::BusId> {
        self.ui.mixer.selected_bus
    }

    /// Test-only: push a bus straight into the registry, bypassing the
    /// engine round-trip, and refresh the output-choice cache the engine
    /// handlers would normally keep fresh.
    #[doc(hidden)]
    pub fn test_add_bus(&mut self, bus_id: resonance_audio::types::BusId, name: &str) {
        let order = self.registry.busses.len();
        self.registry
            .busses
            .push(state::BusState::new(bus_id, order, name.to_owned()));
        self.registry.next_bus_order = self.registry.busses.len();
        self.ui.view_caches.rebuild_output(&self.registry.busses);
    }

    /// Test-only: read the Arrange multi-track selection set, in click
    /// order. Drives `tests/group_creation_from_selection.rs`.
    #[doc(hidden)]
    pub fn test_selected_tracks(&self) -> &[resonance_audio::types::TrackId] {
        &self.ui.interaction.selected_tracks
    }

    /// Test-only: every track in the registry, sub-tracks included.
    #[doc(hidden)]
    pub fn test_tracks(&self) -> &[state::TrackState] {
        &self.registry.tracks
    }

    /// Test-only: borrow the track-group registry so a reducer test can
    /// assert that "Group selected" created the expected group.
    #[doc(hidden)]
    pub fn test_track_groups(&self) -> &state::TrackGroupRegistry {
        &self.track_groups
    }

    /// Test-only: the root group id a track resolves to for mixer
    /// clustering (walking up one level of nesting), or `None` when the
    /// track is ungrouped. Drives `tests/mixer_group_clustering.rs`.
    #[doc(hidden)]
    pub fn test_mixer_root_group_of(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<resonance_audio::types::TrackId> {
        self.mixer_root_group_of(track_id).map(|g| g.id)
    }

    /// Test-only: borrow the active drag-and-drop membership drag (todo
    /// #685), so a reducer test can assert it opens on a grab, tracks the
    /// hovered target, and clears on drop / cancel.
    #[doc(hidden)]
    pub fn test_membership_drag(&self) -> Option<&state::MembershipDragState> {
        self.ui.interaction.membership_drag.as_ref()
    }

    /// Test-only: seed the Arrange multi-track selection directly, bypassing
    /// the per-click `SelectTrack` plumbing.
    #[doc(hidden)]
    pub fn test_set_selected_tracks(&mut self, ids: Vec<resonance_audio::types::TrackId>) {
        self.ui.interaction.selected_track = ids.last().copied();
        self.ui.interaction.selected_tracks = ids;
    }

    /// Test-only: render the standalone group-header row component
    /// (todo #680) so `tests/group_header.rs` can snapshot it without the
    /// component being wired into the live timeline column yet (#681/#686).
    #[doc(hidden)]
    pub fn test_group_header_view(
        &self,
        group: &resonance_common::track_group::TrackGroup,
        member_count: usize,
    ) -> iced::Element<'static, crate::message::Message> {
        crate::view::track_header::group_header::view_group_header(group, member_count)
    }

    /// Test-only: mutable borrow of the track-group registry so tests can
    /// set up group state (todo #688).
    #[doc(hidden)]
    pub fn test_track_groups_mut(&mut self) -> &mut state::TrackGroupRegistry {
        &mut self.track_groups
    }

    /// Test-only: render a standalone track-header cell for a member
    /// track (todo #688) so tests can snapshot the "via group" solo chip.
    #[doc(hidden)]
    pub fn test_track_header_view(
        &self,
        track: &state::TrackState,
    ) -> iced::Element<'static, crate::message::Message> {
        crate::view::track_header::track::view_track_header(self, track, false, (56.0, 30.0))
    }

    /// Test-only: read the open track context menu state, if any (ba todo
    /// #581).
    #[doc(hidden)]
    pub fn test_track_menu(&self) -> Option<&crate::state::TrackMenuState> {
        self.ui.interaction.track_menu.as_ref()
    }

    /// Test-only: render the floating "N tracks selected · Group ⌘G" bar
    /// (todo #684) so `tests/selection_bar.rs` can snapshot it standalone.
    #[doc(hidden)]
    pub fn test_selection_bar_view(&self, count: usize) -> iced::Element<'static, crate::message::Message> {
        crate::view::selection_bar::selection_bar_with_count(
            count,
            crate::view::shortcut_hint::chord_text(self, crate::commands::CommandId::GroupSelectedTracks),
        )
    }

    /// Test-only: run the startup default-track send that `Resonance::new`
    /// runs (FU-D4a) — `new_for_test*` is hermetic and never runs it, so a
    /// test that wants to pin the startup race closed drives it
    /// explicitly. See `send_startup_default_track`'s doc comment
    /// (`state/ids.rs`) for what this guards.
    #[doc(hidden)]
    pub fn test_send_startup_default_track(&mut self) {
        self.send_startup_default_track();
    }

    /// Test-only: open the realtime bounce-in-place dialog for
    /// `source_track_id` without staging an external-MIDI track, so a test
    /// can drive the dialog's own messages.
    #[doc(hidden)]
    pub fn test_open_bounce_dialog(&mut self, source_track_id: resonance_audio::types::TrackId) {
        self.modals.bounce_dialog = Some(state::BounceDialogState {
            source_track_id,
            selected_device: None,
            selected_port: 0,
            mono: false,
        });
    }

    /// Test-only: the open bounce-in-place dialog, if any.
    #[doc(hidden)]
    pub fn test_bounce_dialog(&self) -> Option<&state::BounceDialogState> {
        self.modals.bounce_dialog.as_ref()
    }
}
