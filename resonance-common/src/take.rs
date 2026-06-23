//! Take-lanes data model for loop/cycle recording and comping
//! (design doc #165, epic #15).
//!
//! One serializable definition lives here, below `resonance-audio` in the
//! dependency graph, so the realtime engine, the app and project persistence
//! all agree on what a take is and how a comp stitches segments of takes into a
//! single composite performance — mirroring the [`crate::automation`] pattern.
//!
//! A [`TakeGroup`] binds a set of alternate recordings ("takes") to one loop
//! `slot` on a track. Its [`Comp`] is an ordered, non-overlapping cover of that
//! slot: each [`CompSegment`] names the take that plays over its range. The
//! comping helpers ([`Comp::promote`], [`Comp::split_comp`], …) are the single
//! source of truth for editing that cover, so the engine's playback/bounce path
//! and the app's UI never disagree about which take is audible where.

use serde::{Deserialize, Serialize};

use crate::automation::TrackId;

/// Identifier for a [`Take`] within a [`TakeGroup`], unique within a project.
pub type TakeId = u64;
/// Identifier for a [`TakeGroup`], unique within a project.
pub type TakeGroupId = u64;
/// Reference to a recorded audio clip. Mirrors `resonance_audio::types::ClipId`
/// (both are plain `u64`); defined here because `resonance-common` sits below
/// `resonance-audio` and cannot import it.
pub type ClipId = u64;

/// A half-open position range `[start, end())` on the timeline, measured in
/// sample frames (the same unit as [`crate::automation::Breakpoint::time_frames`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimelineRange {
    /// Inclusive start position, in sample frames.
    pub start: u64,
    /// Length of the range, in sample frames. The range covers
    /// `[start, start + length)`.
    pub length: u64,
}

impl TimelineRange {
    /// Builds a range from a start and a length.
    pub fn new(start: u64, length: u64) -> Self {
        Self { start, length }
    }

    /// Builds a range from inclusive `start` and exclusive `end`. If `end`
    /// precedes `start` the range is empty.
    pub fn from_bounds(start: u64, end: u64) -> Self {
        Self {
            start,
            length: end.saturating_sub(start),
        }
    }

    /// Exclusive end position (`start + length`).
    pub fn end(&self) -> u64 {
        self.start + self.length
    }

    /// True when the range covers no positions.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// True when `pos` falls within the half-open range `[start, end())`.
    pub fn contains(&self, pos: u64) -> bool {
        pos >= self.start && pos < self.end()
    }

    /// True when this range shares at least one position with `other`.
    pub fn overlaps(&self, other: &TimelineRange) -> bool {
        self.start < other.end() && other.start < self.end()
    }
}

/// A single note within a MIDI take.
///
/// Mirrors the fields of `resonance_audio::types::MidiNote` so an instrument
/// take captured by the engine round-trips through persistence without the
/// `resonance-audio` dependency. Positions are in MIDI ticks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TakeNote {
    /// MIDI pitch (0–127).
    pub note: u8,
    /// Velocity, normalized `0.0..=1.0`.
    pub velocity: f32,
    /// Note-on position, in ticks from the take's start.
    pub start_tick: u64,
    /// Sounding length, in ticks.
    pub duration_ticks: u64,
}

/// The recorded content of a [`Take`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TakeContent {
    /// An audio take: a reference to the recorded clip for the slot.
    Audio { clip_ref: ClipId },
    /// A MIDI take: the notes captured for the slot (instrument tracks).
    Midi { notes: Vec<TakeNote> },
}

/// A single recorded pass over a [`TakeGroup`]'s slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Take {
    /// Unique identifier within the owning [`TakeGroup`].
    pub id: TakeId,
    /// Zero-based index of the loop pass that produced this take.
    pub pass_index: u32,
    /// Capture wall-clock time, in unix milliseconds.
    pub captured_at: i64,
    /// The recorded content.
    pub content: TakeContent,
}

impl Take {
    /// Builds a take.
    pub fn new(id: TakeId, pass_index: u32, captured_at: i64, content: TakeContent) -> Self {
        Self {
            id,
            pass_index,
            captured_at,
            content,
        }
    }
}

/// A contiguous segment of a [`Comp`] that plays one take over its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompSegment {
    /// The slice of the timeline this segment covers.
    pub range: TimelineRange,
    /// The take audible over `range`.
    pub take_id: TakeId,
}

/// The composite ("comp") assembled from segments of a group's takes.
///
/// `segments` is kept sorted ascending by `range.start` and non-overlapping;
/// adjacent segments referencing the same take are merged. Use the helpers
/// ([`Comp::promote`], [`Comp::split_comp`]) to maintain those invariants
/// rather than mutating `segments` directly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comp {
    /// Ordered, non-overlapping segments making up the comp.
    pub segments: Vec<CompSegment>,
}

impl Comp {
    /// An empty comp.
    pub fn new() -> Self {
        Self::default()
    }

    /// The segment covering `pos`, if any.
    pub fn comp_at(&self, pos: u64) -> Option<&CompSegment> {
        self.segments.iter().find(|seg| seg.range.contains(pos))
    }

    /// Splits the segment containing `pos` into two abutting segments at `pos`,
    /// both referencing the same take — creating a comp boundary the caller can
    /// then promote against.
    ///
    /// A no-op when `pos` sits on a segment boundary or outside every segment,
    /// since no segment's interior is cut.
    pub fn split_comp(&mut self, pos: u64) {
        let Some(idx) = self
            .segments
            .iter()
            .position(|seg| seg.range.contains(pos) && seg.range.start != pos)
        else {
            return;
        };
        let seg = self.segments[idx];
        self.segments[idx].range = TimelineRange::from_bounds(seg.range.start, pos);
        self.segments.insert(
            idx + 1,
            CompSegment {
                range: TimelineRange::from_bounds(pos, seg.range.end()),
                take_id: seg.take_id,
            },
        );
    }

    /// Promotes `take_id` across `range`, replacing any overlapping coverage.
    ///
    /// Existing segments are trimmed (or split, when `range` lands inside one)
    /// around `range`, the new segment is inserted, and adjacent segments
    /// referencing the same take are merged. An empty `range` is a no-op.
    pub fn promote(&mut self, range: TimelineRange, take_id: TakeId) {
        if range.is_empty() {
            return;
        }

        let mut next: Vec<CompSegment> = Vec::with_capacity(self.segments.len() + 2);
        for seg in &self.segments {
            if !seg.range.overlaps(&range) {
                next.push(*seg);
                continue;
            }
            // Keep the portion of `seg` left of the promoted range...
            if seg.range.start < range.start {
                next.push(CompSegment {
                    range: TimelineRange::from_bounds(seg.range.start, range.start),
                    take_id: seg.take_id,
                });
            }
            // ...and the portion right of it (both fire when `range` is strictly
            // inside `seg`, splitting it around the new segment).
            if seg.range.end() > range.end() {
                next.push(CompSegment {
                    range: TimelineRange::from_bounds(range.end(), seg.range.end()),
                    take_id: seg.take_id,
                });
            }
        }
        next.push(CompSegment { range, take_id });
        next.sort_by_key(|seg| seg.range.start);
        self.segments = merge_adjacent(next);
    }

    /// True when the comp contiguously covers `slot` with no gaps.
    ///
    /// An empty `slot` is trivially covered. Assumes the sorted,
    /// non-overlapping invariant the helpers maintain.
    pub fn is_full_cover(&self, slot: TimelineRange) -> bool {
        if slot.is_empty() {
            return true;
        }
        let mut cursor = slot.start;
        for seg in &self.segments {
            if seg.range.start > cursor {
                return false; // gap before this segment
            }
            cursor = cursor.max(seg.range.end());
            if cursor >= slot.end() {
                return true;
            }
        }
        cursor >= slot.end()
    }
}

/// Merges adjacent segments that touch end-to-start and reference the same
/// take. Assumes the input is sorted by start position and non-overlapping.
fn merge_adjacent(segments: Vec<CompSegment>) -> Vec<CompSegment> {
    let mut merged: Vec<CompSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(last) = merged.last_mut() {
            if last.take_id == seg.take_id && last.range.end() == seg.range.start {
                last.range.length += seg.range.length;
                continue;
            }
        }
        merged.push(seg);
    }
    merged
}

/// The alternate takes recorded for one loop slot on a track, plus the comp
/// that stitches them into a single performance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TakeGroup {
    /// Unique identifier within a project.
    pub id: TakeGroupId,
    /// The track this group's takes were recorded on.
    pub track_id: TrackId,
    /// The loop region the group is bound to (the recorded slot).
    pub slot: TimelineRange,
    /// All recorded takes, in capture order.
    pub takes: Vec<Take>,
    /// The composite assembled from segments of `takes`.
    pub comp: Comp,
    /// The take soloed for full-slot playback, overriding the comp when set.
    pub active_take: Option<TakeId>,
}

impl TakeGroup {
    /// Builds an empty group bound to `slot`.
    pub fn new(id: TakeGroupId, track_id: TrackId, slot: TimelineRange) -> Self {
        Self {
            id,
            track_id,
            slot,
            takes: Vec::new(),
            comp: Comp::new(),
            active_take: None,
        }
    }

    /// Appends a take to the group.
    pub fn add_take(&mut self, take: Take) {
        self.takes.push(take);
    }

    /// The take with the given id, if present.
    pub fn take(&self, id: TakeId) -> Option<&Take> {
        self.takes.iter().find(|t| t.id == id)
    }

    /// True when the comp contiguously covers this group's slot.
    pub fn is_full_cover(&self) -> bool {
        self.comp.is_full_cover(self.slot)
    }
}
