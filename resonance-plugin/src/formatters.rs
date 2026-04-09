/// Value formatting and parsing functions for plugin parameters.

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
        let s = s.trim().trim_end_matches(" dB").trim_end_matches("dB").trim();
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
