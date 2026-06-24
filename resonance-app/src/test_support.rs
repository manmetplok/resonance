//! Test-support accessors for [`Resonance`].
//!
//! Public read-only accessors / mutators for integration tests. These
//! exist so `tests/*.rs` files (which are external compile units and
//! see only public API) can verify reducer-driven state changes
//! without poking at private fields. They're `#[doc(hidden)]` because
//! they aren't part of the library's user-facing surface —
//! application code inside the crate still goes through the
//! `pub(crate)` fields directly.
//!
//! Kept out of `lib.rs` purely for separation of concerns; this is a
//! plain `impl Resonance` continuation, not behind any feature gate
//! (integration tests can't see `#[cfg(test)]` items, and a required
//! cargo feature would complicate `cargo test`).

use crate::state;
use crate::Resonance;

impl Resonance {
    #[doc(hidden)]
    pub fn test_tempo_map(&self) -> &resonance_audio::types::TempoMap {
        &self.tempo_map
    }

    #[doc(hidden)]
    pub fn test_tempo_events(&self) -> &[state::TempoEvent] {
        &self.tempo_events
    }

    #[doc(hidden)]
    pub fn test_signature_events(&self) -> &[state::SignatureEvent] {
        &self.signature_events
    }

    #[doc(hidden)]
    pub fn test_transport_bpm(&self) -> f32 {
        self.transport.bpm
    }

    #[doc(hidden)]
    pub fn test_transport_time_sig(&self) -> (u8, u8) {
        (self.transport.time_sig_num, self.transport.time_sig_den)
    }

    #[doc(hidden)]
    pub fn test_selected_global_event(&self) -> Option<state::SelectedGlobalEvent> {
        self.interaction.selected_global_event
    }

    /// Test-only: borrow the track registry to walk `sorted_tracks()` /
    /// inspect sub-track links from an integration test (which doesn't
    /// see `pub(crate)` fields). Used by
    /// `tests/mixer_sub_track_grouping.rs` to assert the displayed
    /// strip order without parsing the rendered widget tree.
    #[doc(hidden)]
    pub fn test_registry(&self) -> &state::TrackRegistry {
        &self.registry
    }

    /// Test-only: read the mixer-side expanded-sub-track-parents set,
    /// also driven from `tests/mixer_sub_track_grouping.rs`.
    #[doc(hidden)]
    pub fn test_expanded_sub_track_parents(
        &self,
    ) -> &std::collections::HashSet<resonance_audio::types::TrackId> {
        &self.mixer.expanded_sub_track_parents
    }

    /// Test-only: forcibly clear an expanded-sub-track-parent flag so
    /// the test can flip between expanded / collapsed without dragging
    /// in the full `Message` plumbing.
    #[doc(hidden)]
    pub fn test_collapse_sub_track_parent(
        &mut self,
        parent_id: resonance_audio::types::TrackId,
    ) {
        self.mixer.expanded_sub_track_parents.remove(&parent_id);
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

    /// Test-only: read the GUI-side MIDI clip list. Used by reducer
    /// tests under `tests/` that need to inspect post-drag/trim clip
    /// geometry without poking at the engine round-trip.
    #[doc(hidden)]
    pub fn test_midi_clips(&self) -> &[state::MidiClipState] {
        &self.midi_clips
    }

    /// Test-only: push a MIDI clip directly into GUI state, bypassing
    /// the engine notification round-trip. Returns the clip's id so
    /// the test can dispatch trim/drag messages against it.
    #[doc(hidden)]
    pub fn test_push_midi_clip(&mut self, clip: state::MidiClipState) {
        self.midi_clips.push(clip);
    }

    /// Test-only: overwrite the sample rate. Tempo-map projections used
    /// by the MIDI clip trim reducer depend on `sample_rate`; integration
    /// tests fix it to a known value so the projection math is
    /// deterministic.
    #[doc(hidden)]
    pub fn test_set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
    }

    /// Test-only: rebuild the GUI-side tempo map from the current
    /// `tempo_events` / `signature_events`. Mirrors what the global-
    /// track reducers call after a tempo edit; surfaced so tests can
    /// seed a custom tempo map without going through the message path.
    #[doc(hidden)]
    pub fn test_rebuild_tempo_map(&mut self) {
        self.rebuild_tempo_map();
    }

    /// Test-only: push a tempo event so the rebuilt tempo map has the
    /// requested ramp/step. Caller must follow with
    /// `test_rebuild_tempo_map` (and usually `test_set_sample_rate`).
    #[doc(hidden)]
    pub fn test_push_tempo_event(&mut self, event: state::TempoEvent) {
        self.tempo_events.push(event);
    }

    /// Test-only: overwrite the arrange-view zoom (pixels per second).
    /// MIDI clip trim translates a pointer-pixel delta into samples via
    /// `delta_px / zoom`, so the reducer test fixes a known zoom value
    /// to make the delta arithmetic deterministic.
    #[doc(hidden)]
    pub fn test_set_arrange_zoom(&mut self, zoom: f32) {
        self.viewport.zoom = zoom;
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

    /// Test-only: remove a track's lane-generator config from every
    /// compose section definition, turning a configured compose lane
    /// back into an unconfigured one (placeholder row in the vocal
    /// stack).
    #[doc(hidden)]
    pub fn test_remove_lane_generator(&mut self, track_id: resonance_audio::types::TrackId) {
        for def in &mut self.compose.definitions {
            def.lane_generators.remove(&track_id);
        }
    }

    /// Test-only: flip the project-active flag so the message gate in
    /// `gates_message` lets reducer-driven `MidiClipMessage` /
    /// `ClipMessage` traffic through. Demo seeding does this in the
    /// real app; reducer tests that don't seed the demo flip it
    /// directly.
    #[doc(hidden)]
    pub fn test_set_active_project(&mut self, active: bool) {
        self.io.has_active_project = active;
    }

    /// Test-only: the currently active top-level [`ViewMode`].
    #[doc(hidden)]
    pub fn test_view_mode(&self) -> state::ViewMode {
        self.view_mode
    }

    /// Test-only: directly set the active view (bypassing the reducer)
    /// to establish a starting tab for Performance-mode toggle tests.
    #[doc(hidden)]
    pub fn test_set_view_mode(&mut self, mode: state::ViewMode) {
        self.view_mode = mode;
    }

    /// Test-only: whether the transport reports as playing. Used to
    /// assert that entering/leaving Performance mode never starts or
    /// stops playback.
    #[doc(hidden)]
    pub fn test_transport_playing(&self) -> bool {
        self.transport.playing
    }

    /// Test-only: force the transport's playing flag so a test can prove
    /// a view switch preserves it (no engine round-trip involved).
    #[doc(hidden)]
    pub fn test_set_transport_playing(&mut self, playing: bool) {
        self.transport.playing = playing;
    }

    /// Test-only: arm/disarm the first track's record flag so a test can
    /// assert that record-arm never auto-opens Performance mode.
    #[doc(hidden)]
    pub fn test_arm_first_track(&mut self, armed: bool) {
        if let Some(track) = self.registry.tracks.first_mut() {
            track.record_armed = armed;
        }
    }

    /// Test-only: read the GUI-side audio clip list. Used by the
    /// engine-event mirroring tests to assert that fade/gain events
    /// land on the matching `ClipState`.
    #[doc(hidden)]
    pub fn test_clips(&self) -> &[state::ClipState] {
        &self.clips
    }

    /// Test-only: push an audio clip straight into GUI state, bypassing
    /// the engine `ClipImported` round-trip, so a test can then drive
    /// fade/gain events against a known clip id.
    #[doc(hidden)]
    pub fn test_push_clip(&mut self, clip: state::ClipState) {
        self.clips.push(clip);
    }

    /// Test-only: feed an [`AudioEvent`] through the same engine→app
    /// dispatch the per-frame tick uses, so reducer tests can verify the
    /// one-way mirroring without bringing up a real audio engine.
    #[doc(hidden)]
    pub fn test_apply_engine_event(&mut self, event: resonance_audio::types::AudioEvent) {
        let _ = crate::engine_events::handle_engine_event(self, event);
    }

    /// Test-only: read the Arrange multi-track selection set, in click
    /// order. Drives `tests/group_creation_from_selection.rs`.
    #[doc(hidden)]
    pub fn test_selected_tracks(&self) -> &[resonance_audio::types::TrackId] {
        &self.interaction.selected_tracks
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
        self.interaction.membership_drag.as_ref()
    }

    /// Test-only: seed the Arrange multi-track selection directly, bypassing
    /// the per-click `SelectTrack` plumbing.
    #[doc(hidden)]
    pub fn test_set_selected_tracks(&mut self, ids: Vec<resonance_audio::types::TrackId>) {
        self.interaction.selected_track = ids.last().copied();
        self.interaction.selected_tracks = ids;
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

    /// Test-only: build the shared arrange-row layout exactly as the
    /// track-header column does (sorted arrange tracks + the collapse-aware
    /// registry). Drives `tests/group_creation_from_selection.rs`'
    /// end-to-end fold check: after a `ToggleCollapse` the collapsed
    /// group's member rows must vanish from the layout while its header row
    /// remains, since both the column and the canvas render from this one
    /// layout (todo #686, doc #203).
    #[doc(hidden)]
    pub fn test_arrange_row_layout(&self) -> crate::view::arrange_layout::ArrangeRowLayout {
        let sorted: Vec<&state::TrackState> = self
            .sorted_tracks()
            .iter()
            .filter(|t| t.sub_track.is_none())
            .collect();
        crate::view::arrange_layout::ArrangeRowLayout::build(&sorted, &self.track_groups)
    }

    /// Test-only: render a standalone track-header cell for a member
    /// track (todo #688) so tests can snapshot the "via group" solo chip.
    #[doc(hidden)]
    pub fn test_track_header_view(
        &self,
        track: &state::TrackState,
    ) -> iced::Element<'static, crate::message::Message> {
        crate::view::track_header::track::view_track_header(self, track, false)
    }

    /// Test-only: render the floating "N tracks selected · Group ⌘G" bar
    /// (todo #684) so `tests/selection_bar.rs` can snapshot it standalone.
    #[doc(hidden)]
    pub fn test_selection_bar_view(&self, count: usize) -> iced::Element<'static, crate::message::Message> {
        crate::view::selection_bar::selection_bar_with_count(count)
    }
}
