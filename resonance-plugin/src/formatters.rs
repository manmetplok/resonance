//! Value formatting and parsing functions for plugin parameters.
//!
//! Several of these return boxed closure types that clippy thinks are
//! complex enough to warrant a `type` alias, but they're idiomatic
//! one-shot factories — the boxed closure is unwieldy specifically to
//! match the host's parameter API, so we allow the lint module-wide.
#![allow(clippy::type_complexity)]

use std::sync::Arc;

/// Format a gain value (linear) as decibels, rounded to the given decimal places.
pub fn v2s_f32_gain_to_db(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value: f32| {
        if value < 1e-6 {
            "-inf dB".to_string()
        } else {
            format!("{:.*} dB", decimals, 20.0 * value.log10())
        }
    })
}

/// Parse a dB string back to a linear gain value.
pub fn s2v_f32_gain_to_db() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|s: &str| {
        let s = s
            .trim()
            .trim_end_matches(" dB")
            .trim_end_matches("dB")
            .trim();
        if s == "-inf" {
            return Some(0.0);
        }
        s.parse::<f32>().ok().map(|db| 10.0_f32.powf(db / 20.0))
    })
}

/// Format a 0..1 float as a percentage with the given decimal places.
pub fn v2s_f32_percentage(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value: f32| format!("{:.*}%", decimals, value * 100.0))
}

/// Parse a percentage string back to a 0..1 float.
pub fn s2v_f32_percentage() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|s: &str| {
        let s = s.trim().trim_end_matches('%').trim();
        s.parse::<f32>().ok().map(|pct| pct / 100.0)
    })
}

/// Format a float rounded to the given decimal places.
pub fn v2s_f32_rounded(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value: f32| format!("{:.*}", decimals, value))
}

// ---------------------------------------------------------------------------
// Direct-unit formatters — the value is already in the target unit
// (dB, ms, Hz, …) and just needs to be printed.
//
// Plugins historically duplicated these helpers across their param
// modules; centralising them keeps the display style consistent.
// ---------------------------------------------------------------------------

/// Format a dB value with the given decimal precision. The value is
/// already in dB — this is distinct from [`v2s_f32_gain_to_db`] which
/// takes a linear gain and converts it.
pub fn v2s_f32_db(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value: f32| format!("{:.*} dB", decimals, value))
}

/// Format a millisecond value with the given decimal precision.
pub fn v2s_f32_ms(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value: f32| format!("{:.*} ms", decimals, value))
}

/// Format a Hz value — renders below 1 kHz as `NNN Hz` and above as
/// `NN.NN kHz`, which is the convention every plugin in the Resonance
/// family uses for frequency sliders.
pub fn v2s_f32_hz() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|value: f32| {
        if value >= 1000.0 {
            format!("{:.2} kHz", value / 1000.0)
        } else {
            format!("{:.0} Hz", value)
        }
    })
}

/// Parse a frequency string back to Hz, accepting everything
/// [`v2s_f32_hz`] can print plus the shorthands a user reaches for:
/// `440`, `440 Hz`, `1.2 kHz`, `1.2k`, `1.2K`.
///
/// A param formatted with `v2s_f32_hz` needs this to be typeable (ba
/// todo #1287): the default parse only strips the declared unit, so it
/// reads `440 Hz` but chokes on the `kHz` form its own display produces
/// above 1 kHz.
pub fn s2v_f32_hz() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|s: &str| {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        let (number, multiplier) = if let Some(rest) = lower.strip_suffix("khz") {
            (rest, 1000.0)
        } else if let Some(rest) = lower.strip_suffix("hz") {
            (rest, 1.0)
        } else if let Some(rest) = lower.strip_suffix('k') {
            (rest, 1000.0)
        } else {
            (lower.as_str(), 1.0)
        };
        number.trim().parse::<f32>().ok().map(|v| v * multiplier)
    })
}

/// Format a compression ratio as `N.N:1`.
pub fn v2s_f32_ratio() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|value: f32| format!("{:.1}:1", value))
}

/// Parse a compression ratio, accepting both the `4.0:1` form
/// [`v2s_f32_ratio`] prints and a bare `4`.
pub fn s2v_f32_ratio() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|s: &str| {
        let s = s.trim();
        let number = match s.split_once(':') {
            // Only `N:1` means the ratio N; anything else (`4:2`) is not
            // a form this parameter has a value for.
            Some((left, right)) if right.trim() == "1" => left,
            Some(_) => return None,
            None => s,
        };
        number.trim().parse::<f32>().ok()
    })
}

/// Format a 0..1 mix value as a percentage with the given decimal
/// precision. Different from [`v2s_f32_percentage`] only in that it is
/// the canonical name used by plugin params.
pub fn v2s_f32_percent(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |value: f32| format!("{:.*}%", decimals, value * 100.0))
}
