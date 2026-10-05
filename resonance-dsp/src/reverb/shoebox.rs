//! Image-source early reflections for a rectangular room (Allen & Berkley),
//! first and second order.
//!
//! The room spans `[0, X] × [0, Y] × [0, Z]` metres. The listener faces +y,
//! with +x to their right and +z up. Along one axis, a source at `s` in a
//! room of length `L` has the images
//!
//! - first order: `−s` (the wall at 0) and `2L − s` (the wall at `L`);
//! - second order on the same axis: `s − 2L` and `s + 2L`.
//!
//! The image set is therefore
//!
//! - **first order: 6 taps**, one per wall;
//! - **second order: 18 taps**: 6 that hit the same axis's two walls
//!   (one per axis × 2 directions) and 12 that hit walls on two different
//!   axes (3 axis pairs × 2 × 2 walls),
//!
//! [`MAX_TAPS`] = 24 in all. The direct path is not a tap: it is the dry
//! signal, and a reverb engine replaces it with its pre-delay. Every tap's
//! delay is the absolute path length over the speed of sound; engines that
//! want delays relative to the direct sound subtract
//! [`ShoeboxEr::direct_delay_samples`].
//!
//! **Gain** is frequency independent: `β^k · d_direct / r`, with `k` the
//! number of reflections, `β = √(1 − α)` the pressure reflection
//! coefficient of a wall with energy absorption `α`, and `r` the path
//! length. A tap at the direct distance with no wall loss would read 1.0.
//!
//! **Stereo** is angle based, not two ears: the tap's azimuth from the
//! listener (`sin θ = dx / r_horizontal`) drives a constant-power pan, so a
//! reflection from the right wall lands mostly right and one straight ahead
//! or behind lands centred with `gain_l = gain_r = gain · cos(π/4)`. There
//! is no interaural time difference (one delay per tap); the late tail's
//! decorrelation does the widening.

use crate::constant_power_pan;

/// 6 first-order + 18 second-order image sources.
pub const MAX_TAPS: usize = 24;
/// First-order tap count.
pub const FIRST_ORDER_TAPS: usize = 6;

/// One reflection: delay from emission (samples, fractional) and the
/// per-channel gains.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ErTap {
    pub delay_samples: f32,
    pub gain_l: f32,
    pub gain_r: f32,
    /// Reflections on the path (1 or 2).
    pub order: u8,
}

/// A shoebox room with a source and a listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShoeboxEr {
    /// Room dimensions `(x, y, z)`, metres.
    pub room: [f32; 3],
    pub source: [f32; 3],
    pub listener: [f32; 3],
    /// Wall energy absorption `α ∈ [0, 1)`, the same on every wall.
    pub absorption: f32,
    /// 1 = first order only (6 taps), 2 = first and second (24).
    pub max_order: u8,
    /// Metres per second; 343 by default.
    pub speed_of_sound: f32,
}

impl ShoeboxEr {
    pub fn new(room: [f32; 3], source: [f32; 3], listener: [f32; 3], absorption: f32) -> Self {
        Self {
            room,
            source,
            listener,
            absorption,
            max_order: 2,
            speed_of_sound: 343.0,
        }
    }

    /// The same room with every dimension and position scaled by `factor`
    /// (a `size` knob: same shape, bigger room).
    pub fn scaled(&self, factor: f32) -> Self {
        let s = |v: [f32; 3]| v.map(|c| c * factor);
        Self {
            room: s(self.room),
            source: s(self.source),
            listener: s(self.listener),
            ..*self
        }
    }

    pub fn tap_count(&self) -> usize {
        if self.max_order >= 2 {
            MAX_TAPS
        } else {
            FIRST_ORDER_TAPS
        }
    }

    pub fn direct_distance(&self) -> f32 {
        dist(self.source, self.listener).max(1e-3)
    }

    pub fn direct_delay_samples(&self, sample_rate: f32) -> f32 {
        self.direct_distance() / self.speed_of_sound * sample_rate
    }

    /// Write the taps, sorted by delay, into `out` and return how many
    /// (`min(tap_count(), out.len())`). No allocation: safe on any thread.
    pub fn write_taps(&self, sample_rate: f32, out: &mut [ErTap]) -> usize {
        let beta = (1.0 - self.absorption.clamp(0.0, 0.999)).sqrt();
        let d_ref = self.direct_distance();
        let mut n = 0;
        let mut push = |pos: [f32; 3], order: u8| {
            if n >= out.len() {
                return;
            }
            let r = dist(pos, self.listener).max(1e-3);
            let gain = beta.powi(order as i32) * d_ref / r;
            let dx = pos[0] - self.listener[0];
            let dy = pos[1] - self.listener[1];
            let horiz = (dx * dx + dy * dy).sqrt();
            let pan = if horiz > 1e-6 { dx / horiz } else { 0.0 };
            let (l, rr) = constant_power_pan(pan);
            out[n] = ErTap {
                delay_samples: r / self.speed_of_sound * sample_rate,
                gain_l: gain * l,
                gain_r: gain * rr,
                order,
            };
            n += 1;
        };
        let s = self.source;
        let l = self.room;
        // First order: one wall.
        for axis in 0..3 {
            for img in [-s[axis], 2.0 * l[axis] - s[axis]] {
                let mut p = s;
                p[axis] = img;
                push(p, 1);
            }
        }
        if self.max_order >= 2 {
            // Second order, same axis: both walls of one axis.
            for axis in 0..3 {
                for img in [s[axis] - 2.0 * l[axis], s[axis] + 2.0 * l[axis]] {
                    let mut p = s;
                    p[axis] = img;
                    push(p, 2);
                }
            }
            // Second order, two axes: one wall on each.
            for (a, b) in [(0, 1), (0, 2), (1, 2)] {
                for ia in [-s[a], 2.0 * l[a] - s[a]] {
                    for ib in [-s[b], 2.0 * l[b] - s[b]] {
                        let mut p = s;
                        p[a] = ia;
                        p[b] = ib;
                        push(p, 2);
                    }
                }
            }
        }
        // Insertion sort by delay (≤ 24 entries).
        for i in 1..n {
            let mut j = i;
            while j > 0 && out[j - 1].delay_samples > out[j].delay_samples {
                out.swap(j - 1, j);
                j -= 1;
            }
        }
        n
    }

    /// The taps sorted by delay, allocated (main-thread convenience).
    pub fn taps(&self, sample_rate: f32) -> Vec<ErTap> {
        let mut buf = [ErTap::default(); MAX_TAPS];
        let n = self.write_taps(sample_rate, &mut buf);
        buf[..n].to_vec()
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
