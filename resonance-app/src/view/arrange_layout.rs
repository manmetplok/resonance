//! Shared arrange-row vertical layout (epic #36, doc #203).
//!
//! The Arrange view is no longer a uniform stack of 96 px track lanes:
//! group/folder tracks introduce 60 px **group-header** rows interleaved
//! with the 96 px **track** rows, and a *collapsed* group hides its
//! members entirely. That breaks every `index * TRACK_HEIGHT` assumption
//! that the track-header column and the timeline canvas used to share.
//!
//! [`ArrangeRowLayout`] is the single source of truth for that
//! heterogeneous layout. It enumerates the visible arrange rows in
//! display order, each with its own height and cumulative `y_top`, derived
//! purely from the sorted arrange tracks + the [`TrackGroupRegistry`]
//! (which carries collapse state). Both the header column and the canvas
//! consume it instead of re-deriving row Y from a fixed pitch.
//!
//! The builder is a **pure** function of its inputs (no `Resonance`, no
//! iced types), so it is unit-testable and can be shared by
//! `view::track_header` and `view::timeline` alike. This todo only adds
//! the model + helpers; wiring the two surfaces onto it lands in the
//! follow-up todos (#730–#733).

use std::collections::{HashMap, HashSet};

use resonance_common::automation::{LaneId, TrackId};
use resonance_common::AutomationTarget;

use crate::state::{AutomationState, TrackGroupRegistry, TrackState};
use crate::theme::{AUTOMATION_LANE_ROW_HEIGHT, GROUP_HEADER_HEIGHT, TRACK_HEIGHT};
use crate::view::timeline::automation::{target_belongs_to_track, target_priority};

/// What a single arrange row represents.
///
/// Group ids live in the same id space as track ids (see
/// [`TrackGroupRegistry`]), so both variants carry a [`TrackId`]; the
/// variant disambiguates whether it names a group or a real track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrangeRowKind {
    /// A `GROUP_HEADER_HEIGHT` (60 px) folder-header row for the group
    /// with this id (caret · swatch · name · macros).
    GroupHeader(TrackId),
    /// A `TRACK_HEIGHT` (96 px) lane for the track with this id.
    Track(TrackId),
    /// A slim `AUTOMATION_LANE_ROW_HEIGHT` (44 px) automation sub-row for
    /// one of `track`'s lanes (doc #256). Emitted directly beneath the
    /// owning [`Track`](Self::Track) row — one per lane, in
    /// `target_priority` order — while that track's automation is expanded
    /// (see [`ArrangeAutomationRows`]). Rendering/editing of these rows
    /// lands in todos #1097/#1098.
    AutomationLane { track: TrackId, lane: LaneId },
}

/// One row in the arrange layout: its kind plus the cumulative vertical
/// rectangle it occupies (`y_top` is measured from the top of the lane
/// area; chrome/ruler offsets are applied by the consumer, not here).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrangeRow {
    pub kind: ArrangeRowKind,
    pub y_top: f32,
    pub height: f32,
}

impl ArrangeRow {
    /// The cumulative bottom edge of this row (`y_top + height`).
    pub fn y_bottom(&self) -> f32 {
        self.y_top + self.height
    }

    /// The track id if this row is a [`ArrangeRowKind::Track`], else `None`.
    ///
    /// Deliberately `None` for [`ArrangeRowKind::AutomationLane`] rows: an
    /// automation sub-row belongs to a track but is *not* the track's clip
    /// lane, and every current consumer of this accessor means the latter.
    pub fn track_id(&self) -> Option<TrackId> {
        match self.kind {
            ArrangeRowKind::Track(id) => Some(id),
            ArrangeRowKind::GroupHeader(_) | ArrangeRowKind::AutomationLane { .. } => None,
        }
    }

    /// The group id if this row is a [`ArrangeRowKind::GroupHeader`], else
    /// `None`.
    pub fn group_id(&self) -> Option<TrackId> {
        match self.kind {
            ArrangeRowKind::GroupHeader(id) => Some(id),
            ArrangeRowKind::Track(_) | ArrangeRowKind::AutomationLane { .. } => None,
        }
    }
}

/// The automation inputs to [`ArrangeRowLayout::build`] (doc #256): which
/// lanes each track owns (in display order) and which tracks currently show
/// them as dedicated sub-rows.
///
/// The default value (`no lanes, nothing expanded`) yields a layout
/// byte-identical to the pre-automation-row one, which is what every
/// consumer gets while no track is expanded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArrangeAutomationRows {
    /// Per track, its automation-lane ids in display order — sorted by
    /// [`target_priority`], ties broken by the `DeviceParam` param id and
    /// finally the lane id, mirroring `primary_lane_for_track`'s
    /// deterministic pick. Tracks without lanes are simply absent.
    pub lanes_by_track: HashMap<TrackId, Vec<LaneId>>,
    /// Tracks whose automation sub-rows are expanded. Transient UI state
    /// (not project data): see
    /// `ClipInteractionState::automation_expanded_tracks`.
    pub expanded: HashSet<TrackId>,
}

impl ArrangeAutomationRows {
    /// Collect the per-track sorted lane lists from the live
    /// [`AutomationState`] mirror for the given arrange tracks, together
    /// with the expanded-track set. Membership follows
    /// [`target_belongs_to_track`] (gain/pan/mute, hosted plugin params,
    /// device params); ordering follows [`target_priority`] with the same
    /// tie-breaks as `primary_lane_for_track`, so the first row of an
    /// expanded stack is exactly the lane the collapsed overlay shows.
    pub fn collect(
        automation: &AutomationState,
        sorted_tracks: &[&TrackState],
        expanded: &HashSet<TrackId>,
    ) -> Self {
        let mut lanes_by_track: HashMap<TrackId, Vec<LaneId>> = HashMap::new();
        for track in sorted_tracks {
            let mut lanes: Vec<(u32, String, LaneId)> = automation
                .lanes
                .values()
                .filter(|lane| target_belongs_to_track(&lane.target, track))
                .map(|lane| {
                    let tie = match &lane.target {
                        AutomationTarget::DeviceParam { param_id, .. } => param_id.clone(),
                        _ => String::new(),
                    };
                    (target_priority(lane.target.clone()), tie, lane.id)
                })
                .collect();
            if lanes.is_empty() {
                continue;
            }
            lanes.sort();
            lanes_by_track.insert(track.id, lanes.into_iter().map(|(_, _, id)| id).collect());
        }
        Self {
            lanes_by_track,
            expanded: expanded.clone(),
        }
    }
}

/// The ordered, height-aware list of visible arrange rows.
///
/// Built by [`ArrangeRowLayout::build`]; queried via [`rows`](Self::rows),
/// [`total_height`](Self::total_height), [`row_at_y`](Self::row_at_y),
/// [`track_row_rect`](Self::track_row_rect) and
/// [`group_row_rect`](Self::group_row_rect).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArrangeRowLayout {
    rows: Vec<ArrangeRow>,
}

impl ArrangeRowLayout {
    /// Build the layout from the **sorted** arrange tracks and the group
    /// registry.
    ///
    /// `sorted_tracks` must already be in arrange order (see
    /// `view::timeline::hit_test::sorted_arrange_tracks`). The walk emits,
    /// in arrange order:
    ///
    /// - For each ungrouped track: a [`ArrangeRowKind::Track`] row.
    /// - For each **top-level** group (placed at the position of its first
    ///   encountered member): a [`ArrangeRowKind::GroupHeader`] row, then —
    ///   *only if the group is not collapsed* — its member rows in the
    ///   group's own membership order. Member tracks emit
    ///   [`ArrangeRowKind::Track`] rows; nested sub-groups recurse, emitting
    ///   their own 60 px header followed by their member rows (nesting is
    ///   one level deep).
    ///
    /// A collapsed group emits **only** its header row — its members are
    /// omitted, which is exactly what makes "collapse hides members" and
    /// keeps virtualization happy (a hidden member's automation sub-rows
    /// vanish with it, since they only ever ride along with its track
    /// row). `y_top` accumulates across the mixed 44/60/96 px row pitches.
    ///
    /// `automation` adds the per-lane sub-rows (doc #256): every track in
    /// `automation.expanded` gets one 44 px
    /// [`ArrangeRowKind::AutomationLane`] row per entry of its
    /// `lanes_by_track` list, directly beneath its track row, in list
    /// order. With `ArrangeAutomationRows::default()` (or nothing
    /// expanded) the layout is identical to the automation-unaware one.
    pub fn build(
        sorted_tracks: &[&TrackState],
        groups: &TrackGroupRegistry,
        automation: &ArrangeAutomationRows,
    ) -> Self {
        let present: HashSet<TrackId> = sorted_tracks.iter().map(|t| t.id).collect();
        let mut builder = Builder {
            rows: Vec::new(),
            y: 0.0,
            emitted: HashSet::new(),
            present,
            groups,
            automation,
        };

        for track in sorted_tracks {
            let tid = track.id;
            // Defensive: group ids never appear as real tracks, but if a
            // stale id leaks in, don't treat it as a lane.
            if groups.get_group(tid).is_some() {
                continue;
            }
            match groups.group_of_member(tid) {
                // A grouped track triggers emission of its whole top-level
                // group at this position (the first time it's seen); later
                // members of the same group are skipped via `emitted`.
                Some(direct_group) => {
                    let top = top_level_group(groups, direct_group);
                    builder.emit_group(top);
                }
                // An ungrouped track is a plain lane.
                None => builder.push_track(tid),
            }
        }

        Self { rows: builder.rows }
    }

    /// All rows in display order.
    pub fn rows(&self) -> &[ArrangeRow] {
        &self.rows
    }

    /// Total content height — the cumulative bottom edge of the last row,
    /// or `0.0` when there are no rows. Replaces `visible_tracks *
    /// TRACK_HEIGHT`.
    pub fn total_height(&self) -> f32 {
        self.rows.last().map(ArrangeRow::y_bottom).unwrap_or(0.0)
    }

    /// The row whose vertical span contains `y` (`y_top <= y < y_bottom`),
    /// or `None` if `y` is outside every row. Replaces the `y -> index`
    /// division.
    pub fn row_at_y(&self, y: f32) -> Option<&ArrangeRow> {
        self.rows
            .iter()
            .find(|row| y >= row.y_top && y < row.y_bottom())
    }

    /// The `(y_top, height)` rectangle of the lane for `track_id`, or
    /// `None` if the track has no visible row (e.g. it is a member of a
    /// collapsed group, or simply absent). Replaces the per-clip
    /// `clip_lane_rect` pitch math.
    pub fn track_row_rect(&self, track_id: TrackId) -> Option<(f32, f32)> {
        self.rows
            .iter()
            .find(|row| row.kind == ArrangeRowKind::Track(track_id))
            .map(|row| (row.y_top, row.height))
    }

    /// The `(y_top, height)` rectangle of the header lane for `group_id`,
    /// or `None` if that group is not visible.
    pub fn group_row_rect(&self, group_id: TrackId) -> Option<(f32, f32)> {
        self.rows
            .iter()
            .find(|row| row.kind == ArrangeRowKind::GroupHeader(group_id))
            .map(|row| (row.y_top, row.height))
    }

    /// The `(y_top, height)` rectangle of the automation sub-row for
    /// `track`'s lane `lane`, or `None` when that row is not visible —
    /// the track's automation is collapsed, the track itself is hidden
    /// inside a collapsed group, or the lane doesn't belong to it.
    /// Sibling of [`track_row_rect`](Self::track_row_rect) for the
    /// #1097/#1098 render + input surfaces.
    pub fn automation_row_rect(&self, track: TrackId, lane: LaneId) -> Option<(f32, f32)> {
        self.rows
            .iter()
            .find(|row| row.kind == ArrangeRowKind::AutomationLane { track, lane })
            .map(|row| (row.y_top, row.height))
    }

    /// Number of rows in the layout.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the layout has no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Walk a group's nesting chain up to its top-level ancestor. The registry
/// caps nesting at one level, but the loop is guarded so a malformed cycle
/// can never hang the builder.
fn top_level_group(groups: &TrackGroupRegistry, mut group_id: TrackId) -> TrackId {
    let mut guard = 0;
    while let Some(group) = groups.get_group(group_id) {
        match group.nesting_parent {
            Some(parent) => {
                group_id = parent;
                guard += 1;
                if guard > 64 {
                    break;
                }
            }
            None => break,
        }
    }
    group_id
}

/// Mutable cursor used while accumulating rows.
struct Builder<'a> {
    rows: Vec<ArrangeRow>,
    y: f32,
    /// Group ids already emitted, so a group is only laid out once even
    /// when several of its members are visited.
    emitted: HashSet<TrackId>,
    /// Ids of the tracks actually present in the arrange set — guards
    /// against stale member references producing phantom rows.
    present: HashSet<TrackId>,
    groups: &'a TrackGroupRegistry,
    automation: &'a ArrangeAutomationRows,
}

impl Builder<'_> {
    /// Push a track's 96 px row, followed — when its automation is
    /// expanded — by one 44 px [`ArrangeRowKind::AutomationLane`] row per
    /// lane, in the pre-sorted `lanes_by_track` order. Lane rows only
    /// ever ride along with their track row, so a track hidden inside a
    /// collapsed group (which never reaches here) hides them too.
    fn push_track(&mut self, id: TrackId) {
        self.rows.push(ArrangeRow {
            kind: ArrangeRowKind::Track(id),
            y_top: self.y,
            height: TRACK_HEIGHT,
        });
        self.y += TRACK_HEIGHT;

        if !self.automation.expanded.contains(&id) {
            return;
        }
        let Some(lanes) = self.automation.lanes_by_track.get(&id) else {
            return;
        };
        for &lane in lanes {
            self.rows.push(ArrangeRow {
                kind: ArrangeRowKind::AutomationLane { track: id, lane },
                y_top: self.y,
                height: AUTOMATION_LANE_ROW_HEIGHT,
            });
            self.y += AUTOMATION_LANE_ROW_HEIGHT;
        }
    }

    /// Emit a group's header and (when expanded) its members, recursing one
    /// level deep into nested sub-groups. A no-op if already emitted.
    fn emit_group(&mut self, group_id: TrackId) {
        if !self.emitted.insert(group_id) {
            return;
        }
        let group = match self.groups.get_group(group_id) {
            Some(group) => group,
            None => return,
        };

        self.rows.push(ArrangeRow {
            kind: ArrangeRowKind::GroupHeader(group_id),
            y_top: self.y,
            height: GROUP_HEADER_HEIGHT,
        });
        self.y += GROUP_HEADER_HEIGHT;

        // Collapsed groups contribute only their header — members are
        // omitted so the rows simply vanish from the layout.
        if group.is_collapsed {
            return;
        }

        for &member_id in &group.ordered_members {
            if self.groups.get_group(member_id).is_some() {
                // A nested sub-group: recurse so it lays out its own header
                // followed by its members.
                self.emit_group(member_id);
            } else if self.present.contains(&member_id) {
                self.push_track(member_id);
            }
        }
    }
}
