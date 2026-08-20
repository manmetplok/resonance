//! Shared geometry helpers for the Compose tab. Every lane canvas (chord,
//! track piano-grid, drum step grid, vocal, global tempo/signature rows)
//! pulls its pixel width from these functions so cells stay aligned and
//! a fixed size regardless of OS window width — the workspace gets a
//! horizontal scrollbar instead of stretching.

use resonance_audio::types::TempoMap;

use super::tracks::NAME_COLUMN_WIDTH;

/// Fixed pixel width per beat in every Compose canvas (chord lane, track
/// piano-grid, drum step grid, global tempo/signature rows). Chosen so a
/// 4/4 bar takes 224 px — close to the prototype's stage width without
/// stretching when the OS window grows.
pub const BEAT_PX_COMPOSE: f32 = 56.0;

/// Total beat count across the section, summing each bar's numerator from
/// the tempo map. Bars with different time signatures contribute different
/// beat counts — mirrors the per-canvas math but lets the layout decide
/// the workspace width up front.
pub fn section_total_beats(tempo_map: &TempoMap, start_bar: u32, length_bars: u32) -> u32 {
    (0..length_bars)
        .map(|b| tempo_map.numerator_at_bar(start_bar + b) as u32)
        .sum()
}

/// Total tick span across the section, summing each bar's length from the
/// tempo map.
///
/// This is *not* `section_total_beats * TICKS_PER_QUARTER_NOTE`: ticks are
/// quarter-note-based but a beat is a note of value `1/denominator`, so a
/// 6/8 bar is six beats but only three quarter notes — 1440 ticks, not
/// 2880. Canvases that place notes (which carry real ticks) must use this;
/// canvases that only lay out equal-width beat cells use
/// [`section_total_beats`] (ba todo #1389).
pub fn section_total_ticks(tempo_map: &TempoMap, start_bar: u32, length_bars: u32) -> u64 {
    (0..length_bars)
        .map(|b| tempo_map.bar_len_ticks_at(start_bar + b))
        .sum()
}

/// Pixel width of every Compose-tab lane (chord lane, track lane, drum
/// lane, global tempo/signature rows). Equal to `NAME_COLUMN_WIDTH` plus
/// `section_total_beats * BEAT_PX_COMPOSE`.
pub fn workspace_width(tempo_map: &TempoMap, start_bar: u32, length_bars: u32) -> f32 {
    NAME_COLUMN_WIDTH + section_total_beats(tempo_map, start_bar, length_bars) as f32 * BEAT_PX_COMPOSE
}
