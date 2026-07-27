//! Analysis/design window functions.
//!
//! Besides the fixed Hann helpers this module provides [`WindowMorph`], a
//! grain-envelope generator that morphs boxcar → trapezoid → Hann with a
//! single `texture` parameter (a Tukey window with variable taper, the same
//! idea as the Clouds/Beads "smoothness" morph). It is a granulation
//! primitive: the granular delay (epic #196) and the granular instrument
//! (epic #75) both evaluate it per grain sample on the audio thread.

/// Fill `window` with a symmetric Hann window:
/// `w[i] = 0.5 − 0.5·cos(τ·i / (len − 1))`. Endpoints are zero; for
/// odd lengths the centre tap is exactly 1.
pub fn fill_hann_window(window: &mut [f32]) {
    let len = window.len();
    for (i, w) in window.iter_mut().enumerate() {
        let x = i as f32 / (len as f32 - 1.0);
        *w = 0.5 - 0.5 * (std::f32::consts::TAU * x).cos();
    }
}

/// Allocate a symmetric Hann window of `len` coefficients.
pub fn hann_window(len: usize) -> Vec<f32> {
    let mut window = vec![0.0; len];
    fill_hann_window(&mut window);
    window
}

/// Number of entries in the [`WindowMorph`] lookup table.
const WINDOW_MORPH_LUT_LEN: usize = 4096;

/// Boxcar → trapezoid → Hann morphable grain window (Tukey with variable
/// taper).
///
/// `texture ∈ [0, 1]` sets the total taper fraction of the window:
///
/// * `texture = 0` — boxcar (flat, no taper);
/// * `0 < texture < 1` — raised-cosine-edged trapezoid: a flat plateau of
///   width `1 − texture` with half-Hann ramps of width `texture / 2` on
///   each side, so every `texture > 0` still reaches zero at both
///   endpoints (click-free grain onsets);
/// * `texture = 1` — the plateau vanishes and the shape is exactly the
///   Hann window of [`hann_window`].
///
/// The raised-cosine ramp is precomputed into a 4096-entry LUT at
/// construction and evaluated with linear interpolation, so
/// [`evaluate`](Self::evaluate) performs no allocation and no
/// transcendental math — safe for per-sample use on the audio thread.
#[derive(Debug, Clone)]
pub struct WindowMorph {
    /// Half-Hann rise `0.5 − 0.5·cos(π·x)` sampled at `x = i / (N − 1)`,
    /// so `table[0] = 0` and `table[N − 1] = 1`.
    table: Vec<f32>,
}

impl WindowMorph {
    /// Build the morphable window, precomputing the taper LUT.
    pub fn new() -> Self {
        let mut table = vec![0.0_f32; WINDOW_MORPH_LUT_LEN];
        for (i, t) in table.iter_mut().enumerate() {
            let x = i as f32 / (WINDOW_MORPH_LUT_LEN - 1) as f32;
            *t = 0.5 - 0.5 * (std::f32::consts::PI * x).cos();
        }
        Self { table }
    }

    /// Evaluate the window at `phase ∈ [0, 1]` for taper `texture ∈ [0, 1]`.
    ///
    /// Out-of-range inputs are handled defensively: `phase` outside
    /// `[0, 1]` returns `0.0` (a grain past its span is silent), `texture`
    /// is clamped to `[0, 1]`, and non-finite inputs return `0.0`.
    /// Allocation-free and lock-free.
    pub fn evaluate(&self, phase: f32, texture: f32) -> f32 {
        if !phase.is_finite() || !texture.is_finite() || !(0.0..=1.0).contains(&phase) {
            return 0.0;
        }
        let texture = texture.clamp(0.0, 1.0);
        if texture <= 0.0 {
            return 1.0;
        }
        // Distance to the nearer endpoint; the window is symmetric, so a
        // single rising ramp covers both edges.
        let edge = phase.min(1.0 - phase);
        let half_taper = 0.5 * texture;
        if edge >= half_taper {
            return 1.0;
        }
        // Normalised ramp position in [0, 1): 0 at the endpoint, 1 where
        // the plateau starts. At texture = 1 this makes the whole window
        // `0.5 − 0.5·cos(2π·phase)` — the Hann shape.
        self.lookup(edge / half_taper)
    }

    /// Linear-interpolated LUT read of the half-Hann rise at `x ∈ [0, 1]`.
    fn lookup(&self, x: f32) -> f32 {
        let pos = x * (WINDOW_MORPH_LUT_LEN - 1) as f32;
        let idx = (pos as usize).min(WINDOW_MORPH_LUT_LEN - 2);
        let frac = pos - idx as f32;
        let a = self.table[idx];
        let b = self.table[idx + 1];
        a + (b - a) * frac
    }
}

impl Default for WindowMorph {
    fn default() -> Self {
        Self::new()
    }
}
