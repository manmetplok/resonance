//! Stage 1b — the Fade/Repitch time-mode machine (ba todo #1076, doc
//! #252 §5): what a delay-time change does to the tap this block, and
//! the per-sample delay/gain buffers the rest of the render path reads
//! off it.

use resonance_dsp::SwapFader;

use super::grains::GrainBank;
use super::voice::VoiceStage;
use super::{BlockParams, TimeMode};

/// Fade time mode (ba todo #1076): length of each `SwapFader` leg,
/// seconds. The full transition — wet fades out, the tap swaps on the
/// silent sample, the new origin fades back in — takes twice this,
/// ~20 ms (doc #252 §5: too-long fades color, too-short ones glitch).
pub const FADE_LEG_SECONDS: f32 = 0.010;

/// Repitch time mode (ba todo #1076): one-pole slew time constant of
/// the effective delay, seconds (tape/BBD-style glide).
pub const REPITCH_TAU_SECONDS: f32 = 0.100;

/// Delay-target changes below this are ignored by the Fade swap
/// trigger, seconds — keeps host tempo jitter from re-arming fades.
/// Shared with the Output-only recirc-tap swap in `feedback.rs`.
pub(super) const TIME_EPSILON_SECONDS: f32 = 1.0e-4;

/// Once the Repitch slew is within this of the target it snaps,
/// seconds (sub-sample at any supported rate).
const REPITCH_SNAP_SECONDS: f32 = 1.0e-5;

/// Clamp on the Repitch playback-rate multiplier `1 − d(delay)/dt`
/// (± two octaves), bounding the swoop on extreme jumps.
pub(super) const REPITCH_RATE_MIN: f64 = 0.25;
pub(super) const REPITCH_RATE_MAX: f64 = 4.0;

/// Per-block time-mode resolution (ba todo #1076): how the effective
/// delay behaves across this block.
pub(super) struct TimePlan {
    /// Sample index where a Fade swap lands this block (`usize::MAX`
    /// when none): the silent sample where every engine is hard-reset.
    pub(super) swap_at: usize,
    /// A Fade transition is in flight: `gain_buf` and `delay_buf` carry
    /// the per-sample wet gain and delay.
    pub(super) fade_varying: bool,
    /// The Repitch slew is active: `delay_buf` carries the gliding
    /// per-sample delay.
    pub(super) repitch_slewing: bool,
}

impl TimePlan {
    /// The per-sample delay buffer is live (Fade in flight or Repitch
    /// slewing); otherwise `eff_delay` holds for the whole block.
    pub(super) fn time_varying(&self) -> bool {
        self.fade_varying || self.repitch_slewing
    }
}

/// The dual-tap swap machine and the Repitch slew, plus the per-sample
/// buffers they publish to the render stages.
pub(super) struct TimeMachine {
    /// Fade time mode (ba todo #1076): the dual-tap swap machine. The
    /// payload is the delay value (seconds) the wet path is committed
    /// to; a target change swaps toward the new value through silence.
    fade: SwapFader<f32>,
    /// Most recent value handed to the fader (active or pending) — the
    /// swap re-trigger reference.
    fade_goal: f32,
    /// True while a Fade transition (either leg) is still in flight.
    fade_busy: bool,
    /// Per-sample effective delay for the block, seconds (filled by the
    /// Fade/Repitch pre-pass; unused in Per-Grain mode).
    pub(super) delay_buf: Vec<f32>,
    /// Per-sample wet gain from the Fade swap (1.0 outside fades).
    pub(super) gain_buf: Vec<f32>,
    /// Current effective delay, seconds: the resolved target in
    /// Per-Grain mode, the slewed value in Repitch, the fader's active
    /// payload in Fade. `f64` so the one-pole slew increment never
    /// stalls below the mantissa step of the value itself.
    pub(super) eff_delay: f64,
    /// False until the first block latches the initial delay target
    /// (so activation never fades/slews from an arbitrary value).
    primed: bool,
    /// Per-sample one-pole coefficient of the Repitch slew.
    pub(super) repitch_coeff: f64,
}

impl TimeMachine {
    pub(super) fn new(sample_rate: f32, max_block: usize) -> Self {
        Self {
            fade: SwapFader::new(fade_leg_samples(sample_rate)),
            fade_goal: 0.0,
            fade_busy: false,
            delay_buf: vec![0.0; max_block],
            gain_buf: vec![1.0; max_block],
            eff_delay: 0.0,
            primed: false,
            repitch_coeff: 1.0 - (-1.0 / f64::from(REPITCH_TAU_SECONDS * sample_rate)).exp(),
        }
    }

    pub(super) fn clear(&mut self, sample_rate: f32) {
        // `SwapFader::new` is allocation-free, so rebuilding it here is
        // the cheapest full reset (it has no reset method).
        self.fade = SwapFader::new(fade_leg_samples(sample_rate));
        self.fade_goal = 0.0;
        self.fade_busy = false;
        self.eff_delay = 0.0;
        self.primed = false;
    }

    /// The tap position a whole-block stage should use: the per-sample
    /// buffer's first entry while it is live, the settled effective
    /// delay otherwise.
    pub(super) fn block_tap_seconds(&self, plan: &TimePlan) -> f32 {
        if plan.time_varying() {
            self.delay_buf[0]
        } else {
            self.eff_delay as f32
        }
    }

    /// Decide the per-sample effective delay (and, in Fade mode, the
    /// per-sample wet gain) this block renders with. Per-Grain passes
    /// the target straight through — bit-identical to the pre-#1076
    /// behaviour.
    pub(super) fn resolve(&mut self, frames: usize, params: &BlockParams) -> TimePlan {
        let target = params.delay_seconds;
        let target64 = f64::from(target);
        if !self.primed {
            self.primed = true;
            self.eff_delay = target64;
            self.fade.install(target);
            self.fade_goal = target;
            self.fade_busy = false;
        }
        // Sample index where a Fade swap lands this block (the silent
        // sample; every engine is hard-reset there so the old tap's
        // in-flight grains are retired without a click).
        let mut swap_at = usize::MAX;
        let mut fade_varying = false;
        let mut repitch_slewing = false;
        match params.time_mode {
            TimeMode::PerGrain => {
                // A time change affects only newly spawned grains;
                // in-flight grains finish at their old origin. Keep the
                // fader in sync so entering Fade mode later starts from
                // the current value instead of a stale one.
                self.eff_delay = target64;
                self.fade.install(target);
                self.fade_goal = target;
                self.fade_busy = false;
            }
            TimeMode::Repitch => {
                if (self.eff_delay - target64).abs() > f64::from(REPITCH_SNAP_SECONDS) {
                    repitch_slewing = true;
                    for slot in self.delay_buf[..frames].iter_mut() {
                        self.eff_delay += (target64 - self.eff_delay) * self.repitch_coeff;
                        *slot = self.eff_delay as f32;
                    }
                } else {
                    self.eff_delay = target64;
                }
                let eff = self.eff_delay as f32;
                self.fade.install(eff);
                self.fade_goal = eff;
                self.fade_busy = false;
            }
            TimeMode::Fade => {
                if (target - self.fade_goal).abs() > TIME_EPSILON_SECONDS {
                    self.fade.begin_swap(target);
                    self.fade_goal = target;
                    self.fade_busy = true;
                }
                if self.fade_busy {
                    fade_varying = true;
                    let mut active = self.eff_delay as f32;
                    let mut settled = true;
                    for i in 0..frames {
                        let (g, value) = self.fade.next();
                        let v = value.map_or(target, |v| *v);
                        if v != active {
                            swap_at = i;
                            active = v;
                        }
                        if g < 1.0 {
                            settled = false;
                        }
                        self.gain_buf[i] = g;
                        self.delay_buf[i] = v;
                    }
                    self.eff_delay = f64::from(active);
                    self.fade_busy = !settled;
                } else {
                    self.eff_delay = target64;
                }
            }
        }
        TimePlan {
            swap_at,
            fade_varying,
            repitch_slewing,
        }
    }
}

/// Samples per Fade leg at `sample_rate` (never zero). Shared with the
/// Output-only recirc-tap swap in `feedback.rs`.
pub(super) fn fade_leg_samples(sample_rate: f32) -> u32 {
    ((FADE_LEG_SECONDS * sample_rate) as u32).max(1)
}

/// Stage 2d — Fade time mode (ba todo #1076): the SwapFader's
/// per-sample gain windows the whole granulated wet — audible cloud and
/// feedback tap alike — through the tap swap: out over ~10 ms, swap on
/// the silent sample (where the engines were hard-reset), back in over
/// ~10 ms at the new origin. Applied before the feedback stage so
/// recirculations carry the faded wet coherently.
pub(super) fn apply_fade_gain(
    time: &TimeMachine,
    frames: usize,
    plan: &TimePlan,
    grains: &mut GrainBank,
    voice: &mut VoiceStage,
    fb_render: bool,
    psola_render: bool,
) {
    if !plan.fade_varying {
        return;
    }
    for i in 0..frames {
        let g = time.gain_buf[i];
        grains.wet_l[i] *= g;
        grains.wet_r[i] *= g;
    }
    if fb_render {
        for i in 0..frames {
            let g = time.gain_buf[i];
            grains.fbw_l[i] *= g;
            grains.fbw_r[i] *= g;
        }
    }
    if psola_render {
        for i in 0..frames {
            let g = time.gain_buf[i];
            voice.psola_l[i] *= g;
            voice.psola_r[i] *= g;
        }
    }
}
