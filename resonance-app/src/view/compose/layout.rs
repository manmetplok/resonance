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

/// Section-relative tick of an absolute sample position, for a section
/// that starts at 0-based `start_bar`. Negative before the section. Walks
/// the per-bar tick lengths, so meter changes inside or before the
/// section are honoured.
pub fn sample_to_section_tick(
    tempo_map: &TempoMap,
    sample_rate: u32,
    start_bar: u32,
    sample: u64,
) -> f64 {
    let (bar, frac) = tempo_map.sample_to_bar(sample, sample_rate);
    let mut tick: f64 = 0.0;
    if bar > start_bar {
        for b in start_bar..bar {
            tick += tempo_map.bar_len_ticks_at(b) as f64;
        }
    } else if bar < start_bar {
        for b in bar..start_bar {
            tick -= tempo_map.bar_len_ticks_at(b) as f64;
        }
    }
    tick + frac * tempo_map.bar_len_ticks_at(bar) as f64
}

/// Pixel width of every Compose-tab lane (chord lane, track lane, drum
/// lane, global tempo/signature rows). Equal to `NAME_COLUMN_WIDTH` plus
/// `section_total_beats * BEAT_PX_COMPOSE`.
pub fn workspace_width(tempo_map: &TempoMap, start_bar: u32, length_bars: u32) -> f32 {
    NAME_COLUMN_WIDTH + section_total_beats(tempo_map, start_bar, length_bars) as f32 * BEAT_PX_COMPOSE
}

/// Content hash of everything the Compose canvases read off the tempo map:
/// the default meter, every tempo event, and every signature event. Feeds
/// the per-canvas `canvas::Cache` fingerprints — a tempo or signature edit
/// must repaint bar geometry, while unrelated state churn must not.
pub fn tempo_map_hash(tempo_map: &TempoMap) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tempo_map.bpm.to_bits().hash(&mut h);
    tempo_map.numerator.hash(&mut h);
    tempo_map.denominator.hash(&mut h);
    for e in &tempo_map.tempo_points {
        e.bar.hash(&mut h);
        e.bpm.to_bits().hash(&mut h);
    }
    for e in &tempo_map.signature_points {
        e.bar.hash(&mut h);
        e.numerator.hash(&mut h);
        e.denominator.hash(&mut h);
    }
    h.finish()
}

/// Horizontal padding the Compose page puts around its lane column
/// (`page.rs`), i.e. where canvas x = 0 sits in the workspace
/// `Scrollable`'s content.
pub const WORKSPACE_PAD_X: f32 = 20.0;

/// Granularity of the drawn window. The window is snapped outwards to
/// whole blocks (plus one block of slack each side) so scrolling inside a
/// block keeps every canvas cache warm; only crossing a block repaints.
const VISIBLE_BLOCK_PX: f32 = 1024.0;

/// Window assumed before the workspace `Scrollable` has reported its
/// viewport (it does on its first redraw): wider than any screen.
const DEFAULT_VIEW_WIDTH: f32 = 4096.0;

/// The canvas-x range `[lo, hi]` of the Compose workspace that may be on
/// screen, given the workspace `Scrollable`'s last reported
/// `(offset_x, width)` (FU-V2c). Canvases draw only bars that intersect
/// it, so a 100 000-bar section costs a screenful of geometry, not a
/// section's worth.
///
/// Every lane canvas starts at the lane column's left edge, so one window
/// serves them all. Snapped to [`VISIBLE_BLOCK_PX`] blocks — it is part of
/// the canvases' cache fingerprints.
pub fn visible_x_window(view: Option<(f32, f32)>) -> (f32, f32) {
    let (offset_x, width) = view.unwrap_or((0.0, DEFAULT_VIEW_WIDTH));
    let left = offset_x - WORKSPACE_PAD_X;
    let lo = (left / VISIBLE_BLOCK_PX).floor() * VISIBLE_BLOCK_PX - VISIBLE_BLOCK_PX;
    let hi = ((left + width.max(0.0)) / VISIBLE_BLOCK_PX).ceil() * VISIBLE_BLOCK_PX
        + VISIBLE_BLOCK_PX;
    (lo.max(0.0), hi.max(0.0))
}

/// Which axis a canvas lays bars out on: equal-width beat cells (chord
/// lane) or real tick spans (note grids); see [`section_total_ticks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarUnit {
    Beats,
    Ticks,
}

/// One bar of a section with its section-relative position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionBar {
    /// 0-based bar offset inside the section.
    pub offset: u32,
    /// Section-relative beat at the bar's start.
    pub beat: u64,
    /// Section-relative tick at the bar's start.
    pub tick: u64,
    /// Beats in this bar (its numerator).
    pub beats: u32,
    /// Ticks in this bar.
    pub ticks: u64,
}

/// The bars of a section that intersect `[lo, hi]` (in `unit`, section-
/// relative), in order — what a canvas walks instead of every bar
/// (FU-V2c).
///
/// Work is bounded by the signature changes plus the bars returned, not
/// by the section's length: a stretch between two signature points has
/// one bar size, so it is skipped in one step when it lies wholly outside
/// the range.
pub fn section_bars_in_range(
    tempo_map: &TempoMap,
    start_bar: u32,
    length_bars: u32,
    unit: BarUnit,
    lo: f64,
    hi: f64,
) -> Vec<SectionBar> {
    let mut out = Vec::new();
    let (mut beat, mut tick, mut offset) = (0u64, 0u64, 0u32);
    while offset < length_bars {
        let bar = start_bar + offset;
        // The signature — and so the bar size — is constant up to the
        // next signature point.
        let run_end = tempo_map
            .signature_points
            .iter()
            .map(|p| p.bar)
            .filter(|&b| b > bar)
            .min()
            .map_or(length_bars, |b| (b - start_bar).min(length_bars));
        let run_len = run_end - offset;
        let beats = tempo_map.numerator_at_bar(bar) as u32;
        let ticks = tempo_map.bar_len_ticks_at(bar);
        let (pos, size) = match unit {
            BarUnit::Beats => (beat as f64, beats as f64),
            BarUnit::Ticks => (tick as f64, ticks as f64),
        };
        if pos > hi {
            break;
        }
        if size > 0.0 && pos + run_len as f64 * size >= lo {
            let first = if pos >= lo {
                0
            } else {
                (((lo - pos) / size).floor() as u32).min(run_len)
            };
            for k in first..run_len {
                if pos + k as f64 * size > hi {
                    break;
                }
                out.push(SectionBar {
                    offset: offset + k,
                    beat: beat + k as u64 * beats as u64,
                    tick: tick + k as u64 * ticks,
                    beats,
                    ticks,
                });
            }
        }
        beat += run_len as u64 * beats as u64;
        tick += run_len as u64 * ticks;
        offset = run_end;
    }
    out
}
