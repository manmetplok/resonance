//! Global chord-track data model (epic #33, doc #168).
//!
//! The chord track is the song-wide harmonic backbone: a list of
//! [`ChordRegion`]s carrying a [`Chord`] symbol over an explicit
//! sample span, plus a key context — a list of [`KeyChange`]s whose
//! first entry is the song key. It lives in app `state` next to the
//! tempo/signature tracks (timeline metadata owned by the app), not in
//! the realtime engine: chords are pure metadata and nothing here is
//! ever sent to `resonance-audio`.
//!
//! Positions are in **samples**, matching the playhead, loop range, and
//! the tempo/signature tracks; bar/beat positions are derived via the
//! `tempo_map` for display and snap by the view/handler layers.
//!
//! Region/key ids are allocated by callers from the project's existing
//! monotonic id source (`ComposeState::fresh_id`) — the same counter
//! that hands out section/placement/chord ids. This model only stores
//! the ids and uses them to address regions; it never mints them.
//!
//! Scope of this module (todo #439): the data types, their sort
//! invariants, and lookup/mutation helpers. Message routing and update
//! handlers live in the `ChordTrackMessage` work (todo #441); project
//! persistence in todo #440; the timeline lane render in #442. Undo is
//! wired through [`UndoExtras`](crate::undo::UndoExtras) because the
//! track isn't part of `ProjectFile` yet (persistence is a later todo),
//! so it can't be rebuilt by the replay path alone.

use resonance_music_theory::{Chord, Scale};

/// One chord placed on the global chord track. Spans `start_sample`
/// (inclusive) to `end_sample` (exclusive); by default a region abuts
/// the next one, but the span is explicit so gaps are allowed.
#[derive(Debug, Clone, PartialEq)]
pub struct ChordRegion {
    pub id: u64,
    pub chord: Chord,
    pub start_sample: u64,
    pub end_sample: u64,
    /// User-pinned regions constrain Compose regeneration (see doc #168);
    /// defaults to `false`.
    pub pinned: bool,
}

/// A key/scale change effective from `start_sample` onward, until the
/// next key change (or the end of the song).
#[derive(Debug, Clone, PartialEq)]
pub struct KeyChange {
    pub id: u64,
    pub start_sample: u64,
    pub scale: Scale,
}

/// The global chord track: chord regions plus the song-wide key context.
///
/// Both vectors are kept sorted by `start_sample`. The first
/// `key_changes` entry (lowest `start_sample`) is the **song key**.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChordTrack {
    /// Chord regions, kept sorted by `start_sample`.
    pub regions: Vec<ChordRegion>,
    /// Key/scale changes, kept sorted by `start_sample`; the first is
    /// the song key.
    pub key_changes: Vec<KeyChange>,
}

impl ChordTrack {
    /// An empty chord track (no regions, no key context).
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the track carries no chords. Callers (Performance,
    /// PDF export) fall back to their section-derived behaviour while
    /// the track is empty.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    // -- Regions -------------------------------------------------------

    /// Insert a region, keeping `regions` sorted by `start_sample`.
    /// Ties keep insertion order (the new region lands after existing
    /// regions sharing its start), which mirrors the stable behaviour
    /// of [`resort`](Self::resort).
    pub fn insert_region(&mut self, region: ChordRegion) {
        let idx = self
            .regions
            .partition_point(|r| r.start_sample <= region.start_sample);
        self.regions.insert(idx, region);
    }

    /// Remove the region with `id`, returning it if present.
    pub fn remove_region(&mut self, id: u64) -> Option<ChordRegion> {
        let idx = self.regions.iter().position(|r| r.id == id)?;
        Some(self.regions.remove(idx))
    }

    /// Borrow the region with `id`.
    pub fn region(&self, id: u64) -> Option<&ChordRegion> {
        self.regions.iter().find(|r| r.id == id)
    }

    /// Mutably borrow the region with `id`. After mutating a region's
    /// `start_sample`, call [`resort`](Self::resort) to restore the sort
    /// invariant.
    pub fn region_mut(&mut self, id: u64) -> Option<&mut ChordRegion> {
        self.regions.iter_mut().find(|r| r.id == id)
    }

    /// The region whose span contains `sample` (`start <= sample <
    /// end`). With non-overlapping regions this is unique; if regions
    /// overlap, the earliest-starting match is returned.
    pub fn region_at(&self, sample: u64) -> Option<&ChordRegion> {
        self.regions
            .iter()
            .find(|r| r.start_sample <= sample && sample < r.end_sample)
    }

    // -- Key context ---------------------------------------------------

    /// Insert a key change, keeping `key_changes` sorted by
    /// `start_sample`. The lowest-positioned entry becomes the song key.
    pub fn insert_key_change(&mut self, key_change: KeyChange) {
        let idx = self
            .key_changes
            .partition_point(|k| k.start_sample <= key_change.start_sample);
        self.key_changes.insert(idx, key_change);
    }

    /// Remove the key change with `id`, returning it if present.
    pub fn remove_key_change(&mut self, id: u64) -> Option<KeyChange> {
        let idx = self.key_changes.iter().position(|k| k.id == id)?;
        Some(self.key_changes.remove(idx))
    }

    /// Mutably borrow the key change with `id`. After mutating its
    /// `start_sample`, call [`resort`](Self::resort).
    pub fn key_change_mut(&mut self, id: u64) -> Option<&mut KeyChange> {
        self.key_changes.iter_mut().find(|k| k.id == id)
    }

    /// The song key — the scale of the first (lowest-positioned) key
    /// change. `None` when no key context has been set.
    pub fn song_key(&self) -> Option<Scale> {
        self.key_changes.first().map(|k| k.scale)
    }

    /// The scale in effect at `sample`: the last key change at or before
    /// `sample`, falling back to the song key for positions before the
    /// first change. `None` when there is no key context at all.
    pub fn key_at(&self, sample: u64) -> Option<Scale> {
        let effective = self
            .key_changes
            .iter()
            .take_while(|k| k.start_sample <= sample)
            .last();
        // Before the first change, the song key still applies.
        effective.or_else(|| self.key_changes.first()).map(|k| k.scale)
    }

    // -- Invariant maintenance -----------------------------------------

    /// Re-establish the sort invariant on both vectors after in-place
    /// edits moved a region or key change. Stable so equal-position
    /// entries keep their relative order.
    pub fn resort(&mut self) {
        self.regions.sort_by_key(|r| r.start_sample);
        self.key_changes.sort_by_key(|k| k.start_sample);
    }
}
