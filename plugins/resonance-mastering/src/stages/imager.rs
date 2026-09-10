//! M/S stereo imager.
//!
//! Encodes L/R to Mid/Side, optionally high-pass-filters the side
//! channel (keeps sub-bass mono for vinyl and phone-speaker
//! compatibility), scales the side channel by `width`, and decodes
//! back to L/R. Zero latency — a single biquad per call path.
//!
//! `width == 1.0` is the identity. `width == 0.0` collapses to mono.
//! `width > 1.0` widens the image (caution: can cause mono-sum
//! cancellation). The recommended safe range for mastering band
//! material is `0.8 .. 1.3`.

use resonance_dsp::Biquad;
use resonance_plugin::{Smoother, SmoothingStyle};

use super::retarget;

/// Ramp length for the width smoother and the enable crossfade, in
/// milliseconds. Long enough to spread a full-scale step over ~480
/// samples at 48 kHz (no click), short enough to still feel instant.
const RAMP_MS: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImagerConfig {
    pub enabled: bool,
    /// Side-channel gain. 0 = mono, 1 = unchanged, 2 = doubled side.
    pub width: f32,
    /// Apply a high-pass to the side channel before width scaling?
    pub side_hpf_on: bool,
    /// Side HPF cutoff frequency.
    pub side_hpf_hz: f32,
}

impl Default for ImagerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 1.0,
            side_hpf_on: false,
            side_hpf_hz: 120.0,
        }
    }
}

pub struct Imager {
    sample_rate: f32,
    side_hpf: Biquad,
    /// Last cutoff used to configure the HPF — avoids recomputing
    /// biquad coefficients every block when the param hasn't moved.
    /// Tracked on *every* block, enabled or not, so a cutoff moved
    /// while the stage is off is live the moment it comes back on.
    cached_hpf_hz: f32,
    /// Per-sample width smoother so a width step ramps instead of
    /// clicking. `width_tgt` mirrors the last requested target (see
    /// [`retarget`]).
    width_sm: Smoother,
    width_tgt: f32,
    /// Enable crossfade: blends dry against the M/S-processed signal,
    /// ramping 0 ↔ 1 on toggles so switching the stage mid-signal
    /// fades rather than steps.
    enable_sm: Smoother,
    enable_tgt: f32,
    /// Has any audio streamed through since construction/reset? An
    /// enable on the very first block engages instantly (there is no
    /// audio history to click against); a later enable crossfades in.
    primed: bool,
    was_enabled: bool,
    /// Was the side HPF actually running last block? Used to reset the
    /// biquad's streaming state when filtering (re)starts, so a
    /// re-enable doesn't replay a transient out of stale state.
    was_filtering: bool,
}

impl Imager {
    pub fn new(sample_rate: f32) -> Self {
        let mut width_sm = Smoother::new(SmoothingStyle::Linear(RAMP_MS));
        width_sm.set_sample_rate(sample_rate);
        let mut enable_sm = Smoother::new(SmoothingStyle::Linear(RAMP_MS));
        enable_sm.set_sample_rate(sample_rate);
        Self {
            sample_rate,
            side_hpf: Biquad::identity(),
            cached_hpf_hz: 0.0,
            width_sm,
            width_tgt: f32::NAN,
            enable_sm,
            enable_tgt: f32::NAN,
            primed: false,
            was_enabled: false,
            was_filtering: false,
        }
    }

    pub fn reset(&mut self) {
        self.side_hpf.reset();
        self.width_sm.reset(0.0);
        self.width_tgt = f32::NAN;
        self.enable_sm.reset(0.0);
        self.enable_tgt = f32::NAN;
        self.primed = false;
        self.was_enabled = false;
        self.was_filtering = false;
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &ImagerConfig) {
        // Track the cutoff unconditionally — before any early return —
        // so a value set while the stage (or the HPF) is off never
        // leaves stale coefficients behind for the next enable.
        if (self.cached_hpf_hz - cfg.side_hpf_hz).abs() > 0.5 {
            self.side_hpf
                .set_high_pass(self.sample_rate, cfg.side_hpf_hz.max(20.0), 0.707);
            self.cached_hpf_hz = cfg.side_hpf_hz;
        }

        let width = cfg.width.clamp(0.0, 2.0);
        if cfg.enabled && !self.was_enabled {
            // (Re)engage. The width smoother snaps to the current
            // value — ramping in from whatever it held when the stage
            // was last audible would be meaningless.
            self.width_sm.reset(width);
            self.width_tgt = width;
            if !self.primed {
                // Very first block: engage instantly, there is no
                // running audio to click against.
                self.enable_sm.reset(1.0);
                self.enable_tgt = 1.0;
            }
        } else {
            retarget(&mut self.width_sm, &mut self.width_tgt, width);
        }
        let enable_target = if cfg.enabled { 1.0 } else { 0.0 };
        retarget(&mut self.enable_sm, &mut self.enable_tgt, enable_target);
        self.was_enabled = cfg.enabled;
        self.primed = true;

        // Fully faded out: the stage is a wire, bit-identical to a
        // hard bypass once the disable crossfade has finished.
        let active = cfg.enabled || self.enable_sm.current() > 0.0;
        if !active {
            self.was_filtering = false;
            return;
        }

        let filtering = cfg.side_hpf_on;
        if filtering && !self.was_filtering {
            // Filtering (re)starts: the biquad's z1/z2 still hold the
            // side signal from before the bypass, which would replay
            // as a transient. Start from silence instead.
            self.side_hpf.reset();
        }
        self.was_filtering = filtering;

        let frames = left.len().min(right.len());
        for i in 0..frames {
            let l = left[i];
            let r = right[i];
            let mid = 0.5 * (l + r);
            let mut side = 0.5 * (l - r);
            if filtering {
                side = self.side_hpf.process(side);
            }
            side *= self.width_sm.next();
            let e = self.enable_sm.next();
            let pl = mid + side;
            let pr = mid - side;
            left[i] = l + (pl - l) * e;
            right[i] = r + (pr - r) * e;
        }
    }
}

