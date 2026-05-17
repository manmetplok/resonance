//! Peak-to-Loudness ratios.
//!
//! * **PLR**  = `true_peak_dBTP - integrated_LUFS`
//! * **PSR**  = `short_term_true_peak_dBTP - short_term_LUFS`
//!
//! These are Ian Shepherd's loudness-war detection metrics — they respond
//! to crushed dynamics that LRA alone can miss. Pure arithmetic wrappers;
//! state lives on the source meters.

#[derive(Debug, Clone, Copy, Default)]
pub struct PlrReadout {
    /// Peak-to-Loudness Ratio, integrated.
    pub plr_db: f32,
    /// Peak-to-Short-term Ratio, momentary.
    pub psr_db: f32,
}

/// Stateless computation helper. A plain function wrapped in a unit
/// struct so the mastering plugin can mock or replace it later without
/// touching its callers.
pub struct PlrMeter;

impl PlrMeter {
    /// Compute PLR and PSR.
    ///
    /// Any input that is `NEG_INFINITY` or not finite yields a zero
    /// contribution so the UI doesn't flash a `-inf` when silent.
    pub fn compute(
        true_peak_dbtp: f32,
        short_term_true_peak_dbtp: f32,
        integrated_lufs: f32,
        short_term_lufs: f32,
    ) -> PlrReadout {
        let plr = if integrated_lufs.is_finite() && true_peak_dbtp.is_finite() {
            true_peak_dbtp - integrated_lufs
        } else {
            0.0
        };
        let psr = if short_term_lufs.is_finite() && short_term_true_peak_dbtp.is_finite() {
            short_term_true_peak_dbtp - short_term_lufs
        } else {
            0.0
        };
        PlrReadout {
            plr_db: plr,
            psr_db: psr,
        }
    }
}

