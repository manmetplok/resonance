//! Aggregate meter snapshot published by the plugin's audio thread.
//!
//! A plain `Copy`-able struct that carries every scalar readout the UI
//! cares about. The plugin wraps the latest copy in an `arc_swap::ArcSwap`
//! so the editor thread can read it wait-free.

#[derive(Debug, Clone, Copy)]
pub struct MeterSnapshot {
    pub momentary_lufs: f32,
    pub short_term_lufs: f32,
    pub integrated_lufs: f32,
    pub true_peak_left_dbtp: f32,
    pub true_peak_right_dbtp: f32,
    pub true_peak_max_dbtp: f32,
    pub correlation: f32,
    pub crest_db: f32,
    pub plr_db: f32,
    pub psr_db: f32,
    pub lra_lu: f32,
}

impl Default for MeterSnapshot {
    fn default() -> Self {
        Self {
            momentary_lufs: f32::NEG_INFINITY,
            short_term_lufs: f32::NEG_INFINITY,
            integrated_lufs: f32::NEG_INFINITY,
            true_peak_left_dbtp: -120.0,
            true_peak_right_dbtp: -120.0,
            true_peak_max_dbtp: -120.0,
            correlation: 0.0,
            crest_db: 0.0,
            plr_db: 0.0,
            psr_db: 0.0,
            lra_lu: 0.0,
        }
    }
}
