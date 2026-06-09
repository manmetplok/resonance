//! Bitwise-equivalence tests for `dsp::IrEngine::process_block` against a
//! verbatim reimplementation of the per-sample loop that used to live in
//! `lib.rs::process` (pre-2026-06-09, before the loop moved into the DSP
//! module). Covers the initialize-time `install` path, the fade-in-from-
//! silence path, and the fade-out -> swap -> fade-in crossfade path, with
//! audio streamed in irregular chunk sizes across convolver block
//! boundaries. Also locks in the `tap(block_size - 1)` bypass-delay
//! alignment fix.

use resonance_dsp::DelayLine;
use resonance_ir::dsp::{IrEngine, StereoConvolver, SWAP_FADE_SAMPLES};
use resonance_plugin::{Smoother, SmoothingStyle};

const BLOCK_SIZE: usize = 128;
const SAMPLE_RATE: f32 = 44_100.0;

/// Verbatim copy of the old `lib.rs` per-sample processing loop and swap
/// state machine, kept as the golden reference.
struct Reference {
    active_convolver: Option<StereoConvolver>,
    pending_convolver: Option<StereoConvolver>,
    fade_out_remaining: u32,
    fade_in_remaining: u32,
    bypass_delay_l: DelayLine,
    bypass_delay_r: DelayLine,
    block_size: usize,
}

impl Reference {
    fn new(block_size: usize) -> Self {
        Self {
            active_convolver: None,
            pending_convolver: None,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
            bypass_delay_l: DelayLine::new(block_size),
            bypass_delay_r: DelayLine::new(block_size),
            block_size,
        }
    }

    /// Old mailbox-arrival logic from the top of `process`.
    fn begin_swap(&mut self, conv: StereoConvolver) {
        self.pending_convolver = Some(conv);
        if self.active_convolver.is_some() {
            self.fade_out_remaining = SWAP_FADE_SAMPLES;
            self.fade_in_remaining = 0;
        } else {
            self.active_convolver = self.pending_convolver.take();
            self.fade_in_remaining = SWAP_FADE_SAMPLES;
        }
    }

    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        dry_wet_s: &mut Smoother,
        output_gain_s: &mut Smoother,
    ) -> (f32, f32, f32, f32) {
        let frames = left.len();
        let mut in_peak_l = 0.0f32;
        let mut in_peak_r = 0.0f32;
        let mut out_peak_l = 0.0f32;
        let mut out_peak_r = 0.0f32;

        for i in 0..frames {
            let dry_wet = dry_wet_s.next();
            let output_gain = output_gain_s.next();

            let fade_gain = if self.fade_out_remaining > 0 {
                self.fade_out_remaining -= 1;
                let g = self.fade_out_remaining as f32 / SWAP_FADE_SAMPLES as f32;
                if self.fade_out_remaining == 0 {
                    self.active_convolver = self.pending_convolver.take();
                    self.fade_in_remaining = SWAP_FADE_SAMPLES;
                }
                g
            } else if self.fade_in_remaining > 0 {
                self.fade_in_remaining -= 1;
                1.0 - self.fade_in_remaining as f32 / SWAP_FADE_SAMPLES as f32
            } else {
                1.0
            };

            let dry_l = left[i];
            let dry_r = right[i];
            in_peak_l = in_peak_l.max(dry_l.abs());
            in_peak_r = in_peak_r.max(dry_r.abs());

            let delayed_l = self.bypass_delay_l.tap(self.block_size - 1);
            let delayed_r = self.bypass_delay_r.tap(self.block_size - 1);
            self.bypass_delay_l.push(dry_l);
            self.bypass_delay_r.push(dry_r);

            match &mut self.active_convolver {
                Some(conv) => {
                    let (wet_l, wet_r) = conv.process_sample(dry_l, dry_r);

                    let dry_amount = 1.0 - dry_wet;
                    left[i] = (delayed_l * dry_amount + wet_l * dry_wet) * output_gain * fade_gain;
                    right[i] = (delayed_r * dry_amount + wet_r * dry_wet) * output_gain * fade_gain;
                }
                None => {
                    left[i] = delayed_l * output_gain * fade_gain;
                    right[i] = delayed_r * output_gain * fade_gain;
                }
            }

            out_peak_l = out_peak_l.max(left[i].abs());
            out_peak_r = out_peak_r.max(right[i].abs());
        }

        (in_peak_l, in_peak_r, out_peak_l, out_peak_r)
    }
}

/// Tiny deterministic LCG so both paths see identical "noise".
struct Lcg(u64);

impl Lcg {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32 / (1u64 << 31) as f32) - 1.0
    }
}

fn make_ir(seed: u64, len: usize) -> Vec<f32> {
    let mut rng = Lcg(seed);
    let mut ir: Vec<f32> = (0..len).map(|_| rng.next_f32() * 0.5).collect();
    ir[0] = 1.0; // strong direct tap so output is clearly non-silent
    ir
}

fn make_smoothers() -> (Smoother, Smoother) {
    let mut dry_wet = Smoother::new(SmoothingStyle::Linear(50.0));
    let mut output_gain = Smoother::new(SmoothingStyle::Logarithmic(50.0));
    dry_wet.set_sample_rate(SAMPLE_RATE);
    output_gain.set_sample_rate(SAMPLE_RATE);
    dry_wet.reset(0.3);
    output_gain.reset(0.8);
    // Ramp both so the per-sample smoother ordering is exercised.
    dry_wet.set_target(0.9);
    output_gain.set_target(1.5);
    (dry_wet, output_gain)
}

/// Streams >3 convolver hops of noise through both implementations in
/// irregular chunk sizes, swapping convolvers twice along the way
/// (fade-in from silence, then a full fade-out -> swap -> fade-in), and
/// requires bitwise-identical output and peaks throughout.
#[test]
fn block_path_matches_old_per_sample_path_bitwise() {
    assert_eq!(SWAP_FADE_SAMPLES, 64, "reference loop assumes 64-sample fade");

    let ir_a = make_ir(11, 300);
    let ir_b = make_ir(22, 513);

    let mut engine = IrEngine::new(BLOCK_SIZE);
    let mut reference = Reference::new(BLOCK_SIZE);
    let (mut eng_dw, mut eng_og) = make_smoothers();
    let (mut ref_dw, mut ref_og) = make_smoothers();

    let chunks = [1usize, 7, 128, 33, 200, 5, 64, 128, 250, 9, 128, 301, 17, 128];
    let mut input = Lcg(99);
    let mut nonsilent = false;

    for (n, &chunk) in chunks.iter().enumerate() {
        // Chunk 2: first convolver arrives (no active -> direct swap + fade-in).
        if n == 2 {
            engine.begin_swap(StereoConvolver::new(&ir_a, None, BLOCK_SIZE));
            reference.begin_swap(StereoConvolver::new(&ir_a, None, BLOCK_SIZE));
        }
        // Chunk 7: replacement arrives (active present -> fade-out, swap, fade-in).
        if n == 7 {
            engine.begin_swap(StereoConvolver::new(&ir_b, Some(&ir_b), BLOCK_SIZE));
            reference.begin_swap(StereoConvolver::new(&ir_b, Some(&ir_b), BLOCK_SIZE));
        }
        // Chunk 9: retarget the smoothers mid-stream, identically on both sides.
        if n == 9 {
            eng_dw.set_target(0.2);
            ref_dw.set_target(0.2);
            eng_og.set_target(0.6);
            ref_og.set_target(0.6);
        }

        let mut l: Vec<f32> = (0..chunk).map(|_| input.next_f32()).collect();
        let r: Vec<f32> = l.iter().map(|v| -v * 0.5).collect();
        let mut l_ref = l.clone();
        let mut r_ref = r.clone();
        let mut r = r;

        let peaks = engine.process_block(&mut l, &mut r, &mut eng_dw, &mut eng_og);
        let (ipl, ipr, opl, opr) =
            reference.process(&mut l_ref, &mut r_ref, &mut ref_dw, &mut ref_og);

        assert_eq!(l, l_ref, "left channel diverged in chunk {n}");
        assert_eq!(r, r_ref, "right channel diverged in chunk {n}");
        assert_eq!(
            (peaks.in_l, peaks.in_r, peaks.out_l, peaks.out_r),
            (ipl, ipr, opl, opr),
            "peaks diverged in chunk {n}"
        );
        nonsilent |= l.iter().any(|v| v.abs() > 1e-3);
    }

    assert!(nonsilent, "test never produced audible output");
}

/// The initialize-time `install` path (no crossfade) also matches the old
/// behaviour: convolver set directly, fade counters untouched.
#[test]
fn install_path_matches_reference() {
    let ir = make_ir(7, 256);

    let mut engine = IrEngine::new(BLOCK_SIZE);
    engine.install(StereoConvolver::new(&ir, None, BLOCK_SIZE));
    let mut reference = Reference::new(BLOCK_SIZE);
    reference.active_convolver = Some(StereoConvolver::new(&ir, None, BLOCK_SIZE));

    let (mut eng_dw, mut eng_og) = make_smoothers();
    let (mut ref_dw, mut ref_og) = make_smoothers();

    let mut input = Lcg(3);
    for n in 0..6 {
        let mut l: Vec<f32> = (0..150).map(|_| input.next_f32()).collect();
        let mut r = l.clone();
        let mut l_ref = l.clone();
        let mut r_ref = r.clone();

        engine.process_block(&mut l, &mut r, &mut eng_dw, &mut eng_og);
        reference.process(&mut l_ref, &mut r_ref, &mut ref_dw, &mut ref_og);

        assert_eq!(l, l_ref, "left channel diverged in chunk {n}");
        assert_eq!(r, r_ref, "right channel diverged in chunk {n}");
    }
}

/// With no convolver, the engine is a pure `block_size` delay (the bypass
/// path taps `block_size - 1` before pushing — the off-by-one fix).
#[test]
fn bypass_is_exact_block_size_delay() {
    let mut engine = IrEngine::new(BLOCK_SIZE);

    // Unity smoothers so the output is just the delayed input.
    let mut dry_wet = Smoother::new(SmoothingStyle::Linear(50.0));
    let mut output_gain = Smoother::new(SmoothingStyle::Linear(50.0));
    dry_wet.set_sample_rate(SAMPLE_RATE);
    output_gain.set_sample_rate(SAMPLE_RATE);
    dry_wet.reset(1.0);
    output_gain.reset(1.0);

    let total = BLOCK_SIZE * 3 + 41;
    let src: Vec<f32> = (0..total).map(|i| (i as f32 * 0.7).sin()).collect();
    let mut l = src.clone();
    let mut r = src.clone();
    engine.process_block(&mut l, &mut r, &mut dry_wet, &mut output_gain);

    for i in 0..total {
        let expected = if i < BLOCK_SIZE { 0.0 } else { src[i - BLOCK_SIZE] };
        assert_eq!(l[i], expected, "sample {i} is not a {BLOCK_SIZE}-sample delay");
        assert_eq!(r[i], expected, "sample {i} is not a {BLOCK_SIZE}-sample delay");
    }
}
