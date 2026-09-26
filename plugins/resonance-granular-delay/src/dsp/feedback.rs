//! Stage 3 — the feedback topology (ba todo #1074, doc #252 §1/§5): the
//! in-loop conditioning chains, the bus handed to the next block's
//! buffer write, the dedicated Output-only recirculation rings and the
//! recirculation clock that keeps ticking while the write head is
//! frozen.

use resonance_dsp::{read_hermite_wrapped, DcBlocker, OnePole, SwapFader};

use crate::params::GranularSmoothers;

use super::grains::GrainBank;
use super::time::{fade_leg_samples, TimeMachine, TimePlan, TIME_EPSILON_SECONDS};
use super::{BlockParams, FbRoute, TimeMode};

/// One channel of the in-loop feedback conditioning chain (doc #252 §5):
/// damping filter (LP, or HP as input-minus-LP so the one-pole state
/// stays valid across type switches) → tanh soft clip → DC blocker.
/// The tanh bounds the recirculated signal to ±1 no matter the loop
/// gain, which is what keeps over-unity (>100 %) feedback stable.
struct FeedbackChain {
    filter: OnePole,
    dc: DcBlocker,
}

/// DC-blocker corner inside the feedback loop. Higher than the 5 Hz
/// full-range default on purpose: the loop compounds it on every
/// recirculation, so offset (and sub-sonic build-up from the tanh) has
/// to drain within a few repeats, while 20 Hz still costs a repeat only
/// ~1 dB at 40 Hz. Fixed in Hz, so repeats thin out identically at
/// every sample rate (DSP-04; the old fixed pole sat at 38 Hz at 48 kHz
/// and 152 Hz at 192 kHz).
const FEEDBACK_DC_CUTOFF_HZ: f32 = 20.0;

impl FeedbackChain {
    fn new(sample_rate: f32) -> Self {
        Self {
            filter: OnePole::new(),
            dc: DcBlocker::new(FEEDBACK_DC_CUTOFF_HZ, sample_rate),
        }
    }

    fn reset(&mut self) {
        self.filter.clear();
        self.dc.reset();
    }

    #[inline(always)]
    fn process(&mut self, x: f32, highpass: bool) -> f32 {
        let lp = self.filter.process(x);
        let damped = if highpass { x - lp } else { lp };
        self.dc.process(damped.tanh())
    }
}

pub(super) struct FeedbackStage {
    /// Wet→Buffer feedback bus: the conditioned (damped, soft-clipped,
    /// DC-blocked) wet output of the *previous* block, summed with the
    /// dry input at the write point of the current block. The one-block
    /// loop latency is far below the minimum grain delay (10 ms), so it
    /// is inaudible in the repeat spacing (ba todo #1074).
    pub(super) bus_l: Vec<f32>,
    pub(super) bus_r: Vec<f32>,
    /// Valid prefix of `bus_l`/`bus_r` (0 when the previous block ran
    /// the Output-only route; shrinks safely if the host varies block
    /// size).
    pub(super) bus_len: usize,
    /// Output-only recirculation rings (same length/mask as the source
    /// buffers): hold the wet-path output so "clean repeats" can
    /// recirculate at the delay time without touching the grain source
    /// buffer. Written every block regardless of route so switching
    /// topologies is seamless.
    ring_l: Vec<f32>,
    ring_r: Vec<f32>,
    mask: usize,
    /// In-loop conditioning (shared by both topologies; only one route
    /// runs per block).
    chain_l: FeedbackChain,
    chain_r: FeedbackChain,
    /// Recirculation-time counter: identical to the write head while
    /// streaming, but it keeps advancing while frozen so the
    /// Output-only recirc ring keeps its own time axis when the write
    /// head stops (ba todo #1075).
    pos: u64,
    /// Per-Grain recirc-tap swap machine: in Per-Grain time mode a
    /// delay change lands on `eff_delay` instantly (grains latch their
    /// own tap at spawn), which would jump the Output-only recirc read
    /// tap across a block boundary — an audible click in the feedback
    /// tail. The change instead rides the same fade-to-silence swap the
    /// Fade time mode uses; the payload is the committed tap, seconds.
    recirc_fade: SwapFader<f32>,
    /// Most recent value handed to `recirc_fade` (active or pending) —
    /// the swap re-trigger reference.
    recirc_goal: f32,
    /// True while a recirc-tap swap (either leg) is still in flight.
    recirc_busy: bool,
    /// False until the first block latches the live effective delay
    /// (so activation never fades in from an arbitrary value).
    recirc_primed: bool,
    /// Samples per swap leg (kept so `clear` can rebuild the fader).
    fade_leg: u32,
}

impl FeedbackStage {
    pub(super) fn new(ring_len: usize, max_block: usize, sample_rate: f32) -> Self {
        let fade_leg = fade_leg_samples(sample_rate);
        Self {
            bus_l: vec![0.0; max_block],
            bus_r: vec![0.0; max_block],
            bus_len: 0,
            ring_l: vec![0.0; ring_len],
            ring_r: vec![0.0; ring_len],
            mask: ring_len - 1,
            chain_l: FeedbackChain::new(sample_rate),
            chain_r: FeedbackChain::new(sample_rate),
            pos: 0,
            recirc_fade: SwapFader::new(fade_leg),
            recirc_goal: 0.0,
            recirc_busy: false,
            recirc_primed: false,
            fade_leg,
        }
    }

    pub(super) fn clear(&mut self) {
        self.bus_l.fill(0.0);
        self.bus_r.fill(0.0);
        self.bus_len = 0;
        self.ring_l.fill(0.0);
        self.ring_r.fill(0.0);
        self.chain_l.reset();
        self.chain_r.reset();
        self.pos = 0;
        // `SwapFader::new` is allocation-free, so rebuilding it here is
        // the cheapest full reset (mirrors `TimeMachine::clear`).
        self.recirc_fade = SwapFader::new(self.fade_leg);
        self.recirc_goal = 0.0;
        self.recirc_busy = false;
        self.recirc_primed = false;
    }

    /// Latch the recirc-tap swap machine to the current effective delay
    /// while another route (or a time mode with its own clickless
    /// machinery) owns the tap, so entering the Per-Grain Output-only
    /// path later starts from the live value instead of a stale one
    /// (mirrors `TimeMachine::resolve` keeping its fader in sync).
    fn sync_recirc_tap(&mut self, eff: f32) {
        self.recirc_fade.install(eff);
        self.recirc_goal = eff;
        self.recirc_busy = false;
        self.recirc_primed = true;
    }

    /// Advance the recirculation clock past a rendered block (it runs
    /// even while the write head is frozen).
    pub(super) fn advance(&mut self, frames: usize) {
        self.pos += frames as u64;
    }

    /// Damping cutoff: block-rate coefficient update from the smoothed
    /// value, sample-rate application (doc #252 §5).
    pub(super) fn set_damping(&mut self, cutoff: f32, sample_rate: f32) {
        self.chain_l.filter.set_cutoff(cutoff, sample_rate);
        self.chain_r.filter.set_cutoff(cutoff, sample_rate);
    }

    /// Feedback conditioning (ba todo #1074): wet × feedback → damping
    /// filter → tanh soft clip → DC blocker. The tanh bounds the
    /// recirculated signal regardless of loop gain, which is what keeps
    /// the over-unity (up to 110 %) range stable; the DC blocker stops
    /// offset accumulating across recirculations. The recirc ring runs
    /// on its own clock (`pos`): identical to the write head while
    /// streaming, but it keeps ticking while frozen so Output-only
    /// repeats stay on their own time axis instead of stalling with the
    /// stopped head (ba todo #1075).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        &mut self,
        grains: &mut GrainBank,
        time: &TimeMachine,
        sample_rate: f32,
        frames: usize,
        params: &BlockParams,
        unity_tap: bool,
        plan: &TimePlan,
        smoothers: &mut GranularSmoothers,
    ) {
        let sr = f64::from(sample_rate);
        let fb_base = self.pos as usize;
        match params.fb_route {
            FbRoute::WetToBuffer | FbRoute::PingPong => {
                // Condition this block's wet bus into the feedback bus
                // consumed at the next block's write point, and keep the
                // recirculation ring warm so a route switch is seamless.
                // Ping-pong (ba todo #1077) swaps the channels right
                // here at the feedback write tap, so every
                // recirculation crosses sides. With FB Pitch off the
                // tap carries the un-transposed re-granulation instead
                // of the transposed wet (ba todo #1078).
                let cross = params.fb_route == FbRoute::PingPong;
                self.sync_recirc_tap(time.eff_delay as f32);
                for i in 0..frames {
                    let g = smoothers.feedback.next().clamp(0.0, 1.1);
                    let (tap_l, tap_r) = if unity_tap {
                        (grains.fbw_l[i], grains.fbw_r[i])
                    } else {
                        (grains.wet_l[i], grains.wet_r[i])
                    };
                    let (src_l, src_r) = if cross { (tap_r, tap_l) } else { (tap_l, tap_r) };
                    self.bus_l[i] = self.chain_l.process(src_l * g, params.filter_is_highpass);
                    self.bus_r[i] = self.chain_r.process(src_r * g, params.filter_is_highpass);
                    let idx = (fb_base + i) & self.mask;
                    self.ring_l[idx] = grains.wet_l[i];
                    self.ring_r[idx] = grains.wet_r[i];
                }
                self.bus_len = frames;
            }
            FbRoute::OutputOnly if plan.time_varying() => {
                // Clean repeats with the recirc read tap following the
                // time mode too (ba todo #1076): the tap reads the ring
                // at the per-sample effective delay with a fractional
                // Hermite read. Repitch thereby glides — the tape swoop
                // also repitches the repeats — and Fade jumps on the
                // silent sample with the read masked by the swap gain,
                // so the tap jump cannot click (in the output or in
                // what recirculates).
                self.sync_recirc_tap(time.eff_delay as f32);
                for i in 0..frames {
                    let g = smoothers.feedback.next().clamp(0.0, 1.1);
                    let idx = (fb_base + i) & self.mask;
                    let d = (f64::from(time.delay_buf[i]) * sr).max(1.0);
                    let pos = (fb_base + i) as f64 - d;
                    let tap_gain = if plan.fade_varying {
                        time.gain_buf[i]
                    } else {
                        1.0
                    };
                    let fl = self.chain_l.process(
                        read_hermite_wrapped(&self.ring_l, pos) * tap_gain * g,
                        params.filter_is_highpass,
                    );
                    let fr = self.chain_r.process(
                        read_hermite_wrapped(&self.ring_r, pos) * tap_gain * g,
                        params.filter_is_highpass,
                    );
                    self.ring_l[idx] = grains.wet_l[i] + fl;
                    self.ring_r[idx] = grains.wet_r[i] + fr;
                    grains.wet_l[i] += fl;
                    grains.wet_r[i] += fr;
                }
                self.bus_len = 0;
            }
            FbRoute::OutputOnly => {
                // Clean repeats: recirculate the wet-path output through
                // a dedicated ring read at the delay time; the grain
                // source buffer never sees wet material. Bounded even
                // while frozen: the loop still passes through the tanh.
                //
                // Per-Grain lands a delay-time change on `eff_delay`
                // instantly (grains latch their own tap at spawn), so
                // the read tap would jump across a block boundary — an
                // audible click in the recirculating repeats. The
                // change instead rides the same fade-to-silence swap
                // the Fade time mode uses: the read is masked by the
                // swap gain and the tap jumps on the silent sample.
                // Fade and Repitch keep their own clickless machinery
                // (the time-varying arm above), so here they only latch
                // the fader; with a static delay the fader stays idle
                // and the loop below is untouched.
                let eff = time.eff_delay as f32;
                if params.time_mode != TimeMode::PerGrain || !self.recirc_primed {
                    self.sync_recirc_tap(eff);
                } else if (eff - self.recirc_goal).abs() > TIME_EPSILON_SECONDS {
                    self.recirc_fade.begin_swap(eff);
                    self.recirc_goal = eff;
                    self.recirc_busy = true;
                }
                if self.recirc_busy {
                    let mut settled = true;
                    for i in 0..frames {
                        let g = smoothers.feedback.next().clamp(0.0, 1.1);
                        let (tap_gain, value) = self.recirc_fade.next();
                        let secs = value.map_or(eff, |v| *v);
                        if tap_gain < 1.0 {
                            settled = false;
                        }
                        let delay_samples =
                            ((secs * sample_rate) as usize).clamp(1, self.mask);
                        let idx = (fb_base + i) & self.mask;
                        let ridx = (fb_base + i).wrapping_sub(delay_samples) & self.mask;
                        let fl = self.chain_l.process(
                            self.ring_l[ridx] * tap_gain * g,
                            params.filter_is_highpass,
                        );
                        let fr = self.chain_r.process(
                            self.ring_r[ridx] * tap_gain * g,
                            params.filter_is_highpass,
                        );
                        self.ring_l[idx] = grains.wet_l[i] + fl;
                        self.ring_r[idx] = grains.wet_r[i] + fr;
                        grains.wet_l[i] += fl;
                        grains.wet_r[i] += fr;
                    }
                    self.recirc_busy = !settled;
                } else {
                    let delay_samples =
                        ((time.eff_delay as f32 * sample_rate) as usize).clamp(1, self.mask);
                    for i in 0..frames {
                        let g = smoothers.feedback.next().clamp(0.0, 1.1);
                        let idx = (fb_base + i) & self.mask;
                        let ridx = (fb_base + i).wrapping_sub(delay_samples) & self.mask;
                        let fl = self
                            .chain_l
                            .process(self.ring_l[ridx] * g, params.filter_is_highpass);
                        let fr = self
                            .chain_r
                            .process(self.ring_r[ridx] * g, params.filter_is_highpass);
                        self.ring_l[idx] = grains.wet_l[i] + fl;
                        self.ring_r[idx] = grains.wet_r[i] + fr;
                        grains.wet_l[i] += fl;
                        grains.wet_r[i] += fr;
                    }
                }
                self.bus_len = 0;
            }
        }
    }
}
