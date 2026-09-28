//! The harmonic probe: a sine through the current settings, measured
//! per harmonic order.
//!
//! This is the §7.3 measurement at plugin scope — the harmonic bars in
//! the editor are drawn from it, the factory presets are voiced against
//! it (§2.1's THD targets are defined on exactly this stimulus: a 1 kHz
//! sine at −18 dBFS peak), and `tests/harmonics.rs` pins every mode's
//! signature with it. One function for all three, so the numbers a user
//! reads off the bars are the numbers the tests hold.
//!
//! # What it measures
//!
//! A fresh [`ColorDsp`] at [`PROBE_SAMPLE_RATE`] renders the tone with the
//! given settings, except for the two time-variant stages:
//!
//! - **auto-gain** is forced off: it is a slow linear gain, it changes no
//!   dBc ratio, and its ride would smear the bins;
//! - **flutter** is forced to 0: it is a pitch modulation, and its
//!   sidebands are not harmonics.
//!
//! `mix` stays as set (a parallel blend really does lower the THD), and
//! so does everything else. The tone sits exactly on a bin of the
//! measurement window (coherent sampling), so each harmonic is one
//! single-bin DFT with no window and no leakage.
//!
//! [`probe`] allocates (a fresh DSP's delay lines and the render
//! buffers): call it from a test, never from the audio thread. A caller
//! that probes repeatedly — the editor's probe worker — holds a
//! [`Prober`], which builds its DSP and buffers once and resets the DSP
//! per probe; the result is bit-identical to a fresh [`probe`].

use std::f64::consts::TAU;

use crate::dsp::{ColorDsp, Settings};

/// The probe always runs at 48 kHz, where a 1 kHz tone is exactly 48
/// samples per cycle, whatever rate the host runs at.
pub const PROBE_SAMPLE_RATE: f32 = 48_000.0;
/// §2.1's probe tone.
pub const PROBE_FREQ_HZ: f64 = 1_000.0;
/// §2.1's probe level, dBFS peak.
pub const PROBE_LEVEL_DBFS: f32 = -18.0;
/// Highest order measured (§7.3 reports `h[2..9]`).
pub const MAX_ORDER: usize = 9;
/// Orders the editor draws as bars.
pub const BAR_ORDERS: usize = 7;

/// Settle time before the measurement window: long enough for the DC
/// blocker (5 Hz), the head bump and the HF-loss envelope to settle.
pub const SETTLE_SAMPLES: usize = 14_400;
/// 0.1 s at 48 kHz: 100 cycles of 1 kHz, 10 Hz bins.
pub const WINDOW_SAMPLES: usize = 4_800;
const BLOCK: usize = 480;

/// One probe result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HarmonicSignature {
    /// `h_dbc[k]` is harmonic `k` in dB relative to the fundamental,
    /// for `k = 1..=MAX_ORDER` (`h_dbc[1] = 0`, `h_dbc[0]` unused).
    /// Orders above Nyquist read −300.
    pub h_dbc: [f64; MAX_ORDER + 1],
    /// Total harmonic distortion over orders 2..=MAX_ORDER, in percent.
    pub thd_pct: f64,
    /// The fundamental's gain through the chain, in dB.
    pub gain_db: f64,
}

impl HarmonicSignature {
    /// H2 relative to H3 in dB: positive means even-dominant (§2.1's
    /// "warm" criterion).
    pub fn h2_h3_db(&self) -> f64 {
        self.h_dbc[2] - self.h_dbc[3]
    }

    /// Least-squares slope, in dB per order (negative = falling), of the
    /// harmonics among `orders` that sit above `floor_dbc`. `None` when
    /// fewer than two do.
    pub fn decay_db_per_order(&self, orders: &[usize], floor_dbc: f64) -> Option<f64> {
        let pts: Vec<(f64, f64)> = orders
            .iter()
            .copied()
            .filter(|&k| (2..=MAX_ORDER).contains(&k) && self.h_dbc[k] > floor_dbc)
            .map(|k| (k as f64, self.h_dbc[k]))
            .collect();
        if pts.len() < 2 {
            return None;
        }
        let n = pts.len() as f64;
        let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
        let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
        let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let sxx: f64 = pts.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
        Some(sxy / sxx)
    }
}

/// The settings the probe actually renders: `s` with the time-variant
/// stages switched off (see the module docs).
pub fn probe_settings(s: &Settings) -> Settings {
    Settings {
        auto_gain: false,
        flutter: 0.0,
        ..*s
    }
}

/// Render `freq` Hz at `level_dbfs` peak through `settings` (as given,
/// no overrides) at `sample_rate`, mono into both channels, returning
/// the left channel: `settle` samples of lead-in, then `window` samples.
pub fn render_tone(
    settings: &Settings,
    sample_rate: f32,
    freq: f64,
    level_dbfs: f32,
    settle: usize,
    window: usize,
) -> Vec<f32> {
    let mut dsp = ColorDsp::new(sample_rate, settings);
    let mut out = Vec::with_capacity(settle + window);
    let mut scratch = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    render_into(&mut dsp, settings, freq, level_dbfs, settle, window, &mut scratch, &mut out);
    out
}

/// The body of [`render_tone`] on a DSP the caller owns (already reset to
/// `settings`), into `out` (cleared first; left channel, window only).
#[allow(clippy::too_many_arguments)]
fn render_into(
    dsp: &mut ColorDsp,
    settings: &Settings,
    freq: f64,
    level_dbfs: f32,
    settle: usize,
    window: usize,
    (l, r): &mut ([f32; BLOCK], [f32; BLOCK]),
    out: &mut Vec<f32>,
) {
    let sample_rate = dsp.sample_rate();
    let amp = 10f64.powf(level_dbfs as f64 / 20.0);
    let total = settle + window;
    out.clear();
    let mut n = 0usize;
    while n < total {
        let frames = BLOCK.min(total - n);
        for i in 0..frames {
            let t = (n + i) as f64 / sample_rate as f64;
            let v = (amp * (TAU * freq * t).sin()) as f32;
            l[i] = v;
            r[i] = v;
        }
        dsp.process(&mut l[..frames], &mut r[..frames], settings, None);
        // Keep only the measurement window.
        let keep_from = settle.saturating_sub(n).min(frames);
        out.extend_from_slice(&l[keep_from..frames]);
        n += frames;
    }
}

/// Amplitude of the component at `freq` in `x` (a single-bin DFT; exact
/// for a coherent tone).
pub fn bin_amplitude(x: &[f32], sample_rate: f32, freq: f64) -> f64 {
    let w = TAU * freq / sample_rate as f64;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &s) in x.iter().enumerate() {
        let ph = w * i as f64;
        re += s as f64 * ph.cos();
        im += s as f64 * ph.sin();
    }
    2.0 * (re * re + im * im).sqrt() / x.len() as f64
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-30).log10()
}

/// The harmonic signature of `settings` for a 1 kHz sine at `level_dbfs`
/// peak (§2.1 uses −18, [`PROBE_LEVEL_DBFS`]).
pub fn probe(settings: &Settings, level_dbfs: f32) -> HarmonicSignature {
    Prober::new().probe(settings, level_dbfs)
}

/// A reusable probe: one [`ColorDsp`] at [`PROBE_SAMPLE_RATE`] and the
/// render buffers, built once ([`Prober::new`] allocates) and reused by
/// every [`Prober::probe`], which only resets the DSP. Same numbers as
/// [`probe`], bit for bit (`tests/harmonics.rs`).
pub struct Prober {
    dsp: ColorDsp,
    scratch: Box<([f32; BLOCK], [f32; BLOCK])>,
    window: Vec<f32>,
}

impl Default for Prober {
    fn default() -> Self {
        Self::new()
    }
}

impl Prober {
    pub fn new() -> Self {
        Self {
            dsp: ColorDsp::new(PROBE_SAMPLE_RATE, &Settings::default()),
            scratch: Box::new(([0.0; BLOCK], [0.0; BLOCK])),
            window: Vec::with_capacity(WINDOW_SAMPLES + BLOCK),
        }
    }

    /// The last probe's measurement window (left channel, after the
    /// settle), as [`render_tone`] would render it.
    pub fn window(&self) -> &[f32] {
        &self.window
    }

    /// The harmonic signature of `settings` (see [`probe`]).
    pub fn probe(&mut self, settings: &Settings, level_dbfs: f32) -> HarmonicSignature {
        let s = probe_settings(settings);
        let sr = PROBE_SAMPLE_RATE;
        self.dsp.reset(&s);
        render_into(
            &mut self.dsp,
            &s,
            PROBE_FREQ_HZ,
            level_dbfs,
            SETTLE_SAMPLES,
            WINDOW_SAMPLES,
            &mut self.scratch,
            &mut self.window,
        );
        let x = &self.window;
        let amp = 10f64.powf(level_dbfs as f64 / 20.0);
        let fund = bin_amplitude(x, sr, PROBE_FREQ_HZ);
        let mut h_dbc = [0.0f64; MAX_ORDER + 1];
        let mut sum_sq = 0.0f64;
        for (k, slot) in h_dbc.iter_mut().enumerate().skip(1) {
            let f = PROBE_FREQ_HZ * k as f64;
            if f >= 0.5 * sr as f64 {
                *slot = -300.0;
                continue;
            }
            let a = bin_amplitude(x, sr, f);
            *slot = db(a / fund.max(1e-30));
            if k >= 2 {
                sum_sq += a * a;
            }
        }
        h_dbc[1] = 0.0;
        HarmonicSignature {
            h_dbc,
            thd_pct: 100.0 * sum_sq.sqrt() / fund.max(1e-30),
            gain_db: db(fund / amp),
        }
    }
}
