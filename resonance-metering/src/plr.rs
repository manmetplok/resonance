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

    /// PLR and PSR of a whole measured range (warmth-width-depth.md
    /// §2.1): `plr = true_peak − integrated`, `psr = true_peak − loudest
    /// short-term window`. Unlike [`compute`](Self::compute), a figure
    /// whose loudness input does not exist (silence, a range shorter than
    /// the window) is `None` rather than a `0.0` that reads like a
    /// fully-crushed reading.
    pub fn range(
        true_peak_dbtp: f32,
        integrated_lufs: f32,
        short_term_max_lufs: f32,
    ) -> RangeDynamics {
        let readout = Self::compute(
            true_peak_dbtp,
            true_peak_dbtp,
            integrated_lufs,
            short_term_max_lufs,
        );
        let exists = |lufs: f32| lufs.is_finite() && true_peak_dbtp.is_finite();
        RangeDynamics {
            plr_db: exists(integrated_lufs).then_some(readout.plr_db),
            psr_db: exists(short_term_max_lufs).then_some(readout.psr_db),
        }
    }
}

/// [`PlrMeter::range`]'s result.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RangeDynamics {
    /// True peak over integrated loudness, dB.
    pub plr_db: Option<f32>,
    /// True peak over the loudest 3 s short-term window, dB.
    pub psr_db: Option<f32>,
}

