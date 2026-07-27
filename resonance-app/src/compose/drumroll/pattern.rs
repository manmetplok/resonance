//! Drum patterns — the project-scoped pattern bank.
//!
//! A "pattern" wraps a named [`Vec<DrumGroup>`]. The project owns a small
//! library of patterns and each section definition picks one of them; the
//! same drum track therefore plays different rhythms across the song
//! without forcing the user to author one giant mega-pattern with
//! polymetric tricks. The view re-uses the same group/articulation/cell
//! plumbing — only the *source* of groups changes per section.

use serde::{Deserialize, Serialize};

use super::groups::{default_drum_groups, DrumGroup, GROUP_PALETTE};

/// One entry in the project's pattern bank.
///
/// Holds an id (stable across saves), a display name shown in the picker,
/// an accent color used to tint the picker chip and the lane header, and
/// the groups that make up the pattern. The kit/articulation library is
/// shared project-wide so duplicating a pattern only forks the *rhythm*,
/// not the kit definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrumPattern {
    pub id: u64,
    pub name: String,
    /// Pattern accent color (RGB). Used by the picker chip and the lane's
    /// "PATTERN · Name" header tint so two patterns at a glance look
    /// distinct.
    pub color: [u8; 3],
    /// Drum groups owned by this pattern. Each group keeps its own id,
    /// grid, cycle, phase, articulation pads, and generator knobs — see
    /// [`DrumGroup`].
    pub groups: Vec<DrumGroup>,
    /// Intrinsic bar length of this pattern: how many bars the pattern
    /// spans before it loops. Defaults to `1`. An arrangement entry's
    /// [`EntryLength::RepeatN(n)`] maps to `n * length_bars` concrete bars
    /// — so a 2-bar pattern repeated 3× covers 6 bars. Persisted with a
    /// serde default so projects authored before this field loaded as
    /// single-bar patterns.
    ///
    /// [`EntryLength::RepeatN(n)`]: crate::compose::EntryLength::RepeatN
    #[serde(default = "default_pattern_length_bars")]
    pub length_bars: u32,
}

/// Serde default for [`DrumPattern::length_bars`]: a single bar. Also the
/// value used when a stored pattern records `0`, guarded by
/// [`DrumPattern::bar_span`].
fn default_pattern_length_bars() -> u32 {
    1
}

impl DrumPattern {
    /// Number of groups in this pattern. Cheap accessor for the picker
    /// chip label.
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Effective bar length, guarded to be at least `1` so a malformed
    /// `length_bars: 0` never collapses an arrangement entry to a
    /// zero-bar span. Resolvers should prefer this over the raw field.
    pub fn bar_span(&self) -> u32 {
        self.length_bars.max(1)
    }

    /// Total step count across every group's pattern slot. Surfaced in
    /// the picker as "{n} steps" so a glance hints which pattern is the
    /// busy one. A step counts once per pad per cycle slot.
    pub fn total_steps(&self) -> usize {
        self.groups
            .iter()
            .map(|g| g.pads.iter().map(|p| p.pattern.len()).sum::<usize>())
            .sum()
    }
}

/// Build the project's default pattern bank: a "Main" pattern that
/// inherits the historical default kit/snare/hat/toms/perc layout, plus
/// an empty "B Section" pattern as a starting point so the picker
/// always offers more than one option to switch between.
///
/// `next_id` is bumped past every id this function allocates so the
/// caller can keep using the same monotonic counter for definitions,
/// placements, etc.
pub fn default_drum_patterns(next_id: &mut u64) -> Vec<DrumPattern> {
    fn alloc(next_id: &mut u64) -> u64 {
        *next_id += 1;
        *next_id
    }

    let main_groups = default_drum_groups(next_id);
    let main_id = alloc(next_id);
    let b_id = alloc(next_id);

    vec![
        DrumPattern {
            id: main_id,
            name: "Main".to_string(),
            color: GROUP_PALETTE[0],
            groups: main_groups,
            length_bars: 1,
        },
        DrumPattern {
            id: b_id,
            name: "B section".to_string(),
            color: GROUP_PALETTE[2],
            groups: Vec::new(),
            length_bars: 1,
        },
    ]
}

/// Wrap a legacy flat-`drum_groups` project layout into a single-pattern
/// bank named "Main". Returns `(patterns, default_pattern_id)`. Used on
/// project load so projects authored before the pattern bank existed
/// open unchanged.
///
/// `next_id` is bumped past the synthesized pattern id so the caller's
/// allocator never collides.
pub fn legacy_groups_to_pattern(
    groups: Vec<DrumGroup>,
    next_id: &mut u64,
) -> (Vec<DrumPattern>, u64) {
    *next_id += 1;
    let id = *next_id;
    let pattern = DrumPattern {
        id,
        name: "Main".to_string(),
        color: GROUP_PALETTE[0],
        groups,
        length_bars: 1,
    };
    (vec![pattern], id)
}
