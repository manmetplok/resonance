//! [`FreezeRamp`]: how the absorbing-loop engines (Plate, Room, Chamber,
//! Ambience, Hall) get into and out of Freeze without a click.
//!
//! Freeze is "loop lossless, input muted" (§4.2). Done at once, both
//! halves click: the input is cut wherever it stands, and so is whatever
//! the diffusers and (on the Hall) the build line are still feeding the
//! loop; and every line's absorption jumps from its designed loss to
//! unity, a step in each line's output level of up to `1/g` (×2 for a
//! 200 ms line at a 2 s decay).
//!
//! So Freeze is a ramp, `hold`, from 0 (running) to 1 (frozen) over
//! [`FREEZE_RAMP_MS`], the same way back on release, reversible
//! mid-way:
//!
//! - **The input** (everything the engine hears, so the reflections and
//!   any direct path fade with it) is scaled by `1 − hold`.
//! - **The loop's injection** (what the diffusers or the build line hand
//!   to the loop) is scaled by `1 − hold` again, so it is exactly zero
//!   when the loop closes, however long those stages still ring.
//! - **The loss** goes the same way: the absorption is redesigned for
//!   every T60 stretched by `1 / (1 − hold)` ([`stretch`]), so the loss per
//!   pass falls linearly to none (`T60 = ∞` designs a unity filter).
//!   The redesign runs every [`REDESIGN_EVERY`] samples: each step moves
//!   a line's gain by `ln(1/g) / 300` at most (0.2 % on that 200 ms line),
//!   well under what a sine's own sample step is.
//! - Only at `hold` 1 does the engine switch its loop to the exact
//!   lossless state (and the `Fdn` start fading its modulation out), from
//!   a design that is already unity to within one step. Release leaves
//!   it at once, at zero loss, and ramps back.
//!
//! The held tail is the level the loop had when the ramp ended: the
//! ramp's 100 ms lose a little of a short decay, and nothing after.
//!
//! Lives with the room family's core, the busiest user; Plate and Hall
//! use it from here.

use resonance_dsp::reverb::DecayBands;

/// Engage (and release) time, ms.
pub(in crate::dsp::algo) const FREEZE_RAMP_MS: f32 = 100.0;

/// Samples between loss redesigns while the ramp moves.
pub(in crate::dsp::algo) const REDESIGN_EVERY: u32 = 16;

/// What a [`FreezeRamp::tick`] asks of the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::dsp::algo) enum FreezeTick {
    /// Settled (running or frozen): nothing to do.
    Still,
    /// Moving between redesigns.
    Moving,
    /// Redesign the loop's absorption at [`FreezeRamp::loss`] (also on
    /// landing back at running: loss 1).
    Redesign,
    /// The ramp reached frozen: switch the loop to lossless.
    Engage,
}

/// The Freeze state of one engine (see the module docs).
#[derive(Clone, Copy, Debug)]
pub(in crate::dsp::algo) struct FreezeRamp {
    on: bool,
    /// 0 running … 1 frozen.
    hold: f32,
    step: f32,
    countdown: u32,
}

impl FreezeRamp {
    pub(in crate::dsp::algo) fn new(sample_rate: f32) -> Self {
        Self {
            on: false,
            hold: 0.0,
            step: 1.0 / (FREEZE_RAMP_MS * 0.001 * sample_rate).max(1.0),
            countdown: REDESIGN_EVERY,
        }
    }

    /// The requested state.
    pub(in crate::dsp::algo) fn on(&self) -> bool {
        self.on
    }

    /// Request Freeze on or off; the ramp follows from the next
    /// [`FreezeRamp::tick`].
    pub(in crate::dsp::algo) fn set(&mut self, on: bool) {
        self.on = on;
    }

    /// Land in the requested state at once (nothing is sounding).
    pub(in crate::dsp::algo) fn snap(&mut self) {
        self.hold = if self.on { 1.0 } else { 0.0 };
        self.countdown = REDESIGN_EVERY;
    }

    /// Fully frozen: the loop is (or must be) lossless.
    pub(in crate::dsp::algo) fn held(&self) -> bool {
        self.hold >= 1.0
    }

    /// Gain on the input, and again on the loop's injection.
    #[inline]
    pub(in crate::dsp::algo) fn gain(&self) -> f32 {
        1.0 - self.hold
    }

    /// Fraction of the designed loss the loop carries now.
    #[inline]
    pub(in crate::dsp::algo) fn loss(&self) -> f32 {
        1.0 - self.hold
    }

    /// Advance one sample.
    #[inline]
    pub(in crate::dsp::algo) fn tick(&mut self) -> FreezeTick {
        let target = if self.on { 1.0 } else { 0.0 };
        if self.hold == target {
            return FreezeTick::Still;
        }
        self.hold = if self.on {
            (self.hold + self.step).min(1.0)
        } else {
            (self.hold - self.step).max(0.0)
        };
        if self.hold == target {
            self.countdown = REDESIGN_EVERY;
            return if self.on {
                FreezeTick::Engage
            } else {
                FreezeTick::Redesign
            };
        }
        self.countdown -= 1;
        if self.countdown == 0 {
            self.countdown = REDESIGN_EVERY;
            FreezeTick::Redesign
        } else {
            FreezeTick::Moving
        }
    }
}

/// `bands` with every T60 stretched by `1 / loss` (`loss` 0: infinite,
/// a lossless design).
pub(in crate::dsp::algo) fn stretch(bands: DecayBands, loss: f32) -> DecayBands {
    if loss >= 1.0 {
        return bands;
    }
    let k = |t: f32| if loss <= 0.0 { f32::INFINITY } else { t / loss };
    DecayBands {
        t60_low: k(bands.t60_low),
        t60_mid: k(bands.t60_mid),
        t60_high: k(bands.t60_high),
        ..bands
    }
}
