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
//! **Auto-gain** trims the output by the negated [`static_gain_db`] of the
//! band curve. It is a *static* estimate — a function of the parameters
//! only, recomputed when they change — rather than a level follower, so it
//! is deterministic and cannot pump with the programme.

use resonance_dsp::Biquad;
use resonance_plugin::Smoother;

use crate::band::{configure_stages, BandMs, MAX_STAGES_PER_BAND};
use crate::params::{BandSnapshot, EqParams, NUM_BANDS};

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
            }
        }

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
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let mut l = left[i];
            let mut r = right[i];
            for b in 0..NUM_BANDS {
                let (in_l, in_r) = (l, r);
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
