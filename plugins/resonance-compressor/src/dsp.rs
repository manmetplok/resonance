//! Core compressor DSP: detector → static gain computer → ballistics →
//! apply → makeup → mix.
//!
//! Topology is a classic log-domain feed-forward compressor. Each channel
//! of the detector input (the key when one is connected, else the input)
//! optionally runs through its own sidechain high-pass; the louder of the
//! two filtered channels then feeds both a fast peak envelope and a 30 ms
//! RMS envelope. Tracking the louder channel rather than the mono sum
//! measures the actual level whatever the stereo image: a mono sum reads
//! hard-panned material 6 dB low and anti-phase material near silence
//! (DSP2-04), the same reason the mastering glue compressor does this. A detector
//! blend parameter crossfades between the two in dB space and hands the
//! result to a static soft-knee gain computer that returns a target GR
//! in dB. That target is smoothed with separate attack and release
//! one-pole coefficients and applied to the stereo signal as a gain,
//! after which manual and optional auto makeup gain are added and the
//! parallel mix control blends between the dry input and the compressed
//! path.
//!
//! All intermediate quantities downstream of the detector are in dB so
//! that the soft-knee formula and makeup gain are linear and cheap.
//!
//! # Release modes
//!
//! `Manual` (the default) is the single attack/release envelope above,
//! with the user's release time. `Auto` is the program-dependent release
//! bus compressors are known for (warmth-width-depth.md §3.1, §6.4): two
//! GR envelopes run side by side on the same target and the deeper one
//! wins.
//!
//! - the **fast** envelope attacks with the user's attack and releases in
//!   [`AUTO_FAST_RELEASE_MS`];
//! - the **slow** envelope *charges* only over [`AUTO_SLOW_ATTACK_MS`]
//!   and releases over [`AUTO_SLOW_RELEASE_MS`].
//!
//! A short transient barely charges the slow envelope, so it recovers at
//! the fast rate and doesn't dig a hole after the hit; sustained
//! compression charges it fully, so the gain comes back slowly and the
//! bus doesn't pump. The `release` knob plays no part in Auto, as on the
//! hardware this imitates.

use resonance_dsp::{db_to_linear, linear_to_db, soft_knee_gain_reduction_db, Ballistics, Biquad};
use resonance_plugin::{Smoother, SmoothingStyle};

use crate::params::CompressorParams;
use crate::viz::{CompressorViz, HISTORY_STEP_SAMPLES};

/// Decay time of the peak detector. Short and fixed — just enough to
/// bridge the gaps between a waveform's peaks — so the user's release acts
/// once, on the gain-reduction envelope, instead of being cascaded with a
/// second, identical release on the detector (LIB-06). A release knob set
/// shorter than this also shortens the detector, so it is never slower
/// than asked.
pub const PEAK_DETECTOR_RELEASE_MS: f32 = 5.0;

/// Auto release: release time of the fast GR envelope, ms.
pub const AUTO_FAST_RELEASE_MS: f32 = 50.0;
/// Auto release: how long the slow GR envelope takes to charge, ms.
pub const AUTO_SLOW_ATTACK_MS: f32 = 300.0;
/// Auto release: release time of the slow GR envelope, ms.
pub const AUTO_SLOW_RELEASE_MS: f32 = 1500.0;

pub struct CompressorDsp {
    sample_rate: f32,

    /// Peak envelope of the detector signal (linear magnitude).
    peak_env: f32,
    /// RMS envelope of the detector signal (mean-square, linear).
    rms_env: f32,
    /// One-pole coefficient for the RMS smoother. Independent of attack
    /// because the RMS smoother is a signal-smoother, not a gain smoother.
    rms_coef: f32,

    /// Current gain reduction in dB after attack/release smoothing.
    gr_db: f32,
    /// Auto release: the fast and slow GR envelopes (`gr_db` is the
    /// larger of the two while Auto is on).
    gr_fast_db: f32,
    gr_slow_db: f32,
    /// Whether the previous block ran in Auto, to seed the envelopes on
    /// a mode switch.
    auto_release: bool,

    /// Sidechain high-pass biquads, one per detector channel.
    sc_hpf_l: Biquad,
    sc_hpf_r: Biquad,

    /// Accumulator that decides when to push a GR sample into the viz ring.
    history_accum: u32,

    /// Running peak meters (linear) for the input and output, smoothed so
    /// the meters don't flicker.
    in_peak: f32,
    out_peak: f32,
    meter_decay: f32,

    /// De-zippers for the combined makeup gain (manual + auto, in dB) and
    /// the parallel mix fraction. Host automation lands on the params
    /// instantly (see `Param::set_plain`); these live here — not in the
    /// `FloatParam`s — because `Smoother::next()` needs `&mut self` and the
    /// params sit behind an `Arc`. Retargeted once per block, advanced per
    /// sample.
    makeup_smoother: Smoother,
    mix_smoother: Smoother,
}

/// Manual makeup plus the optional auto-makeup term. Auto-makeup
/// compensates about half the maximum possible GR at 0 dBFS input, which
/// is a good perceptual match for music that rarely hits the full dBFS
/// ceiling. Smoothing the combined value also glides the auto-makeup
/// toggle instead of stepping it.
fn total_makeup_db(params: &CompressorParams) -> f32 {
    let auto_gain_db = if params.auto_makeup.value() {
        let ratio = params.ratio.value().max(1.0);
        -params.threshold.value() * (1.0 - 1.0 / ratio) * 0.5
    } else {
        0.0
    };
    params.makeup.value() + auto_gain_db
}

impl CompressorDsp {
    pub fn new(sample_rate: f32, params: &CompressorParams) -> Self {
        let mut dsp = Self {
            sample_rate,
            peak_env: 0.0,
            rms_env: 0.0,
            rms_coef: 0.0,
            gr_db: 0.0,
            gr_fast_db: 0.0,
            gr_slow_db: 0.0,
            auto_release: false,
            sc_hpf_l: Biquad::identity(),
            sc_hpf_r: Biquad::identity(),
            history_accum: 0,
            in_peak: 0.0,
            out_peak: 0.0,
            meter_decay: 0.0,
            makeup_smoother: Smoother::new(SmoothingStyle::Logarithmic(20.0)),
            mix_smoother: Smoother::new(SmoothingStyle::Linear(20.0)),
        };
        dsp.set_sample_rate(sample_rate);
        // Seed the smoothers from the current param values so the first
        // block doesn't ramp in from zero.
        dsp.makeup_smoother.reset(total_makeup_db(params));
        dsp.mix_smoother.reset(params.mix.value().clamp(0.0, 1.0));
        dsp
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        // RMS smoother time constant: ~30 ms window.
        self.rms_coef = (-1.0_f32 / (0.030 * sr)).exp();
        // Meter decay: ~250 ms to drop ~60 dB visually.
        self.meter_decay = (-1.0_f32 / (0.25 * sr)).exp();
        self.makeup_smoother.set_sample_rate(sr);
        self.mix_smoother.set_sample_rate(sr);
    }

    pub fn reset(&mut self) {
        self.peak_env = 0.0;
        self.rms_env = 0.0;
        self.gr_db = 0.0;
        self.gr_fast_db = 0.0;
        self.gr_slow_db = 0.0;
        self.sc_hpf_l.reset();
        self.sc_hpf_r.reset();
        self.history_accum = 0;
        self.in_peak = 0.0;
        self.out_peak = 0.0;
    }

    /// Process a stereo block in place. All parameter reads happen at the
    /// top of the call so the detector and gain-computer math stay hot
    /// inside the per-sample loop.
    /// Process one block in place.
    ///
    /// `key` is the external sidechain signal when the host has connected
    /// one, and `None` for the ordinary case where the compressor keys off
    /// its own input. Only the DETECTOR changes: the key never reaches the
    /// output, and the SC HPF still applies to whichever signal is feeding
    /// detection — a high-passed key is exactly as useful as a high-passed
    /// self-key when the source has kick energy you don't want triggering
    /// the gain reduction.
    ///
    /// With `key: None` this is bit-identical to the pre-sidechain path,
    /// so existing projects are unaffected.
    ///
    /// Two facts about the key go into the viz object for the editor:
    /// whether one is connected at all, and — while it is — the peak
    /// detector level it reached this block, which is the level the
    /// threshold is compared against.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        key: Option<(&[f32], &[f32])>,
        params: &CompressorParams,
        viz: &CompressorViz,
    ) {
        // Publish the detector source before anything else — including
        // before the empty-block bail-out, since routing is a fact about
        // the connection, not about this block having audio in it. It is
        // set here, rather than at the call site, so the flag the editor
        // reads is the same `Option` the detector below branches on.
        viz.store_key_connected(key.is_some());

        let frames = left.len().min(right.len());
        if frames == 0 {
            return;
        }

        // Self-heal non-finite recursive state before the block runs.
        // Every envelope here is a one-pole recursion of the shape
        // `x + (state - x) * coef`, which never leaves NaN once it
        // holds one — a single bad detector sample (the plugin's own
        // math at an extreme setting, a host that doesn't scrub, an
        // upstream Inf) would otherwise mute or garble the track until
        // reset. Checking at block rate costs the hot path nothing and
        // bounds recovery to one block.
        if !(self.gr_db.is_finite()
            && self.peak_env.is_finite()
            && self.rms_env.is_finite()
            && self.gr_fast_db.is_finite()
            && self.gr_slow_db.is_finite())
        {
            self.gr_db = 0.0;
            self.peak_env = 0.0;
            self.rms_env = 0.0;
            self.gr_fast_db = 0.0;
            self.gr_slow_db = 0.0;
        }
        if !(self.in_peak.is_finite() && self.out_peak.is_finite()) {
            self.in_peak = 0.0;
            self.out_peak = 0.0;
        }

        // --- Snapshot parameters for this block ---
        let threshold = params.threshold.value();
        let ratio = params.ratio.value().max(1.0);
        let knee = params.knee.value().max(0.0);
        let attack_ms = params.attack.value().max(0.05);
        let release_ms = params.release.value().max(1.0);
        let detector_mix = params.detector_mix.value().clamp(0.0, 1.0);
        let sc_hpf_on = params.sc_hpf_on.value();
        let sc_hpf_freq = params.sc_hpf_freq.value();

        // Makeup and mix are de-zippered: retarget the smoothers once per
        // block, then pull per-sample values inside the loop so dragging
        // (or automating) either knob ramps instead of zippering.
        self.makeup_smoother.set_target(total_makeup_db(params));
        self.mix_smoother
            .set_target(params.mix.value().clamp(0.0, 1.0));

        // --- Derived quantities ---
        // Attack/release coefficients: one-pole exponential convergence.
        // `exp(-1 / (time_seconds * sr))` is the fraction kept each sample.
        let ballistics = Ballistics::from_times(self.sample_rate, attack_ms, release_ms);
        let detector_release_ms = release_ms.min(PEAK_DETECTOR_RELEASE_MS);

        // Auto release: two envelopes (see the module docs). Entering
        // Auto hands the current reduction to the fast envelope, so the
        // switch itself is seamless; leaving it keeps `gr_db`, which is
        // already the envelope the Manual path continues from.
        let auto_release = params.release_mode.value() == 1;
        if auto_release && !self.auto_release {
            self.gr_fast_db = self.gr_db;
            self.gr_slow_db = 0.0;
        }
        self.auto_release = auto_release;
        let fast = Ballistics::from_times(self.sample_rate, attack_ms, AUTO_FAST_RELEASE_MS);
        let slow =
            Ballistics::from_times(self.sample_rate, AUTO_SLOW_ATTACK_MS, AUTO_SLOW_RELEASE_MS);
        let peak_release_coef = (-1.0 / (detector_release_ms * 0.001 * self.sample_rate)).exp();

        // Update SC HPF coefficients once per block. When the HPF is
        // disabled we bypass by using an identity biquad (same coefficient
        // path, effectively a no-op).
        if sc_hpf_on {
            self.sc_hpf_l
                .set_high_pass(self.sample_rate, sc_hpf_freq, 0.707);
            self.sc_hpf_r
                .set_high_pass(self.sample_rate, sc_hpf_freq, 0.707);
        } else {
            self.sc_hpf_l.set_identity();
            self.sc_hpf_r.set_identity();
        }

        let half_knee = knee * 0.5;
        let slope = 1.0 - 1.0 / ratio;

        // Per-sample loop.
        let mut in_peak_block: f32 = self.in_peak;
        let mut out_peak_block: f32 = self.out_peak;
        // Peak detector level reached by the KEY this block, for the
        // editor's key meter. Started at `-inf` every block: with no key
        // it stays there and the editor draws no meter, so a key that is
        // unrouted mid-session cannot leave a level frozen on screen.
        // Tracking it costs one compare per sample and only while a key
        // is actually connected (ba todo #1342).
        let key_present = key.is_some();
        let mut key_peak_db: f32 = f32::NEG_INFINITY;

        for i in 0..frames {
            let l = left[i];
            let r = right[i];

            // Detection signal: the KEY when one is connected, else this
            // track's own input, each channel through the optional
            // sidechain HPF, then the louder channel. HPF is biquad; an
            // identity biquad returns the sample unchanged with a tiny
            // state cost. A key shorter than the block reads as silence
            // rather than panicking — the host is supposed to hand over a
            // full-length buffer, but a truncated one must degrade.
            let (dl, dr) = match key {
                Some((kl, kr)) => (
                    kl.get(i).copied().unwrap_or(0.0),
                    kr.get(i).copied().unwrap_or(0.0),
                ),
                None => (l, r),
            };
            let hl = self.sc_hpf_l.process(dl);
            let hr = self.sc_hpf_r.process(dr);
            // A non-finite detector sample reads as silence — the
            // envelopes below release naturally instead of latching
            // NaN. The biquads' delay lines were just poisoned by that
            // same sample, so clear them too; the HPF re-settling over a
            // few samples is nothing next to a latched NaN. Guarded on
            // the filter OUTPUT so one branch covers both a bad input
            // sample and delay-line state that was already latched.
            // Checked per channel before the max: `f32::max` drops a NaN,
            // which would hide a poisoned filter instead of clearing it.
            let det_sample = if hl.is_finite() && hr.is_finite() {
                hl.abs().max(hr.abs())
            } else {
                self.sc_hpf_l.reset();
                self.sc_hpf_r.reset();
                0.0
            };

            // Peak envelope: instant attack, short fixed decay
            // ([`PEAK_DETECTOR_RELEASE_MS`]). The user's release is applied
            // once, by the GR ballistics below.
            let abs_sample = det_sample.abs();
            self.peak_env = if abs_sample > self.peak_env {
                abs_sample
            } else {
                abs_sample + (self.peak_env - abs_sample) * peak_release_coef
            };

            // RMS envelope: 30 ms mean-square smoother.
            let sq = det_sample * det_sample;
            self.rms_env = sq + (self.rms_env - sq) * self.rms_coef;

            // Convert to dB and blend.
            let peak_db = linear_to_db(self.peak_env);
            let rms_db = linear_to_db(self.rms_env.sqrt());
            let detector_db = peak_db * (1.0 - detector_mix) + rms_db * detector_mix;

            // The key meter shows this number, not the raw key samples:
            // it is what the threshold is compared against one line
            // below, so it is the only level that explains the gain
            // reduction the user is looking at.
            if key_present {
                key_peak_db = key_peak_db.max(detector_db);
            }

            // Static knee/ratio nonlinearity.
            let target_gr_db =
                soft_knee_gain_reduction_db(detector_db, threshold, knee, half_knee, slope);

            // Attack/release ballistics on the GR envelope. When new GR is
            // larger than current (the comp needs to clamp harder) we use
            // the attack coefficient; otherwise the slower release.
            if auto_release {
                self.gr_fast_db = fast.step_envelope(self.gr_fast_db, target_gr_db);
                self.gr_slow_db = slow.step_envelope(self.gr_slow_db, target_gr_db);
                self.gr_db = self.gr_fast_db.max(self.gr_slow_db);
            } else {
                self.gr_db = ballistics.step_envelope(self.gr_db, target_gr_db);
            }

            // Apply the gain reduction plus the smoothed makeup.
            let apply_db = self.makeup_smoother.next() - self.gr_db;
            let apply_lin = db_to_linear(apply_db);

            let wet_l = l * apply_lin;
            let wet_r = r * apply_lin;

            // Parallel mix, also smoothed per sample.
            let mix = self.mix_smoother.next();
            let out_l = l * (1.0 - mix) + wet_l * mix;
            let out_r = r * (1.0 - mix) + wet_r * mix;

            left[i] = out_l;
            right[i] = out_r;

            // Meter envelopes (slow decay, instant attack). The input
            // meter shows the INPUT, so an external key — which can be
            // far louder than the signal being compressed — must not be
            // folded into it; it gets its own meter from `key_peak_db`
            // above. Without a key the detector is derived from the
            // input anyway, and including it is the pre-existing
            // behaviour, kept so the meter reads identically.
            let abs_in = match key {
                Some(_) => l.abs().max(r.abs()),
                None => abs_sample.max(l.abs()).max(r.abs()),
            };
            in_peak_block = if abs_in > in_peak_block {
                abs_in
            } else {
                in_peak_block * self.meter_decay
            };
            let abs_out = out_l.abs().max(out_r.abs());
            out_peak_block = if abs_out > out_peak_block {
                abs_out
            } else {
                out_peak_block * self.meter_decay
            };

            // GR history ring: push the current GR once every
            // HISTORY_STEP_SAMPLES samples.
            self.history_accum += 1;
            if self.history_accum >= HISTORY_STEP_SAMPLES {
                self.history_accum = 0;
                viz.push_gr(self.gr_db);
            }
        }

        self.in_peak = in_peak_block;
        self.out_peak = out_peak_block;

        // Publish the latest scalar meter values once per block.
        viz.store_levels(
            linear_to_db(in_peak_block),
            linear_to_db(out_peak_block),
            self.gr_db,
            key_peak_db,
        );
    }
}

/// Public, pure helper reused by the editor to render the transfer curve
/// without instantiating a whole DSP.
pub fn transfer_curve_db(
    input_db: f32,
    threshold: f32,
    ratio: f32,
    knee: f32,
    makeup_db: f32,
) -> f32 {
    let ratio = ratio.max(1.0);
    let slope = 1.0 - 1.0 / ratio;
    let half_knee = knee * 0.5;
    let gr = soft_knee_gain_reduction_db(input_db, threshold, knee, half_knee, slope);
    input_db - gr + makeup_db
}
