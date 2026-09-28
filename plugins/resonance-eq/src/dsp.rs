//! Per-channel cascade state and the top-level EQ process loop.
//!
//! Coefficient updates are pulled from the live `EqParams` once per audio
//! block. Stage state (z1/z2) is preserved across updates so sweeping a
//! band doesn't click. The actual per-sample arithmetic is a simple two
//! channels × 8 bands × up-to-4 stages biquad cascade plus a trailing
//! output gain.
//!
//! A band in `Mid` or `Side` mode filters only that component of the
//! signal: the sample is split into `m = (L+R)/2`, `s = (L-R)/2`, one
//! component runs through the band's (channel-0) stages, and the pair is
//! recombined. A `Stereo` band runs exactly the arithmetic it always did.
//!
//! A **dynamic** band (`dyn_on`) pulls its own gain down while its
//! frequency region is loud: a detector filter matched to the band (a
//! band-pass for a bell, a low-pass under a low shelf or lift, a
//! high-pass over a high shelf or air) feeds a peak detector, a
//! soft-knee gain computer (threshold, ratio) and attack/release
//! ballistics, and the band is re-voiced at `gain - GR` every
//! [`DYN_UPDATE_SAMPLES`] samples. A Stereo band's detector follows
//! both channels and takes the louder, `max(|L|, |R|)` after the filter,
//! so a hard-panned or antiphase source is heard at its real level; a Mid
//! or Side band's detector hears the component it filters. With `dyn_sc`
//! on and a key connected (the plugin's sidechain input), the detector
//! follows the key instead — through the same filter, louder channel —
//! so a band can duck from another source (the vocal's presence region
//! out of the guitars); with no key it falls back to the band's own input.
//! A bell at 0 dB with dynamics on is a
//! pure de-harsh cut that only acts when the region is hot; with a static
//! boost it is a boost that backs off when the region gets loud. Only
//! the kinds [`BandKind::supports_dyn`] names take dynamics (bell,
//! shelves, air); on the cuts, Tilt and LF Lift+Dip the switch is
//! ignored.
//!
//! **Auto-gain** trims the output by the negated [`static_gain_db`] of the
//! band curve. It is a *static* estimate — a function of the parameters
//! only, recomputed when they change — rather than a level follower, so it
//! is deterministic and cannot pump with the programme.

use resonance_dsp::dynamics::{soft_knee_gain_reduction_db, Ballistics};
use resonance_dsp::Biquad;
use resonance_plugin::Smoother;

use crate::band::{configure_stages, BandKind, BandMs, MAX_STAGES_PER_BAND};
use crate::params::{BandSnapshot, DynSnapshot, EqParams, NUM_BANDS};

/// How often a dynamic band is re-voiced, in samples. Recomputing the
/// coefficients per sample would cost a `powf` and a `sin_cos` each; at
/// 16 samples the gain steps are far below audibility.
pub const DYN_UPDATE_SAMPLES: u32 = 16;
/// Largest cut a dynamic band makes, dB.
pub const DYN_MAX_GR_DB: f32 = 24.0;
/// Soft-knee width of the dynamic bands' gain computer, dB.
const DYN_KNEE_DB: f32 = 6.0;
/// Decay of the detector's peak follower, ms — just enough to bridge the
/// gaps between a waveform's peaks, as in the compressor.
const DYN_PEAK_RELEASE_MS: f32 = 5.0;
/// GR change below which a band is not re-voiced, dB.
const DYN_REVOICE_EPS_DB: f32 = 0.01;

/// One band's dynamics state.
#[derive(Clone, Copy)]
struct DynState {
    /// Dynamics on, band enabled, and a kind that supports them.
    active: bool,
    /// Detector filter, matched to the band's kind and frequency, one
    /// per channel (a Mid or Side band uses only the first).
    detector: [Biquad; 2],
    /// Detect on the sidechain key when one is connected.
    sidechain: bool,
    peak_env: f32,
    /// Smoothed gain reduction, dB (>= 0).
    gr_db: f32,
    /// The GR the band's coefficients currently carry.
    applied_gr_db: f32,
    threshold_db: f32,
    slope: f32,
    ballistics: Ballistics,
    last: Option<(DynSnapshot, BandSnapshot)>,
}

impl DynState {
    fn new() -> Self {
        Self {
            active: false,
            detector: [Biquad::identity(); 2],
            sidechain: false,
            peak_env: 0.0,
            gr_db: 0.0,
            applied_gr_db: 0.0,
            threshold_db: 0.0,
            slope: 0.0,
            ballistics: Ballistics::from_times(48_000.0, 10.0, 150.0),
            last: None,
        }
    }

    /// Detector level update for one sample: `a` alone, or the louder of
    /// `a` and `b` (each through its own filter) when `pair`.
    #[inline]
    fn detect(&mut self, a: f32, b: f32, pair: bool, peak_coef: f32) {
        let y0 = self.detector[0].process(a);
        let y1 = if pair { self.detector[1].process(b) } else { 0.0 };
        // Both checked: `max` would hide a NaN in one filter for good.
        let a = if y0.is_finite() && y1.is_finite() {
            y0.abs().max(y1.abs())
        } else {
            for d in &mut self.detector {
                d.reset();
            }
            0.0
        };
        self.peak_env = if a > self.peak_env {
            a
        } else {
            a + (self.peak_env - a) * peak_coef
        };
        let level_db = 20.0 * self.peak_env.max(1e-9).log10();
        let target = soft_knee_gain_reduction_db(
            level_db,
            self.threshold_db,
            DYN_KNEE_DB,
            DYN_KNEE_DB * 0.5,
            self.slope,
        )
        .min(DYN_MAX_GR_DB);
        self.gr_db = self.ballistics.step_envelope(self.gr_db, target);
        if !self.gr_db.is_finite() {
            self.gr_db = 0.0;
        }
    }
}

/// Configure a dynamic band's detector filter for the band it follows.
fn configure_detector(d: &mut Biquad, s: &BandSnapshot, sr: f32) {
    match s.kind {
        BandKind::Bell => d.set_band_pass(sr, s.freq, s.q.max(0.3)),
        BandKind::LowShelf => d.set_low_pass(sr, s.freq, 0.707),
        BandKind::HighShelf | BandKind::Air => d.set_high_pass(sr, s.freq, 0.707),
        // No dynamics on these (`BandKind::supports_dyn`).
        BandKind::LowCut | BandKind::HighCut | BandKind::Tilt | BandKind::LfLiftDip => {
            d.set_identity()
        }
    }
}

pub struct EqDsp {
    sample_rate: f32,
    /// Per-channel cascade: [channel][band][stage].
    channels: [[[Biquad; MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
    /// How many stages of each band are actually in use (same for both channels).
    active_stages: [usize; NUM_BANDS],
    /// Stereo / Mid / Side routing of each band.
    band_ms: [BandMs; NUM_BANDS],
    /// Last-applied snapshots, used to skip coefficient work when nothing changed.
    last_snapshot: [Option<BandSnapshot>; NUM_BANDS],
    /// The stages a band ran *before* its last kind change, still on their
    /// old coefficients and state, crossfaded out while the restarted new
    /// stages fade in (FU-M6c): [channel][band][stage].
    fade_stages: [[[Biquad; MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
    /// Active stage count of `fade_stages` per band.
    fade_active: [usize; NUM_BANDS],
    /// M/S routing the fading-out stages ran with.
    fade_ms: [BandMs; NUM_BANDS],
    /// Samples left in each band's kind-change crossfade; 0 = none running.
    fade_remaining: [u32; NUM_BANDS],
    /// Crossfade length in samples (~5 ms at the current rate).
    fade_len: u32,
    /// Auto-gain trim in dB (0 while auto-gain is off), recomputed only
    /// when a band or the switch changed.
    auto_gain_db: f32,
    /// Whether `auto_gain_db` is up to date for the current snapshots.
    auto_gain_valid: bool,
    auto_gain_on: bool,
    /// Trig of the auto-gain estimate's frequency grid at this sample rate:
    /// `(cos w, sin w, cos 2w, sin 2w)` per point, so a re-estimate is
    /// arithmetic only.
    grid: Vec<[f32; 4]>,
    /// Dynamic-band state, one per band.
    dyn_state: [DynState; NUM_BANDS],
    /// Whether any band is dynamic — the per-sample loop skips all
    /// dynamics work when none is.
    any_dyn: bool,
    /// Samples until the dynamic bands are next re-voiced.
    dyn_countdown: u32,
    /// Peak-follower decay coefficient at this sample rate.
    dyn_peak_coef: f32,
}

/// Length of the crossfade a band runs when its kind changes. Long enough
/// that the old-to-new difference (up to the signal's own size) is spread
/// into a ramp well under the steady tone's own slope; short enough to read
/// as an instant switch.
const KIND_FADE_MS: f32 = 5.0;

/// Frequency grid of the static gain estimate: 1/6-octave from 20 Hz to
/// 20 kHz. Equal weight per point on a log axis is equal weight per
/// octave, i.e. pink-noise weighting.
const GRID_LO_HZ: f32 = 20.0;
const GRID_HI_HZ: f32 = 20_000.0;
const GRID_PER_OCTAVE: f32 = 6.0;

/// Share of programme energy the static estimate assumes is in the mid
/// channel (the rest is side). 0.8 / 0.2 is a side-to-mid ratio of about
/// -6 dB, the middle of the healthy range for a whole mix
/// (warmth-width-depth.md §2.2). Only Mid and Side bands are affected by
/// it: for a Stereo band both channels see the same curve.
const MID_ENERGY_SHARE: f32 = 0.8;

/// Largest trim auto-gain applies, dB.
pub const AUTO_GAIN_LIMIT_DB: f32 = 24.0;

/// The trim auto-gain applies for the given bands, dB: the negated
/// [`static_gain_db`], clamped to [`AUTO_GAIN_LIMIT_DB`].
pub fn auto_gain_trim_db(snapshots: &[BandSnapshot], sr: f32) -> f32 {
    (-static_gain_db(snapshots, sr)).clamp(-AUTO_GAIN_LIMIT_DB, AUTO_GAIN_LIMIT_DB)
}

fn grid(sr: f32) -> Vec<[f32; 4]> {
    let octaves = (GRID_HI_HZ / GRID_LO_HZ).log2();
    let n = (octaves * GRID_PER_OCTAVE).round() as usize + 1;
    (0..n)
        .map(|i| {
            let f = GRID_LO_HZ * 2f32.powf(i as f32 / GRID_PER_OCTAVE);
            let w = 2.0 * std::f32::consts::PI * f.min(sr * 0.499) / sr;
            let (s1, c1) = w.sin_cos();
            let (s2, c2) = (2.0 * w).sin_cos();
            [c1, s1, c2, s2]
        })
        .collect()
}

/// `|H|²` of one biquad at a grid point.
#[inline]
fn mag_sq(b: &Biquad, [c1, s1, c2, s2]: [f32; 4]) -> f32 {
    let nr = b.b0 + b.b1 * c1 + b.b2 * c2;
    let ni = -b.b1 * s1 - b.b2 * s2;
    let dr = 1.0 + b.a1 * c1 + b.a2 * c2;
    let di = -b.a1 * s1 - b.a2 * s2;
    (nr * nr + ni * ni) / (dr * dr + di * di).max(1e-30)
}

fn static_gain_on_grid(snapshots: &[BandSnapshot], sr: f32, grid: &[[f32; 4]]) -> f32 {
    // Fixed-size, so a re-estimate on the audio thread never allocates.
    let mut bands = [(BandMs::Stereo, 0usize, [Biquad::identity(); MAX_STAGES_PER_BAND]); NUM_BANDS];
    for (slot, snap) in bands.iter_mut().zip(snapshots) {
        slot.0 = snap.ms;
        slot.1 = configure_stages(snap, sr, &mut slot.2);
    }
    let mut acc = 0.0f64;
    for &pt in grid {
        let (mut mid, mut side) = (1.0f32, 1.0f32);
        for (ms, n, stages) in &bands {
            let p: f32 = stages[..*n].iter().map(|b| mag_sq(b, pt)).product();
            match ms {
                BandMs::Stereo => {
                    mid *= p;
                    side *= p;
                }
                BandMs::Mid => mid *= p,
                BandMs::Side => side *= p,
            }
        }
        acc += (MID_ENERGY_SHARE * mid + (1.0 - MID_ENERGY_SHARE) * side) as f64;
    }
    let mean = acc / grid.len().max(1) as f64;
    (10.0 * mean.max(1e-12).log10()) as f32
}

/// Static loudness estimate of the band curve, in dB: the power gain of
/// the whole EQ averaged over a 1/6-octave grid from 20 Hz to 20 kHz with
/// equal weight per octave — what the curve does to pink noise.
///
/// Mid and Side bands are weighted by an assumed programme split of
/// [`MID_ENERGY_SHARE`] mid to the rest side. Auto-gain trims the output
/// by the negation of this, so a broad +3 dB shelf reads as roughly
/// level-neutral while a narrow +3 dB bell barely moves the output — the
/// same judgement a loudness-matched A/B makes, from the parameters alone
/// and therefore without any programme-dependent pumping.
pub fn static_gain_db(snapshots: &[BandSnapshot], sr: f32) -> f32 {
    static_gain_on_grid(snapshots, sr, &grid(sr))
}

impl EqDsp {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            channels: [[[Biquad::identity(); MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
            active_stages: [0; NUM_BANDS],
            band_ms: [BandMs::Stereo; NUM_BANDS],
            last_snapshot: [None; NUM_BANDS],
            fade_stages: [[[Biquad::identity(); MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
            fade_active: [0; NUM_BANDS],
            fade_ms: [BandMs::Stereo; NUM_BANDS],
            fade_remaining: [0; NUM_BANDS],
            fade_len: ((KIND_FADE_MS * 0.001 * sample_rate).round() as u32).max(1),
            auto_gain_db: 0.0,
            auto_gain_valid: false,
            auto_gain_on: false,
            grid: grid(sample_rate),
            dyn_state: [DynState::new(); NUM_BANDS],
            any_dyn: false,
            dyn_countdown: 0,
            dyn_peak_coef: (-1.0 / (DYN_PEAK_RELEASE_MS * 0.001 * sample_rate.max(1.0))).exp(),
        }
    }

    pub fn clear_state(&mut self) {
        for ch in self.channels.iter_mut() {
            for band in ch.iter_mut() {
                for stage in band.iter_mut() {
                    stage.reset();
                }
            }
        }
        self.fade_remaining = [0; NUM_BANDS];
        for d in self.dyn_state.iter_mut() {
            for det in &mut d.detector {
                det.reset();
            }
            d.peak_env = 0.0;
            d.gr_db = 0.0;
        }
    }

    /// Current gain reduction of band `band`'s dynamics, dB (0 when the
    /// band is not dynamic).
    pub fn dyn_gain_reduction_db(&self, band: usize) -> f32 {
        self.dyn_state
            .get(band)
            .filter(|d| d.active)
            .map_or(0.0, |d| d.gr_db)
    }

    /// The current auto-gain trim in dB — 0 while auto-gain is off.
    pub fn auto_gain_db(&self) -> f32 {
        self.auto_gain_db
    }

    /// Refresh coefficients from the current parameter values for any band
    /// whose snapshot has changed since the last call. Called once per block.
    pub fn update_from_params(&mut self, params: &EqParams) {
        for (i, band) in params.bands.iter().enumerate() {
            let snapshot = band.snapshot();
            let changed = match self.last_snapshot[i] {
                Some(prev) => !snapshots_equal(&prev, &snapshot),
                None => true,
            };
            if changed {
                self.auto_gain_valid = false;
                // A kind change restarts the band's stages from zero (below),
                // which on loud material is a click. Keep the old stages
                // running as they are and crossfade out of them. Only when
                // they were producing something: from a bypassed band there
                // is nothing to fade from. A second change mid-fade starts
                // over from the current (new) stages. Moving a band between
                // Stereo, Mid and Side repurposes its stages the same way.
                let kind_changed = self.last_snapshot[i]
                    .is_some_and(|p| p.kind != snapshot.kind || p.ms != snapshot.ms);
                let prev_n = self.active_stages[i];
                if kind_changed && prev_n > 0 {
                    for ch in 0..2 {
                        self.fade_stages[ch][i] = self.channels[ch][i];
                    }
                    self.fade_active[i] = prev_n;
                    self.fade_ms[i] = self.band_ms[i];
                    self.fade_remaining[i] = self.fade_len;
                }
                // Write coefficients into both channels (L and R share coeffs
                // but carry independent delay-line state).
                let n = configure_stages(&snapshot, self.sample_rate, &mut self.channels[0][i]);
                let _ = configure_stages(&snapshot, self.sample_rate, &mut self.channels[1][i]);
                // Stages that were not running hold z1/z2 frozen from
                // another input (and maybe other coefficients); TDF-II
                // injects that straight into the output. Start them clean.
                // A kind change repurposes every stage, so clear them all.
                let fresh_from = if kind_changed { 0 } else { prev_n.min(n) };
                for ch in self.channels.iter_mut() {
                    for stage in &mut ch[i][fresh_from..n] {
                        stage.reset();
                    }
                }
                self.active_stages[i] = n;
                self.band_ms[i] = snapshot.ms;
                self.last_snapshot[i] = Some(snapshot);
                // The stages now carry the static gain again.
                self.dyn_state[i].applied_gr_db = 0.0;
            }

            // Dynamics: (re)configure when the band or its dynamics moved.
            let dy = band.dyn_snapshot();
            let d = &mut self.dyn_state[i];
            if d.last != Some((dy, snapshot)) {
                let active = dy.on && snapshot.enabled && snapshot.kind.supports_dyn();
                if active && !d.active {
                    for det in &mut d.detector {
                        det.reset();
                    }
                    d.peak_env = 0.0;
                    d.gr_db = 0.0;
                }
                if !active && d.applied_gr_db != 0.0 {
                    // Back to the static curve.
                    configure_stages(&snapshot, self.sample_rate, &mut self.channels[0][i]);
                    configure_stages(&snapshot, self.sample_rate, &mut self.channels[1][i]);
                    d.applied_gr_db = 0.0;
                }
                d.active = active;
                for det in &mut d.detector {
                    configure_detector(det, &snapshot, self.sample_rate);
                }
                d.sidechain = dy.sidechain;
                d.threshold_db = dy.threshold_db;
                d.slope = 1.0 - 1.0 / dy.ratio.max(1.0);
                d.ballistics = Ballistics::from_times(self.sample_rate, dy.attack_ms, dy.release_ms);
                d.last = Some((dy, snapshot));
            }
            // A changed band was just re-voiced at its static gain. If it is
            // cutting, put the cut back now: waiting for the next revoice
            // (up to DYN_UPDATE_SAMPLES away, and not aligned to the block)
            // lets the first samples of the block out uncut — a spike of the
            // whole GR on every automated block.
            if changed && d.active && d.gr_db != 0.0 {
                let mut s = snapshot;
                s.gain_db -= d.gr_db;
                configure_stages(&s, self.sample_rate, &mut self.channels[0][i]);
                configure_stages(&s, self.sample_rate, &mut self.channels[1][i]);
                d.applied_gr_db = d.gr_db;
            }
        }
        self.any_dyn = self.dyn_state.iter().any(|d| d.active);

        let on = params.auto_gain.value();
        if on != self.auto_gain_on {
            self.auto_gain_on = on;
            self.auto_gain_valid = false;
        }
        if !self.auto_gain_valid {
            self.auto_gain_db = if on {
                let snaps: [BandSnapshot; NUM_BANDS] =
                    std::array::from_fn(|i| params.bands[i].snapshot());
                (-static_gain_on_grid(&snaps, self.sample_rate, &self.grid))
                    .clamp(-AUTO_GAIN_LIMIT_DB, AUTO_GAIN_LIMIT_DB)
            } else {
                0.0
            };
            self.auto_gain_valid = true;
        }
    }

    /// Process a stereo block in-place. `output_gain` is a smoother over the
    /// *linear* output gain (the caller converts its dB param to linear once
    /// when retargeting), ramped per sample to avoid audible zippering when
    /// the output knob moves.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        output_gain: &mut Smoother,
    ) {
        self.process_stereo_keyed(left, right, None, output_gain);
    }

    /// [`process_stereo`](Self::process_stereo) with the sidechain key,
    /// when the host has connected one. Only dynamic bands with `dyn_sc`
    /// read it; a key shorter than the block reads as silence past its end.
    pub fn process_stereo_keyed(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        key: Option<(&[f32], &[f32])>,
        output_gain: &mut Smoother,
    ) {
        let frames = left.len().min(right.len());
        for i in 0..frames {
            if self.any_dyn {
                if self.dyn_countdown == 0 {
                    self.revoice_dynamic_bands();
                    self.dyn_countdown = DYN_UPDATE_SAMPLES;
                }
                self.dyn_countdown -= 1;
            }
            let mut l = left[i];
            let mut r = right[i];
            for b in 0..NUM_BANDS {
                let (in_l, in_r) = (l, r);
                if self.any_dyn && self.dyn_state[b].active {
                    let d = &mut self.dyn_state[b];
                    match key.filter(|_| d.sidechain) {
                        // The key, louder channel.
                        Some((kl, kr)) => d.detect(
                            kl.get(i).copied().unwrap_or(0.0),
                            kr.get(i).copied().unwrap_or(0.0),
                            true,
                            self.dyn_peak_coef,
                        ),
                        // What the band filters: both channels (louder one)
                        // for a Stereo band, the mid or the side otherwise.
                        None => match self.band_ms[b] {
                            BandMs::Stereo => d.detect(l, r, true, self.dyn_peak_coef),
                            BandMs::Mid => d.detect((l + r) * 0.5, 0.0, false, self.dyn_peak_coef),
                            BandMs::Side => d.detect((l - r) * 0.5, 0.0, false, self.dyn_peak_coef),
                        },
                    }
                }
                let n = self.active_stages[b];
                let [ch0, ch1] = &mut self.channels;
                (l, r) = run_band(&mut ch0[b], &mut ch1[b], n, self.band_ms[b], l, r);
                let left_in_fade = self.fade_remaining[b];
                if left_in_fade > 0 {
                    let [f0, f1] = &mut self.fade_stages;
                    let (ol, or) =
                        run_band(&mut f0[b], &mut f1[b], self.fade_active[b], self.fade_ms[b], in_l, in_r);
                    // Weight of the old stages: 1 -> 0 over the fade.
                    let w = left_in_fade as f32 / self.fade_len as f32;
                    l += w * (ol - l);
                    r += w * (or - r);
                    self.fade_remaining[b] = left_in_fade - 1;
                }
            }
            let gain_lin = output_gain.next();
            left[i] = l * gain_lin;
            right[i] = r * gain_lin;
        }
    }
}

impl EqDsp {
    /// Re-voice every dynamic band whose GR moved: the same kind, freq and
    /// Q at `gain - GR`. Coefficients only — the stages keep their state.
    fn revoice_dynamic_bands(&mut self) {
        for b in 0..NUM_BANDS {
            let d = &mut self.dyn_state[b];
            if !d.active || (d.gr_db - d.applied_gr_db).abs() < DYN_REVOICE_EPS_DB {
                continue;
            }
            let Some(mut s) = self.last_snapshot[b] else {
                continue;
            };
            s.gain_db -= d.gr_db;
            configure_stages(&s, self.sample_rate, &mut self.channels[0][b]);
            configure_stages(&s, self.sample_rate, &mut self.channels[1][b]);
            d.applied_gr_db = d.gr_db;
        }
    }
}

/// One band's cascade on one stereo sample. `Stereo` is the original
/// per-channel arithmetic, operation for operation; `Mid` / `Side` filter
/// one component through the channel-0 stages and pass the other.
#[inline]
fn run_band(
    ch0: &mut [Biquad; MAX_STAGES_PER_BAND],
    ch1: &mut [Biquad; MAX_STAGES_PER_BAND],
    n: usize,
    ms: BandMs,
    mut l: f32,
    mut r: f32,
) -> (f32, f32) {
    if n == 0 {
        // A bypassed band must not even split and recombine: that round
        // trip is not bit-exact.
        return (l, r);
    }
    match ms {
        BandMs::Stereo => {
            for s in 0..n {
                l = ch0[s].process(l);
                r = ch1[s].process(r);
            }
            (l, r)
        }
        BandMs::Mid => {
            let side = (l - r) * 0.5;
            let mut mid = (l + r) * 0.5;
            for stage in ch0.iter_mut().take(n) {
                mid = stage.process(mid);
            }
            (mid + side, mid - side)
        }
        BandMs::Side => {
            let mid = (l + r) * 0.5;
            let mut side = (l - r) * 0.5;
            for stage in ch0.iter_mut().take(n) {
                side = stage.process(side);
            }
            (mid + side, mid - side)
        }
    }
}

fn snapshots_equal(a: &BandSnapshot, b: &BandSnapshot) -> bool {
    a.enabled == b.enabled
        && a.kind == b.kind
        && a.slope == b.slope
        && a.ms == b.ms
        && (a.freq - b.freq).abs() < 1e-4
        && (a.gain_db - b.gain_db).abs() < 1e-4
        && (a.q - b.q).abs() < 1e-4
}
