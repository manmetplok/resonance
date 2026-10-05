//! Offline impulse-response analysis for reverbs (reverb-algorithms.md
//! §5.1): decay times, echo density, modal colour and stereo spread.
//!
//! Whole-buffer, allocating, not for the audio thread. Every function takes
//! f32 slices and a sample rate; times are in seconds.
//!
//! - [`edc`]: Schroeder energy decay curve with a simplified Lundeby
//!   noise-floor truncation; EDT, T20, T30 ([`Edc`], [`DecayTimes`]).
//! - [`bands`]: octave-band filters 125 Hz – 8 kHz, and
//!   [`band_decay_times`] per band.
//! - [`density`]: Abel & Huang normalised echo density profile.
//! - [`peakiness`]: modal peakiness of the late tail.
//! - [`stereo`]: late IACC and mono-fold loss.
//! - [`ImpulseReport`]: all of it for one stereo response, printable as a
//!   table row.
//! - [`program`]: the same decay times read off a stop in program
//!   material instead of an impulse ([`program_decay`], the
//!   `meter.measure` `decay` detail).

pub mod bands;
pub mod density;
pub mod edc;
pub mod peakiness;
pub mod program;
pub mod stereo;

use std::fmt;

pub use bands::{octave_band, OctaveBandFilter, OCTAVE_BANDS_HZ};
pub use density::{echo_density_profile, EchoDensity, ECHO_DENSITY_HOP_S, ECHO_DENSITY_WINDOW_S};
pub use edc::{edc_from_energy, energy_decay_curve, DecayTimes, Edc};
pub use peakiness::{
    modal_peakiness_db, PEAKINESS_BAND_HZ, PEAKINESS_SEGMENT_S, PEAKINESS_START_S,
};
pub use program::{program_decay, required_range_db, DecayEnd, ProgramDecay, CLEAN_DECAY_DB};
pub use stereo::{late_iacc, mono_fold_db, IACC_MAX_LAG_S, LATE_START_S, MONO_FOLD_FLOOR_DB};

/// Decay times of one octave band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandDecay {
    pub center_hz: f32,
    pub times: DecayTimes,
}

/// EDT/T20/T30 of a mono response.
pub fn decay_times(ir: &[f32], sample_rate: f32) -> DecayTimes {
    energy_decay_curve(ir, sample_rate).times()
}

/// EDT/T20/T30 of a stereo response, from the summed energy `L² + R²`.
pub fn stereo_decay_times(l: &[f32], r: &[f32], sample_rate: f32) -> DecayTimes {
    let n = l.len().min(r.len());
    let e: Vec<f64> = (0..n)
        .map(|i| (l[i] as f64).powi(2) + (r[i] as f64).powi(2))
        .collect();
    edc_from_energy(&e, sample_rate).times()
}

/// Per-octave-band decay times of a mono response, one per
/// [`OCTAVE_BANDS_HZ`] entry.
pub fn band_decay_times(ir: &[f32], sample_rate: f32) -> [BandDecay; 7] {
    stereo_band_decay_times(ir, ir, sample_rate)
}

/// Per-octave-band decay times of a stereo response (each band's energy
/// summed over L and R). Pass the same buffer twice for mono.
pub fn stereo_band_decay_times(l: &[f32], r: &[f32], sample_rate: f32) -> [BandDecay; 7] {
    let n = l.len().min(r.len());
    OCTAVE_BANDS_HZ.map(|fc| {
        let bl = OctaveBandFilter::new(sample_rate, fc).filter(&l[..n]);
        let br = OctaveBandFilter::new(sample_rate, fc).filter(&r[..n]);
        let e: Vec<f64> = bl.iter().zip(&br).map(|(a, b)| a * a + b * b).collect();
        BandDecay {
            center_hz: fc,
            times: edc_from_energy(&e, sample_rate).times(),
        }
    })
}

/// Everything §5.1 measures, for one stereo impulse response whose
/// excitation is at sample 0.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpulseReport {
    pub sample_rate: f32,
    /// Largest |sample| over both channels.
    pub peak: f32,
    /// Every sample finite.
    pub finite: bool,
    /// RMS over the first 2 s of both channels, dBFS (the silence guard).
    pub rms_2s_dbfs: f32,
    /// Broadband decay times of `L² + R²`.
    pub broadband: DecayTimes,
    pub bands: [BandDecay; 7],
    /// Mean of the L and R echo density profiles
    /// ([`ECHO_DENSITY_WINDOW_S`] window).
    pub echo_density: EchoDensity,
    /// Mean of the L and R peakiness from [`PEAKINESS_START_S`] over
    /// [`PEAKINESS_BAND_HZ`].
    pub peakiness_db: Option<f32>,
    /// From [`LATE_START_S`].
    pub late_iacc: Option<f32>,
    /// From [`LATE_START_S`].
    pub mono_fold_db: Option<f32>,
}

impl ImpulseReport {
    /// Analyse a stereo response with the §5.1 defaults.
    pub fn analyze(l: &[f32], r: &[f32], sample_rate: f32) -> Self {
        let n = l.len().min(r.len());
        let (l, r) = (&l[..n], &r[..n]);
        let peak = l.iter().chain(r).fold(0.0f32, |m, x| m.max(x.abs()));
        let finite = l.iter().chain(r).all(|x| x.is_finite());
        let n2 = n.min((2.0 * sample_rate) as usize);
        let e2: f64 = l[..n2]
            .iter()
            .chain(&r[..n2])
            .map(|&x| (x as f64) * (x as f64))
            .sum();
        let rms_2s_dbfs = if n2 > 0 {
            (10.0 * (e2 / (2 * n2) as f64).max(1e-30).log10()) as f32
        } else {
            -300.0
        };

        let dl = echo_density_profile(l, sample_rate, ECHO_DENSITY_WINDOW_S);
        let dr = echo_density_profile(r, sample_rate, ECHO_DENSITY_WINDOW_S);
        let echo_density = EchoDensity {
            hop_s: dl.hop_s,
            values: dl
                .values
                .iter()
                .zip(&dr.values)
                .map(|(a, b)| 0.5 * (a + b))
                .collect(),
        };

        let (f_lo, f_hi) = PEAKINESS_BAND_HZ;
        let pk = |x: &[f32]| modal_peakiness_db(x, sample_rate, PEAKINESS_START_S, f_lo, f_hi);
        let peakiness_db = match (pk(l), pk(r)) {
            (Some(a), Some(b)) => Some(0.5 * (a + b)),
            (a, b) => a.or(b),
        };

        Self {
            sample_rate,
            peak,
            finite,
            rms_2s_dbfs,
            broadband: stereo_decay_times(l, r, sample_rate),
            bands: stereo_band_decay_times(l, r, sample_rate),
            echo_density,
            peakiness_db,
            late_iacc: late_iacc(l, r, sample_rate, LATE_START_S),
            mono_fold_db: mono_fold_db(l, r, sample_rate, LATE_START_S),
        }
    }

    /// Mid-frequency T30: the mean of the 500 Hz and 1 kHz bands (ISO
    /// 3382's `T30,mid`). `None` if either is.
    pub fn mid_t30(&self) -> Option<f32> {
        let t = |hz: f32| self.bands.iter().find(|b| b.center_hz == hz)?.times.t30;
        Some(0.5 * (t(500.0)? + t(1_000.0)?))
    }

    /// Header line matching [`ImpulseReport`]'s `Display` row.
    pub fn table_header() -> String {
        let mut s = String::from("  T30     EDT    ");
        for b in OCTAVE_BANDS_HZ {
            s.push_str(&format!(" T30@{:<5}", band_label(b)));
        }
        s.push_str("  dens0.9 dens1.0  peaky_dB IACC   mono_dB  rms2s");
        s
    }
}

fn band_label(hz: f32) -> String {
    if hz >= 1_000.0 {
        format!("{}k", hz / 1_000.0)
    } else {
        format!("{hz}")
    }
}

fn opt(v: Option<f32>, prec: usize) -> String {
    v.map_or_else(|| "-".to_string(), |x| format!("{x:.prec$}"))
}

/// One table row: broadband T30 and EDT, T30 per band, the times the
/// echo density reaches 0.9 and 1.0, peakiness, IACC, mono fold and the
/// 2 s RMS. `-` marks a figure that could not be measured.
impl fmt::Display for ImpulseReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:>6}s {:>6}s",
            opt(self.broadband.t30, 3),
            opt(self.broadband.edt, 3)
        )?;
        for b in &self.bands {
            write!(f, " {:>9}", opt(b.times.t30, 3))?;
        }
        write!(
            f,
            "  {:>7} {:>7}  {:>7}  {:>5}  {:>7}  {:>6.1}",
            opt(self.echo_density.time_to_reach(0.9).map(|t| t * 1e3), 0),
            opt(self.echo_density.time_to_reach(1.0).map(|t| t * 1e3), 0),
            opt(self.peakiness_db, 1),
            opt(self.late_iacc, 3),
            opt(self.mono_fold_db, 2),
            self.rms_2s_dbfs
        )
    }
}
