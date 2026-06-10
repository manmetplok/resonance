//! Freeze mode must not accumulate fresh input energy: with the tank
//! input muted, continuous input during freeze leaves the output
//! bounded instead of growing until it clips.

use resonance_reverb::dsp::ReverbDsp;

const SAMPLE_RATE: f32 = 48_000.0;

/// Tiny deterministic noise source (xorshift) for a continuous input signal.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn rms(samples: &[f32]) -> f32 {
    let sumsq: f64 = samples.iter().map(|&x| (x as f64) * (x as f64)).sum();
    (sumsq / samples.len() as f64).sqrt() as f32
}

#[test]
fn freeze_with_continuous_input_stays_bounded() {
    let mut reverb = ReverbDsp::new(SAMPLE_RATE);
    reverb.set_size(0.5);
    reverb.set_decay(2.0);
    reverb.set_damping(20_000.0);

    let mut noise = Noise(0x2545_f491);
    let block = SAMPLE_RATE as usize; // 1-second measurement windows

    // Build up some tail energy with normal (unfrozen) processing.
    for _ in 0..block {
        let x = noise.next() * 0.5;
        reverb.process(x, x, 0.7, 1.0);
    }

    // Engage freeze and keep feeding input.
    reverb.set_freeze(true);
    let mut window = vec![0.0f32; block];
    let capture = |reverb: &mut ReverbDsp, noise: &mut Noise, window: &mut [f32]| {
        for w in window.iter_mut() {
            let x = noise.next() * 0.5;
            let (l, r) = reverb.process(x, x, 0.7, 1.0);
            assert!(l.is_finite() && r.is_finite(), "non-finite output");
            *w = (l + r) * 0.5;
        }
    };

    capture(&mut reverb, &mut noise, &mut window);
    let rms_at_freeze = rms(&window);

    // Skip ahead several seconds of frozen processing with live input.
    for _ in 0..4 {
        capture(&mut reverb, &mut noise, &mut window);
    }
    let rms_later = rms(&window);

    assert!(
        rms_later <= rms_at_freeze * 1.5,
        "frozen tank accumulated input energy: rms at freeze {rms_at_freeze}, later {rms_later}"
    );
}
