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
