//! Shoebox early reflections for the room family (Room, Chamber,
//! Ambience): two ears in an image-source box, a short scattering
//! diffuser per ear, and a crossfade whenever the geometry changes.
//!
//! **Geometry.** The box's dimensions are interpolated log-linearly per
//! axis from [`EarlyVoicing::small`] (`size` 0) to [`EarlyVoicing::large`]
//! (`size` 1). Source and listener sit at fixed *fractions* of the box,
//! off-centre on every axis so no two image paths coincide (a centred
//! pair folds the 24 images onto a handful of delays: a comb, not a
//! room). The listener is two ears, ±9 cm along x (not scaled with the
//! box): each ear gets its own image set, so the reflections carry a real
//! interaural time difference on top of `ShoeboxEr`'s angle-based level
//! pan. The left ear's taps read the left input, the right ear's the
//! right input.
//!
//! **Delays** are relative to the direct path (the pre-delay replaces the
//! direct sound), measured from the centre between the ears, then scaled
//! by `er_time`: `2^(2·er_time − 1)`, so 0.5 is the physical room, 0 half
//! the spacing, 1 double. Rounded to whole samples: the taps never move
//! while sounding (a geometry change crossfades), so a fractional read
//! would buy nothing.
//!
//! **Level.** Second-order taps are weighted by
//! [`EarlyVoicing::second_order_gain`] (Chamber's "denser" ER) and every
//! tap by the decay envelope at its delay (`−60 dB` per
//! `decay × EarlyVoicing::envelope_scale`: on Room and Chamber the walls
//! absorb at the rate the tail decays, so the cluster does not read as a
//! second, faster slope in front of the tail; Ambience shortens it on
//! purpose). Then each ear's
//! set is normalised to unit energy, so the ER loudness is
//! `er_level × EarlyVoicing::level` at every size, spacing and decay. The
//! physical 1/r and wall loss shape the pattern, not its level.
//!
//! **Scatter.** Each ear's tap sum runs through three short Schroeder
//! allpasses (0.2–4 ms, per voicing): real walls scatter, and a bare
//! tap train reads as discrete clicks and keeps the echo density low.
//! Allpasses keep the energy, so the level above still holds.
//!
//! **Geometry changes** (`size`, `er_time`) never move a sounding tap:
//! the whole old set fades out while the new one fades in over
//! `FADE_MS`, linearly, with a change landing mid-fade queued (newest
//! wins) — the pattern `dsp/er.rs` uses. Before the first sample (or
//! after `clear`) a change snaps.

use resonance_dsp::reverb::{Allpass, ErTap, ShoeboxEr, MAX_TAPS};
use resonance_dsp::DelayLine;

use super::super::super::er::ER_TAPS;

/// Crossfade length for a geometry change, ms.
const FADE_MS: f32 = 20.0;
/// Half the distance between the ears, metres.
const EAR_OFFSET_M: f32 = 0.09;
/// Largest `er_time` spacing multiplier (`er_time` = 1).
const TIME_SCALE_MAX: f32 = 2.0;
/// Allpass stages in each ear's scattering diffuser.
pub(in crate::dsp::algo) const SCATTER_STAGES: usize = 3;

/// The fixed, per-algorithm character of the reflections.
#[derive(Clone, Copy, Debug)]
pub(in crate::dsp::algo) struct EarlyVoicing {
    /// Box dimensions `(x, y, z)` at `size` 0 and 1, metres.
    pub small: [f32; 3],
    pub large: [f32; 3],
    /// Source and listener (the point between the ears) as fractions of
    /// the box.
    pub source: [f32; 3],
    pub listener: [f32; 3],
    /// Wall energy absorption, every wall.
    pub absorption: f32,
    /// Weight on the second-order taps before normalisation.
    pub second_order_gain: f32,
    /// Scattering allpass lengths per ear, ms.
    pub scatter_ms: [[f32; SCATTER_STAGES]; 2],
    /// Scattering allpass coefficient.
    pub scatter_gain: f32,
    /// Output level at `er_level` 1.
    pub level: f32,
    /// The decay the tap envelope follows, as a fraction of `decay`
    /// (1: the room's own rate). Ambience shortens it so its cluster dies
    /// faster than its tail.
    pub envelope_scale: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Tap {
    delay: usize,
    /// The image's physical weight (1/r, wall loss, order weight).
    base: f32,
    /// `base` under the decay envelope, normalised: what is applied.
    gain: f32,
}

type TapSet = [[Tap; MAX_TAPS]; 2];

pub(in crate::dsp::algo) struct ShoeboxEarly {
    sample_rate: f32,
    v: EarlyVoicing,
    line_l: DelayLine,
    line_r: DelayLine,
    /// The longest tap any geometry can produce, samples.
    max_delay: usize,
    taps: TapSet,
    /// The outgoing set while a crossfade runs.
    from: TapSet,
    fade_left: u32,
    fade_total: u32,
    /// Newest `(size, er_time)` requested while a fade ran.
    pending: Option<(f32, f32)>,
    /// Last `(size, er_time)` requested (dedupes per-block calls); NaN
    /// until the first.
    requested: (f32, f32),
    /// The geometry `taps` was built for.
    applied: (f32, f32),
    primed: bool,
    level: f32,
    /// The decay the tap envelope follows, seconds.
    t60: f32,
    scatter: [[Allpass; SCATTER_STAGES]; 2],
    viz_times_ms: [(f32, f32); ER_TAPS],
    viz_gains: [(f32, f32); ER_TAPS],
}

impl ShoeboxEarly {
    pub(in crate::dsp::algo) fn new(sample_rate: f32, v: EarlyVoicing) -> Self {
        // The longest image path in the largest box is shorter than twice
        // its diagonal doubled on every axis; the spacing scale and the
        // ear offset ride on top.
        let [x, y, z] = v.large;
        let reach_m = 2.0 * ((2.0 * x).powi(2) + (2.0 * y).powi(2) + (2.0 * z).powi(2)).sqrt();
        let max_delay = (reach_m / 343.0 * sample_rate * TIME_SCALE_MAX).ceil() as usize + 64;
        let scatter = std::array::from_fn(|ear| {
            std::array::from_fn(|k| {
                let d = ((v.scatter_ms[ear][k] * 0.001 * sample_rate).round() as usize).max(1);
                Allpass::new(d, d, v.scatter_gain)
            })
        });
        let mut me = Self {
            sample_rate,
            v,
            line_l: DelayLine::new(max_delay + 2),
            line_r: DelayLine::new(max_delay + 2),
            max_delay,
            taps: [[Tap::default(); MAX_TAPS]; 2],
            from: [[Tap::default(); MAX_TAPS]; 2],
            fade_left: 0,
            fade_total: ((FADE_MS * 0.001 * sample_rate) as u32).max(1),
            pending: None,
            requested: (f32::NAN, f32::NAN),
            applied: (0.5, 0.5),
            primed: false,
            level: 0.0,
            t60: 2.0,
            scatter,
            viz_times_ms: [(0.0, 0.0); ER_TAPS],
            viz_gains: [(0.0, 0.0); ER_TAPS],
        };
        me.build(0.5, 0.5);
        me
    }

    /// The box at `size`, without ears.
    fn room(&self, size: f32) -> ShoeboxEr {
        let s = size.clamp(0.0, 1.0);
        let dims: [f32; 3] =
            std::array::from_fn(|a| self.v.small[a] * (self.v.large[a] / self.v.small[a]).powf(s));
        let at = |frac: [f32; 3]| std::array::from_fn(|a| frac[a] * dims[a]);
        ShoeboxEr::new(
            dims,
            at(self.v.source),
            at(self.v.listener),
            self.v.absorption,
        )
    }

    /// Rebuild `taps` (and the viz copies) for a geometry. No allocation.
    fn build(&mut self, size: f32, er_time: f32) {
        self.applied = (size, er_time);
        let room = self.room(size);
        let scale = 2f32.powf(2.0 * er_time.clamp(0.0, 1.0) - 1.0);
        let direct = room.direct_delay_samples(self.sample_rate);
        let mut raw = [ErTap::default(); MAX_TAPS];
        for (ear, sign) in [(0usize, -1.0f32), (1, 1.0)] {
            let mut ear_room = room;
            ear_room.listener[0] += sign * EAR_OFFSET_M;
            let n = ear_room.write_taps(self.sample_rate, &mut raw);
            for (k, t) in raw[..n].iter().enumerate() {
                let rel = ((t.delay_samples - direct) * scale).max(1.0);
                let order_gain = if t.order >= 2 {
                    self.v.second_order_gain
                } else {
                    1.0
                };
                let g = order_gain * if ear == 0 { t.gain_l } else { t.gain_r };
                self.taps[ear][k] = Tap {
                    delay: (rel.round() as usize).min(self.max_delay),
                    base: g,
                    gain: 0.0,
                };
            }
        }
        self.apply_envelope();
    }

    /// Weight every tap by the decay at its delay (`−60 dB` per `t60`),
    /// then normalise each ear to unit energy. The cluster then falls at
    /// the room's own rate, so it hands over to the tail without a kink
    /// in the decay curve (a short decay in a big box would otherwise
    /// read its ERs as a second, faster slope).
    fn apply_envelope(&mut self) {
        let k = -3.0 * std::f32::consts::LN_10 / (self.t60.max(0.01) * self.sample_rate);
        for ear in &mut self.taps {
            let mut energy = 0.0f32;
            for t in ear.iter_mut() {
                t.gain = t.base * (k * t.delay as f32).exp();
                energy += t.gain * t.gain;
            }
            let norm = 1.0 / energy.max(1e-20).sqrt();
            for t in ear.iter_mut() {
                t.gain *= norm;
            }
        }
        let ms = 1000.0 / self.sample_rate;
        for k in 0..ER_TAPS {
            let (l, r) = (self.taps[0][k], self.taps[1][k]);
            self.viz_times_ms[k] = (l.delay as f32 * ms, r.delay as f32 * ms);
            self.viz_gains[k] = (l.gain, r.gain);
        }
    }

    /// Set the geometry (`size` and `er_time`, both `0..=1`). See the
    /// module docs for how a change on running audio is handled.
    pub(in crate::dsp::algo) fn set_geometry(&mut self, size: f32, er_time: f32) {
        if (size, er_time) == self.requested {
            return;
        }
        self.requested = (size, er_time);
        if !self.primed {
            self.fade_left = 0;
            self.pending = None;
            self.build(size, er_time);
        } else if self.fade_left > 0 {
            self.pending = Some((size, er_time));
        } else {
            self.begin_fade(size, er_time);
        }
    }

    fn begin_fade(&mut self, size: f32, er_time: f32) {
        self.from = self.taps;
        self.build(size, er_time);
        self.fade_left = self.fade_total;
    }

    /// The decay (mid T60, seconds) the tap envelope follows. Moves the
    /// tap gains, never their delays, so it takes effect at once.
    pub(in crate::dsp::algo) fn set_decay(&mut self, t60: f32) {
        let t60 = t60 * self.v.envelope_scale;
        if t60 != self.t60 {
            self.t60 = t60;
            self.apply_envelope();
        }
    }

    pub(in crate::dsp::algo) fn set_level(&mut self, er_level: f32) {
        self.level = er_level.clamp(0.0, 1.0) * self.v.level;
    }

    #[inline]
    fn sum(line: &DelayLine, taps: &[Tap; MAX_TAPS]) -> f32 {
        let mut acc = 0.0f32;
        for t in taps {
            acc += line.tap(t.delay) * t.gain;
        }
        acc
    }

    /// One sample in, the scattered reflections (level applied) out.
    #[inline]
    pub(in crate::dsp::algo) fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.line_l.push(l);
        self.line_r.push(r);
        self.primed = true;
        let (mut el, mut er) = (
            Self::sum(&self.line_l, &self.taps[0]),
            Self::sum(&self.line_r, &self.taps[1]),
        );
        if self.fade_left > 0 {
            self.fade_left -= 1;
            let x = (self.fade_total - self.fade_left) as f32 / self.fade_total as f32;
            let ol = Self::sum(&self.line_l, &self.from[0]);
            let or = Self::sum(&self.line_r, &self.from[1]);
            el = ol + x * (el - ol);
            er = or + x * (er - or);
            if self.fade_left == 0 {
                if let Some((s, t)) = self.pending.take() {
                    if (s, t) != self.applied {
                        self.begin_fade(s, t);
                    }
                }
            }
        }
        for ap in &mut self.scatter[0] {
            el = ap.process(el);
        }
        for ap in &mut self.scatter[1] {
            er = ap.process(er);
        }
        (el * self.level, er * self.level)
    }

    /// Back to the fresh state for the newest requested geometry.
    pub(in crate::dsp::algo) fn clear(&mut self) {
        self.line_l.clear();
        self.line_r.clear();
        for ap in self.scatter.iter_mut().flatten() {
            ap.clear();
        }
        self.fade_left = 0;
        self.pending = None;
        let (s, t) = self.requested;
        if !s.is_nan() && (s, t) != self.applied {
            self.build(s, t);
        }
        self.primed = false;
    }

    pub(in crate::dsp::algo) fn tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        self.viz_times_ms
    }

    pub(in crate::dsp::algo) fn tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        self.viz_gains
    }
}
