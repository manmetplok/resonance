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

/// FFT length of the detailed-analysis pass ([`analyze_stereo`]).
///
/// Four times the realtime [`FFT_SIZE`]: at 48 kHz a bin is 1.46 Hz wide,
/// which is what lets a 1/3-octave band at 20 Hz (4.6 Hz wide) or a
/// 1/6-octave band at 20 Hz (2.3 Hz) contain real bins instead of reading
/// one leaky bin. A frame is 0.68 s at 48 kHz, so any range worth
/// measuring averages many of them.
pub const DETAIL_FFT_SIZE: usize = 32_768;

/// Averaged single-sided auto- and cross-power spectra of a stereo pair,
/// scaled to **mean-square units**: summing [`ll`](Self::ll) over every
/// bin gives the mean square of the left channel (DC excluded), so a band
/// sum is that band's power in absolute terms.
///
/// Produced by [`analyze_stereo`]. Bin `k` is centred at
/// `k * sample_rate / fft_size` Hz and is treated as covering
/// `[k - 0.5, k + 0.5) * bin_hz`; [`band`](Self::band) integrates with
/// fractional bin overlap, so a band narrower than a bin still reads the
/// density of the bin it sits in rather than zero.
#[derive(Debug, Clone)]
pub struct StereoSpectrum {
    /// Left auto-power per bin.
    pub ll: Vec<f64>,
    /// Right auto-power per bin.
    pub rr: Vec<f64>,
    /// Real part of the L·conj(R) cross-power per bin. Summed over a band
    /// it is that band's `E[L·R]`, the numerator of its correlation.
    pub lr: Vec<f64>,
    /// Sample rate the analysis ran at, in Hz.
    pub sample_rate: f32,
    /// FFT length the bins came from.
    pub fft_size: usize,
    /// Number of frames averaged. Zero only for an empty input.
    pub frames: usize,
}

/// Which spectrum [`StereoSpectrum::band`] integrates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Left auto-power.
    Left,
    /// Right auto-power.
    Right,
    /// Mean of the two auto-powers — the LTAS of the stereo signal,
    /// which (unlike the mono sum) keeps anti-phase side content.
    Both,
    /// Real cross-power L·R.
    Cross,
}

impl StereoSpectrum {
    /// Width of one FFT bin in Hz.
    pub fn bin_hz(&self) -> f64 {
        self.sample_rate as f64 / self.fft_size as f64
    }

    /// Centre frequency of bin `k` in Hz.
    pub fn center_hz(&self, k: usize) -> f64 {
        k as f64 * self.bin_hz()
    }

    /// Bin `k` of `which`.
    pub fn value(&self, which: Channel, k: usize) -> f64 {
        match which {
            Channel::Left => self.ll[k],
            Channel::Right => self.rr[k],
            Channel::Both => 0.5 * (self.ll[k] + self.rr[k]),
            Channel::Cross => self.lr[k],
        }
    }

    /// Power of `which` in `[lo_hz, hi_hz)`, integrating each bin by the
    /// fraction of its width that falls inside the band. DC (bin 0) is
    /// never counted.
    pub fn band(&self, which: Channel, lo_hz: f64, hi_hz: f64) -> f64 {
        let bin = self.bin_hz();
        if self.ll.is_empty() || !(bin > 0.0) || !(hi_hz > lo_hz) {
            return 0.0;
        }
        let first = (lo_hz / bin - 0.5).floor().max(1.0) as usize;
        let last = ((hi_hz / bin + 0.5).ceil().max(0.0) as usize).min(self.ll.len());
        let mut sum = 0.0;
        for k in first..last {
            let b_lo = (k as f64 - 0.5) * bin;
            let b_hi = (k as f64 + 0.5) * bin;
            let overlap = (b_hi.min(hi_hz) - b_lo.max(lo_hz)).max(0.0) / bin;
            if overlap > 0.0 {
                sum += overlap * self.value(which, k);
            }
        }
        sum
    }
}

/// Welch-style whole-buffer analysis of a stereo pair: `fft_size`-point
/// Hann frames at 50 % overlap, averaged, scaled to mean-square units
/// (see [`StereoSpectrum`]).
///
/// A buffer shorter than one frame is analysed as one frame with a Hann
/// window of its own length, zero-padded, so its level still reads right.
/// Analyses `min(left.len(), right.len())` samples. Allocates; never call
/// it from the audio thread.
pub fn analyze_stereo(
    sample_rate: f32,
    left: &[f32],
    right: &[f32],
    fft_size: usize,
) -> StereoSpectrum {
    let half = fft_size / 2;
    let mut out = StereoSpectrum {
        ll: vec![0.0; half],
        rr: vec![0.0; half],
        lr: vec![0.0; half],
        sample_rate,
        fft_size,
        frames: 0,
    };
    let n = left.len().min(right.len());
    if n < 2 || fft_size < 2 || !sample_rate.is_finite() || sample_rate <= 0.0 {
        return out;
    }

    let win_len = n.min(fft_size);
    let mut window = vec![0.0_f32; win_len];
    resonance_dsp::fill_hann_window(&mut window);
    let win_sq: f64 = window.iter().map(|&w| (w as f64) * (w as f64)).sum();
    if win_sq <= 0.0 {
        return out;
    }
    // Parseval, single-sided: every non-DC bin carries both halves.
    let scale = 2.0 / (fft_size as f64 * win_sq);

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_size);
    let zero = Complex::new(0.0_f32, 0.0_f32);
    let mut xl = vec![zero; fft_size];
    let mut xr = vec![zero; fft_size];
    let hop = (win_len / 2).max(1);

    let mut start = 0usize;
    while start + win_len <= n {
        for i in 0..fft_size {
            if i < win_len {
                let w = window[i];
                xl[i] = Complex::new(left[start + i] * w, 0.0);
                xr[i] = Complex::new(right[start + i] * w, 0.0);
            } else {
                xl[i] = zero;
                xr[i] = zero;
            }
        }
        fft.process(&mut xl);
        fft.process(&mut xr);
        for k in 1..half {
            let (lre, lim) = (xl[k].re as f64, xl[k].im as f64);
            let (rre, rim) = (xr[k].re as f64, xr[k].im as f64);
            out.ll[k] += lre * lre + lim * lim;
            out.rr[k] += rre * rre + rim * rim;
            out.lr[k] += lre * rre + lim * rim;
        }
        out.frames += 1;
        start += hop;
    }

    let inv = scale / out.frames.max(1) as f64;
    for k in 0..half {
        out.ll[k] *= inv;
        out.rr[k] *= inv;
        out.lr[k] *= inv;
    }
    out
}

/// FFT size of [`sixth_octave_ltas`]: a balance of resolution and number
/// of averages for a ~10 s buffer.
pub const LTAS_FFT_SIZE: usize = 4096;
/// Hop of [`sixth_octave_ltas`] (50 % overlap).
pub const LTAS_HOP: usize = LTAS_FFT_SIZE / 2;
/// Level [`sixth_octave_ltas`] reports for silence (or a buffer shorter
/// than one frame), dB.
pub const LTAS_FLOOR_DB: f32 = -120.0;

/// The mastering assistant's long-term average spectrum: a Welch average
/// of the mono sum `(L + R) / 2`, [`LTAS_FFT_SIZE`]-point Hann frames at
/// 50 % overlap, averaged in power and aggregated to the
/// [`NUM_OCTAVE_BINS`][super::NUM_OCTAVE_BINS] 1/6-octave bands of
/// [`OctaveTable`][super::octave::OctaveTable] (so each band carries the
/// loudest FFT bin inside it — a per-bin density, where pink noise slopes
/// −3 dB/oct).
///
/// One definition for both callers: the plugin's assistant on its captured
/// buffer and the engine's offline `master.assist` measurement on a
/// rendered range, so the two compare the same thing against the same
/// target bands.
pub fn sixth_octave_ltas(sample_rate: f32, left: &[f32], right: &[f32]) -> Vec<f32> {
    use super::octave::OctaveTable;
    use super::NUM_OCTAVE_BINS;

    let n = left.len().min(right.len());
    if n < LTAS_FFT_SIZE {
        return vec![LTAS_FLOOR_DB; NUM_OCTAVE_BINS];
    }

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(LTAS_FFT_SIZE);

    let window = resonance_dsp::hann_window(LTAS_FFT_SIZE);

    let mut scratch = vec![Complex::new(0.0, 0.0); LTAS_FFT_SIZE];
    let mut power_sum = vec![0.0_f64; LTAS_FFT_SIZE / 2];
    let mut frames = 0_usize;

    let mut start = 0_usize;
    while start + LTAS_FFT_SIZE <= n {
        for i in 0..LTAS_FFT_SIZE {
            let mono = 0.5 * (left[start + i] + right[start + i]) * window[i];
            scratch[i] = Complex::new(mono, 0.0);
        }
        fft.process(&mut scratch);
        let norm = 4.0 / LTAS_FFT_SIZE as f32;
        for k in 0..LTAS_FFT_SIZE / 2 {
            let re = scratch[k].re;
            let im = scratch[k].im;
            let mag = (re * re + im * im).sqrt() * norm;
            power_sum[k] += (mag as f64) * (mag as f64);
        }
        frames += 1;
        start += LTAS_HOP;
    }

    if frames == 0 {
        return vec![LTAS_FLOOR_DB; NUM_OCTAVE_BINS];
    }

    let mut mag_db = vec![LTAS_FLOOR_DB; LTAS_FFT_SIZE / 2];
    for k in 0..LTAS_FFT_SIZE / 2 {
        let avg_power = power_sum[k] / frames as f64;
        let avg_mag = avg_power.sqrt() as f32;
        mag_db[k] = 20.0 * avg_mag.max(1e-10).log10();
    }

    let table = OctaveTable::new();
    let mut out = vec![LTAS_FLOOR_DB; NUM_OCTAVE_BINS];
    table.aggregate(&mag_db, sample_rate, &mut out, LTAS_FLOOR_DB);
    out
}
