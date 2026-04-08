/// Utility functions for the Resonance application.

/// Convert dB to linear gain. -60 dB or below maps to 0.0 (silence).
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0f32.powf(db / 20.0)
    }
}

/// Format a dB value for display. Returns "-inf" for -60 dB or below.
pub fn format_db(db: f32) -> String {
    if db <= -60.0 {
        "-inf".to_string()
    } else {
        format!("{:.1}", db)
    }
}

/// Format a pan value for display. Returns "C", "L50", "R50" etc.
pub fn format_pan(pan: f32) -> String {
    if pan.abs() < 0.01 {
        "C".to_string()
    } else if pan < 0.0 {
        format!("L{:.0}", -pan * 100.0)
    } else {
        format!("R{:.0}", pan * 100.0)
    }
}
