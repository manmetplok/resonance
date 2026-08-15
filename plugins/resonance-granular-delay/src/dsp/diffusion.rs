//! Stage 3b — diffusion: an allpass smear of the wet path (ba todo
//! #1321, doc #252 §9 "Diffusion").
//!
//! # The choice (recorded per the todo's DoD)
//!
//! Four cascaded Schroeder allpasses per channel with mutually prime
//! delay lengths — the classic diffuser, unity magnitude response, so
//! it smears transients in time without colouring the spectrum. The
//! right channel uses its own lengths, so diffusion also decorrelates
//! the two wet buses slightly (the usual reverb-diffuser practice)
//! instead of collapsing them.
//!
//! Two decisions worth stating:
//!
//! * **Amount is a crossfade, not the allpass coefficient.** Scaling
//!   `g` toward 0 does not approach a bypass — a Schroeder allpass at
//!   `g = 0` is a *pure delay* of its full length, so a "Diffusion 0"
//!   built that way would time-shift the wet path. The coefficient is
//!   fixed at [`ALLPASS_G`] and the knob crossfades dry↔diffused, which
//!   makes 0 an exact identity and every setting in between monotonic.
//! * **The stage sits after the feedback tap**, next to the width
//!   stage: the loop recirculates the *undiffused* wet, so the smear is
//!   heard on every repeat but cannot accumulate inside the loop or
//!   affect its stability. (An in-loop diffuser is a reverb, not a
//!   delay control; if that character is ever wanted it belongs behind
//!   its own parameter.)
//!
//! At Diffusion 0 the stage is skipped outright — not multiplied by
//! zero — so the wet buses are the same bits they were before this
//! stage existed. It arms itself from silence when the knob leaves 0,
//! and the smoothed amount ramps the smear in, so engaging it is
//! click-free.

use crate::params::GranularSmoothers;

/// Allpass coefficient of every stage (the amount knob crossfades; see
/// the module docs for why it is not the amount itself). 0.62 is inside
/// the usual 0.5–0.7 diffuser range: dense smear, no ringing.
const ALLPASS_G: f32 = 0.62;

/// Left-channel allpass delay lengths, seconds — mutually prime-ish so
/// the cascade's echo pattern never lines up.
const LENGTHS_L: [f32; 4] = [0.0047, 0.0083, 0.0137, 0.0211];
/// Right-channel lengths: the same spread, offset, so the diffuser
/// decorrelates the wet pair instead of collapsing it.
const LENGTHS_R: [f32; 4] = [0.0053, 0.0091, 0.0149, 0.0223];

/// One Schroeder allpass: `v[n] = x[n] + g·v[n−M]`,
/// `y[n] = v[n−M] − g·v[n]`.
struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len_samples: usize) -> Self {
        Self {
            buf: vec![0.0; len_samples.max(1)],
            pos: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }

    #[inline]
    fn process(&mut self, x: f32, g: f32) -> f32 {
        let delayed = self.buf[self.pos];
        let v = x + g * delayed;
        self.buf[self.pos] = v;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        delayed - g * v
    }
}

/// The diffusion stage: one four-stage allpass chain per channel plus
/// the engagement gate.
pub(super) struct DiffusionStage {
    chain_l: [Allpass; 4],
    chain_r: [Allpass; 4],
    /// Whether the chain rendered last block — used to detect the
    /// idle→active transition, where the (stale) delay lines are
    /// cleared so the smear fades in from silence.
    engaged: bool,
}

impl DiffusionStage {
    pub(super) fn new(sample_rate: f32) -> Self {
        let build = |lengths: [f32; 4]| {
            lengths.map(|seconds| Allpass::new((seconds * sample_rate).round() as usize))
        };
        Self {
            chain_l: build(LENGTHS_L),
            chain_r: build(LENGTHS_R),
            engaged: false,
        }
    }

    pub(super) fn clear(&mut self) {
        for ap in self.chain_l.iter_mut().chain(self.chain_r.iter_mut()) {
            ap.clear();
        }
        self.engaged = false;
    }

    /// Whether the stage touched the wet buses on the last block
    /// (test/metering aid).
    pub(super) fn engaged(&self) -> bool {
        self.engaged
    }

    /// Smear the wet buses in place.
    ///
    /// Skipped entirely — the buses are not read or written — while the
    /// knob is at 0 and the smoother has settled there, so Diffusion 0
    /// is bit-for-bit the pre-#1321 wet path.
    pub(super) fn run(
        &mut self,
        wet_l: &mut [f32],
        wet_r: &mut [f32],
        frames: usize,
        target: f32,
        smoothers: &mut GranularSmoothers,
    ) {
        let settled = smoothers.diffusion.current() <= 0.0;
        if target <= 0.0 && settled {
            self.engaged = false;
            // Keep the smoother's clock in step with the block without
            // touching the audio.
            smoothers.diffusion.skip(frames as u32);
            return;
        }
        if !self.engaged {
            // Arming from idle: the delay lines hold whatever was in
            // them when the stage was last switched off, and the
            // smoother starts the fade at 0, so start them empty.
            for ap in self.chain_l.iter_mut().chain(self.chain_r.iter_mut()) {
                ap.clear();
            }
            self.engaged = true;
        }
        for i in 0..frames {
            let amount = smoothers.diffusion.next().clamp(0.0, 1.0);
            let (dry_l, dry_r) = (wet_l[i], wet_r[i]);
            let mut l = dry_l;
            let mut r = dry_r;
            for ap in &mut self.chain_l {
                l = ap.process(l, ALLPASS_G);
            }
            for ap in &mut self.chain_r {
                r = ap.process(r, ALLPASS_G);
            }
            wet_l[i] = dry_l + amount * (l - dry_l);
            wet_r[i] = dry_r + amount * (r - dry_r);
        }
    }
}
