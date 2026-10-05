//! Late-tail stereo figures: interaural cross-correlation and mono fold.
//!
//! Both are computed over the response from `start_s` (default 80 ms, the
//! ISO 3382 boundary between early and late energy) to the end of the
//! buffer, in seconds from its start.

/// Default start of the late part, seconds.
pub const LATE_START_S: f32 = 0.080;
/// IACC lag range, seconds (±1 ms, as ISO 3382-1).
pub const IACC_MAX_LAG_S: f32 = 0.001;
/// Lower bound of [`mono_fold_db`], for an L/R pair that cancels.
pub const MONO_FOLD_FLOOR_DB: f32 = -60.0;

fn late(l: &[f32], r: &[f32], sample_rate: f32, start_s: f32) -> Option<(usize, usize)> {
    let n = l.len().min(r.len());
    let start = (start_s * sample_rate).round() as usize;
    (start < n).then_some((start, n))
}

/// Late IACC: the largest |normalised cross-correlation| of L and R over
/// lags of ±1 ms, from `start_s`. 1 = identical (or one a delayed copy of
/// the other), ~0 = decorrelated. `None` if either side is silent there.
pub fn late_iacc(l: &[f32], r: &[f32], sample_rate: f32, start_s: f32) -> Option<f32> {
    let (start, n) = late(l, r, sample_rate, start_s)?;
    let (l, r) = (&l[start..n], &r[start..n]);
    let el: f64 = l.iter().map(|&x| (x as f64) * (x as f64)).sum();
    let er: f64 = r.iter().map(|&x| (x as f64) * (x as f64)).sum();
    if el <= 0.0 || er <= 0.0 || el.is_nan() || er.is_nan() {
        return None;
    }
    let norm = (el * er).sqrt();
    let max_lag = (IACC_MAX_LAG_S * sample_rate).round() as isize;
    let len = l.len() as isize;
    let mut best = 0.0f64;
    for lag in -max_lag..=max_lag {
        let mut acc = 0.0f64;
        let (i0, i1) = (0.max(-lag), len.min(len - lag));
        for i in i0..i1 {
            acc += l[i as usize] as f64 * r[(i + lag) as usize] as f64;
        }
        best = best.max((acc / norm).abs());
    }
    Some(best as f32)
}

/// Mono fold of the late tail: energy of `(L+R)/2` against the mean of
/// the L and R energies, dB. 0 dB = identical channels (nothing lost),
/// −3 dB = uncorrelated, toward [`MONO_FOLD_FLOOR_DB`] = anti-phase. `None`
/// if the late part is silent.
pub fn mono_fold_db(l: &[f32], r: &[f32], sample_rate: f32, start_s: f32) -> Option<f32> {
    let (start, n) = late(l, r, sample_rate, start_s)?;
    let (mut em, mut es) = (0.0f64, 0.0f64);
    for i in start..n {
        let (a, b) = (l[i] as f64, r[i] as f64);
        em += 0.25 * (a + b) * (a + b);
        es += 0.5 * (a * a + b * b);
    }
    if es <= 0.0 || es.is_nan() {
        return None;
    }
    Some(((10.0 * (em / es).max(1e-30).log10()) as f32).max(MONO_FOLD_FLOOR_DB))
}
