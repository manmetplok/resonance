//! Pure drum-arrangement resolution.
//!
//! A section owns an ordered [`Vec<PatternEntry>`](PatternEntry) — its
//! *arrangement* — describing which drum pattern plays over which bars.
//! This module turns that declarative arrangement plus the section's bar
//! length into a flat, ordered list of [`ArrangementSpan`]s (one per
//! contiguous run of a single pattern) along with an
//! [`ArrangementCoverage`] status saying whether the entries fill the
//! section exactly, leave a tail gap, or overflow past the end.
//!
//! The resolver is deliberately free-standing and deterministic: it takes
//! the entries, the section length, and a `pattern_len` lookup closure so
//! it can be unit-tested without a whole [`ComposeState`]. `ComposeState`
//! wraps it in
//! [`resolve_arrangement_for`](crate::compose::ComposeState::resolve_arrangement_for),
//! supplying real per-pattern bar lengths from the pattern bank.

use super::section::{EntryLength, PatternEntry};

/// One contiguous run of bars mapping to a single concrete pattern.
///
/// Bars are 0-based and section-relative; the range is half-open
/// `[bar_start, bar_end)`, so `bar_end - bar_start` is the span's bar
/// count and a span always covers at least one bar. `is_fill` marks the
/// span produced by an entry's [`PatternEntry::fill`] (its last bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrangementSpan {
    /// First bar of the span (inclusive, 0-based within the section).
    pub bar_start: u32,
    /// One past the last bar of the span (exclusive).
    pub bar_end: u32,
    /// Concrete pattern that plays across this span.
    pub pattern_id: u64,
    /// `true` when this span is an entry's fill bar.
    pub is_fill: bool,
}

impl ArrangementSpan {
    /// Number of bars this span covers.
    pub fn bar_count(&self) -> u32 {
        self.bar_end - self.bar_start
    }
}

// ---------------------------------------------------------------------------
// Display helpers for the ARRANGEMENT strip (view) — pure string/label logic
// kept next to the model so they can be unit-tested without a renderer.
// ---------------------------------------------------------------------------

/// Human-readable bar span of a single arrangement entry, as shown on its
/// strip chip. A `RepeatN(n)` entry over a `pattern_bars`-bar pattern reads
/// `"2b ×3 = 6b"`; a `Bars(b)` entry reads `"4b fixed"`. `pattern_bars` is
/// guarded to at least `1` to mirror [`super::state`]'s resolver.
pub fn entry_span_label(length: EntryLength, pattern_bars: u32) -> String {
    let pattern_bars = pattern_bars.max(1);
    match length {
        EntryLength::RepeatN(n) => {
            let total = n.saturating_mul(pattern_bars);
            format!("{pattern_bars}b ×{n} = {total}b")
        }
        EntryLength::Bars(b) => format!("{b}b fixed"),
    }
}

/// Compact value shown inside an entry's inline stepper: `"×3"` for a
/// `RepeatN(3)` entry, `"4b"` for a `Bars(4)` entry.
pub fn entry_stepper_label(length: EntryLength) -> String {
    match length {
        EntryLength::RepeatN(n) => format!("×{n}"),
        EntryLength::Bars(b) => format!("{b}b"),
    }
}

/// Apply a `±1` stepper nudge to an entry length, preserving its mode and
/// clamping the value to a minimum of `1` (a zero-bar entry contributes
/// nothing, so the stepper never drives below one). `delta` is typically
/// `-1` or `+1`.
pub fn step_entry_length(length: EntryLength, delta: i32) -> EntryLength {
    fn stepped(v: u32, delta: i32) -> u32 {
        (i64::from(v) + i64::from(delta)).clamp(1, i64::from(u32::MAX)) as u32
    }
    match length {
        EntryLength::RepeatN(n) => EntryLength::RepeatN(stepped(n, delta)),
        EntryLength::Bars(b) => EntryLength::Bars(stepped(b, delta)),
    }
}

/// The raw stepper value of an entry, used to disable the `−` button at the
/// `1` floor regardless of length mode.
pub fn entry_stepper_value(length: EntryLength) -> u32 {
    match length {
        EntryLength::RepeatN(n) => n,
        EntryLength::Bars(b) => b,
    }
}

impl ArrangementCoverage {
    /// Bars actually covered by the entries, reconstructed from the coverage
    /// delta against `section_bars`: `Exact` covers the whole section,
    /// `Gap { bars }` falls `bars` short, `Overflow { bars }` exceeds it by
    /// `bars`.
    pub fn covered_bars(&self, section_bars: u32) -> u32 {
        match self {
            ArrangementCoverage::Exact => section_bars,
            ArrangementCoverage::Gap { bars } => section_bars.saturating_sub(*bars),
            ArrangementCoverage::Overflow { bars } => section_bars.saturating_add(*bars),
        }
    }

    /// Text for the coverage pill chip: `"8 / 8 bars"` when exact,
    /// `"4 / 8 · gap 4"` when under-filled, `"10 / 8 · overflow 2"` when
    /// over-filled. `section_bars` is the denominator (the section length).
    pub fn chip_label(&self, section_bars: u32) -> String {
        let covered = self.covered_bars(section_bars);
        match self {
            ArrangementCoverage::Exact => format!("{covered} / {section_bars} bars"),
            ArrangementCoverage::Gap { bars } => {
                format!("{covered} / {section_bars} · gap {bars}")
            }
            ArrangementCoverage::Overflow { bars } => {
                format!("{covered} / {section_bars} · overflow {bars}")
            }
        }
    }
}

/// How an arrangement's total bar span lines up with the section length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrangementCoverage {
    /// Entries fill the section exactly — no gap, no overflow.
    Exact,
    /// Entries fall short by `bars`. The trailing bars have no span; a
    /// caller resolves them against the section's primary/default pattern.
    Gap { bars: u32 },
    /// Entries exceed the section by `bars`. Spans are clipped at the
    /// section end; `bars` records how much was dropped.
    Overflow { bars: u32 },
}

/// The result of resolving an arrangement: the ordered, clipped spans plus
/// the coverage status relative to the section length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedArrangement {
    /// Ordered, non-overlapping spans clipped to the section length. Empty
    /// when the arrangement is empty (or every entry was zero-length).
    pub spans: Vec<ArrangementSpan>,
    /// Coverage of the section by the (unclipped) entries.
    pub coverage: ArrangementCoverage,
}

impl ResolvedArrangement {
    /// Find the span covering `bar` (0-based, section-relative), if any.
    /// Returns `None` for bars in a trailing gap or past an overflow clip.
    pub fn span_at(&self, bar: u32) -> Option<&ArrangementSpan> {
        self.spans
            .iter()
            .find(|s| bar >= s.bar_start && bar < s.bar_end)
    }
}

/// Resolve an `arrangement` over a section of `section_bars` bars into an
/// ordered list of spans and a coverage status.
///
/// `pattern_len` returns a pattern's intrinsic bar length (see
/// [`crate::compose::DrumPattern::bar_span`]); it is consulted only for
/// [`EntryLength::RepeatN`] entries to expand repeats into concrete bars.
///
/// Semantics:
/// - Entries are laid down head-to-tail starting at bar 0.
/// - `RepeatN(n)` spans `n * pattern_len` bars; `Bars(b)` spans `b` bars.
/// - A zero-bar entry (`RepeatN(0)`, `Bars(0)`, or a pattern that somehow
///   reports zero length) contributes nothing and is skipped.
/// - An entry's `fill` replaces the **last bar** of that entry's span; if
///   the entry is a single bar, the whole span becomes the fill.
/// - Coverage is computed from the unclipped total; spans are then clipped
///   to `[0, section_bars)` and empty results dropped.
///
/// The function is pure and deterministic — same inputs, same output.
pub fn resolve_arrangement(
    arrangement: &[PatternEntry],
    section_bars: u32,
    pattern_len: impl Fn(u64) -> u32,
) -> ResolvedArrangement {
    let mut spans: Vec<ArrangementSpan> = Vec::new();
    let mut cursor: u32 = 0;

    for entry in arrangement {
        let span_bars = match entry.length {
            EntryLength::RepeatN(n) => n.saturating_mul(pattern_len(entry.pattern_id).max(1)),
            EntryLength::Bars(b) => b,
        };
        if span_bars == 0 {
            continue;
        }

        let start = cursor;
        let end = cursor.saturating_add(span_bars);

        match entry.fill {
            // Fill replaces the final bar. `span_bars >= 1`, so `end > start`.
            Some(fill_id) => {
                let fill_start = end - 1;
                if fill_start > start {
                    spans.push(ArrangementSpan {
                        bar_start: start,
                        bar_end: fill_start,
                        pattern_id: entry.pattern_id,
                        is_fill: false,
                    });
                }
                spans.push(ArrangementSpan {
                    bar_start: fill_start,
                    bar_end: end,
                    pattern_id: fill_id,
                    is_fill: true,
                });
            }
            None => spans.push(ArrangementSpan {
                bar_start: start,
                bar_end: end,
                pattern_id: entry.pattern_id,
                is_fill: false,
            }),
        }

        cursor = end;
    }

    let total = cursor;
    let coverage = match total.cmp(&section_bars) {
        std::cmp::Ordering::Equal => ArrangementCoverage::Exact,
        std::cmp::Ordering::Less => ArrangementCoverage::Gap {
            bars: section_bars - total,
        },
        std::cmp::Ordering::Greater => ArrangementCoverage::Overflow {
            bars: total - section_bars,
        },
    };

    // Clip spans to the section: drop those starting past the end, trim
    // any straddling the boundary, then discard spans emptied by the trim.
    spans.retain(|s| s.bar_start < section_bars);
    for s in spans.iter_mut() {
        if s.bar_end > section_bars {
            s.bar_end = section_bars;
        }
    }
    spans.retain(|s| s.bar_start < s.bar_end);

    ResolvedArrangement { spans, coverage }
}
