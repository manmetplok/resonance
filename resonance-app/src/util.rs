/// Utility functions for the Resonance application.
use std::borrow::Cow;

/// Convert dB to linear gain. -60 dB or below maps to 0.0 (silence).
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0f32.powf(db / 20.0)
    }
}

/// Format a dB value for display. Returns "-inf" for -60 dB or below.
pub fn format_db(db: f32) -> Cow<'static, str> {
    if db <= -60.0 {
        Cow::Borrowed("-inf")
    } else {
        Cow::Owned(format!("{:.1}", db))
    }
}

/// Truncate `s` to at most `max` characters, replacing the tail with
/// `…` when it overflows. Char-counted (not bytes), so multi-byte
/// names truncate cleanly. The result, including the ellipsis, never
/// exceeds `max` chars.
pub fn short(s: &str, max: usize) -> String {
    short_with(s, max, "\u{2026}")
}

/// `short` with a caller-chosen ellipsis suffix (e.g. ".." or "...")
/// for sites whose rendered width was tuned around an ASCII suffix.
pub fn short_with(s: &str, max: usize, ellipsis: &str) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep = max.saturating_sub(ellipsis.chars().count());
        let mut t: String = s.chars().take(keep).collect();
        t.push_str(ellipsis);
        t
    }
}

/// 2^64 / φ (the splitmix64 increment). Multiplying or adding this
/// constant spreads small sequential ids across the full u64 range, so
/// it's the canonical mixer for deriving and stepping the deterministic
/// generator seeds used by the compose tab.
pub const GOLDEN_RATIO_SEED: u64 = 0x9E3779B97F4A7C15;

/// Derive a well-mixed initial generator seed from a small id
/// (definition id, group id, …).
pub fn seed_from_id(id: u64) -> u64 {
    id.wrapping_mul(GOLDEN_RATIO_SEED)
}

/// Step a generator seed to its next value ("re-roll"). The `+ 1`
/// keeps a zero seed moving; the multiply decorrelates successive
/// seeds so re-rolls don't produce near-identical output.
pub fn next_seed(seed: u64) -> u64 {
    seed.wrapping_add(1).wrapping_mul(GOLDEN_RATIO_SEED)
}

/// Step a generator seed by an additive mix constant (plus 1 so a zero
/// mix still moves). Used where several distinct actions step the same
/// seed and must land on different streams — each action passes its own
/// mix constant, typically [`GOLDEN_RATIO_SEED`].
pub fn bump_seed(seed: u64, mix: u64) -> u64 {
    seed.wrapping_add(mix).wrapping_add(1)
}

/// Format a pan value for display. Returns "C", "L50", "R50" etc.
pub fn format_pan(pan: f32) -> Cow<'static, str> {
    if pan.abs() < 0.01 {
        Cow::Borrowed("C")
    } else if pan < 0.0 {
        Cow::Owned(format!("L{:.0}", -pan * 100.0))
    } else {
        Cow::Owned(format!("R{:.0}", pan * 100.0))
    }
}
