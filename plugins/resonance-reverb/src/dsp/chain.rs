//! Reverb orchestrator: the stages every algorithm shares, around the
//! selected engine.
//!
//! Signal flow:
//!   Input -> Return EQ -> Pre-delay -> Engine (er, late) -> ER/Tail balance -> Width -> Stereo Output
//!
//! The return EQ (off by default) filters what *feeds* the room; the
//! ER/tail balance (centred by default) weights the engine's two wet
//! components against each other just before they are summed. The
//! engines themselves (Classic's diffusion + FDN + ER, and those that
//! follow) live in [`super::algo`]; [`EngineBank`] runs the selected one
//! and crossfades on an algorithm switch.

use resonance_dsp::DelayLine;

use super::algo::switch::EngineBank;
use super::algo::Algorithm;
use super::er::ER_TAPS;
use super::return_eq::ReturnEq;
use super::CHANNELS;

/// Maximum pre-delay in seconds. The delay lines (and the tap clamp)
/// are sized from this at the *actual* sample rate at construction —
/// this used to be a hardcoded 48 000 samples, which was only "1
/// second" at 48 kHz and silently halved the maximum at 96 kHz.
pub(crate) const MAX_PREDELAY_SECONDS: f32 = 1.0;
/// Pre-delay tap-move crossfade length, ms (see `set_predelay`).
const PREDELAY_FADE_MS: f32 = 20.0;

/// The complete reverb processor.
pub struct ReverbDsp {
    sample_rate: f32,

    // Pre-delay. The read tap never relocates mid-signal: a change
    // crossfades from the old tap to the new one (see `set_predelay`).
    predelay_l: DelayLine,
    predelay_r: DelayLine,
    /// Settled tap length — and the fade *destination* while a fade runs.
    predelay_samples: usize,
    /// Tap being faded out while a fade runs.
    predelay_from: usize,
    /// Remaining crossfade samples; 0 means no fade is running.
    predelay_fade_left: u32,
    /// Total crossfade length in samples at this sample rate.
    predelay_fade_total: u32,
    /// Newest tap requested while a fade was already running; started
    /// as the next fade the moment the running one completes.
    predelay_pending: Option<usize>,
    /// Last tap length requested — dedupes per-block `set_predelay`.
    predelay_requested: usize,
    /// False until the first processed sample. While false a pre-delay
    /// change snaps (configuring a fresh/reset instance must not fade
    /// in from a stale tap).
    predelay_primed: bool,
    /// `MAX_PREDELAY_SECONDS` at the actual sample rate.
    max_predelay_samples: usize,

    /// Every engine, built here, and the switch between them.
    engines: EngineBank,

    /// Wet HPF/LPF on the input, before everything else.
    return_eq: ReturnEq,
    /// ER and tail weights from `er_tail_balance`; both exactly 1.0 at
    /// the centred default, which multiplies through bit-exactly.
    er_gain: f32,
    tail_gain: f32,

    /// Running sum-of-squares for wet RMS (reset by `take_wet_rms`).
    wet_sumsq: f64,
    wet_count: u32,
}

impl ReverbDsp {
    /// A processor with one engine per built algorithm, on Classic.
    pub fn new(sample_rate: f32) -> Self {
        Self::with_engines(sample_rate, Algorithm::BUILT)
    }

    /// A processor whose engine bank holds exactly `slots`, starting on
    /// slot 0. [`ReverbDsp::new`] uses every built algorithm once; tests
    /// use this to switch between two instances of one algorithm.
    #[doc(hidden)]
    pub fn with_engines(sample_rate: f32, slots: &[Algorithm]) -> Self {
        let max_predelay_samples = (MAX_PREDELAY_SECONDS * sample_rate) as usize;

        Self {
            sample_rate,
            predelay_l: DelayLine::new(max_predelay_samples),
            predelay_r: DelayLine::new(max_predelay_samples),
            predelay_samples: 0,
            predelay_from: 0,
            predelay_fade_left: 0,
            predelay_fade_total: ((PREDELAY_FADE_MS * 0.001 * sample_rate) as u32).max(1),
            predelay_pending: None,
            predelay_requested: 0,
            predelay_primed: false,
            max_predelay_samples,
            engines: EngineBank::new(slots, sample_rate),
            return_eq: ReturnEq::new(),
            er_gain: 1.0,
            tail_gain: 1.0,
            wet_sumsq: 0.0,
            wet_count: 0,
        }
    }

    /// Select the algorithm. A change while audio is running crossfades
    /// over [`super::algo::switch::SWITCH_FADE_MS`]; before the first
    /// sample (or after `clear`) it snaps. An algorithm this processor
    /// has no engine for is ignored.
    pub fn set_algorithm(&mut self, algorithm: Algorithm) {
        if let Some(slot) = self.engines.slot_of(algorithm) {
            self.engines.request(slot);
        }
    }

    /// The algorithm of the engine receiving input.
    pub fn algorithm(&self) -> Algorithm {
        self.engines.active().algorithm()
    }

    /// Select engine slot `slot` of a [`ReverbDsp::with_engines`] bank.
    #[doc(hidden)]
    pub fn set_engine_slot(&mut self, slot: usize) {
        self.engines.request(slot);
    }

    /// The slot receiving input.
    #[doc(hidden)]
    pub fn engine_slot(&self) -> usize {
        self.engines.active_slot()
    }

    /// Number of engines in the bank.
    #[doc(hidden)]
    pub fn engine_count(&self) -> usize {
        self.engines.slot_count()
    }

    /// True while an algorithm switch is fading.
    pub fn switching(&self) -> bool {
        self.engines.switching()
    }

    /// Set early-reflections level (0..1, normalized).
    pub fn set_er_level(&mut self, norm: f32) {
        self.engines.cfg.er_level = Some(norm);
        self.engines.for_live(|e| e.set_er_level(norm));
    }

    /// Set early-reflections time scaling (0..1, normalized).
    pub fn set_er_time(&mut self, norm: f32) {
        self.engines.cfg.er_time = Some(norm);
        self.engines.for_live(|e| e.set_er_time(norm));
    }

    /// Bass decay multiplier, bass crossover (Hz) and treble decay
    /// multiplier. Read by the engines with frequency-dependent
    /// absorption; Classic ignores them.
    pub fn set_decay_shape(&mut self, low_mult: f32, low_xover_hz: f32, high_mult: f32) {
        self.engines.cfg.decay_shape = Some((low_mult, low_xover_hz, high_mult));
        self.engines
            .for_live(|e| e.set_decay_shape(low_mult, low_xover_hz, high_mult));
    }

    /// The tail's build-up time, `0..=1` (Hall; ignored elsewhere).
    pub fn set_build(&mut self, build: f32) {
        self.engines.cfg.build = Some(build);
        self.engines.for_live(|e| e.set_build(build));
    }

    /// Configure the return EQ (the wet HPF/LPF before the tank). `steep`
    /// selects 18 dB/oct over 12 dB/oct for both filters.
    pub fn set_wet_filters(
        &mut self,
        hpf_on: bool,
        hpf_hz: f32,
        lpf_on: bool,
        lpf_hz: f32,
        steep: bool,
    ) {
        self.return_eq
            .configure(self.sample_rate, hpf_on, hpf_hz, lpf_on, lpf_hz, steep);
    }

    /// Set the ER/tail depth balance, `-1..=1`.
    ///
    /// `0` is the plugin's original mix of the two. Toward `-1` the tail
    /// fades out, leaving the early reflections (a source placed close,
    /// in the room); toward `+1` the early reflections fade out, leaving
    /// the diffuse wash (a source far away). One side is always at full
    /// level, so the crossfade never dips the whole wet signal.
    pub fn set_er_tail_balance(&mut self, balance: f32) {
        let b = balance.clamp(-1.0, 1.0);
        self.er_gain = if b > 0.0 { 1.0 - b } else { 1.0 };
        self.tail_gain = if b < 0.0 { 1.0 + b } else { 1.0 };
    }

    /// Snapshot the current scaled ER tap times (ms) for the editor.
    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        self.engines.active().er_tap_times_ms()
    }

    /// Snapshot the ER tap gains (incl. polarity) for the editor.
    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        self.engines.active().er_tap_gains()
    }

    /// Snapshot the smoothed per-FDN-channel energies for the tank view.
    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        self.engines.active().channel_energies()
    }

    /// Current FDN delay lengths in ms (affected by `size`).
    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        self.engines.active().fdn_delay_ms()
    }

    /// Take the RMS of the wet output since the last call and reset the accumulator.
    /// Returns 0.0 on the first call after construction / clear.
    pub fn take_wet_rms(&mut self) -> f32 {
        if self.wet_count == 0 {
            return 0.0;
        }
        let mean = self.wet_sumsq / self.wet_count as f64;
        self.wet_sumsq = 0.0;
        self.wet_count = 0;
        (mean as f32).sqrt()
    }

    /// Reconfigure for a new sample rate. Clears all state.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        *self = Self::new(sample_rate);
    }

    /// Update room size (0..1).
    pub fn set_size(&mut self, size_normalized: f32) {
        self.engines.cfg.size = Some(size_normalized);
        self.engines.for_live(|e| e.set_size(size_normalized));
    }

    /// Set decay time (mid-frequency T60) in seconds.
    pub fn set_decay(&mut self, rt60_seconds: f32) {
        self.engines.cfg.decay = Some(rt60_seconds);
        self.engines.for_live(|e| e.set_decay(rt60_seconds));
    }

    /// Set or unset freeze mode, on every engine: each holds its own
    /// tail at full loop gain with its input muted.
    pub fn set_freeze(&mut self, freeze: bool) {
        self.engines.set_freeze(freeze);
    }

    /// Set high-frequency damping cutoff.
    pub fn set_damping(&mut self, cutoff_hz: f32) {
        self.engines.cfg.damping = Some(cutoff_hz);
        self.engines.for_live(|e| e.set_damping(cutoff_hz));
    }

    /// Set pre-delay in milliseconds.
    ///
    /// The read tap never relocates mid-signal: a change starts a
    /// linear crossfade from the old tap to the new one over
    /// `PREDELAY_FADE_MS`. Crossfading (rather than a Doppler glide)
    /// keeps the pre-delay pitch-stable — a bending tap here would bend
    /// the *entire* reverb input. The fade is linear, not equal-power:
    /// during a sweep the two taps are a few ms apart and strongly
    /// correlated, where an equal-power law would bulge by up to +3 dB
    /// at the midpoint. A change landing mid-fade is queued (newest
    /// wins) and started when the running fade completes, so a
    /// continuous sweep resolves into back-to-back short crossfades.
    pub fn set_predelay(&mut self, ms: f32) {
        let samples =
            ((ms * 0.001 * self.sample_rate) as usize).min(self.max_predelay_samples - 1);
        if samples == self.predelay_requested {
            return;
        }
        self.predelay_requested = samples;
        if !self.predelay_primed {
            // Nothing audible is in flight yet — snap, exactly like the
            // pre-crossfade code did on activation.
            self.predelay_samples = samples;
            self.predelay_fade_left = 0;
            self.predelay_pending = None;
        } else if self.predelay_fade_left > 0 {
            self.predelay_pending = Some(samples);
        } else if samples != self.predelay_samples {
            self.predelay_from = self.predelay_samples;
            self.predelay_samples = samples;
            self.predelay_fade_left = self.predelay_fade_total;
        }
    }

    /// Set modulation depth (0..1 normalized).
    pub fn set_mod_depth(&mut self, depth: f32) {
        self.engines.cfg.mod_depth = Some(depth);
        self.engines.for_live(|e| e.set_mod_depth(depth));
    }

    /// Set modulation rate (Hz).
    pub fn set_mod_rate(&mut self, rate_hz: f32) {
        self.engines.cfg.mod_rate = Some(rate_hz);
        self.engines.for_live(|e| e.set_mod_rate(rate_hz));
    }

    /// Process a single stereo sample pair. Returns (wet_l, wet_r).
    pub fn process(
        &mut self,
        left: f32,
        right: f32,
        diffusion_amount: f32,
        width: f32,
    ) -> (f32, f32) {
        // Return EQ first: it shapes everything the room is fed, ER and
        // tank alike. A no-op (not even touched) with both filters off.
        let (left, right) = self.return_eq.process(left, right);

        // Pre-delay. A stationary tap reads exactly as before; while a
        // crossfade is in flight both taps are read and mixed.
        let (dl, dr) = if self.predelay_fade_left > 0 {
            self.predelay_fade_left -= 1;
            let x = (self.predelay_fade_total - self.predelay_fade_left) as f32
                / self.predelay_fade_total as f32;
            let old_l = self.predelay_l.tap(self.predelay_from);
            let old_r = self.predelay_r.tap(self.predelay_from);
            let new_l = self.predelay_l.tap(self.predelay_samples);
            let new_r = self.predelay_r.tap(self.predelay_samples);
            if self.predelay_fade_left == 0 {
                if let Some(next) = self.predelay_pending.take() {
                    if next != self.predelay_samples {
                        self.predelay_from = self.predelay_samples;
                        self.predelay_samples = next;
                        self.predelay_fade_left = self.predelay_fade_total;
                    }
                }
            }
            (old_l + x * (new_l - old_l), old_r + x * (new_r - old_r))
        } else {
            (
                self.predelay_l.tap(self.predelay_samples),
                self.predelay_r.tap(self.predelay_samples),
            )
        };
        self.predelay_l.push(left);
        self.predelay_r.push(right);
        self.predelay_primed = true;

        // The engine: its early and late parts, kept apart so the
        // balance below applies to every algorithm.
        let wet = self.engines.process(dl, dr, diffusion_amount);
        let mut sum_l = wet.late_l;
        let mut sum_r = wet.late_r;

        // ER/tail balance. Both gains are exactly 1.0 when centred, and
        // `x * 1.0 == x` bit for bit, so the default path is unchanged.
        sum_l *= self.tail_gain;
        sum_r *= self.tail_gain;

        // Sum ER into the wet bus before the width/mix stage so ER also
        // respects width and mix.
        sum_l += wet.er_l * self.er_gain;
        sum_r += wet.er_r * self.er_gain;

        // Width: 0 = mono, 1 = full stereo
        let mid = (sum_l + sum_r) * 0.5;
        let side = (sum_l - sum_r) * 0.5;
        let out_l = mid + side * width;
        let out_r = mid - side * width;

        // Wet RMS accumulator for the impulse-view live trace polygon.
        self.wet_sumsq += (out_l as f64) * (out_l as f64) + (out_r as f64) * (out_r as f64);
        self.wet_count += 2;

        (out_l, out_r)
    }

    /// Clear all internal state (delay lines, filters, feedback).
    pub fn clear(&mut self) {
        self.predelay_l.clear();
        self.predelay_r.clear();
        // Cancel any fade and land on the newest requested tap, so a
        // reset instance matches a fresh one that was configured once.
        self.predelay_fade_left = 0;
        self.predelay_pending = None;
        self.predelay_samples = self.predelay_requested;
        self.predelay_primed = false;
        self.engines.clear();
        self.return_eq.clear();
        self.wet_sumsq = 0.0;
        self.wet_count = 0;
    }
}
