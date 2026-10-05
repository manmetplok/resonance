//! The Hall's early reflections: a thinned image-source pattern of a large
//! shoebox, spread over 30–120 ms.
//!
//! [`ShoeboxEr`] gives a concert-hall-shaped room's first- and
//! second-order reflections (gains and pans from the geometry). The floor
//! bounce and anything else within a few ms of the direct sound is
//! dropped (it fuses with the dry signal and only colours it), the rest
//! is thinned to every other tap, and the survivors' arrival times are
//! remapped linearly onto 30–120 ms (at `er_time` 0.5 and `size` 0.5),
//! keeping the room's order and relative spacing. That is the "sparse,
//! widely spread" hall ER of reverb-algorithms.md §2.1: a handful of
//! distinct reflections, not a cluster.
//!
//! The gains taper linearly to 30 % across the window (the later
//! reflections hand over to the late field) and are normalised to an
//! energy of 0.25 per side before `er_level`.
//!
//! `er_time` scales the spacing 0.5×–1.5×, `size` 0.8×–1.2×. A spacing
//! change glides each tap (linear-interpolated read, slewed at the Hall's
//! `GLIDE`): no click, a short Doppler bend while it moves.
//! The right channel's taps sit 1.7 % + 0.3 ms later than the left's, so a
//! reflection straight ahead does not arrive identically on both sides.

use resonance_dsp::reverb::{ErTap, ShoeboxEr, MAX_TAPS};
use resonance_dsp::DelayLine;

use super::super::super::er::ER_TAPS;
use super::glide;

/// First and last reflection at unit spacing, ms after the direct sound.
const FIRST_MS: f32 = 30.0;
const LAST_MS: f32 = 120.0;
/// Largest spacing multiplier: `er_time` 1 × `size` 1.
const MAX_SCALE: f32 = 1.5 * 1.2;
/// Reflections closer than this to the direct sound are dropped.
const FUSE_MS: f32 = 3.0;
/// Per-channel energy of the tap set before `er_level` (Σg² per side).
const ENERGY: f32 = 0.25;
/// Gain taper across the window: the last reflection is `1 − TAPER` of
/// what the geometry gives it, relative to the first.
const TAPER: f32 = 0.7;

pub(super) struct HallEr {
    sample_rate: f32,
    line_l: DelayLine,
    line_r: DelayLine,
    /// Tap times at unit spacing, ms (L, R).
    base_ms: [(f32, f32); ER_TAPS],
    gains: [(f32, f32); ER_TAPS],
    /// Current read positions and their targets, samples (L, R).
    pos: [(f32, f32); ER_TAPS],
    target: [(f32, f32); ER_TAPS],
    max_pos: f32,
    level: f32,
    time_scale: f32,
    size_scale: f32,
}

impl HallEr {
    pub(super) fn new(sample_rate: f32) -> Self {
        // A 34 × 46 × 17 m hall, source on stage, listener mid-stalls.
        let room = ShoeboxEr::new(
            [34.0, 46.0, 17.0],
            [15.0, 9.0, 1.6],
            [19.5, 27.0, 1.2],
            0.18,
        );
        let mut all = [ErTap::default(); MAX_TAPS];
        let n = room.write_taps(sample_rate, &mut all);
        let direct = room.direct_delay_samples(sample_rate);
        let to_ms = 1000.0 / sample_rate;
        let mut kept = [ErTap::default(); MAX_TAPS];
        let mut k = 0;
        for t in &all[..n] {
            if (t.delay_samples - direct) * to_ms >= FUSE_MS {
                kept[k] = *t;
                k += 1;
            }
        }
        // Every other one, from the first: 12 of the ~22 that survive.
        let mut taps = [ErTap::default(); ER_TAPS];
        let mut m = 0;
        for t in kept[..k].iter().step_by(2) {
            if m == ER_TAPS {
                break;
            }
            taps[m] = *t;
            m += 1;
        }
        let rel = |t: &ErTap| (t.delay_samples - direct) * to_ms;
        let (r0, r1) = (
            rel(&taps[0]),
            rel(&taps[m.max(1) - 1]).max(rel(&taps[0]) + 1.0),
        );
        let mut base_ms = [(0.0, 0.0); ER_TAPS];
        let mut gains = [(0.0, 0.0); ER_TAPS];
        let mut energy = 0.0f32;
        for i in 0..m {
            let pos = (rel(&taps[i]) - r0) / (r1 - r0);
            let ms = FIRST_MS + (LAST_MS - FIRST_MS) * pos;
            base_ms[i] = (ms, ms * 1.017 + 0.3);
            // The later reflections hand over to the late field.
            let taper = 1.0 - TAPER * pos;
            gains[i] = (taps[i].gain_l * taper, taps[i].gain_r * taper);
            energy += 0.5 * (gains[i].0.powi(2) + gains[i].1.powi(2));
        }
        let norm = (ENERGY / energy.max(1e-9)).sqrt();
        for g in &mut gains {
            *g = (g.0 * norm, g.1 * norm);
        }
        let max_pos = ((LAST_MS * 1.017 + 0.3) * MAX_SCALE * 0.001 * sample_rate).ceil() + 4.0;
        let mut er = Self {
            sample_rate,
            line_l: DelayLine::new(max_pos as usize + 4),
            line_r: DelayLine::new(max_pos as usize + 4),
            base_ms,
            gains,
            pos: [(0.0, 0.0); ER_TAPS],
            target: [(0.0, 0.0); ER_TAPS],
            max_pos,
            level: 0.4,
            time_scale: 1.0,
            size_scale: 1.0,
        };
        er.retarget();
        er.snap();
        er
    }

    pub(super) fn set_level(&mut self, norm: f32) {
        self.level = norm.clamp(0.0, 1.0);
    }

    /// `er_time` 0..1 → spacing 0.5×–1.5×. Returns whether it moved.
    pub(super) fn set_time(&mut self, norm: f32) -> bool {
        let s = 0.5 + norm.clamp(0.0, 1.0);
        let moved = s != self.time_scale;
        self.time_scale = s;
        moved
    }

    /// `size` 0..1 → spacing 0.8×–1.2×. Returns whether it moved.
    pub(super) fn set_size(&mut self, norm: f32) -> bool {
        let s = 0.8 + 0.4 * norm.clamp(0.0, 1.0);
        let moved = s != self.size_scale;
        self.size_scale = s;
        moved
    }

    /// Recompute the tap targets from the two spacing factors.
    pub(super) fn retarget(&mut self) {
        let k = self.time_scale * self.size_scale * 0.001 * self.sample_rate;
        for (t, b) in self.target.iter_mut().zip(&self.base_ms) {
            *t = ((b.0 * k).min(self.max_pos), (b.1 * k).min(self.max_pos));
        }
    }

    /// Put every read head on its target (fresh/reset, nothing to glide).
    pub(super) fn snap(&mut self) {
        self.pos = self.target;
    }

    #[inline]
    pub(super) fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.line_l.push(l);
        self.line_r.push(r);
        let (mut out_l, mut out_r) = (0.0f32, 0.0f32);
        for i in 0..ER_TAPS {
            let (p, t) = (&mut self.pos[i], self.target[i]);
            glide(&mut p.0, t.0);
            glide(&mut p.1, t.1);
            out_l += self.gains[i].0 * self.line_l.tap_linear(p.0);
            out_r += self.gains[i].1 * self.line_r.tap_linear(p.1);
        }
        (out_l * self.level, out_r * self.level)
    }

    pub(super) fn clear(&mut self) {
        self.line_l.clear();
        self.line_r.clear();
        self.snap();
    }

    /// Current tap times, ms (L, R), for the editor.
    pub(super) fn tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        let k = 1000.0 / self.sample_rate;
        self.target.map(|(l, r)| (l * k, r * k))
    }

    /// Tap gains (L, R) including `er_level`, for the editor.
    pub(super) fn tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        self.gains.map(|(l, r)| (l * self.level, r * self.level))
    }

    /// Bytes held in delay buffers.
    pub(super) fn buffer_bytes(&self) -> usize {
        2 * (self.max_pos as usize + 4).next_power_of_two() * std::mem::size_of::<f32>()
    }
}
