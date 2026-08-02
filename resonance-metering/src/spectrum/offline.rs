//! Whole-buffer (offline) power-spectrum analysis.
//!
//! The realtime path in [`ring`][super::ring] / [`fft_worker`][super::fft_worker]
//! is a lock-free SPSC ring fed by the audio thread and drained by a worker
//! thread; it can only ever see the audio that is streaming past it. Offline
//! mix analysis instead has the whole rendered buffer in hand, so this module
//! offers a plain Welch-style entry point next to the realtime one: the same
//! [`FFT_SIZE`] Hann-windowed frames at the same 50 % overlap, but averaged
//! over the entire buffer and reported as **linear power** rather than the
//! peak-held dB bars the UI wants.
//!
//! Linear power (not dB) is the point: energy shares between frequency bands
//! are sums of `|X[k]|²`, and the realtime
//! [`OctaveTable`][super::octave::OctaveTable] aggregates by taking the *max*
//! of dB magnitudes per band, which deliberately throws that energy away.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use super::{FFT_SIZE, HOP_SIZE};

/// Averaged single-sided power spectrum of a mono buffer.
///
/// Bin `k` is centred at `k * sample_rate / FFT_SIZE` Hz. Values are mean
/// `|X[k]|²` over all analysis frames, in arbitrary (window-dependent) units
/// — only *ratios* between bins are meaningful, which is all the band-share
/// maths needs.
#[derive(Debug, Clone)]
pub struct PowerSpectrum {
    /// One entry per positive-frequency FFT bin (`FFT_SIZE / 2` of them).
    pub power: Vec<f64>,
    /// Sample rate the analysis ran at, in Hz.
    pub sample_rate: f32,
    /// Number of `FFT_SIZE` frames averaged. Zero only for an empty input.
    pub frames: usize,
}

impl PowerSpectrum {
    /// Width of one FFT bin in Hz.
    pub fn bin_hz(&self) -> f32 {
        self.sample_rate / FFT_SIZE as f32
    }

    /// Centre frequency of bin `k` in Hz.
    pub fn center_hz(&self, k: usize) -> f32 {
        k as f32 * self.bin_hz()
    }

    /// Total power in `[lo_hz, hi_hz)`, assigning every bin to the band its
    /// centre frequency falls in. DC (bin 0) is never counted: it carries no
    /// audible energy and a DC offset would otherwise swamp the low band.
    pub fn band_power(&self, lo_hz: f32, hi_hz: f32) -> f64 {
        let bin_hz = self.bin_hz();
        if self.power.is_empty()
            || !lo_hz.is_finite()
            || !hi_hz.is_finite()
            || hi_hz <= lo_hz
            || !(bin_hz.is_finite() && bin_hz > 0.0)
        {
            return 0.0;
        }
        // Bin k is in the band when lo <= k*bin_hz < hi.
        let k_lo = (lo_hz / bin_hz).ceil().max(1.0) as usize;
        let k_hi = (hi_hz / bin_hz).ceil().max(0.0) as usize;
        let k_hi = k_hi.min(self.power.len());
        if k_lo >= k_hi {
            return 0.0;
        }
        self.power[k_lo..k_hi].iter().sum()
    }
}

/// Welch-style whole-buffer analysis of a mono signal.
///
/// Runs `FFT_SIZE`-point Hann-windowed FFTs at 50 % overlap and averages the
/// single-sided power spectra. A buffer shorter than one frame is analysed as
/// a single zero-padded frame; the samples after the last whole frame are
/// dropped, as usual for Welch averaging.
///
/// Allocates its scratch buffers and the FFT plan once per call and nothing
/// inside the frame loop, so it is cheap enough to run on a rendered mix but
/// must **not** be called from the audio thread.
pub fn analyze_mono(sample_rate: f32, mono: &[f32]) -> PowerSpectrum {
    let half = FFT_SIZE / 2;
    let mut spectrum = PowerSpectrum {
        power: vec![0.0; half],
        sample_rate,
        frames: 0,
    };
    if mono.is_empty() || !sample_rate.is_finite() || sample_rate <= 0.0 {
        return spectrum;
    }

    let mut window = vec![0.0_f32; FFT_SIZE];
    resonance_dsp::fill_hann_window(&mut window);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut scratch = vec![Complex::new(0.0_f32, 0.0_f32); FFT_SIZE];

    let mut start = 0usize;
    loop {
        let available = mono.len().saturating_sub(start);
        if available == 0 || (spectrum.frames > 0 && available < FFT_SIZE) {
            break;
        }
        let take = available.min(FFT_SIZE);
        for i in 0..take {
            scratch[i] = Complex::new(mono[start + i] * window[i], 0.0);
        }
        // Zero-pad the (only possible) short frame.
        for slot in scratch.iter_mut().take(FFT_SIZE).skip(take) {
            *slot = Complex::new(0.0, 0.0);
        }
        fft.process(&mut scratch[..]);
        for (k, acc) in spectrum.power.iter_mut().enumerate() {
            let re = scratch[k].re as f64;
            let im = scratch[k].im as f64;
            *acc += re * re + im * im;
        }
        spectrum.frames += 1;
        start += HOP_SIZE;
    }

    if spectrum.frames > 1 {
        let inv = 1.0 / spectrum.frames as f64;
        for acc in spectrum.power.iter_mut() {
            *acc *= inv;
        }
    }
    spectrum
}
