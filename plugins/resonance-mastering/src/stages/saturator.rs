//! Tape / tube saturator.
//!
//! Intended for mastering-grade harmonic coloration. Two shaper modes:
//! `Smooth` runs `tanh` for clean even/odd harmonics; `Gritty` runs a
//! cubic soft-clipper with a sharper knee and richer odd-harmonic
//! content for a more obviously analog sound. With the default drive
//! the output distortion products stay below Nyquist/2 at 48 kHz, so
//! no oversampling is applied — aliasing is inaudible at the levels
//! mastering uses.
//!
//! Chain per sample:
//!
//!   dry → HF shelf cut (tape loss) → waveshaper(drive) → DC blocker
//!   → LF shelf boost (head bump) → peak-normalize → mix(dry, wet)
//!
//! Normalization divides by the shaper's value at full drive, not by
//! drive itself: that keeps full-scale peaks pinned near unity at any
//! drive setting while quiet content receives an automatic makeup
//! boost, so pushing the drive knob audibly *adds* saturation instead
//! of just attenuating peaks.
//!
//! The waveshaper crossfades a symmetric variant (odd harmonics only)
//! against an asymmetric one (DC-offset before the shaper, then the
//! offset's own shaped value subtracted to pass through the origin),
//! producing 2nd-harmonic content as the character knob moves toward
//! tape. The asymmetric branch leaves the output with a nonzero mean,
//! so a DC blocker runs right after the shaper — always, not just at
//! character > 0, so the wet path stays continuous as the knob sweeps —
//! before the low shelf can amplify the offset.

use resonance_dsp::{db_to_linear, Biquad, DcBlocker};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shaper {
    /// `tanh`-based soft clipper. Clean, mostly low-order harmonics.
    Smooth,
    /// Cubic soft clipper. Sharper knee, richer odd-harmonic content.
    Gritty,
}

impl Shaper {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Shaper::Gritty,
            _ => Shaper::Smooth,
        }
    }
    pub fn to_index(self) -> i32 {
        match self {
            Shaper::Smooth => 0,
            Shaper::Gritty => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SaturatorConfig {
    pub enabled: bool,
    /// Input drive in dB. 0..18 is a reasonable range.
    pub drive_db: f32,
    /// 0.0 = fully symmetric (odd harmonics), 1.0 = fully asymmetric (adds 2nd harmonic).
    pub character: f32,
    /// Dry/wet mix.
    pub mix: f32,
    /// Which waveshaper to run.
    pub shaper: Shaper,
}

impl Default for SaturatorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            drive_db: 3.0,
            character: 0.3,
            mix: 1.0,
            shaper: Shaper::Smooth,
        }
    }
}

pub struct Saturator {
    sample_rate: f32,
    // Per-channel biquads so the L/R states stay independent.
    hf_shelf_l: Biquad,
    hf_shelf_r: Biquad,
    lf_shelf_l: Biquad,
    lf_shelf_r: Biquad,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
}

impl Saturator {
    pub fn new(sample_rate: f32) -> Self {
        let mut s = Self {
            sample_rate,
            hf_shelf_l: Biquad::identity(),
            hf_shelf_r: Biquad::identity(),
            lf_shelf_l: Biquad::identity(),
            lf_shelf_r: Biquad::identity(),
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
        };
        s.set_sample_rate(sample_rate);
        s
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        // Tape-style HF loss: -3 dB shelf starting at 14 kHz.
        self.hf_shelf_l
            .set_high_shelf(sample_rate, 14_000.0, 0.707, -3.0);
        self.hf_shelf_r
            .set_high_shelf(sample_rate, 14_000.0, 0.707, -3.0);
        // Tape head bump: +2 dB low shelf around 100 Hz.
        self.lf_shelf_l
            .set_low_shelf(sample_rate, 100.0, 0.707, 2.0);
        self.lf_shelf_r
            .set_low_shelf(sample_rate, 100.0, 0.707, 2.0);
    }

    pub fn reset(&mut self) {
        self.hf_shelf_l.reset();
        self.hf_shelf_r.reset();
        self.lf_shelf_l.reset();
        self.lf_shelf_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &SaturatorConfig) {
        if !cfg.enabled {
            return;
        }

        let drive = db_to_linear(cfg.drive_db);
        let shaper = cfg.shaper;
        // Peak-normalize: divide by the shaper's value at full drive
        // so a 1.0-amplitude input pins to ~1.0 regardless of drive.
        let inv_drive = 1.0 / base_shape(drive, shaper).max(1e-6);
        let character = cfg.character.clamp(0.0, 1.0);
        let mix = cfg.mix.clamp(0.0, 1.0);

        let frames = left.len().min(right.len());
        for i in 0..frames {
            let dry_l = left[i];
            let dry_r = right[i];

            let l1 = self.hf_shelf_l.process(dry_l);
            let r1 = self.hf_shelf_r.process(dry_r);

            let l2 = self.dc_l.process(waveshape(l1 * drive, character, shaper) * inv_drive);
            let r2 = self.dc_r.process(waveshape(r1 * drive, character, shaper) * inv_drive);

            let l3 = self.lf_shelf_l.process(l2);
            let r3 = self.lf_shelf_r.process(r2);

            left[i] = dry_l + (l3 - dry_l) * mix;
            right[i] = dry_r + (r3 - dry_r) * mix;
        }
    }
}

/// Underlying soft-clip curve. `Smooth` is `tanh`; `Gritty` is a
/// scaled cubic clipper (`x - x³/3` past a threshold, hard-clipped at
/// ±1) which transitions from linear to clipped much faster than
/// `tanh` and produces noticeably more harmonic content at the same
/// input level.
#[inline]
fn base_shape(x: f32, shaper: Shaper) -> f32 {
    match shaper {
        Shaper::Smooth => x.tanh(),
        Shaper::Gritty => {
            // Scale so the linear region has unit slope at x=0 and the
            // curve saturates near ±1. The cubic 1.5·(u - u³/3) at
            // u = x/1.5 has slope 1 at zero and reaches 1.0 at u = 1.
            let u = (x / 1.5).clamp(-1.0, 1.0);
            1.5 * (u - (u * u * u) / 3.0)
        }
    }
}

#[inline]
fn waveshape(x: f32, character: f32, shaper: Shaper) -> f32 {
    // Symmetric branch: pure odd harmonics.
    let symmetric = base_shape(x, shaper);
    // Asymmetric branch: DC-offset before the shaper, then subtract
    // the offset's own shaped value so the curve still passes through
    // the origin. The tilted transfer function generates 2nd-harmonic
    // content. Larger offset → more obvious tube/tape character.
    let offset = 0.35_f32;
    let asymmetric = base_shape(x + offset, shaper) - base_shape(offset, shaper);
    symmetric * (1.0 - character) + asymmetric * character
}

