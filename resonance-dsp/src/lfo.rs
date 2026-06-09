/// Simple sine LFO backed by a shared 1024-entry wavetable.
///
/// The reverb runs 8 LFOs per sample; a real `f32::sin()` in the hot
/// loop costs ~80 cycles each, which dominates the 256-sample budget
/// on its own. A 1024-point table with linear interpolation keeps
/// THD below the NAM model's own quantization noise floor and turns
/// each `next()` call into two table loads and a lerp.
pub struct Lfo {
    phase: f32,
    phase_inc: f32,
}

const TABLE_BITS: usize = 10;
const TABLE_SIZE: usize = 1 << TABLE_BITS;
const TABLE_MASK: usize = TABLE_SIZE - 1;

static SINE_TABLE: [f32; TABLE_SIZE] = build_sine_table();

const fn build_sine_table() -> [f32; TABLE_SIZE] {
    // `f32::sin` isn't const, so approximate with a Taylor series.
    // 11 terms across [0, 2π] stays below 1e-7 absolute error — well
    // under the 1e-3 bound the table-vs-sin spot check enforces.
    let mut table = [0.0f32; TABLE_SIZE];
    let two_pi = 2.0 * std::f64::consts::PI;
    let mut i = 0;
    while i < TABLE_SIZE {
        let x = two_pi * (i as f64) / (TABLE_SIZE as f64);
        table[i] = taylor_sin(x) as f32;
        i += 1;
    }
    table
}

const fn taylor_sin(x: f64) -> f64 {
    // Range-reduce to [-π, π] so the Taylor series around 0 converges fast.
    let pi = std::f64::consts::PI;
    let two_pi = 2.0 * pi;
    let mut y = x % two_pi;
    if y > pi {
        y -= two_pi;
    } else if y < -pi {
        y += two_pi;
    }
    let y2 = y * y;
    let mut term = y;
    let mut sum = 0.0;
    let mut n = 1u32;
    while n < 22 {
        sum += term;
        // next term: -term * y^2 / ((n+1)(n+2))
        let denom = ((n + 1) * (n + 2)) as f64;
        term = -term * y2 / denom;
        n += 2;
    }
    sum
}

/// Phase increment per sample for the given rate, sanitized so the
/// hot-path `next()` never has to branch on bad input: a non-finite or
/// negative increment (NaN/±inf rate, zero/negative/NaN sample rate)
/// freezes the LFO at its current phase instead of poisoning `phase`
/// — `phase += NaN` would make every subsequent output NaN forever.
fn sanitized_phase_inc(rate_hz: f32, sample_rate: f32) -> f32 {
    let inc = rate_hz / sample_rate;
    if inc.is_finite() && inc >= 0.0 {
        inc
    } else {
        0.0
    }
}

impl Lfo {
    pub fn new(rate_hz: f32, sample_rate: f32, initial_phase: f32) -> Self {
        // Wrap the phase into [0, 1) and reject non-finite values for
        // the same reason as the increment: `next()` assumes a sane
        // phase and must stay branch-free.
        let phase = if initial_phase.is_finite() {
            initial_phase - initial_phase.floor()
        } else {
            0.0
        };
        Self {
            phase,
            phase_inc: sanitized_phase_inc(rate_hz, sample_rate),
        }
    }

    pub fn set_rate(&mut self, rate_hz: f32, sample_rate: f32) {
        self.phase_inc = sanitized_phase_inc(rate_hz, sample_rate);
    }

    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        let scaled = self.phase * TABLE_SIZE as f32;
        let idx = scaled as usize & TABLE_MASK;
        let frac = scaled - (scaled as usize) as f32;
        // Direct indexing is safe and BCE-clean: `SINE_TABLE` has a
        // statically-known length of TABLE_SIZE and both indices were
        // pre-masked into 0..TABLE_SIZE. Benchmarked equivalent to
        // `get_unchecked` (within 0.05% on x86_64).
        let a = SINE_TABLE[idx];
        let b = SINE_TABLE[(idx + 1) & TABLE_MASK];
        let out = a + frac * (b - a);

        self.phase += self.phase_inc;
        self.phase -= self.phase.floor();
        out
    }
}

