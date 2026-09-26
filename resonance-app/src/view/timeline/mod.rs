//! Timeline canvas: the arrangement view for tracks, audio clips, and
//! MIDI clips. The canvas's three concerns are split across files:
//!
//! - this file: [`TimelineCanvas`] struct, small geometry helpers, and
//!   the [`canvas::Program`] impl that orchestrates per-event dispatch
//!   and per-frame drawing.
//! - [`input`](crate::view::timeline::input): pointer / wheel /
//!   keyboard event handling and the [`TimelineState`] drag tracker.
//! - [`draw`](crate::view::timeline::draw): pure-draw routines for
//!   the ruler, grid, global tracks, and clips.
//! - [`snap`](crate::view::timeline::snap): the snap-to-grid helpers
//!   shared with the clip-drag and seek paths.
use std::time::Instant;

use iced::widget::canvas;
use iced::{keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::message::*;
use crate::state::{self, ClipState, MidiClipState, TrackState};
use crate::theme;
use crate::view::arrange_layout::{ArrangeRowKind, ArrangeRowLayout};
use self::input::{BreakpointDrag, ClipInteraction, MarkerDrag, TakePromoteDrag, TempoDrag};

use resonance_audio::types::{ClipId, TempoMap, TrackId};
use resonance_common::AutomationTarget;

pub mod automation;
pub mod cull;
pub mod draw;
pub mod hit_test;
pub mod input;
pub mod placement;
pub mod scrollbar;
pub mod snap;
pub mod takes;
pub(crate) mod viewport_probe;

// Snap helpers are external public API for this canvas — re-export them
// from the snap submodule so existing call sites keep working.
pub use self::snap::{snap_sample_to_grid, snap_sample_to_grid_tempo};

/// Data passed to the timeline canvas for rendering.
#[derive(Debug)]
pub struct TimelineCanvas<'a> {
    pub tracks: &'a [TrackState],
    pub track_groups: &'a state::TrackGroupRegistry,
    pub clips: &'a [ClipState],
    pub playhead: u64,
    pub sample_rate: u32,
    pub zoom: f32,
    pub scroll_offset: f32,
    pub recording_tracks: Vec<TrackId>,
    pub recording_start_sample: u64,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub scroll_offset_y: f32,
    pub loop_enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
    pub selected_clip: Option<ClipId>,
    pub midi_clips: &'a [MidiClipState],
    pub selected_midi_clip: Option<ClipId>,
    pub selected_track: Option<TrackId>,
    pub global_tracks_expanded: bool,
    pub tempo_map: &'a TempoMap,
    pub selected_global_event: Option<crate::state::SelectedGlobalEvent>,
    /// Compose section placements + definitions, threaded so the
    /// section-pill band can sit above the lanes. Empty slices => no band.
    pub section_placements: &'a [crate::compose::SectionPlacementState],
    pub section_definitions: &'a [crate::compose::SectionDefinitionState],
    pub selected_placement_id: Option<u64>,
    /// App-side mirror of the engine's parameter-automation lanes. The
    /// timeline renders the primary lane for each track as an overlay band
    /// (doc #162 §3); empty => no lanes drawn.
    pub automation: &'a crate::state::AutomationState,
    /// Display names for `DeviceParam` lane targets, resolved at view-model
    /// build time via [`automation::device_param_labels`] (todo #1094) so the
    /// canvas needs no device-registry access. Lanes absent from the map fall
    /// back to their raw param id in the band's label chip.
    pub device_param_labels: std::collections::HashMap<AutomationTarget, String>,
    /// Arrangement markers (flags + region spans) rendered in the ruler
    /// band. Empty slice => nothing drawn. `selected_marker_id` recolors
    /// the matching flag / span with the accent (todo #368).
    pub markers: &'a [state::ArrangementMarker],
    pub selected_marker_id: Option<u64>,
    /// Tracks that are frozen (valid cache *or* stale). Their lanes render
    /// the frozen-render treatment — warm/audio waveform language overlaid
    /// with the frost wash and relabelled "frozen render" (design doc
    /// #181) — and, playing back from a cache with no editable sample
    /// source, their audio clips get the "unsupported" degradation surface
    /// (diagonal hatch, no fade handles; clip gain still applies — design
    /// doc #153). A track absent from the set renders live as usual.
    pub frozen_tracks: std::collections::HashSet<TrackId>,
    /// In-flight drag-to-timeline placement (doc #175, todo #605), or `None`
    /// when nothing is being dragged. When `Some`, the canvas previews the
    /// drop (lit lane, dashed ghost clip, drag pill, tooltip) and the
    /// new-audio-track drop zone below the last lane, and its pointer
    /// handlers publish `DragMessage::Hover` / `Drop`. Drawn in the uncached
    /// overlay pass so it repaints as the cursor moves.
    pub drag: Option<&'a state::DragPlacement>,
    /// Tracks whose automation lanes are expanded into dedicated slim
    /// arrange sub-rows (doc #256, todo #1096) — the transient
    /// `ClipInteractionState::automation_expanded_tracks` set, threaded
    /// in so the canvas builds the same automation-aware
    /// [`ArrangeRowLayout`] as the header column and hit-testing.
    /// Rendering of the sub-rows themselves lands in todo #1097.
    pub automation_expanded_tracks: &'a std::collections::HashSet<TrackId>,
    /// App-side mirror of the engine's cycle-record take groups (epic #15,
    /// doc #165). Every group draws a comp ribbon on its track's lane —
    /// expanded or not — and, when the track's take lane is expanded, a
    /// stack of take cards in dedicated sub-rows. Empty => no take lanes.
    pub take_groups: &'a crate::state::TakeGroupState,
    /// Tracks whose take lanes are expanded into stacked take sub-rows
    /// (`ClipInteractionState::take_lane_expanded_tracks`), threaded in so
    /// the canvas builds the same take-aware [`ArrangeRowLayout`] as the
    /// header column.
    pub take_lane_expanded_tracks: &'a std::collections::HashSet<TrackId>,
    /// Per-instance memo for [`arrange_layout`](Self::arrange_layout).
    /// One view build used to run the sort + row build 2-3× per frame
    /// (cached pass, overlay pass, hover, pointer, `content_height_px`).
    /// Everything the layout reads is behind the `&'a` borrows above, so
    /// it cannot change for the lifetime of this instance — any state
    /// edit rebuilds the view and gets a fresh, empty cell.
    pub layout_memo: std::cell::OnceCell<ArrangeRowLayout>,
    /// Per-instance memo for the O(project) content half of
    /// [`fingerprint`](Self::fingerprint). Same reasoning as
    /// `layout_memo`: the hashed data is frozen behind `&'a`, so hashing
    /// it once per view build is exactly as safe as hashing it once per
    /// rendered frame — and hover / pure-redraw frames stop paying the
    /// full walk at 60 Hz.
    pub content_fingerprint_memo: std::cell::OnceCell<TimelineFingerprint>,
    /// The *visible* part of this canvas, in canvas-local coordinates —
    /// written by the [`viewport_probe::ViewportProbe`] wrapper right
    /// before each `Widget::draw`, because the `canvas::Program` API
    /// never sees the outer `Scrollable`'s viewport. `None` (tests, or
    /// before the first draw) disables horizontal culling and everything
    /// is drawn, which is always correct — just slower.
    pub visible_viewport: std::rc::Rc<std::cell::Cell<Option<Rectangle>>>,
}

impl TimelineCanvas<'_> {
    /// Height of the always-visible global-shelf header strip (the
    /// "GLOBAL · 6/8 · 90 BPM · …" summary bar). Present regardless of
    /// the expanded state — the chat brief made the shelf "collapsable
    /// above the regular tab", so the summary line is always there.
    pub(crate) fn global_shelf_header_height(&self) -> f32 {
        theme::GLOBAL_SHELF_HEADER_HEIGHT
    }

    /// Height of the *expanded* global-tracks lane area — three rows
    /// stacked (chords + tempo + signature). Returns 0.0 when the
    /// shelf is collapsed so the lane area drops to zero and only the
    /// header strip stays.
    pub(crate) fn global_tracks_lanes_height(&self) -> f32 {
        if self.global_tracks_expanded {
            theme::GLOBAL_TRACK_CHORD_HEIGHT
                + theme::GLOBAL_TRACK_TEMPO_HEIGHT
                + theme::GLOBAL_TRACK_SIG_HEIGHT
        } else {
            0.0
        }
    }

    /// Total height of the global-tracks region (header strip + lanes).
    /// Used by `fixed_header_height` and the track-header column to
    /// keep their Y offsets in sync.
    pub(crate) fn global_tracks_height(&self) -> f32 {
        self.global_shelf_header_height() + self.global_tracks_lanes_height()
    }

    /// Height of the section-pill band sitting under the ruler. Returns
    /// 0.0 when no sections are placed so empty projects don't take a
    /// vertical hit.
    pub(crate) fn section_band_height(&self) -> f32 {
        if self.section_placements.is_empty() {
            0.0
        } else {
            theme::SECTION_BAND_HEIGHT
        }
    }

    /// Total fixed header height: ruler + section band + global tracks area.
    /// This is the Y offset where regular track rows begin.
    pub(crate) fn fixed_header_height(&self) -> f32 {
        theme::RULER_HEIGHT + self.section_band_height() + self.global_tracks_height()
    }

    /// Convert a sample position to pixel x coordinate.
    pub(crate) fn sample_to_x(&self, sample: u64) -> f32 {
        (sample as f64 / self.sample_rate as f64) as f32 * self.zoom - self.scroll_offset
    }

    /// The layout constants a drop resolution needs (see
    /// [`placement::resolve_drop`]).
    pub(crate) fn placement_geometry(&self) -> placement::PlacementGeometry {
        placement::PlacementGeometry {
            header_height: self.fixed_header_height(),
            scroll_offset_y: self.scroll_offset_y,
            zoom: self.zoom,
            sample_rate: self.sample_rate,
            bpm: self.bpm,
            time_sig_num: self.time_sig_num,
        }
    }

    /// Rightmost pixel needed to show all content (clips + MIDI clips).
    /// Always returns at least `viewport_width * 1.5` so users can scroll a
    /// bit past the last clip, and never less than `viewport_width` itself.
    pub(crate) fn content_width_px(&self, viewport_width: f32) -> f32 {
        let mut max_sample: u64 = 0;
        for c in self.clips {
            let end = c.start_sample + c.duration_samples;
            if end > max_sample {
                max_sample = end;
            }
        }
        for c in self.midi_clips {
            let end = self.tempo_map.tick_to_abs_sample(
                c.start_sample,
                c.duration_ticks,
                self.sample_rate,
            );
            if end > max_sample {
                max_sample = end;
            }
        }
        let content = (max_sample as f64 / self.sample_rate as f64) as f32 * self.zoom;
        content.max(viewport_width * 1.5).max(viewport_width)
    }

    /// Same as `content_width_px` but adds a fixed trailing pad in bars
    /// instead of inflating to 1.5× a viewport that the canvas never
    /// directly knows. Used by `view_timeline` to size the canvas
    /// inside the horizontal `Scrollable` — bounding the canvas to its
    /// own natural size lets `canvas::Cache` hit across window resizes
    /// (the cache invalidates on `bounds.size()` changes).
    pub(crate) fn content_width_natural(&self) -> f32 {
        let mut max_sample: u64 = 0;
        for c in self.clips {
            let end = c.start_sample + c.duration_samples;
            if end > max_sample {
                max_sample = end;
            }
        }
        for c in self.midi_clips {
            let end = self.tempo_map.tick_to_abs_sample(
                c.start_sample,
                c.duration_ticks,
                self.sample_rate,
            );
            if end > max_sample {
                max_sample = end;
            }
        }
        let content = (max_sample as f64 / self.sample_rate as f64) as f32 * self.zoom;
        // 8 bars of trailing pad so the user can drop new clips just
        // past the last existing one without immediately scrolling out
        // of canvas. Floor of 800 px keeps empty projects usable.
        let seconds_per_bar =
            self.time_sig_num as f32 * 60.0 / self.bpm.max(1.0);
        let pad = 8.0 * seconds_per_bar * self.zoom;
        (content + pad).max(800.0)
    }

    /// Total vertical content height (tracks + ruler + global tracks).
    /// Excludes sub-tracks since the arrange view hides them entirely.
    ///
    /// The lane area height comes from the shared [`ArrangeRowLayout`]
    /// (doc #203), so interleaved 60 px group-header rows and the missing
    /// rows of collapsed groups are reflected in the scrollbar extent —
    /// not the old `visible_tracks * TRACK_HEIGHT` uniform-pitch guess.
    pub(crate) fn content_height_px(&self) -> f32 {
        self.fixed_header_height() + self.arrange_layout().total_height()
    }

    /// Tracks visible in the arrange view, sorted by `order`. Excludes
    /// sub-tracks (rendered only in the mixer view).
    pub(super) fn visible_tracks_sorted(&self) -> Vec<&TrackState> {
        hit_test::sorted_arrange_tracks(self.tracks)
    }

    /// The shared heterogeneous arrange-row layout (group-header rows +
    /// track rows, collapse-aware) for this canvas. Both lane rendering
    /// and clip placement consume it instead of `index * TRACK_HEIGHT`.
    ///
    /// Memoized per canvas instance (`layout_memo`): the first caller in
    /// a view build pays for the sort + row build, every later caller —
    /// the overlay pass, hover, pointer, `content_height_px` — reuses
    /// it. The inputs all sit behind this struct's `&'a` borrows, so the
    /// memo can never go stale within an instance's lifetime.
    pub fn arrange_layout(&self) -> &ArrangeRowLayout {
        self.layout_memo.get_or_init(|| self.build_arrange_layout())
    }

    /// Uncached [`arrange_layout`](Self::arrange_layout) — the actual row
    /// build. Public only so the memo's null test can compare a fresh
    /// build against the memoized one; production code goes through the
    /// memo.
    #[doc(hidden)]
    pub fn build_arrange_layout(&self) -> ArrangeRowLayout {
        let sorted = self.visible_tracks_sorted();
        let automation_rows = crate::view::arrange_layout::ArrangeAutomationRows::collect(
            self.automation,
            &sorted,
            self.automation_expanded_tracks,
        );
        let take_rows = crate::view::arrange_layout::ArrangeTakeRows::collect(
            self.take_groups,
            &sorted,
            self.take_lane_expanded_tracks,
        );
        ArrangeRowLayout::build_with_takes(
            &sorted,
            self.track_groups,
            &automation_rows,
            &take_rows,
        )
    }

    /// The quantized horizontal cull window for the cached draw pass, in
    /// canvas-local x pixels. `None` — no probe write yet (tests, first
    /// use) — means "draw everything". See [`cull`] for the quantization
    /// and its covering guarantee.
    pub(crate) fn cull_window(&self) -> Option<(f32, f32)> {
        self.visible_viewport
            .get()
            .map(cull::quantized_window)
    }

    /// Horizontal pixel span of an audio clip's body, before the small
    /// per-track group indent (the cull predicate pads for it).
    pub(crate) fn audio_clip_x_span(&self, clip: &ClipState) -> (f32, f32) {
        cull::sample_span_x(
            clip.start_sample,
            clip.start_sample + clip.duration_samples,
            self.sample_rate,
            self.zoom,
            self.scroll_offset,
        )
    }

    /// Horizontal pixel span of a MIDI clip's body (tick length resolved
    /// through the tempo map, exactly as `draw_midi_clip` does).
    pub(crate) fn midi_clip_x_span(&self, clip: &MidiClipState) -> (f32, f32) {
        let end = self.tempo_map.tick_to_abs_sample(
            clip.start_sample,
            clip.duration_ticks,
            self.sample_rate,
        );
        cull::sample_span_x(
            clip.start_sample,
            end,
            self.sample_rate,
            self.zoom,
            self.scroll_offset,
        )
    }
}

/// Local state for the timeline canvas, tracking active drag operations.
#[derive(Debug, Default)]
pub struct TimelineState {
    pub(super) dragging_loop: bool,
    pub(super) clip_interaction: Option<ClipInteraction>,
    pub(super) last_reported_width: f32,
    pub(super) last_reported_height: f32,
    pub(super) last_reported_content_width: f32,
    pub(super) last_reported_content_height: f32,
    /// Tracks the most recent click on a MIDI clip for double-click detection.
    pub(super) last_midi_click: Option<(Instant, ClipId)>,
    /// Horizontal scrollbar drag in progress. Stores the x-offset of the
    /// grab point relative to the left edge of the thumb (in track pixels).
    pub(super) h_scrollbar_grab: Option<f32>,
    /// Vertical scrollbar drag in progress (y-offset within the thumb).
    pub(super) v_scrollbar_grab: Option<f32>,
    /// Tracks the most recent click on a global track for double-click detection.
    pub(super) last_global_click: Option<(Instant, state::GlobalTrackKind)>,
    /// Active tempo-event drag.
    pub(super) tempo_drag: Option<TempoDrag>,
    /// Active automation-breakpoint drag (todo #382).
    pub(super) breakpoint_drag: Option<BreakpointDrag>,
    /// The breakpoint the last press landed on, kept for keyboard delete.
    /// Cleared by any press that doesn't hit a breakpoint. View-local —
    /// there's no on-canvas highlight for it yet.
    pub(super) selected_breakpoint: Option<(AutomationTarget, usize)>,
    /// Last breakpoint press, for double-click (curve-kind toggle) detection.
    pub(super) last_breakpoint_click: Option<(Instant, AutomationTarget, usize)>,
    /// In-flight comping gesture on a take card (epic #15, todo #414).
    /// Opened by the press, resolved by the release into either a solo
    /// (click) or a promote (drag) — see [`TakePromoteDrag`].
    pub(super) take_promote_drag: Option<TakePromoteDrag>,
    /// Active arrangement-marker drag (start move or region-edge resize).
    pub(super) marker_drag: Option<MarkerDrag>,
    /// Most recent click on a marker flag, for double-click (rename) detection.
    pub(super) last_marker_click: Option<(Instant, u64)>,
    /// Whether the timeline owns the keyboard (last mouse press landed on
    /// it) — gates Delete so it never fires from another surface.
    pub(super) key_focus: crate::focus::KeyFocus,
    /// Geometry cache — re-runs the draw closure only when the cached
    /// fingerprint mismatches. Skips a full redraw on every hover /
    /// sibling-update event, which is most of them.
    pub(super) cache: canvas::Cache,
    /// Snapshot of the input fields that affect the rendered geometry
    /// at the moment the cache was last filled. The draw routine
    /// compares the current frame's fingerprint to this and invalidates
    /// the cache when any field changes.
    pub(super) cache_fingerprint: std::cell::Cell<TimelineFingerprint>,
}

/// Compact summary of the data the timeline reads. When any of these
/// values changes between frames, the canvas geometry needs a redraw.
/// `Default` returns a sentinel value the first frame can never match,
/// so the very first draw fills the cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimelineFingerprint {
    pub clips_len: usize,
    pub midi_clips_len: usize,
    pub tracks_len: usize,
    // NOTE: playhead and recording_start_sample are intentionally NOT
    // in the fingerprint. They change continuously during playback /
    // recording and would invalidate the cache every frame, defeating
    // the whole purpose. The playhead overlay (line + tab) and the
    // recording overlay are drawn as a separate uncached Geometry in
    // `Program::draw` below.
    pub zoom_bits: u32,
    pub scroll_x_bits: u32,
    pub scroll_y_bits: u32,
    pub recording_count: usize,
    pub loop_enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
    pub selected_clip: Option<ClipId>,
    pub selected_midi_clip: Option<ClipId>,
    pub selected_track: Option<TrackId>,
    pub global_tracks_expanded: bool,
    pub tempo_points: usize,
    pub signature_points: usize,
    /// Hash of every tempo event's `(bar, bpm)` so the cache invalidates
    /// when an *existing* event's value changes (drag, transport-side
    /// BPM commit, pick_list edit). Just tracking `tempo_points.len()`
    /// would miss in-place edits and leave the canvas curve stale.
    pub tempo_events_hash: u64,
    /// Same idea for signature events: their `(bar, numerator,
    /// denominator)` so pill markers + label text redraw on edit.
    pub signature_events_hash: u64,
    /// Currently selected tempo/signature event. The draw routine
    /// recolors the selected dot / pill marker with the accent so this
    /// has to enter the fingerprint, otherwise a fresh click "lands"
    /// in state but doesn't repaint until something else does.
    pub selected_global_event: Option<state::SelectedGlobalEvent>,
    pub bpm_bits: u32,
    pub time_sig_num: u8,
    pub section_placements_len: usize,
    pub section_definitions_len: usize,
    pub selected_placement_id: Option<u64>,
    /// Sum of every section definition's chord count. Drives the
    /// chord-lane redraw inside the global shelf — without this the
    /// canvas cache would hold a stale chord layout after a chord is
    /// added / removed / re-rolled inside Compose.
    pub section_chord_total: usize,
    /// Hash of every automation lane's target, Read state and breakpoints.
    /// Repaints the cached lane geometry when a lane is added, cleared, or
    /// edited. The *live* playhead value is drawn uncached, so it is
    /// intentionally excluded here (same discipline as `playhead`).
    pub automation_hash: u64,
    pub markers_len: usize,
    /// Hash of every marker's `(id, start, end, name, color)` so the cache
    /// invalidates when a marker is added, moved, renamed, recolored or
    /// turned into a region — tracking the count alone would miss in-place
    /// edits and leave a stale flag / span on screen.
    pub markers_hash: u64,
    pub selected_marker_id: Option<u64>,
    /// Hash of every clip's geometry + fade/gain shaping
    /// (`id, track, start, duration, fade-in/out frames+curve, gain`).
    /// Without this the static clip layer would hold a stale render
    /// after a clip is moved, trimmed, or has its fade / gain edited —
    /// `clips_len` alone misses every in-place change. Keeps the canvas
    /// to the view-performance rule: repaint on edit, not per frame.
    pub clips_hash: u64,
    /// Order-independent hash of the frozen-track set (a `HashSet` has no
    /// stable iteration order). Invalidates the cache when a track freezes
    /// / unfreezes so its lane repaints with (or without) the frozen-render
    /// treatment and the "unsupported" hatch.
    pub frozen_hash: u64,
    /// Order-independent hash of the automation-expanded-track set (doc
    /// #256, todo #1097). Expanding / collapsing a track's lane sub-rows
    /// reshapes the whole `ArrangeRowLayout` under the cached layer (rows
    /// shift, the overlay band is suppressed, lane rows appear), so the
    /// toggle must repaint the canvas — without this the cached geometry
    /// goes stale the moment the caret is clicked.
    pub automation_expanded_hash: u64,
    /// Order-independent hash of the track-group registry — per group its
    /// `(id, ordered_members, nesting_parent, is_collapsed,
    /// identity_color)` plus the group count. Groups reshape the cached
    /// layer wholesale: the `ArrangeRowLayout` interleaves 60 px header
    /// bands and drops a collapsed group's member rows, the band paints
    /// the identity wash (or the #733 consolidated overview), and every
    /// row below shifts Y. Without this, `GroupMessage::ToggleCollapse`
    /// (header caret or canvas double-click) and drag-and-drop membership
    /// edits leave the canvas drawing pre-toggle geometry until an
    /// unrelated input happens to invalidate the cache.
    pub groups_hash: u64,
    /// Hash of every take group's identity, slot, takes and comp cover
    /// (epic #15). Repaints the cached layer when a pass is captured, the
    /// comp is edited, or the active take changes — all of which reshape
    /// both the ribbon and the stacked cards. Nothing in the take lane is
    /// playhead-driven, so the whole feature stays inside the cached pass.
    pub takes_hash: u64,
    /// Order-independent hash of the take-lane-expanded track set.
    /// Expanding a lane inserts sub-rows and shifts every row below it, so
    /// the toggle must repaint (same discipline as
    /// `automation_expanded_hash`).
    pub take_expanded_hash: u64,
    /// Hash of every MIDI clip's geometry + note-minimap content
    /// (`id, track, start, duration/trim ticks, name`, plus each note's
    /// pitch and horizontal extent). `update_midi_clip_drag` mutates
    /// `start_sample` / `track_id` in place mid-drag and the piano roll
    /// edits `notes` in place, and there is no MIDI drag ghost in the
    /// uncached overlay pass — so `midi_clips_len` alone left a stale
    /// arrange render behind. Same discipline as `clips_hash` on the
    /// audio side. Note velocity is not drawn by the minimap and is
    /// deliberately excluded (same reasoning as the playhead).
    pub midi_clips_hash: u64,
    /// The quantized horizontal cull window the cached pass drew, as the
    /// f32 bit patterns of its two x endpoints (±∞ = no culling). The
    /// window is re-derived from the live viewport on every rendered
    /// frame, so including it here guarantees that scrolling past the
    /// drawn margin repaints in the same frame that reveals it — see
    /// [`cull`].
    pub cull_x0_bits: u32,
    pub cull_x1_bits: u32,
}

impl<'a> TimelineCanvas<'a> {
    /// The full cache fingerprint for this frame: the memoized O(project)
    /// content hash (`content_fingerprint`, computed at most once per
    /// view build) plus the per-frame quantized cull window. Only the
    /// window can change between two draws of the same instance — every
    /// piece of hashed content sits behind `&'a` borrows.
    pub(crate) fn fingerprint(&self) -> TimelineFingerprint {
        let mut fp = *self
            .content_fingerprint_memo
            .get_or_init(|| self.content_fingerprint());
        let (x0, x1) = self
            .cull_window()
            .unwrap_or((f32::NEG_INFINITY, f32::INFINITY));
        fp.cull_x0_bits = x0.to_bits();
        fp.cull_x1_bits = x1.to_bits();
        fp
    }

    fn content_fingerprint(&self) -> TimelineFingerprint {
        // Hash the full tempo + signature event content so any
        // *in-place* edit (drag, pick_list change, transport-bar
        // commit) invalidates the cache and the curve / pill markers
        // redraw on the next frame.
        use std::hash::{Hash, Hasher};
        let mut th = std::collections::hash_map::DefaultHasher::new();
        for e in &self.tempo_map.tempo_points {
            e.bar.hash(&mut th);
            e.bpm.to_bits().hash(&mut th);
        }
        let tempo_events_hash = th.finish();
        let mut sh = std::collections::hash_map::DefaultHasher::new();
        for e in &self.tempo_map.signature_points {
            e.bar.hash(&mut sh);
            e.numerator.hash(&mut sh);
            e.denominator.hash(&mut sh);
        }
        let signature_events_hash = sh.finish();

        // Hash lane geometry so an add / clear / breakpoint edit invalidates
        // the cached lane layer. Iterating the HashMap in arbitrary order is
        // fine: equal lane sets must hash equal, so fold each lane's hash into
        // an order-independent xor accumulator.
        let mut automation_hash: u64 = self.automation.lanes.len() as u64;
        for lane in self.automation.lanes.values() {
            let mut lh = std::collections::hash_map::DefaultHasher::new();
            lane.target.hash(&mut lh);
            lane.id.hash(&mut lh);
            lane.enabled.hash(&mut lh);
            for p in &lane.points {
                p.time_frames.hash(&mut lh);
                p.value.to_bits().hash(&mut lh);
                p.curve.hash(&mut lh);
            }
            automation_hash ^= lh.finish();
        }
        // The band's label chip renders resolved device-param names (todo
        // #1094), so a preset change that renames a lane's label must also
        // invalidate the cached lane layer even though the lane itself is
        // untouched. Same order-independent xor fold.
        for (target, name) in &self.device_param_labels {
            let mut lh = std::collections::hash_map::DefaultHasher::new();
            target.hash(&mut lh);
            name.hash(&mut lh);
            automation_hash ^= lh.finish();
        }
        // The per-track chip-cycle lane selection (todo #1095) decides
        // *which* lane the cached band draws, so switching lanes must
        // repaint even though no lane data changed. Same order-independent
        // xor fold; the leading tag keeps a selection entry from ever
        // cancelling a lane or label hash.
        for (track, lane_id) in &self.automation.lane_selection {
            let mut lh = std::collections::hash_map::DefaultHasher::new();
            0x1095_u16.hash(&mut lh);
            track.hash(&mut lh);
            lane_id.hash(&mut lh);
            automation_hash ^= lh.finish();
        }

        // Hash the full marker content so any in-place edit (move, rename,
        // recolor, region resize) invalidates the cache and the flag / span
        // redraws on the next frame.
        let mut mh = std::collections::hash_map::DefaultHasher::new();
        for m in self.markers {
            m.id.hash(&mut mh);
            m.start_sample.hash(&mut mh);
            m.end_sample.hash(&mut mh);
            m.name.hash(&mut mh);
            m.color.hash(&mut mh);
        }
        let markers_hash = mh.finish();

        // Hash every clip's geometry + fade/gain shaping so the cached
        // clip layer invalidates on move / trim / fade / gain edits.
        let mut clip_h = std::collections::hash_map::DefaultHasher::new();
        for c in self.clips {
            c.id.hash(&mut clip_h);
            c.track_id.hash(&mut clip_h);
            c.start_sample.hash(&mut clip_h);
            c.duration_samples.hash(&mut clip_h);
            c.fade_in_frames.hash(&mut clip_h);
            c.fade_in_curve.hash(&mut clip_h);
            c.fade_out_frames.hash(&mut clip_h);
            c.fade_out_curve.hash(&mut clip_h);
            c.gain_db.to_bits().hash(&mut clip_h);
        }
        let clips_hash = clip_h.finish();

        // Hash every MIDI clip's geometry + note-minimap content so the
        // cached layer invalidates on move / trim / rename / note edits.
        // `update_midi_clip_drag` mutates `start_sample` / `track_id` in
        // place mid-drag and the piano roll edits `notes` in place, so —
        // exactly like `clips_hash` above — the count alone would leave a
        // stale arrange render on screen.
        let mut midi_h = std::collections::hash_map::DefaultHasher::new();
        for c in self.midi_clips {
            c.id.hash(&mut midi_h);
            c.track_id.hash(&mut midi_h);
            c.start_sample.hash(&mut midi_h);
            c.duration_ticks.hash(&mut midi_h);
            c.trim_start_ticks.hash(&mut midi_h);
            c.trim_end_ticks.hash(&mut midi_h);
            c.name.hash(&mut midi_h);
            // Pitch + horizontal extent are what the note minimap (and
            // the frozen-render silhouette) read; velocity is not drawn
            // and is deliberately excluded.
            c.notes.len().hash(&mut midi_h);
            for n in &c.notes {
                n.note.hash(&mut midi_h);
                n.start_tick.hash(&mut midi_h);
                n.duration_ticks.hash(&mut midi_h);
            }
        }
        let midi_clips_hash = midi_h.finish();

        // XOR-fold the frozen track ids so the hash is independent of the
        // set's iteration order (a `HashSet` has no stable order). Seeded
        // with the set's length — the same idiom as the expanded-set
        // folds below — so the empty set can never collide with a set
        // whose ids XOR to zero.
        let frozen_hash = self
            .frozen_tracks
            .iter()
            .fold(self.frozen_tracks.len() as u64, |acc, id| {
                acc ^ id.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            });

        // Same order-independent fold for the automation-expanded set
        // (todo #1097): a toggle restructures the arrange rows, so the
        // cached layer must repaint. Fold the count in too so the empty
        // set can never collide with a set whose ids XOR to zero.
        let automation_expanded_hash = self
            .automation_expanded_tracks
            .iter()
            .fold(self.automation_expanded_tracks.len() as u64, |acc, id| {
                acc ^ id.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            });

        // Same order-independent fold for the track-group registry (its
        // backing `HashMap` has no stable iteration order): each group's
        // layout- and paint-driving fields, folded over the group count so
        // an empty registry can never collide with hashes that XOR to
        // zero. Macro mute/solo/level are deliberately excluded — they
        // cascade to the mixer, not to this canvas.
        let mut groups_hash: u64 = self.track_groups.len() as u64;
        for group in self.track_groups.get_all_groups() {
            let mut gh = std::collections::hash_map::DefaultHasher::new();
            group.id.hash(&mut gh);
            group.ordered_members.hash(&mut gh);
            group.nesting_parent.hash(&mut gh);
            group.is_collapsed.hash(&mut gh);
            group.identity_color.hash(&mut gh);
            groups_hash ^= gh.finish();
        }

        // Take groups are an ordered Vec, so a plain sequential hash is
        // enough — but hash the *content* (slot, every take's id / pass /
        // content shape, the comp cover and the active take), not just the
        // count: a comp edit or an active-take change leaves the group
        // count untouched and would otherwise leave a stale ribbon.
        let mut take_h = std::collections::hash_map::DefaultHasher::new();
        self.take_groups.groups.len().hash(&mut take_h);
        for group in &self.take_groups.groups {
            group.id.hash(&mut take_h);
            group.track_id.hash(&mut take_h);
            group.slot.hash(&mut take_h);
            group.active_take.hash(&mut take_h);
            for take in &group.takes {
                take.id.hash(&mut take_h);
                take.pass_index.hash(&mut take_h);
                // `captured_at` is the key `effective_cover` sorts on to
                // pick the latest take, so two groups differing only in
                // capture order resolve to different covers and must not
                // share a fingerprint.
                take.captured_at.hash(&mut take_h);
                // The take's own recorded span. Since todo #1396 this is
                // what the card's *width* is, so a card cannot be allowed
                // to keep a cached geometry from a different extent.
                take.extent.hash(&mut take_h);
                // The waveform the card carries, read off the take's WAV
                // (todo #1400). Length rather than content: a peak table
                // is derived once, from a file, and never edited — every
                // way it can change for a fixed `(group, take)` key
                // (capture, re-capture, project load, a failed read
                // leaving it empty) changes how many buckets it has or
                // clears it, and hashing 3000 f32 pairs per take on the
                // fingerprint path would cost more than the repaint it
                // saves.
                match &take.content {
                    resonance_common::TakeContent::Audio { clip_ref } => {
                        0u8.hash(&mut take_h);
                        clip_ref.hash(&mut take_h);
                        // The waveform the card carries, read off this
                        // recording (todo #1400). Length rather than
                        // content: a peak table is derived once, from a
                        // file, and never edited — every way it can change
                        // for a fixed `(group, take, clip_ref)` triple
                        // (capture, re-capture, project load, a failed
                        // read leaving it absent) changes how many buckets
                        // it has or drops it, and hashing thousands of f32
                        // pairs per take on the fingerprint path would
                        // cost more than the repaint it saves.
                        self.take_groups
                            .peaks(group.id, take.id, *clip_ref)
                            .len()
                            .hash(&mut take_h);
                    }
                    resonance_common::TakeContent::Midi { notes } => {
                        1u8.hash(&mut take_h);
                        notes.len().hash(&mut take_h);
                        for n in notes {
                            n.note.hash(&mut take_h);
                            n.start_tick.hash(&mut take_h);
                            n.duration_ticks.hash(&mut take_h);
                        }
                    }
                }
            }
            for seg in &group.comp.segments {
                seg.take_id.hash(&mut take_h);
                seg.range.hash(&mut take_h);
            }
        }
        // Takes whose recorded WAV was absent at load (todo #412) draw the
        // hatched missing-media card instead of a waveform, so relinking
        // one has to repaint. Order-independent fold: a `HashSet` has no
        // stable iteration order.
        let missing_fold = self
            .take_groups
            .missing_takes
            .iter()
            .fold(self.take_groups.missing_takes.len() as u64, |acc, (g, t)| {
                acc ^ g
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .rotate_left(17)
                    .wrapping_add(*t)
            });
        missing_fold.hash(&mut take_h);
        let takes_hash = take_h.finish();

        // Order-independent fold over the take-lane-expanded set, with the
        // count folded in so the empty set can never collide with a set
        // whose ids XOR to zero.
        let take_expanded_hash = self
            .take_lane_expanded_tracks
            .iter()
            .fold(self.take_lane_expanded_tracks.len() as u64, |acc, id| {
                acc ^ id.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            });

        TimelineFingerprint {
            clips_len: self.clips.len(),
            midi_clips_len: self.midi_clips.len(),
            tracks_len: self.tracks.len(),
            zoom_bits: self.zoom.to_bits(),
            scroll_x_bits: self.scroll_offset.to_bits(),
            scroll_y_bits: self.scroll_offset_y.to_bits(),
            recording_count: self.recording_tracks.len(),
            loop_enabled: self.loop_enabled,
            loop_in: self.loop_in,
            loop_out: self.loop_out,
            selected_clip: self.selected_clip,
            selected_midi_clip: self.selected_midi_clip,
            selected_track: self.selected_track,
            global_tracks_expanded: self.global_tracks_expanded,
            tempo_points: self.tempo_map.tempo_points.len(),
            signature_points: self.tempo_map.signature_points.len(),
            tempo_events_hash,
            signature_events_hash,
            selected_global_event: self.selected_global_event,
            bpm_bits: self.bpm.to_bits(),
            time_sig_num: self.time_sig_num,
            section_placements_len: self.section_placements.len(),
            section_definitions_len: self.section_definitions.len(),
            selected_placement_id: self.selected_placement_id,
            section_chord_total: self
                .section_definitions
                .iter()
                .map(|d| d.chords.len())
                .sum(),
            automation_hash,
            markers_len: self.markers.len(),
            markers_hash,
            selected_marker_id: self.selected_marker_id,
            clips_hash,
            frozen_hash,
            automation_expanded_hash,
            groups_hash,
            takes_hash,
            take_expanded_hash,
            midi_clips_hash,
            // Patched per frame by `fingerprint` from the live viewport;
            // the memoized content half always carries the no-cull
            // sentinel.
            cull_x0_bits: f32::NEG_INFINITY.to_bits(),
            cull_x1_bits: f32::INFINITY.to_bits(),
        }
    }
}

impl canvas::Program<Message> for TimelineCanvas<'_> {
    type State = TimelineState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        state.key_focus.track(event, bounds, cursor);
        let result = match event {
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                self.handle_wheel(*delta, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                self.handle_press(state, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                self.handle_right_press(state, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                self.handle_move(state, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                self.handle_release(state)
            }
            iced::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                self.handle_key(state, key)
            }
            _ => None,
        };
        if result.is_some() {
            return result;
        }
        if let Some(msg) = self.report_viewport(state, bounds) {
            return Some(canvas::Action::publish(msg));
        }
        None
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        self.hover_interaction(state, bounds, cursor)
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        // Cache invalidation: re-runs the body only when our fingerprint
        // changes. Pure hover/sibling redraws hit the cached geometry.
        // `playhead` and `recording_start_sample` are intentionally
        // excluded from the fingerprint — the playhead line + tab and
        // the per-track recording overlay are drawn in a second
        // uncached pass below so they update every frame without
        // invalidating the rest of the timeline.
        let fp = self.fingerprint();
        if state.cache_fingerprint.get() != fp {
            state.cache.clear();
            state.cache_fingerprint.set(fp);
        }
        let cached = state.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_into(frame, bounds);
        });
        let mut overlay = canvas::Frame::new(renderer, bounds.size());
        self.draw_overlay_into(&mut overlay, bounds, state, cursor);
        vec![cached, overlay.into_geometry()]
    }
}

impl<'a> TimelineCanvas<'a> {
    /// Run the full draw routine onto the given frame. Split out so the
    /// `Program::draw` impl can wrap it in `canvas::Cache`.
    fn draw_into(&self, frame: &mut canvas::Frame, bounds: Rectangle) {
        let ruler_height = theme::RULER_HEIGHT;
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;

        // Draw ruler background
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(bounds.width, ruler_height),
            theme::BG_1,
        );

        // Section-pill band — sits under the ruler when at least one
        // compose section is placed. Render before global tracks so the
        // global tracks shift down accordingly.
        let band_top = ruler_height;
        let band_height = self.section_band_height();
        if band_height > 0.0 {
            self.draw_section_band(frame, bounds.width, band_top, band_height);
        }

        // Draw global tracks area (tempo + time signature) between the
        // section band and the regular tracks.
        self.draw_global_tracks(frame, bounds.width, band_top + band_height);

        // Draw track backgrounds. Only non-sub-tracks are rendered; the
        // mixer view is where sub-track lanes live. The layout enumerates
        // the heterogeneous rows (group-header bands + track lanes) with
        // their own heights / cumulative Y — replacing the old uniform
        // `index * TRACK_HEIGHT` pitch (doc #203).
        let layout = self.arrange_layout();
        let track_area_height = layout.total_height();

        // Everything inside the lane region — track rows, grid lines,
        // clips, and the loop in/out dim overlays — is clipped to the
        // area below the fixed header. Without this, a track straddling
        // the header_height boundary (sub-row vertical scroll, or a
        // partial top row when scrolled mid-row) paints its background
        // and clip body over the ruler / section-pill band / global
        // tracks above. Ruler labels and loop / playhead markers stay
        // outside the clip on purpose so their handles can sit on top
        // of the ruler.
        let lane_clip = Rectangle {
            x: 0.0,
            y: header_height,
            width: bounds.width,
            height: (bounds.height - header_height).max(0.0),
        };
        let loop_dim_color = Color::from_rgba(0.0, 0.0, 0.0, 0.15);
        let loop_total_height_below_header =
            (track_area_height - y_off).max(bounds.height - header_height);
        let loop_dim_height = loop_total_height_below_header.max(0.0);
        frame.with_clip(lane_clip, |frame| {
            // Zebra striping alternates over *track* rows only, so an
            // interleaved 60 px group-header band doesn't flip the
            // parity of the lanes around it.
            let mut zebra = 0usize;
            for row in layout.rows() {
                let y = header_height + row.y_top - y_off;

                // Skip rows entirely above or below the visible area.
                if y + row.height < header_height || y > bounds.height {
                    if matches!(row.kind, ArrangeRowKind::Track(_)) {
                        zebra += 1;
                    }
                    continue;
                }

                match row.kind {
                    ArrangeRowKind::GroupHeader(group_id) => {
                        // Group-colour lane band. When the group is
                        // expanded this is the faint "spans all members"
                        // identity tint; a collapsed group gets a plain
                        // band repainted as the consolidated overview —
                        // every member clip flattened onto the lane as a
                        // tinted block (todo #733).
                        if let Some(group) = self.track_groups.get_group(group_id) {
                            let (_base, wash, line) =
                                theme::group_identity_colors(group.identity_color);
                            let band = if group.is_collapsed { theme::BG_2 } else { wash };
                            frame.fill_rectangle(
                                Point::new(0.0, y),
                                Size::new(bounds.width, row.height),
                                band,
                            );
                            // Identity-coloured bottom rule so the band
                            // reads as a structural divider.
                            frame.fill_rectangle(
                                Point::new(0.0, y + row.height - 1.0),
                                Size::new(bounds.width, 1.0),
                                line,
                            );
                        }
                    }
                    ArrangeRowKind::Track(track_id) => {
                        let is_selected = self.selected_track == Some(track_id);
                        let bg = if is_selected {
                            theme::BG_2
                        } else if zebra % 2 == 0 {
                            theme::BG
                        } else {
                            theme::PANEL_DARK
                        };
                        frame.fill_rectangle(
                            Point::new(0.0, y),
                            Size::new(bounds.width, row.height),
                            bg,
                        );

                        // Recording overlay is drawn in the uncached
                        // overlay pass — see `draw_overlay_into`. It grows
                        // with `playhead`, which would otherwise
                        // invalidate the cache every frame.

                        // Track separator line
                        frame.fill_rectangle(
                            Point::new(0.0, y + row.height - 1.0),
                            Size::new(bounds.width, 1.0),
                            theme::LINE_2,
                        );
                        zebra += 1;
                    }
                    // Dedicated automation sub-row (doc #256, todo #1097):
                    // a slightly recessed band background so the lane rows
                    // read subordinate to their track, plus the same
                    // bottom hairline as track rows. The lane's envelope,
                    // breakpoints and label are drawn by
                    // `draw_automation_lanes` below (after clips), sharing
                    // this row's geometry through the layout. Zebra parity
                    // deliberately doesn't advance — lane rows ride along
                    // with their track.
                    ArrangeRowKind::AutomationLane { .. } => {
                        frame.fill_rectangle(
                            Point::new(0.0, y),
                            Size::new(bounds.width, row.height),
                            theme::BG_2,
                        );
                        frame.fill_rectangle(
                            Point::new(0.0, y + row.height - 1.0),
                            Size::new(bounds.width, 1.0),
                            theme::LINE_2,
                        );
                    }
                    // Take sub-row (epic #15): the same recessed substrate
                    // as an automation lane row, so both stacks read as
                    // detail hanging off their track. The take card itself
                    // is drawn by `draw_take_rows` below, after the clips.
                    // Zebra parity deliberately doesn't advance.
                    ArrangeRowKind::TakeRow { .. } => {
                        frame.fill_rectangle(
                            Point::new(0.0, y),
                            Size::new(bounds.width, row.height),
                            theme::BG_2,
                        );
                        frame.fill_rectangle(
                            Point::new(0.0, y + row.height - 1.0),
                            Size::new(bounds.width, 1.0),
                            theme::LINE_2,
                        );
                    }
                }
            }

            // Draw bar/beat grid lines through track area
            self.draw_grid_lines(
                frame,
                bounds.width,
                header_height,
                track_area_height,
                y_off,
            );

            // Consolidated overviews for collapsed groups (todo #733) sit
            // above the grid, like the real clip bodies they stand in for.
            for row in layout.rows() {
                let ArrangeRowKind::GroupHeader(group_id) = row.kind else {
                    continue;
                };
                let Some(group) = self.track_groups.get_group(group_id) else {
                    continue;
                };
                if group.is_collapsed {
                    let y = header_height + row.y_top - y_off;
                    self.draw_collapsed_group_overview(frame, group, y, row.height);
                }
            }

            // Draw audio clips. Horizontally culled to the quantized
            // visible window (see [`cull`]): the canvas spans the whole
            // song but only the outer `Scrollable`'s viewport is ever on
            // screen, and the window is a fingerprint field re-derived on
            // every rendered frame — so a scroll that would reveal a
            // skipped clip repaints in the very frame that reveals it.
            let cull = self.cull_window();
            for clip in self.clips {
                if !cull::span_may_be_visible(self.audio_clip_x_span(clip), cull) {
                    continue;
                }
                self.draw_clip(
                    frame,
                    clip,
                    &layout,
                    header_height,
                    y_off,
                    bounds.height,
                );
            }

            // Automatic crossfades where two same-track audio clips
            // overlap — drawn after the clip bodies so the lavender
            // overlap wash + crossing curves sit on top of both clips.
            self.draw_crossfades(frame, &layout, header_height, y_off, bounds.height);

            // Draw MIDI clips — same horizontal cull as the audio clips
            // above (the note minimap is the per-clip cost here).
            for clip in self.midi_clips {
                if !cull::span_may_be_visible(self.midi_clip_x_span(clip), cull) {
                    continue;
                }
                self.draw_midi_clip(
                    frame,
                    clip,
                    &layout,
                    header_height,
                    y_off,
                    bounds.height,
                );
            }

            // Automation lane overlay (static layer: axis, segments, dots).
            // Drawn over the clips so the envelope reads on top of them; the
            // live playhead value rides the uncached overlay pass below.
            self.draw_automation_lanes(frame, &layout, header_height, y_off, bounds);

            // Take lanes (epic #15, doc #165). The comp ribbon rides the
            // track's own lane so a *folded* take lane still shows which
            // take is audible where; the stacked take cards fill the
            // dedicated sub-rows an expanded lane adds. Both live in this
            // cached pass — nothing in a take lane follows the playhead.
            self.draw_take_comp_ribbons(frame, &layout, header_height, y_off, bounds);
            self.draw_take_rows(frame, &layout, header_height, y_off, bounds);

            // Lane-area portion of the loop in/out markers — the dim
            // overlays. The vertical loop lines, amber range fill, and
            // triangle handles are drawn below in the unclipped pass so
            // they cross over the ruler.
            if self.loop_enabled {
                let loop_in_x = self.sample_to_x(self.loop_in);
                let loop_out_x = self.sample_to_x(self.loop_out);

                if loop_in_x > 0.0 {
                    frame.fill_rectangle(
                        Point::new(0.0, header_height),
                        Size::new(loop_in_x.min(bounds.width), loop_dim_height),
                        loop_dim_color,
                    );
                }
                if loop_out_x < bounds.width {
                    let right_start = loop_out_x.max(0.0);
                    frame.fill_rectangle(
                        Point::new(right_start, header_height),
                        Size::new((bounds.width - right_start).max(0.0), loop_dim_height),
                        loop_dim_color,
                    );
                }
            }
        });

        // Draw bar/beat ruler (after the clipped lane pass so the ruler
        // labels always sit on top of the fixed-header backdrop).
        self.draw_ruler(frame, bounds.width, ruler_height);

        // Draw the unclipped portions of the loop markers — the ruler
        // amber fill, the vertical loop lines, and the triangle handles.
        // These intentionally cross the ruler / section band so the
        // handles read as draggable from above the lanes.
        if self.loop_enabled {
            let loop_in_x = self.sample_to_x(self.loop_in);
            let loop_out_x = self.sample_to_x(self.loop_out);
            let total_height = (header_height + track_area_height - y_off).max(bounds.height);
            let loop_color = theme::WARM;

            // Amber range fill in ruler area
            let range_x = loop_in_x.max(0.0);
            let range_w = (loop_out_x - range_x).max(0.0).min(bounds.width - range_x);
            if range_w > 0.0 {
                frame.fill_rectangle(
                    Point::new(range_x, 0.0),
                    Size::new(range_w, ruler_height),
                    Color::from_rgba(0.9, 0.72, 0.1, 0.15),
                );
            }

            // Loop In line + handle
            if loop_in_x >= -1.0 && loop_in_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(loop_in_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    loop_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(loop_in_x - 6.0, 0.0));
                    b.line_to(Point::new(loop_in_x + 6.0, 0.0));
                    b.line_to(Point::new(loop_in_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, loop_color);
            }

            // Loop Out line + handle
            if loop_out_x >= -1.0 && loop_out_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(loop_out_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    loop_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(loop_out_x - 6.0, 0.0));
                    b.line_to(Point::new(loop_out_x + 6.0, 0.0));
                    b.line_to(Point::new(loop_out_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, loop_color);
            }
        }

        // Arrangement-marker flags + region spans, drawn in the ruler band
        // on top of the bar/beat ticks and the loop range so the flags and
        // labels stay legible. Static layer — invalidates only on marker
        // edits via the fingerprint, never per playback frame.
        self.draw_markers(frame, bounds.width, ruler_height);

        // Playhead is drawn in the uncached overlay pass — see
        // `draw_overlay_into`. Keeping it out of the cached path lets
        // the rest of the timeline geometry stay cached during playback.

        // Smart scrollbars — drawn last so they sit above clips + playhead.
        let (h_rects, v_rects) = self.scrollbar_rects(bounds);

        let track_color = Color::from_rgba(0.08, 0.08, 0.08, 0.8);
        let thumb_color = Color::from_rgba(0.45, 0.45, 0.45, 0.85);

        if let Some(sb) = h_rects {
            frame.fill_rectangle(
                Point::new(sb.track.x, sb.track.y),
                Size::new(sb.track.width, sb.track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(sb.thumb.x, sb.thumb.y),
                Size::new(sb.thumb.width, sb.thumb.height),
                thumb_color,
            );
        }
        if let Some(sb) = v_rects {
            frame.fill_rectangle(
                Point::new(sb.track.x, sb.track.y),
                Size::new(sb.track.width, sb.track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(sb.thumb.x, sb.thumb.y),
                Size::new(sb.thumb.width, sb.thumb.height),
                thumb_color,
            );
        }

    }

    /// Draw the parts of the timeline that change every frame during
    /// playback / recording: the playhead line + tab, and the per-track
    /// recording overlay. Called from `Program::draw` on a fresh
    /// uncached `Frame` so these don't trigger cache invalidation.
    fn draw_overlay_into(
        &self,
        frame: &mut canvas::Frame,
        bounds: Rectangle,
        state: &TimelineState,
        cursor: mouse::Cursor,
    ) {
        let ruler_height = theme::RULER_HEIGHT;
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let layout = self.arrange_layout();
        let track_area_height = layout.total_height();

        // Per-track recording overlay (one shaded strip per armed
        // track that spans from record-start to playhead, wrapping
        // around loop bounds when looping is active). Same clipping
        // discipline as `draw_into`: the strip is clipped to the lane
        // area so a partially-scrolled top row doesn't bleed its red
        // wash into the ruler / section band / global tracks header.
        if !self.recording_tracks.is_empty() {
            let lane_clip = Rectangle {
                x: 0.0,
                y: header_height,
                width: bounds.width,
                height: (bounds.height - header_height).max(0.0),
            };
            frame.with_clip(lane_clip, |frame| {
                for &track_id in &self.recording_tracks {
                    // A recording member of a collapsed group has no
                    // visible lane (`track_row_rect` -> None), so its
                    // overlay is skipped — matching the hidden clips.
                    let Some((row_y_top, row_height)) = layout.track_row_rect(track_id)
                    else {
                        continue;
                    };
                    let y = header_height + row_y_top - y_off;
                    if y + row_height < header_height || y > bounds.height {
                        continue;
                    }
                    let (overlay_start, overlay_end) = if self.loop_enabled {
                        (self.loop_in, self.playhead.min(self.loop_out))
                    } else {
                        (self.recording_start_sample, self.playhead)
                    };
                    let start_x = self.sample_to_x(overlay_start);
                    let end_x = self.sample_to_x(overlay_end);
                    let overlay_x = start_x.max(0.0);
                    let overlay_w =
                        (end_x - overlay_x).max(0.0).min(bounds.width - overlay_x);
                    if overlay_w > 0.0 {
                        frame.fill_rectangle(
                            Point::new(overlay_x, y),
                            Size::new(overlay_w, row_height),
                            Color::from_rgba(0.8, 0.2, 0.2, 0.08),
                        );
                    }
                }
            });
        }

        // Playhead — warm 1px line + a rounded tab at the top.
        let playhead_seconds = (self.playhead as f64 / self.sample_rate as f64) as f32;
        let playhead_x = playhead_seconds * self.zoom - self.scroll_offset;
        if playhead_x >= 0.0 && playhead_x <= bounds.width {
            let total_height = (header_height + track_area_height - y_off).max(bounds.height);
            frame.fill_rectangle(
                Point::new(playhead_x - 0.5, 0.0),
                Size::new(1.0, total_height),
                theme::WARM,
            );
            let tab_w = 11.0;
            let tab_h = 11.0;
            let tab = canvas::Path::rounded_rectangle(
                Point::new(playhead_x - tab_w / 2.0, 0.0),
                Size::new(tab_w, tab_h),
                iced::border::radius(0.0).bottom(6.0),
            );
            frame.fill(&tab, theme::WARM);
        }

        // Live automated-value indicators ride the uncached overlay so they
        // follow the playhead without invalidating the cached lane geometry.
        self.draw_automation_live_values(frame, &layout, header_height, y_off, bounds);

        // Drag-to-timeline placement affordances (doc #175, todo #605):
        // the lit target lane, dashed grid-snapped ghost clip, the
        // new-audio-track drop zone, and the drag pill + drop tooltip. Drawn
        // in this uncached pass so they track the cursor every frame.
        if let Some(drag) = self.drag {
            self.draw_drag_placement(frame, bounds, drag);
        }

        // Take-lane comping affordances (epic #15, todo #414): the
        // in-flight promote's preview band and the hover captions that
        // say what a gesture is about to do. Uncached on purpose — they
        // follow the pointer, while everything todo #413 draws in a take
        // lane follows nothing and stays in the cached layer.
        self.draw_take_interaction(
            frame,
            &layout,
            bounds,
            state.take_promote_drag.as_ref(),
            cursor,
        );

        // The ruler-height local is unused if neither overlay fires;
        // keep it so future overlay additions (e.g. selection brushes)
        // can use it without reintroducing the variable.
        let _ = ruler_height;
    }
}
