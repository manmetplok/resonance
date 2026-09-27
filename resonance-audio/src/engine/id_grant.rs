//! The engine's standing grant of clip ids (ARCH-04 D-7d, design doc
//! `docs/design/D-6-engine-created-ids.md` §4.2).
//!
//! The engine creates clips at moments the app cannot decide ahead of
//! time: a record run's first buffer (C3), every loop-seam pass (C4), a
//! live-MIDI clip opened by the first note on an armed track (C5), and the
//! realtime bounce (which is C3). It does not count ids for them itself.
//! The app hands it ranges from its one clip allocator
//! (`AudioCommand::GrantIds`), the engine draws from the front, in order,
//! and asks for more (`AudioEvent::IdGrantLow`) once fewer than
//! [`CLIP_GRANT_LOW_WATER`] are left. The app counts a granted id as issued
//! the moment it grants it, so an id drawn here is never handed out twice,
//! and one left unused is only a gap.
//!
//! `ClearAll` revokes the grant ([`ClipIdGrant::revoke`]): a project loaded
//! next may already hold ids inside it, and every replay ends by sending a
//! fresh one (§4.4).
//!
//! Engine-thread only: every draw site runs on the engine command thread,
//! never in the audio callback.

use std::collections::VecDeque;
use std::ops::Range;

use crate::types::{ClipId, CLIP_GRANT_LOW_WATER};

/// The clip ids the engine may still use, as the ranges the app granted,
/// oldest first.
#[derive(Debug, Default)]
pub struct ClipIdGrant {
    ranges: VecDeque<Range<ClipId>>,
    /// Latched once `IdGrantLow` has been reported for the current dip
    /// below the mark; cleared when a grant lifts the total back to the
    /// mark or above, or by a revoke.
    low_reported: bool,
}

impl ClipIdGrant {
    /// An empty grant: every draw fails until the app sends one.
    pub fn new() -> Self {
        Self::default()
    }

    /// A grant holding exactly `range` — for tests that drive
    /// `RecordingState::roll_audio_pass` directly.
    pub fn from_range(range: Range<ClipId>) -> Self {
        let mut grant = Self::new();
        grant.extend(range);
        grant
    }

    /// Append `range` (the `GrantIds` handler). An empty range is ignored.
    pub fn extend(&mut self, range: Range<ClipId>) {
        if range.is_empty() {
            return;
        }
        self.ranges.push_back(range);
        if self.len() >= CLIP_GRANT_LOW_WATER {
            self.low_reported = false;
        }
    }

    /// How many ids are left.
    pub fn len(&self) -> u64 {
        self.ranges.iter().map(|r| r.end - r.start).sum()
    }

    /// Whether no id is left.
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Draw the next id, or `None` when the grant is used up.
    pub fn take(&mut self) -> Option<ClipId> {
        let front = self.ranges.front_mut()?;
        let id = front.start;
        front.start += 1;
        if front.is_empty() {
            self.ranges.pop_front();
        }
        Some(id)
    }

    /// Draw the next id whose `audio/clip_<id>.wav` does not exist yet, or
    /// `None` when the grant runs out first. Used where the id names a WAV
    /// the engine is about to create (C3, C4). An existing file is skipped
    /// rather than overwritten: a grant issued before a Save As into an
    /// existing bundle can name a file that bundle already holds (STATE-12
    /// — a backup or redo stack may still point at it). Skipped ids are
    /// only gaps.
    pub fn take_unused_wav(&mut self, audio_dir: &std::path::Path) -> Option<ClipId> {
        loop {
            let id = self.take()?;
            if !audio_dir.join(format!("clip_{id}.wav")).exists() {
                return Some(id);
            }
            tracing::warn!("clip id grant: skipping {id}, its clip WAV already exists");
        }
    }

    /// Drop every id (`ClearAll`).
    pub fn revoke(&mut self) {
        self.ranges.clear();
        self.low_reported = false;
    }

    /// `Some(ids left)` exactly once per dip below
    /// [`CLIP_GRANT_LOW_WATER`]: the caller sends `IdGrantLow` with it. No
    /// repeat until a grant lifts the total back to the mark.
    pub fn low_water_report(&mut self) -> Option<u64> {
        let left = self.len();
        if left < CLIP_GRANT_LOW_WATER && !self.low_reported {
            self.low_reported = true;
            Some(left)
        } else {
            None
        }
    }
}
