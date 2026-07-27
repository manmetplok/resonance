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

use std::collections::HashSet;

use resonance_common::automation::TrackId;

use crate::state::{TrackGroupRegistry, TrackState};
use crate::theme::{GROUP_HEADER_HEIGHT, TRACK_HEIGHT};

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
    pub fn track_id(&self) -> Option<TrackId> {
        match self.kind {
            ArrangeRowKind::Track(id) => Some(id),
            ArrangeRowKind::GroupHeader(_) => None,
        }
    }

    /// The group id if this row is a [`ArrangeRowKind::GroupHeader`], else
    /// `None`.
    pub fn group_id(&self) -> Option<TrackId> {
        match self.kind {
            ArrangeRowKind::GroupHeader(id) => Some(id),
            ArrangeRowKind::Track(_) => None,
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
    /// keeps virtualization happy. `y_top` accumulates across the mixed
    /// 60/96 px row pitches.
    pub fn build(sorted_tracks: &[&TrackState], groups: &TrackGroupRegistry) -> Self {
        let present: HashSet<TrackId> = sorted_tracks.iter().map(|t| t.id).collect();
        let mut builder = Builder {
            rows: Vec::new(),
            y: 0.0,
            emitted: HashSet::new(),
            present,
            groups,
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
}

impl Builder<'_> {
    fn push_track(&mut self, id: TrackId) {
        self.rows.push(ArrangeRow {
            kind: ArrangeRowKind::Track(id),
            y_top: self.y,
            height: TRACK_HEIGHT,
        });
        self.y += TRACK_HEIGHT;
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
