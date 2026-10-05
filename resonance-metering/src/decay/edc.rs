//! Schroeder energy decay curve and the decay times fitted to it.
//!
//! ## Onset
//!
//! The curve starts at the response's onset: the first sample whose
//! energy is within 20 dB of the peak sample's (ISO 3382-1 §A.3.2). A
//! pre-delay, or a band filter's group delay, would otherwise add a flat
//! 0 dB stretch that drags EDT long.
//!
//! ## Noise floor: a simplified Lundeby truncation
//!
//! Backward integration of a response with a noise floor flattens the
//! curve's end (the floor integrates to a straight line in energy, a
//! knee in dB), so the integration is truncated where the decay meets the
//! floor and the energy the decay would have had beyond that point is
//! added back analytically. The steps, after Lundeby et al. (Acustica 81,
//! 1995), simplified to one window length:
//!
//! 1. Square the response and average it over 10 ms windows (dB).
//! 2. Estimate the floor as the mean energy of the last 10 % of the
//!    response.
//! 3. Fit a line to the windowed envelope from its peak to the last
//!    window still 10 dB above the floor; the crossing point is where that
//!    line meets the floor.
//! 4. Re-estimate the floor from the stretch starting 10 dB of decay past
//!    the crossing (at least the last 10 %), refit, and repeat until the
//!    crossing moves by less than a window (at most 5 times).
//! 5. Integrate backwards from the crossing, adding the fitted line's
//!    tail energy beyond it, `p(tc) / (1 − e^(−k))` with `k` the fitted
//!    per-sample decay rate.
//!
//! A clean response (offline renders, synthetic decays) has no floor. Then
//! step 2 measures the response's own last 10 %, the crossing lands near
//! its end, and step 5 replaces the last few percent with their own
//! extrapolation: the curve stays exact, and the usual "truncated at the
//! end of the buffer" bias of plain Schroeder integration goes away.
//! Exactly silent windows (a gap after a sparse early reflection, or a
//! flush-to-zero tail) are left out of the fit, so they cannot pull the
//! line down.

/// Energy of a silent window, so dB stays finite (−300 dB).
const ENERGY_FLOOR: f64 = 1e-30;
/// Envelope window for the noise-floor search, seconds.
const ENVELOPE_WINDOW_S: f64 = 0.010;
/// Onset threshold below the peak sample, dB.
const ONSET_DB: f64 = -20.0;

/// A backward-integrated energy decay curve, normalised to 0 dB at the
/// onset.
#[derive(Debug, Clone, PartialEq)]
pub struct Edc {
    /// Curve in dB, one value per sample from [`Edc::onset`] on (index 0
    /// is the onset). Beyond [`Edc::truncation`] it is the extrapolated
    /// line, not measured energy.
    pub db: Vec<f32>,
    pub sample_rate: f32,
    /// Onset in samples from the start of the analysed buffer.
    pub onset: usize,
    /// Truncation point in samples from the onset: where the decay meets
    /// the noise floor, or the end of the buffer.
    pub truncation: usize,
    /// Estimated noise floor, dB relative to the loudest 10 ms window of
    /// the response's energy envelope (around `−300` for a buffer whose
    /// tail is exact zeros).
    pub noise_floor_db: f32,
}

/// EDT, T20 and T30 of one response or band, seconds. `None` when the
/// curve does not reach the end of the fit range above the noise floor
/// (for T30: −35 dB), or the response is silent.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DecayTimes {
    /// Early decay time: fit over 0 … −10 dB, extrapolated to 60 dB.
    pub edt: Option<f32>,
    /// Fit over −5 … −25 dB.
    pub t20: Option<f32>,
    /// Fit over −5 … −35 dB.
    pub t30: Option<f32>,
}

impl Edc {
    /// Time to decay 60 dB, from a least-squares line fitted to the curve
    /// between its first crossings of `start_db` and `end_db` (both ≤ 0,
    /// `end_db < start_db`). `None` if the curve does not reach `end_db`
    /// before the truncation point, or does not fall.
    pub fn decay_time(&self, start_db: f32, end_db: f32) -> Option<f32> {
        let i0 = self.db.iter().position(|&d| d <= start_db)?;
        let i1 = i0 + self.db[i0..].iter().position(|&d| d <= end_db)?;
        if i1 > self.truncation || i1 <= i0 + 1 {
            return None;
        }
        let slope_per_sample = fit_slope(&self.db[i0..=i1]);
        if slope_per_sample >= 0.0 {
            return None;
        }
        Some((-60.0 / (slope_per_sample * self.sample_rate as f64)) as f32)
    }

    /// Fit 0 … −10 dB.
    pub fn edt(&self) -> Option<f32> {
        self.decay_time(0.0, -10.0)
    }

    /// Fit −5 … −25 dB.
    pub fn t20(&self) -> Option<f32> {
        self.decay_time(-5.0, -25.0)
    }

    /// Fit −5 … −35 dB.
    pub fn t30(&self) -> Option<f32> {
        self.decay_time(-5.0, -35.0)
    }

    /// All three.
    pub fn times(&self) -> DecayTimes {
        DecayTimes {
            edt: self.edt(),
            t20: self.t20(),
            t30: self.t30(),
        }
    }
}

/// Least-squares slope of `y` against its index.
fn fit_slope(y: &[f32]) -> f64 {
    let n = y.len() as f64;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = y.iter().map(|&v| v as f64).sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, &v) in y.iter().enumerate() {
        let dx = i as f64 - mean_x;
        sxy += dx * (v as f64 - mean_y);
        sxx += dx * dx;
    }
    sxy / sxx
}

/// Least-squares line `y = a + b·x` through `(x, y)` points.
fn fit_line(pts: &[(f64, f64)]) -> Option<(f64, f64)> {
    if pts.len() < 2 {
        return None;
    }
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for &(x, y) in pts {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
    }
    if sxx == 0.0 {
        return None;
    }
    let b = sxy / sxx;
    Some((my - b * mx, b))
}

fn db(e: f64) -> f64 {
    10.0 * e.max(ENERGY_FLOOR).log10()
}

/// Energy decay curve of a mono response.
pub fn energy_decay_curve(ir: &[f32], sample_rate: f32) -> Edc {
    let energy: Vec<f64> = ir.iter().map(|&x| (x as f64) * (x as f64)).collect();
    edc_from_energy(&energy, sample_rate)
}

/// Energy decay curve of an instantaneous-energy sequence (for a stereo
/// response, `L² + R²` per sample). See the module docs for the method.
pub fn edc_from_energy(energy: &[f64], sample_rate: f32) -> Edc {
    let silent = Edc {
        db: Vec::new(),
        sample_rate,
        onset: 0,
        truncation: 0,
        noise_floor_db: db(0.0) as f32,
    };
    let peak = energy.iter().copied().fold(0.0_f64, f64::max);
    if peak <= 0.0 || !peak.is_finite() {
        return silent;
    }
    let threshold = peak * 10f64.powf(ONSET_DB / 10.0);
    let onset = energy.iter().position(|&e| e >= threshold).unwrap_or(0);
    let e = &energy[onset..];
    let n = e.len();

    let Lundeby {
        truncation,
        tail,
        noise_db,
        rate_db_per_s,
        peak_db,
    } = lundeby(e, sample_rate as f64);

    // Backward integration over [0, truncation), plus the analytic tail.
    let mut db_curve = vec![0.0f32; n];
    let mut acc = tail;
    let mut cum = vec![0.0f64; truncation];
    for i in (0..truncation).rev() {
        acc += e[i];
        cum[i] = acc;
    }
    let total = if truncation > 0 { cum[0] } else { tail };
    if total.is_nan() || total <= 0.0 {
        return silent;
    }
    for i in 0..truncation {
        db_curve[i] = (db(cum[i]) - db(total)) as f32;
    }
    // Beyond the truncation: the extrapolated tail keeps falling at the
    // fitted rate, so the curve stays monotonic and finite.
    if truncation < n {
        let base = db(tail.max(ENERGY_FLOOR)) - db(total);
        let rate_db = rate_db_per_s / sample_rate as f64;
        for (k, slot) in db_curve[truncation..].iter_mut().enumerate() {
            *slot = (base + rate_db * k as f64) as f32;
        }
    }

    Edc {
        db: db_curve,
        sample_rate,
        onset,
        truncation,
        noise_floor_db: (noise_db - peak_db) as f32,
    }
}

/// Windowed envelope in dB: `(centre time s, level dB)` per window.
fn envelope(e: &[f64], win: usize, sr: f64) -> Vec<(f64, f64)> {
    e.chunks(win)
        .enumerate()
        .map(|(k, c)| {
            let mean = c.iter().sum::<f64>() / c.len() as f64;
            let t = (k * win) as f64 / sr + c.len() as f64 / (2.0 * sr);
            (t, db(mean))
        })
        .collect()
}

fn mean_db(e: &[f64]) -> f64 {
    if e.is_empty() {
        return db(0.0);
    }
    db(e.iter().sum::<f64>() / e.len() as f64)
}

/// Fit the envelope from its peak to the last window still 10 dB above
/// `noise_db`. Silent windows (exact zeros: the gap between a sparse
/// early reflection and the tank, or a flushed-to-zero tail) carry no
/// level and are left out of the fit. Returns `(a, b)` in dB and dB/s.
fn fit_to_floor(env: &[(f64, f64)], noise_db: f64) -> Option<(f64, f64)> {
    let peak_idx = env
        .iter()
        .enumerate()
        .max_by(|a, b| a.1 .1.total_cmp(&b.1 .1))
        .map(|(i, _)| i)?;
    let stop = env
        .iter()
        .rposition(|&(_, l)| l >= noise_db + 10.0)
        .map_or(peak_idx, |p| p + 1)
        .max(peak_idx);
    let silent = db(0.0) + 1.0;
    let pts: Vec<(f64, f64)> = env[peak_idx..stop]
        .iter()
        .copied()
        .filter(|&(_, l)| l > silent)
        .collect();
    fit_line(&pts)
}

struct Lundeby {
    /// Samples from the onset.
    truncation: usize,
    /// Energy of the fitted line beyond the truncation point.
    tail: f64,
    /// Noise floor, dB (absolute, not normalised).
    noise_db: f64,
    /// Slope of the fitted line, dB/s (negative).
    rate_db_per_s: f64,
    /// Loudest envelope window, dB (absolute).
    peak_db: f64,
}

fn lundeby(e: &[f64], sr: f64) -> Lundeby {
    let n = e.len();
    let win = ((ENVELOPE_WINDOW_S * sr).round() as usize).max(1);
    let env = envelope(e, win, sr);
    let peak_db = env.iter().map(|w| w.1).fold(f64::NEG_INFINITY, f64::max);
    let last_tenth = n - (n / 10).max(1).min(n);
    let mut noise_db = mean_db(&e[last_tenth..]);
    let mut line = fit_to_floor(&env, noise_db);
    let mut cross_s = f64::INFINITY;

    for _ in 0..5 {
        let Some((a, b)) = line else { break };
        if b >= 0.0 {
            line = None;
            break;
        }
        let new_cross = (noise_db - a) / b;
        let moved = (new_cross - cross_s).abs();
        cross_s = new_cross;
        if moved < ENVELOPE_WINDOW_S {
            break;
        }
        // Noise from 10 dB of decay past the crossing, at least the last
        // tenth of the response.
        let start_s = cross_s + 10.0 / -b;
        let start = ((start_s * sr).max(0.0) as usize).min(last_tenth);
        noise_db = mean_db(&e[start..]);
        line = fit_to_floor(&env, noise_db);
    }

    let Some((a, b)) = line.filter(|&(_, b)| b < 0.0) else {
        return Lundeby {
            truncation: n,
            tail: 0.0,
            noise_db,
            rate_db_per_s: -60.0,
            peak_db,
        };
    };
    let cross_s = (noise_db - a) / b;
    let truncation = ((cross_s * sr).max(1.0) as usize).min(n);
    let t = truncation as f64 / sr;
    let p = 10f64.powf((a + b * t) / 10.0);
    // b dB/s → per-sample energy ratio r = 10^(b / (10·sr)); Σ p·r^k = p/(1−r).
    let r = 10f64.powf(b / (10.0 * sr));
    let tail = if r < 1.0 { p / (1.0 - r) } else { 0.0 };
    Lundeby {
        truncation,
        tail,
        noise_db,
        rate_db_per_s: b,
        peak_db,
    }
}
