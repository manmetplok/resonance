//! Arrangement markers state for the timeline.

use serde::{Deserialize, Serialize};

/// A named point or ranged region on the arrangement timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrangementMarker {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    pub start_sample: u64,
    /// `Some` => ranged region (section span); `None` => point marker (flag).
    pub end_sample: Option<u64>,
}

impl ArrangementMarker {
    /// Create a new point marker at the given sample position.
    pub fn new_point(id: u64, name: String, color: [u8; 3], start_sample: u64) -> Self {
        Self {
            id,
            name,
            color,
            start_sample,
            end_sample: None,
        }
    }

    /// Create a new ranged region marker.
    pub fn new_region(
        id: u64,
        name: String,
        color: [u8; 3],
        start_sample: u64,
        end_sample: u64,
    ) -> Self {
        Self {
            id,
            name,
            color,
            start_sample,
            end_sample: Some(end_sample),
        }
    }

    /// Returns true if this is a point marker (no end sample).
    pub fn is_point(&self) -> bool {
        self.end_sample.is_none()
    }

    /// Returns true if this is a ranged region marker.
    pub fn is_region(&self) -> bool {
        self.end_sample.is_some()
    }

    /// Get the end sample, or the start sample if this is a point marker.
    pub fn effective_end(&self) -> u64 {
        self.end_sample.unwrap_or(self.start_sample)
    }
}

/// A collection of arrangement markers, kept sorted by `start_sample`,
/// with a monotonic id counter for handing out fresh marker ids.
#[derive(Debug, Clone)]
pub struct ArrangementMarkers {
    pub markers: Vec<ArrangementMarker>,
    /// Next id to hand out from [`Self::allocate_id`]. Recomputed as
    /// `max(existing id) + 1` whenever the collection is rebuilt from a
    /// persisted `Vec` (see the `From` impl), mirroring how the track
    /// registry restores `next_sub_track_id` on project load.
    next_id: u64,
}

impl Default for ArrangementMarkers {
    fn default() -> Self {
        Self {
            markers: Vec::new(),
            next_id: 1,
        }
    }
}

impl ArrangementMarkers {
    /// Create a new empty markers collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh, unused marker id. Mirrors the track registry's
    /// `allocate_sub_track_id`: hands out `next_id` and bumps the counter,
    /// skipping any id already present so a counter restored from a loaded
    /// project (or externally-built ids) can never collide.
    pub fn allocate_id(&mut self) -> u64 {
        loop {
            let candidate = self.next_id;
            self.next_id += 1;
            if !self.markers.iter().any(|m| m.id == candidate) {
                return candidate;
            }
        }
    }

    /// Get a marker by ID.
    pub fn get(&self, id: u64) -> Option<&ArrangementMarker> {
        self.markers.iter().find(|m| m.id == id)
    }

    /// Get a mutable marker by ID.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut ArrangementMarker> {
        self.markers.iter_mut().find(|m| m.id == id)
    }

    /// Add a marker and return its ID.
    pub fn add(&mut self, marker: ArrangementMarker) -> u64 {
        let id = marker.id;
        self.markers.push(marker);
        self.sort();
        id
    }

    /// Remove a marker by ID.
    pub fn remove(&mut self, id: u64) -> Option<ArrangementMarker> {
        let idx = self.markers.iter().position(|m| m.id == id)?;
        Some(self.markers.remove(idx))
    }

    /// Sort markers by start_sample, maintaining stable order for markers at the same position.
    pub fn sort(&mut self) {
        self.markers.sort_by_key(|m| m.start_sample);
    }

    /// Get the marker covering a given sample position. A point marker
    /// covers only its exact `start_sample`; a region covers
    /// `[start_sample, effective_end()]` inclusive.
    pub fn marker_at(&self, sample: u64) -> Option<&ArrangementMarker> {
        self.markers
            .iter()
            .find(|m| m.start_sample <= sample && sample <= m.effective_end())
    }

    /// Get the next marker after a given sample position.
    pub fn next_marker(&self, sample: u64) -> Option<&ArrangementMarker> {
        self.markers
            .iter()
            .find(|m| m.start_sample > sample)
            .or_else(|| self.markers.first())
    }

    /// Get the previous marker before a given sample position.
    pub fn prev_marker(&self, sample: u64) -> Option<&ArrangementMarker> {
        self.markers
            .iter()
            .rev()
            .find(|m| m.start_sample < sample)
            .or_else(|| self.markers.last())
    }

    /// Move a marker to a new start position.
    pub fn move_start(&mut self, id: u64, new_start: u64) -> bool {
        if let Some(marker) = self.get_mut(id) {
            marker.start_sample = new_start;
            self.sort();
            true
        } else {
            false
        }
    }

    /// Set the end of a region marker.
    pub fn set_region_end(&mut self, id: u64, end_sample: Option<u64>) -> bool {
        if let Some(marker) = self.get_mut(id) {
            marker.end_sample = end_sample;
            true
        } else {
            false
        }
    }

    /// Check if a marker exists with the given ID.
    pub fn contains(&self, id: u64) -> bool {
        self.markers.iter().any(|m| m.id == id)
    }

    /// Get all markers as a slice.
    pub fn as_slice(&self) -> &[ArrangementMarker] {
        &self.markers
    }

    /// Clear all markers.
    pub fn clear(&mut self) {
        self.markers.clear();
    }

    /// Get the number of markers.
    pub fn len(&self) -> usize {
        self.markers.len()
    }

    /// Check if there are no markers.
    pub fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }
}

impl std::ops::Deref for ArrangementMarkers {
    type Target = Vec<ArrangementMarker>;

    fn deref(&self) -> &Self::Target {
        &self.markers
    }
}

impl std::ops::DerefMut for ArrangementMarkers {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.markers
    }
}

impl From<Vec<ArrangementMarker>> for ArrangementMarkers {
    fn from(markers: Vec<ArrangementMarker>) -> Self {
        // Restore the id counter to one past the highest persisted id so a
        // fresh `allocate_id` can never collide with a loaded marker.
        let next_id = markers.iter().map(|m| m.id).max().map_or(1, |m| m + 1);
        let mut s = Self { markers, next_id };
        s.sort();
        s
    }
}

impl From<ArrangementMarkers> for Vec<ArrangementMarker> {
    fn from(markers: ArrangementMarkers) -> Self {
        markers.markers
    }
}
