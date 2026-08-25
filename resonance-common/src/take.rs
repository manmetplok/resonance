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
//!
//! # One definition of the cover (todo #1395)
//!
//! What a group *sounds like* is resolved in exactly one place —
//! [`effective_cover`] — and both layers call it: the mixer's
//! `take_comp::resolve_spans` maps its spans to take clips, and the timeline's
//! take lane draws them. Its three tiers are
//!
//! 1. an [`active_take`](TakeGroup::active_take) solos the whole slot,
//!    overriding the comp;
//! 2. otherwise each [`CompSegment`] plays its take over its range;
//! 3. **anything the comp leaves uncovered plays [`latest_take`]** — the
//!    user's 2026-08-24 ruling. Comping is progressive refinement, not
//!    assembly from silence: promote one phrase of take 2 into bar 2 of a
//!    four-bar loop and bars 1, 3 and 4 keep playing the most recent pass, so
//!    the part is complete from the very first gesture.
//!
//! Tier 3 is a safety net rather than the normal path, because
//! [`Comp::promote`] takes a [`SlotCover`] and seeds the remainder of the slot
//! before it edits: a comp that has been promoted into is a gap-free cover *by
//! construction*, which is what makes the "ordered, gap-free cover" above an
//! invariant instead of an aspiration.
//!
//! # A cover is not a guarantee of audio (todo #1396)
//!
//! [`effective_cover`] answers *which take* plays over each part of the slot.
//! It says nothing about whether that take has any material there — a pass
//! that punched in late, or was cut short at stop, covers less than its slot.
//! [`Take::extent`] records what each pass really recorded and
//! [`Take::audible_extent`] intersects it with the slot; both consumers (the
//! app's promote clamp and the take lane) resolve it there rather than each
//! guessing from whatever clip they happen to hold.

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
    /// Zero-based ordinal of this take **within its group** — the row it
    /// occupies in the take lane, and the number its `T1`, `T2`, … label
    /// is derived from.
    ///
    /// It was the loop pass's index while a group was exactly one record
    /// run. Since todo #1392 a group accumulates takes across runs (and
    /// across a save/load), and a run's own pass counter restarts at 0 at
    /// every record press, so the engine allocates this from the group
    /// instead — one past the highest it already holds.
    pub pass_index: u32,
    /// Capture wall-clock time, in unix milliseconds.
    pub captured_at: i64,
    /// The stretch of timeline this pass actually recorded over, in sample
    /// frames — **not** its group's slot. See [`Take::audible_extent`] for
    /// what it means and why it is stored rather than derived.
    pub extent: TimelineRange,
    /// The recorded content.
    pub content: TakeContent,
}

impl Take {
    /// Builds a take.
    ///
    /// `extent` is required rather than defaulted on purpose: a take whose
    /// extent is unknown cannot be told apart from one that genuinely
    /// filled its slot, and every consumer would have to guess (ba todo
    /// #1396).
    pub fn new(
        id: TakeId,
        pass_index: u32,
        captured_at: i64,
        extent: TimelineRange,
        content: TakeContent,
    ) -> Self {
        Self {
            id,
            pass_index,
            captured_at,
            extent,
            content,
        }
    }

    /// The stretch of `slot` this take can actually sound over: its
    /// [`extent`](Self::extent) intersected with the slot, empty when the
    /// two do not meet at all.
    ///
    /// # A take does not necessarily fill its lane
    ///
    /// Cycle recording gives pass 0 the **punch-in point** as its clip
    /// start rather than the loop start, and any pass cut short at
    /// transport stop ends before the slot does — so a take's recorded
    /// material can sit strictly inside its slot at either end. Everything
    /// that presents or edits a comp has to know which: the mixer clamps a
    /// span to the take's clip before reading it (todo #409), a promote
    /// over a stretch the take never recorded would write a silent hole
    /// into the composite, and a lane drawing a punched-in take at slot
    /// width claims audio the engine will not play.
    ///
    /// # Why the extent is stored on the take
    ///
    /// For an audio take it duplicates a fact the *engine* can derive from
    /// the clip — but the **app never holds a take's clip**. A cycle-record
    /// pass reaches the app only through `AudioEvent::TakeCaptured` (no
    /// `RecordingFinished` is emitted for it), and a project load
    /// materialises a take's clip in the **engine** without ever mirroring
    /// it app-side, because a take clip is not a timeline clip and must
    /// never enter `Resonance::clips` (ba todo #1396). So a field carried
    /// on the capture event alone would be authoritative on one path and
    /// absent on the other, which is worse than no field: the consumer
    /// could not tell which it had. Persisting it on the take makes the two
    /// paths identical, and costs two `u64`s per pass.
    ///
    /// It also turned out to be what makes the reload *possible*. The
    /// engine has to put each restored take clip back at the origin capture
    /// gave it — `mix_track_comp` intersects every span with the clip's
    /// own `[start_sample, +duration_frames())` — and this field is the
    /// only persisted record of that origin, since
    /// `RolledAudioTake::extent` is defined as the rolled clip's own
    /// position. `LoadTakeClipFromWav` reads its `start_sample` straight
    /// off it (ba todo #1402).
    ///
    /// # MIDI takes
    ///
    /// A MIDI take's extent is **the whole region the run cycled over**.
    /// Its notes are its content and there is no medium that can be short:
    /// silence inside a MIDI take is a rest, not a hole, and the engine
    /// plays the take's notes and nothing else wherever the comp selects
    /// it. Clamping a MIDI promote to the notes' own span would refuse a
    /// legitimate edit (promoting a bar of rest out of take 3) without
    /// preventing any silence, because the rest *is* the material.
    ///
    /// Note "the run's region", not "the group's slot": since todo #1392 a
    /// run joins an existing lane whose slot may sit up to the engine's
    /// same-slot tolerance away, and the extent stored is the run's own
    /// measurement either way (`engine::transport::finalize_loop_record_pass`).
    /// This method is what reconciles the two — the intersection below is
    /// the whole reason a nudged run cannot claim material outside the lane
    /// it joined.
    pub fn audible_extent(&self, slot: TimelineRange) -> TimelineRange {
        TimelineRange::from_bounds(
            self.extent.start.max(slot.start),
            self.extent.end().min(slot.end()),
        )
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

/// What a [`Comp`] is a cover *of*: the slot it has to fill end to end, and
/// the take that fills whatever the user has not explicitly promoted.
///
/// [`Comp::promote`] takes one so a promote cannot leave a hole behind. Build
/// it with [`SlotCover::of`] for a real group; [`SlotCover::NONE`] seeds
/// nothing, for callers assembling a comp span by span rather than editing a
/// group's cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotCover {
    /// The range the comp must cover with no gaps.
    pub slot: TimelineRange,
    /// The take that fills anything the comp does not name. `None` when the
    /// group holds no takes at all, in which case nothing can be seeded.
    pub filler: Option<TakeId>,
}

impl SlotCover {
    /// The cover context of `group`: its slot, filled by [`latest_take`].
    pub fn of(group: &TakeGroup) -> Self {
        Self {
            slot: group.slot,
            filler: latest_take(group).map(|take| take.id),
        }
    }

    /// A context that seeds nothing, so [`Comp::promote`] performs bare
    /// segment surgery. The degenerate value — an empty slot has no gaps to
    /// fill — not an opt-out: a comp edited through this is not a cover.
    pub const NONE: Self = Self {
        slot: TimelineRange {
            start: 0,
            length: 0,
        },
        filler: None,
    };
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

    /// Fills every stretch of `cover.slot` this comp does not already cover
    /// with `cover.filler`, leaving existing segments untouched.
    ///
    /// This is tier 3 of [`effective_cover`] made explicit: the group is
    /// *already* playing the filler take over those gaps, so writing them into
    /// the comp changes nothing audible — it only makes the cover editable.
    /// A [`SlotCover`] with no filler, or an empty slot, is a no-op.
    pub fn seed_cover(&mut self, cover: SlotCover) {
        let Some(filler) = cover.filler else {
            return;
        };
        if cover.slot.is_empty() {
            return;
        }

        let mut next = self.segments.clone();
        next.sort_by_key(|seg| seg.range.start);

        let mut gaps: Vec<TimelineRange> = Vec::new();
        let mut cursor = cover.slot.start;
        for seg in &next {
            if seg.range.end() <= cover.slot.start {
                continue;
            }
            if seg.range.start >= cover.slot.end() {
                break;
            }
            if seg.range.start > cursor {
                gaps.push(TimelineRange::from_bounds(cursor, seg.range.start));
            }
            cursor = cursor.max(seg.range.end());
        }
        if cursor < cover.slot.end() {
            gaps.push(TimelineRange::from_bounds(cursor, cover.slot.end()));
        }
        if gaps.is_empty() {
            return;
        }

        next.extend(gaps.into_iter().map(|range| CompSegment {
            range,
            take_id: filler,
        }));
        next.sort_by_key(|seg| seg.range.start);
        self.segments = merge_adjacent(next);
    }

    /// Promotes `take_id` across `range`, replacing any overlapping coverage.
    ///
    /// The remainder of `cover`'s slot is [seeded](Self::seed_cover) first, so
    /// the result is a genuine gap-free cover rather than an island in a hole
    /// — the user's ruling that the latest take fills the gaps, made true by
    /// construction (todo #1395). Pass [`SlotCover::NONE`] for bare segment
    /// surgery on a comp that is not a cover of anything.
    ///
    /// Existing segments are then trimmed (or split, when `range` lands inside
    /// one) around `range`, the new segment is inserted, and adjacent segments
    /// referencing the same take are merged. An empty `range` is a no-op —
    /// including the seeding, since a promote that changes nothing must not
    /// rewrite the comp.
    pub fn promote(&mut self, range: TimelineRange, take_id: TakeId, cover: SlotCover) {
        if range.is_empty() {
            return;
        }
        self.seed_cover(cover);

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

    /// Remove take `id` and re-cover the slot from the takes that remain,
    /// returning the take that was removed (`None` when the group does not
    /// hold it).
    ///
    /// **One definition of a removal**, for the same reason [`effective_cover`]
    /// is one definition of the cover (todo #1395): the engine's
    /// `AudioCommand::RemoveTake` and any app-side mirror have to agree about
    /// what a group looks like afterwards, and three things have to go at once
    /// or the disagreement is audible.
    ///
    /// - **The take.**
    /// - **Every [`CompSegment`] naming it.** A segment pointing at a take the
    ///   group no longer holds is skipped by [`effective_cover`] and plays
    ///   whatever tier 3 puts there — so a dangling reference is invisible
    ///   until something serializes it, and then it outlives the session. The
    ///   holes those segments leave go to the take the cover would *now* fall
    ///   back to, which is the take tier 3 would have chosen for them anyway:
    ///   nothing audible moves except where the removed take used to play.
    /// - **The solo**, when it named the removed take. An `active_take` the
    ///   group does not hold falls through to the comp rather than silencing
    ///   the group, so leaving it would not be fatal — but it would leave the
    ///   lane drawing a solo that is not soloing anything.
    ///
    /// A comp that ends up with **no** surviving segments is left empty rather
    /// than re-seeded. Empty and fully-seeded are equally audible (tier 3
    /// covers the slot with the same take either way), but they draw
    /// differently: seeding would turn the survivor's fallback into segments
    /// that read as spans the user promoted. Deleting a take is not a
    /// promotion of the rest.
    ///
    /// Removing the **last** take leaves a group with nothing to play and no
    /// way to draw itself. A caller owning a store of groups should drop the
    /// group instead — which is what `AudioCommand::RemoveTake` does.
    pub fn remove_take(&mut self, id: TakeId) -> Option<Take> {
        let idx = self.takes.iter().position(|take| take.id == id)?;
        let removed = self.takes.remove(idx);
        if self.active_take == Some(id) {
            self.active_take = None;
        }

        // Computed *after* the take is gone, so the filler is the survivor
        // the cover now falls back to rather than the one it used to.
        let cover = SlotCover::of(self);
        self.comp.segments.retain(|seg| seg.take_id != id);
        if !self.comp.segments.is_empty() {
            self.comp.seed_cover(cover);
        }
        Some(removed)
    }

    /// This group's [`effective_cover`] as an explicit [`Comp`] — what the
    /// group is audibly playing right now, written out as editable segments.
    ///
    /// Comp editing starts here. A group that has never been comped carries
    /// **no** segments yet is not silent, and one with an
    /// [`active_take`](Self::active_take) plays that take regardless of what
    /// its segments say; materializing the cover first is what makes a first
    /// split or promote edit *what the user is hearing* instead of appearing
    /// to do nothing (or silently swapping the audible take out from under a
    /// solo — the #1395 tier-1 case).
    ///
    /// Adjacent spans naming the same take are merged, so the result satisfies
    /// the same invariants the comp helpers maintain. Empty only for a group
    /// with no takes, or an empty slot.
    pub fn effective_comp(&self) -> Comp {
        let segments = effective_cover(self)
            .into_iter()
            .map(|span| CompSegment {
                range: span.range,
                take_id: span.take_id,
            })
            .collect();
        Comp {
            segments: merge_adjacent(segments),
        }
    }
}

/// The take a group plays wherever its comp names none: **the most recently
/// captured pass**, by `(captured_at, pass_index, id)`.
///
/// # MIDI takes are candidates (the #1395 ruling)
///
/// "Latest" is deliberately content-agnostic. The engine used to read it as
/// "the last *audio* take in vector order" and the take lane as "the newest
/// take of any kind", which pick different takes the moment a group holds both
/// kinds — and a lane that lights T3 while the mixer plays T4 is the exact
/// divergence this function exists to remove. Excluding MIDI would also
/// contradict the ruling itself: the user asked to hear *the most recent pass*
/// over the gaps, and on an instrument track that pass is a MIDI take.
///
/// The audio path is unaffected by the choice in the ordinary case, because
/// capture is per-track and a track records audio *or* MIDI, never both. Where
/// a group does hold both, a MIDI take resolving over the gaps contributes no
/// *audio* spans — the notes play through the instrument instead — which is
/// the same by-design behaviour as soloing a MIDI take
/// (`TakeGroupState::active_take_silences_audio`).
///
/// `captured_at` is the primary key rather than vector order, which is not
/// guaranteed to be capture order once a project has been saved, reloaded and
/// restored; `pass_index` and `id` are deterministic tie-breaks for passes
/// captured inside the same millisecond.
pub fn latest_take(group: &TakeGroup) -> Option<&Take> {
    group
        .takes
        .iter()
        .max_by_key(|take| (take.captured_at, take.pass_index, take.id))
}

/// How a span of a group's slot came to be covered by its take. Callers that
/// present the comp draw the three differently, so "I chose this" never reads
/// the same as "this is what you get by default".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverSource {
    /// The take is soloed for the whole slot ([`TakeGroup::active_take`]),
    /// overriding the comp entirely.
    ActiveTake,
    /// An explicit [`CompSegment`] the user promoted.
    CompSegment,
    /// The comp covers no part of this span, so it plays [`latest_take`].
    LatestFallback,
}

/// One span of a group's slot, the take audible over it, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverSpan {
    /// The stretch of timeline this span covers.
    pub range: TimelineRange,
    /// The take audible over `range`.
    pub take_id: TakeId,
    /// Which tier of the resolution put it there.
    pub source: CoverSource,
}

/// The take that plays over each part of `group`'s slot — an ordered, gap-free
/// cover, so every point of the slot maps to exactly one take.
///
/// **The single definition of what a take group sounds like** (todo #1395).
/// The mixer resolves its render spans from this and the take lane draws from
/// it, so the two cannot disagree about which take is audible where. See the
/// module header for the three tiers and the ruling behind the last one.
///
/// Spans carry their [`CoverSource`], because a caller that *draws* the cover
/// has to distinguish a deliberate promotion from a fallback even though both
/// are equally audible. Adjacent spans may therefore name the same take with
/// different sources; a caller that only cares about the audible take (the
/// mixer) merges them.
///
/// Returns an empty vector for an empty slot or a group with no takes.
/// Segments naming a take the group does not hold are skipped, segments
/// outside the slot are ignored, and one overhanging an edge is clamped to it
/// — a mirror adopting whatever the engine echoed must not be able to produce
/// a backwards or out-of-slot span. An `active_take` the group does not hold
/// is likewise ignored rather than silencing the group; the tiers below it
/// still describe something audible.
pub fn effective_cover(group: &TakeGroup) -> Vec<CoverSpan> {
    if group.slot.is_empty() || group.takes.is_empty() {
        return Vec::new();
    }

    // Tier 1: the whole-slot solo override wins outright.
    if let Some(active) = group.active_take {
        if group.take(active).is_some() {
            return vec![CoverSpan {
                range: group.slot,
                take_id: active,
                source: CoverSource::ActiveTake,
            }];
        }
    }

    let Some(latest) = latest_take(group).map(|take| take.id) else {
        return Vec::new();
    };

    // The model's helpers keep `segments` sorted, but a mirror adopts what it
    // is handed wholesale, so sort defensively rather than trusting it.
    let mut segments = group.comp.segments.clone();
    segments.sort_by_key(|seg| seg.range.start);

    let mut spans: Vec<CoverSpan> = Vec::new();
    let mut cursor = group.slot.start;
    for seg in segments {
        // Tier 2: an explicit promotion, clamped into the slot and to
        // whatever is left of it.
        if group.take(seg.take_id).is_none() {
            continue;
        }
        let start = seg.range.start.max(cursor);
        let end = seg.range.end().min(group.slot.end());
        if end <= start {
            continue;
        }
        // Tier 3: the gap before it plays the most recent pass.
        if start > cursor {
            spans.push(CoverSpan {
                range: TimelineRange::from_bounds(cursor, start),
                take_id: latest,
                source: CoverSource::LatestFallback,
            });
        }
        spans.push(CoverSpan {
            range: TimelineRange::from_bounds(start, end),
            take_id: seg.take_id,
            source: CoverSource::CompSegment,
        });
        cursor = end;
    }
    if cursor < group.slot.end() {
        spans.push(CoverSpan {
            range: TimelineRange::from_bounds(cursor, group.slot.end()),
            take_id: latest,
            source: CoverSource::LatestFallback,
        });
    }
    spans
}
