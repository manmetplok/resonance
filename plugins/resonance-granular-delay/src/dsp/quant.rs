//! The plugin-side quantized-transpose draw (ba todo #1078): the RNG
//! and slicing that let each short render slice latch its own quantized
//! effective transpose, so grains spawned within one block land on
//! (near-)independent lattice values.

use resonance_dsp::SimpleRng;

/// Seed for the plugin-side quantized-transpose draws (ba todo #1078).
pub(super) const QUANT_SEED: u64 = 0x0AB5_C41E;

/// Engine-render slice length while pitch quantization is active, in
/// samples: the block is processed in slices no longer than this, each
/// with its own independently drawn, quantized effective transpose, so
/// grains latch (near-)independent quantized values at spawn. The
/// slice is far shorter than the minimum inter-onset time at maximum
/// density (480 samples at 100 grains/s, 48 kHz), so two grains almost
/// never share a draw. Engine output is slice-invariant (onset
/// scheduling carries across process calls), so quantize-off behaviour
/// is untouched.
pub(super) const QUANT_SLICE: usize = 64;

/// The plugin-side quantized-transpose RNG (ba todo #1078): while
/// quantization is on, the engines' own detune draw is bypassed (spread
/// passed as 0) and the effective transpose is drawn and quantized
/// here, per render slice.
pub(super) struct QuantDraw {
    rng: SimpleRng,
    /// Sign of the next quantized-spread draw; alternates like the
    /// engine's detune sign so the quantized cloud stays symmetric.
    sign: f32,
}

impl QuantDraw {
    pub(super) fn new() -> Self {
        Self {
            rng: SimpleRng::new(QUANT_SEED),
            sign: 1.0,
        }
    }

    pub(super) fn clear(&mut self) {
        self.rng = SimpleRng::new(QUANT_SEED);
        self.sign = 1.0;
    }

    /// Uniform draw in `[0, 1)` for the plugin-side quantized-spread
    /// magnitude (same mapping as the engine's own RNG draws).
    fn unit(&mut self) -> f32 {
        (self.rng.next_u32() >> 8) as f32 * (1.0 / (1 << 24) as f32)
    }

    /// One slice's detune offset in semitones, drawn from `spread_cents`
    /// with the alternating sign that keeps the quantized cloud
    /// symmetric around the base transpose.
    pub(super) fn next_spread_semitones(&mut self, spread_cents: f32) -> f32 {
        let magnitude = self.unit() * spread_cents;
        let offset = self.sign * magnitude * (1.0 / 100.0);
        self.sign = -self.sign;
        offset
    }
}
