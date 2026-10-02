//! `arrangement.*` — structural edits that move everything after a point.
//!
//! Every other editing method addresses ONE object: a clip, a note, a
//! placement. That is enough to write a song and useless to restructure
//! one. "Give the chorus another 8 bars" means moving every audio clip,
//! MIDI clip, section placement, marker and automation breakpoint after
//! bar N — about 160 objects in a real project — with no way to do it in
//! one transaction and no way to tell whether one was missed. A field
//! agent hit exactly this, gave up, rendered twice and spliced the WAV
//! outside the project, which left the session no longer matching the
//! delivered audio (ba doc #275 P2).
//!
//! Both methods here move things MUSICALLY: positions are converted to
//! ticks, shifted by the tick length of the affected bars, and converted
//! back, so a shift across a tempo change lands on the right beat rather
//! than the right number of samples. Section placements are bar-based
//! already and move by whole bars, and so are the tempo and signature
//! tracks: a meter change written for the bridge travels with the bridge
//! (ba todo #1388). The one event that never moves is the song's opening
//! one at bar 0, because every bar needs a tempo and a meter in force.
//!
//! `remove_bars` CLAMPS an event inside the removed span onto the cut
//! rather than dropping it, so the music that survives the splice keeps
//! the tempo and meter it was written in; when that puts two events of
//! one kind on the cut bar, the later one wins.

use crate::ids::ClipId;
use serde::{Deserialize, Serialize};

/// `arrangement.insert_bars` — open a gap, pushing everything at or
/// after `at_bar` later ([`InsertBarsParams`] -> [`ShiftResult`]).
pub const INSERT_BARS: &str = "arrangement.insert_bars";
/// `arrangement.remove_bars` — close a gap, pulling everything after the
/// removed span earlier ([`RemoveBarsParams`] -> [`ShiftResult`]).
///
/// Content that STARTS inside the removed span is deleted, so this needs
/// `confirm: true` when there is any; the refusal says what would go.
pub const REMOVE_BARS: &str = "arrangement.remove_bars";

/// All `arrangement.*` method names.
pub const METHODS: &[&str] = &[INSERT_BARS, REMOVE_BARS];

/// Params for `arrangement.insert_bars`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InsertBarsParams {
    /// 1-based bar the gap opens at. Everything starting at or after
    /// this bar moves later; anything that starts before it stays, even
    /// if it plays across the insertion point.
    pub at_bar: u32,
    /// How many bars to insert. Must be at least 1, and the span
    /// (`at_bar + count - 1`) must end at or before bar 100000
    /// ([`crate::MAX_BARS`]).
    pub count: u32,
}

/// Params for `arrangement.remove_bars`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RemoveBarsParams {
    /// 1-based first bar to remove.
    pub at_bar: u32,
    /// How many bars to remove. Must be at least 1, and the span
    /// (`at_bar + count - 1`) must end at or before bar 100000
    /// ([`crate::MAX_BARS`]).
    pub count: u32,
    /// Required when anything starts inside the removed span — those
    /// clips and placements are deleted.
    #[serde(default)]
    pub confirm: bool,
}

/// What a shift actually moved. Counts rather than id lists, except for
/// the deletions, which a caller may be holding ids for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ShiftResult {
    /// Echo of the requested edit.
    pub at_bar: u32,
    pub count: u32,
    /// How far things moved, in samples at the current tempo map.
    /// Negative for `remove_bars`.
    pub shift_samples: i64,
    pub audio_clips_moved: u32,
    pub midi_clips_moved: u32,
    pub placements_moved: u32,
    pub markers_moved: u32,
    pub automation_points_moved: u32,
    /// Tempo / meter changes carried across the cut with the music they
    /// were written for.
    #[serde(default)]
    pub tempo_events_moved: u32,
    #[serde(default)]
    pub signature_events_moved: u32,
    /// Tempo / meter changes dropped because another event of the same
    /// kind ended up on the same bar and is the one in force when the
    /// music after the cut starts. Always 0 for `insert_bars`.
    #[serde(default)]
    pub tempo_events_removed: u32,
    #[serde(default)]
    pub signature_events_removed: u32,
    /// Clips deleted because they started inside a removed span. Always
    /// empty for `insert_bars`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips_deleted: Vec<ClipId>,
    /// Section placements deleted for the same reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub placements_deleted: Vec<u64>,
    pub revision: u64,
}
