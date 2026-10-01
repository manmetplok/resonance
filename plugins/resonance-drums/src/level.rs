//! Levels in decibels (drums-plugin-rework.md §7 E9, D6).
//!
//! Every level the drums expose — master, pad volume, the per-mic trims —
//! is a dB parameter. The bottom of each range is **−∞**: a value at or
//! below [`MIN_DB`] is silence, shown as `-inf dB`, so a fader pulled all
//! the way down mutes exactly rather than leaving −60 dB of kit.
//!
//! 0 dB is unity and maps to a gain of exactly `1.0` (`10^0`), which is
//! what lets the master stage skip its multiply at the default.

/// The floor of every level range; it reads, and sounds, as −∞.
pub const MIN_DB: f32 = -60.0;

/// The top of the volume and master ranges.
pub const MAX_VOLUME_DB: f32 = 6.0;

/// The top of the per-mic trim range: a quiet mic may be lifted further
/// than a whole pad.
pub const MAX_TRIM_DB: f32 = 12.0;

/// The gain of `db`: `0.0` at or below [`MIN_DB`], exactly `1.0` at 0 dB.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= MIN_DB || db.is_nan() {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// The level, in dB, of a linear `gain`, floored at [`MIN_DB`] (silence
/// or anything quieter than the floor reads as −∞). The v1 → v2 state
/// conversion (`params::upgrade_v1_levels`) goes through this.
pub fn gain_to_db(gain: f32) -> f32 {
    if gain.is_nan() || gain <= 0.0 {
        return MIN_DB;
    }
    (20.0 * gain.log10()).max(MIN_DB)
}

/// How a level reads: `-inf dB` at the floor, else signed with one
/// decimal (`+1.5 dB`, `0.0 dB`, `-12.0 dB`).
pub fn db_label(db: f32) -> String {
    if db <= MIN_DB {
        "-inf dB".to_string()
    } else {
        let rounded = (db * 10.0).round() / 10.0;
        if rounded > 0.0 {
            format!("+{rounded:.1} dB")
        } else if rounded == 0.0 {
            // Never "-0.0 dB".
            "0.0 dB".to_string()
        } else {
            format!("{rounded:.1} dB")
        }
    }
}

/// Parse what [`db_label`] wrote (or a bare number, with or without
/// `dB`; `-inf`/`inf` for silence) back to a level.
pub fn db_from_label(text: &str) -> Option<f32> {
    // A typographic minus (U+2212, what `−6 dB` is often typed or pasted
    // as) reads as an ASCII one.
    let normalized;
    let t = if text.contains('\u{2212}') {
        normalized = text.replace('\u{2212}', "-");
        normalized.trim()
    } else {
        text.trim()
    };
    let t = t
        .strip_suffix("dB")
        .or_else(|| t.strip_suffix("db"))
        .or_else(|| t.strip_suffix("DB"))
        .unwrap_or(t)
        .trim();
    if t.eq_ignore_ascii_case("-inf") || t.eq_ignore_ascii_case("inf") || t == "-∞" || t == "∞" {
        return Some(MIN_DB);
    }
    let v: f32 = t.trim_start_matches('+').parse().ok()?;
    v.is_finite().then_some(v)
}
