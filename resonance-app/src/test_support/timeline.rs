//! Arrange-timeline test hooks: clip lists, markers, automation
//! state, the shared arrange-row layout, drag placement, and the
//! timeline-canvas event/hit-test/fingerprint helpers.

use crate::state;
use crate::Resonance;

impl Resonance {
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

    /// Test-only: the note selection of the clip open in the MIDI editor,
    /// or `None` when no editor is open.
    #[doc(hidden)]
    pub fn test_editing_selected_notes(&self) -> Option<std::collections::BTreeSet<usize>> {
        self.interaction
            .editing_midi_clip
            .as_ref()
            .map(|e| e.selected_notes.clone())
    }

    /// Test-only: overwrite the arrange-view zoom (pixels per second).
    /// MIDI clip trim translates a pointer-pixel delta into samples via
    /// `delta_px / zoom`, so the reducer test fixes a known zoom value
    /// to make the delta arithmetic deterministic.
    #[doc(hidden)]
    pub fn test_set_arrange_zoom(&mut self, zoom: f32) {
        self.viewport.zoom = zoom;
    }

    /// Test-only: the arrange-view zoom (pixels per second). Pairs with
    /// [`test_set_arrange_zoom`](Self::test_set_arrange_zoom) so a gesture
    /// test can convert a sample position into the canvas x the pointer
    /// has to land on without hard-coding the default zoom.
    #[doc(hidden)]
    pub fn test_arrange_zoom(&self) -> f32 {
        self.viewport.zoom
    }

    /// Test-only: the arrange timeline's vertical scroll offset (px).
    #[doc(hidden)]
    pub fn test_arrange_scroll_y(&self) -> f32 {
        self.viewport.scroll_offset_y
    }

    /// Test-only: the arrange view's horizontal scroll offset (px) as
    /// state knows it — the outer `Scrollable`'s reported offset, or
    /// the target playhead follow last asked it for (review FU-D1).
    #[doc(hidden)]
    pub fn test_arrange_scroll_x(&self) -> f32 {
        self.viewport
            .follow_pending_x
            .unwrap_or(self.viewport.scroll_offset)
    }

    /// Test-only: the left x of the timeline canvas's in-canvas vertical
    /// scrollbar track (`None` when the lanes fit), for a canvas of
    /// `canvas_size` of which `visible` (canvas-local; `None` = no probe
    /// write yet) is on screen inside the outer `Scrollable` (VIEW-33).
    #[doc(hidden)]
    pub fn test_timeline_vscrollbar_x(
        &self,
        canvas_size: iced::Size,
        visible: Option<iced::Rectangle>,
    ) -> Option<f32> {
        let canvas = self.timeline_canvas_data();
        canvas.visible_viewport.set(visible);
        canvas
            .test_scrollbar_rects(iced::Rectangle::new(iced::Point::ORIGIN, canvas_size))
            .map(|sb| sb.track.x)
    }

    /// Test-only: whether playhead follow is paused by a manual scroll.
    #[doc(hidden)]
    pub fn test_follow_paused(&self) -> bool {
        self.viewport.follow_paused
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

    /// Test-only: read the GUI-side automation state (mirrored lanes +
    /// transient live values). Used by the engine-event mirroring tests
    /// to assert lane reconstruction and live-value tracking.
    #[doc(hidden)]
    pub fn test_automation(&self) -> &state::AutomationState {
        &self.automation
    }

    /// Test-only: borrow the arrangement-marker collection so the marker
    /// reducer tests can assert post-dispatch state (count, order,
    /// names, colours, region bounds) without poking private fields.
    #[doc(hidden)]
    pub fn test_markers(&self) -> &state::ArrangementMarkers {
        &self.markers
    }

    /// Test-only: insert a marker straight into state, bypassing the
    /// `AddAtPlayhead` snap path, so a test can seed markers at exact
    /// sample positions before exercising rename / move / jump / loop
    /// reducers. Returns the marker's id.
    #[doc(hidden)]
    pub fn test_add_marker(&mut self, marker: state::ArrangementMarker) -> u64 {
        self.markers.add(marker)
    }

    /// Test-only: the currently selected arrangement-marker id. Driven by
    /// the ruler hit-testing / `MarkerUiMessage::Select` (todo #369).
    #[doc(hidden)]
    pub fn test_selected_marker_id(&self) -> Option<u64> {
        self.interaction.selected_marker_id
    }

    /// Test-only: the open marker context menu, if any (todo #369).
    #[doc(hidden)]
    pub fn test_marker_menu(&self) -> Option<&state::MarkerMenuState> {
        self.interaction.marker_menu.as_ref()
    }

    /// Test-only: the in-progress inline marker rename, if any (todo #369).
    #[doc(hidden)]
    pub fn test_marker_rename(&self) -> Option<&state::MarkerRenameState> {
        self.interaction.marker_rename.as_ref()
    }

    /// Test-only: whether the arrangement-markers overview popover is open
    /// (todo #370). Toggled by `UiMessage::ToggleMarkersOverview` and
    /// dismissed by `UiMessage::CloseMarkersOverview`.
    #[doc(hidden)]
    pub fn test_markers_overview_open(&self) -> bool {
        self.mixer.markers_overview_open
    }

    /// Test-only: which audio clip's vocal pitch editor is open, if any
    /// (doc #160). Set by the `VocalTuningMessage::OpenPitchEditor`
    /// reducer when opened on a vocal clip.
    #[doc(hidden)]
    pub fn test_editing_pitch_clip(&self) -> Option<resonance_audio::types::ClipId> {
        self.interaction.editing_pitch_clip
    }

    /// Test-only: borrow the in-flight drag-placement state so gesture /
    /// render tests can assert the drag pill / lit lane / ghost / tooltip
    /// inputs (the dragged asset, cursor, and resolved drop target).
    #[doc(hidden)]
    pub fn test_drag_placement(&self) -> Option<&crate::state::DragPlacement> {
        self.media.drag_placement.as_ref()
    }

    /// Test-only: install an in-flight drag directly, standing in for the
    /// browser-row press + pointer moves so a golden-image snapshot can
    /// render a deterministic drag state without simulating the gesture.
    #[doc(hidden)]
    pub fn test_set_drag_placement(&mut self, drag: crate::state::DragPlacement) {
        self.media.drag_placement = Some(drag);
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
        crate::view::arrange_layout::ArrangeRowLayout::build_with_takes(
            &sorted,
            &self.track_groups,
            &self.arrange_automation_rows(),
            &self.arrange_take_rows(),
        )
    }

    /// Test-only: resolve the arrange-canvas drag-drop target lane under a
    /// canvas-Y. Drives `tests/timeline_group_hit_test.rs`' coverage that a
    /// clip dragged over a group-header lane (or a collapsed member's hidden
    /// row) resolves to no track, while a track lane resolves correctly
    /// under the mixed 60/96 px pitch (epic #36, doc #203, todo #732).
    #[doc(hidden)]
    pub fn test_track_id_at_arrange_y(&self, y: f32) -> Option<resonance_audio::types::TrackId> {
        self.track_id_at_arrange_y(y)
    }

    /// Test-only: the fixed arrange-header height (ruler + section band +
    /// global-tracks shelf) above the first track lane, so tests can build
    /// canvas-Y coordinates that match the live layout.
    #[doc(hidden)]
    pub fn test_arrange_header_offset(&self) -> f32 {
        self.arrange_header_offset()
    }

    /// Test-only: run one iced event through the timeline canvas's real
    /// `canvas::Program::update` input path — the same code the live
    /// pointer flows through — and return the `Message` it publishes, if
    /// any. `state` is the canvas-local `TimelineState` a test threads
    /// across a multi-step gesture (press → move → release); `(x, y)` is
    /// the cursor position in canvas space. Drives the per-row automation
    /// gesture-routing coverage of doc #256 / todo #1097.
    #[doc(hidden)]
    pub fn test_timeline_canvas_event(
        &self,
        state: &mut crate::view::timeline::TimelineState,
        event: &iced::Event,
        x: f32,
        y: f32,
    ) -> Option<crate::message::Message> {
        use iced::widget::canvas::Program as _;
        let width = if self.viewport.viewport_width > 0.0 {
            self.viewport.viewport_width
        } else {
            1200.0
        };
        let height = if self.viewport.viewport_height > 0.0 {
            self.viewport.viewport_height
        } else {
            900.0
        };
        let bounds = iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(width, height));
        let cursor = iced::mouse::Cursor::Available(iced::Point::new(x, y));
        let canvas = self.timeline_canvas_data();
        let action = canvas.update(state, event, bounds, cursor)?;
        action.into_inner().0
    }

    /// Test-only: the timeline canvas's "add a breakpoint here" resolution
    /// for a canvas-space position — `Some((target, snapped_frame, value))`
    /// when the position lands in an editable automation band (an overlay
    /// band on a collapsed track, or a dedicated lane row on an expanded
    /// one), `None` otherwise. Pins the todo #1097 overlay-suppression /
    /// per-row routing rules without a live canvas.
    #[doc(hidden)]
    pub fn test_timeline_band_add_at(
        &self,
        x: f32,
        y: f32,
    ) -> Option<(resonance_common::AutomationTarget, u64, f32)> {
        self.timeline_canvas_data()
            .band_add_at(iced::Point::new(x, y))
    }

    /// Test-only: the breakpoint dot under a canvas-space position, as
    /// `(lane target, point index)` — resolved per-row on expanded tracks
    /// (todo #1097).
    #[doc(hidden)]
    pub fn test_timeline_breakpoint_hit(
        &self,
        x: f32,
        y: f32,
    ) -> Option<(resonance_common::AutomationTarget, usize)> {
        self.timeline_canvas_data()
            .breakpoint_hit(iced::Point::new(x, y))
            .map(|hit| (hit.target, hit.index))
    }

    /// Test-only: the take card under a canvas-space position, as
    /// `(group, take, slot_start, slot_end)` — the comping hit region of
    /// epic #15 / todo #414.
    ///
    /// The slot is returned deliberately: the take **card** is drawn over
    /// the take's audible extent, but the comp addresses the whole slot,
    /// so this is what a promote is aimed at. A press over the "no audio
    /// here" part of a punched-in take's row is a hit, not a miss.
    #[doc(hidden)]
    pub fn test_take_card_at(&self, x: f32, y: f32) -> Option<(u64, u64, u64, u64)> {
        self.timeline_canvas_data()
            .take_card_at(iced::Point::new(x, y))
            .map(|hit| (hit.group_id, hit.take_id, hit.slot.start, hit.slot.end()))
    }

    /// Test-only: the comp-ribbon group under a canvas-space position —
    /// the split gesture's hit region, present whether the take lane is
    /// folded or expanded (epic #15, todo #414).
    #[doc(hidden)]
    pub fn test_comp_ribbon_at(&self, x: f32, y: f32) -> Option<u64> {
        self.timeline_canvas_data()
            .comp_ribbon_at(iced::Point::new(x, y))
            .map(|hit| hit.group_id)
    }

    /// Test-only: the mouse cursor the timeline canvas would show at a
    /// canvas-space position, given `state`. Pins the take lane's
    /// affordances — `NotAllowed` over a comp ribbon whose split has no
    /// cut point, `Grab` over a take card (todo #414).
    #[doc(hidden)]
    pub fn test_timeline_cursor(
        &self,
        state: &crate::view::timeline::TimelineState,
        x: f32,
        y: f32,
    ) -> iced::mouse::Interaction {
        use iced::widget::canvas::Program as _;
        let width = if self.viewport.viewport_width > 0.0 {
            self.viewport.viewport_width
        } else {
            1200.0
        };
        let height = if self.viewport.viewport_height > 0.0 {
            self.viewport.viewport_height
        } else {
            900.0
        };
        let bounds = iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(width, height));
        let cursor = iced::mouse::Cursor::Available(iced::Point::new(x, y));
        self.timeline_canvas_data()
            .mouse_interaction(state, bounds, cursor)
    }

    /// Test-only: the timeline canvas's cache fingerprint for the current
    /// app state. Two states whose fingerprints differ repaint the cached
    /// geometry layer; equal fingerprints reuse it. Pins that transient
    /// view state which reshapes the canvas (e.g. the automation
    /// lane-row expansion set, todo #1097) invalidates the cache.
    #[doc(hidden)]
    pub fn test_timeline_fingerprint(&self) -> crate::view::timeline::TimelineFingerprint {
        self.timeline_canvas_data().fingerprint()
    }

    /// Test-only: read the mirrored cycle-record take groups (epic #15).
    /// Drives `tests/timeline/take_group_mirror.rs`, which asserts that
    /// `TakeCaptured` events alone reconstruct the take lanes — the app
    /// never reads takes back out of the engine.
    #[doc(hidden)]
    pub fn test_take_groups(&self) -> &[resonance_common::TakeGroup] {
        &self.take_groups.groups
    }

    /// Test-only: whether `group_id`'s active take mutes the group's
    /// recorded audio — a MIDI take soloed over audio takes (epic #15).
    /// Drives `tests/timeline/take_comp_edits.rs`, which pins that the
    /// state is *reported* rather than left looking like a bug.
    #[doc(hidden)]
    pub fn test_active_take_silences_audio(&self, group_id: u64) -> bool {
        self.take_groups.active_take_silences_audio(group_id)
    }

    /// Test-only: `(group, take)` pairs whose recorded WAV was absent when
    /// the project loaded (todo #412). Drives
    /// `tests/io/take_lanes_persistence.rs`, which asserts a take with no
    /// audio on disk is flagged rather than dropped out of the comp.
    #[doc(hidden)]
    pub fn test_missing_takes(&self) -> Vec<(u64, u64)> {
        let mut pairs: Vec<(u64, u64)> = self.take_groups.missing_takes.iter().copied().collect();
        pairs.sort_unstable();
        pairs
    }

    /// Test-only: the waveform peaks the app derived from a take's
    /// recording (ba todo #1400), or an empty slice when it has none.
    ///
    /// This is what makes the take lane's waveform a *fact about the
    /// recording on disk* rather than a fabrication: before #1400 a test
    /// gave a take a silhouette by pushing a `ClipState` the running app
    /// never produces, and the same fabrication was what stopped every
    /// take drawing as `media missing`. Drives
    /// `tests/timeline/take_lane_render.rs`.
    #[doc(hidden)]
    pub fn test_take_peaks(&self, group_id: u64, take_id: u64) -> &[(f32, f32)] {
        let clip_ref = self
            .take_groups
            .group(group_id)
            .and_then(|g| g.take(take_id))
            .and_then(|t| match t.content {
                resonance_common::TakeContent::Audio { clip_ref } => Some(clip_ref),
                resonance_common::TakeContent::Midi { .. } => None,
            });
        // A MIDI take names no recording, so it can hold no table — and
        // an audio take's table only counts when it was read from the
        // recording that take names (ba todo #1400).
        clip_ref.map_or(&[][..], |c| self.take_groups.peaks(group_id, take_id, c))
    }

    /// Test-only: whether the take lane would draw this take as hatched
    /// `media missing` (ba todo #1400).
    ///
    /// The draw pass's own predicate, not the flag behind it. Before
    /// #1400 the two disagreed for *every* audio take — the lane ORed in
    /// a clip lookup that could never resolve — and no headless
    /// assertion in the suite could see it, because the divergence
    /// existed only in pixels and the verify gate is allowed to skip
    /// goldens. Anything asserting "this take is fine" has to ask the
    /// question the card asks.
    #[doc(hidden)]
    pub fn test_take_draws_missing_media(&self, group_id: u64, take_id: u64) -> bool {
        let canvas = self.timeline_canvas_data();
        let group = canvas
            .take_groups
            .group(group_id)
            .unwrap_or_else(|| panic!("no mirrored take group {group_id}"));
        let take = group
            .take(take_id)
            .unwrap_or_else(|| panic!("no take {take_id} in group {group_id}"));
        canvas.take_is_missing(group, take)
    }

    /// Test-only: flag a take's recorded WAV as absent from this machine,
    /// the state a project load reaches through `restore_pool` (todo
    /// #412). Standing in for the load so a render / fingerprint test can
    /// exercise the flag directly — the *other* route into the missing
    /// state, an unresolvable `clip_ref`, needs no hook.
    #[doc(hidden)]
    pub fn test_mark_take_missing(&mut self, group_id: u64, take_id: u64) {
        self.take_groups.mark_missing(group_id, take_id);
    }

    /// Test-only: whether a track's take lane is currently unfolded into
    /// stacked take sub-rows (epic #15). Driven by
    /// `UiMessage::ToggleTakeLane`; read by the take-lane render tests to
    /// assert the caret's state without scraping the widget tree.
    #[doc(hidden)]
    pub fn test_take_lane_expanded(&self, track_id: resonance_audio::types::TrackId) -> bool {
        self.interaction
            .take_lane_expanded_tracks
            .contains(&track_id)
    }
}
