//! Band-limited sample-rate conversion (LIB-01).
//!
//! A Kaiser-windowed-sinc polyphase filter evaluated at the exact
//! rational output instants `n * in_rate / out_rate`:
//!
//! - **Cutoff** at the lower of the two Nyquist frequencies, so a
//!   downsample low-passes before decimating (no fold-back) and an
//!   upsample suppresses the spectral images. The kernel is stretched
//!   by `in/out` when downsampling, which is what scales the cutoff.
//! - **Quality:** 40 zero crossings per side at the cutoff scale
//!   (80 taps per phase when upsampling, `80 * in/out` when
//!   downsampling) and Kaiser β = 9, i.e. ≈ 90 dB stopband. The
//!   transition band is centred on the lower Nyquist and ±7 % of it
//!   wide, so the passband is flat (well inside ±0.1 dB) to ≈ 0.93 of
//!   the lower Nyquist and everything that would alias into the
//!   audible band is ≥ 80 dB down.
//! - **Zero phase:** the kernel is symmetric and centred on each output
//!   instant, so output frame `n` is input time `n * in/out` exactly —
//!   converted clips do not shift. The streaming form pays for that
//!   with `half_taps` input frames of lookahead (≈ 1 ms), not with an
//!   offset in its output.
//! - **Edges** extend the first / last input frame instead of padding
//!   with zeros, so audio that starts or ends at a non-zero level is
//!   not faded in or out.
//! - **Phases:** when the reduced up-factor `out / gcd` is ≤
//!   [`MAX_EXACT_PHASES`] every output instant has its own
//!   pre-computed row (all common audio rate pairs). Otherwise a
//!   [`MAX_EXACT_PHASES`]-row table is linearly interpolated between
//!   rows (error ≈ -120 dB).
//!
//! Each row is normalised to unit DC gain, so a constant stays exactly
//! that constant (to `f32` rounding).

/// Zero crossings of the sinc on each side of the centre tap.
const ZERO_CROSSINGS: f64 = 40.0;
/// Kaiser window shape; β = 9 gives ≈ 90 dB of stopband rejection.
const KAISER_BETA: f64 = 9.0;
/// Largest up-factor that gets one exact table row per output phase.
const MAX_EXACT_PHASES: u64 = 1024;

/// Pre-computed polyphase kernel for one `in_rate -> out_rate` pair.
struct Kernel {
    /// Reduced ratio: `out/in = up/down`.
    up: u64,
    down: u64,
    /// Taps on each side of the output instant; `2 * half` per row.
    half: usize,
    /// Number of table rows minus one (row `phases` is fraction 1.0).
    phases: u64,
    /// `phases == up`: every output phase has an exact row.
    exact: bool,
    /// `(phases + 1) * 2 * half` coefficients, row-major.
    table: Vec<f32>,
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Zeroth-order modified Bessel function of the first kind.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    let mut k = 1.0;
    while term > sum * 1e-17 {
        term *= q / (k * k);
        sum += term;
        k += 1.0;
    }
    sum
}

impl Kernel {
    fn new(in_rate: u64, out_rate: u64) -> Self {
        let g = gcd(in_rate, out_rate);
        let (up, down) = (out_rate / g, in_rate / g);
        // Cutoff in cycles per input sample: the lower Nyquist.
        let fc = 0.5 * (up as f64 / down as f64).min(1.0);
        // Kernel reach in input samples.
        let reach = ZERO_CROSSINGS / (2.0 * fc);
        let half = reach.ceil() as usize;
        let taps = 2 * half;
        let (phases, exact) = if up <= MAX_EXACT_PHASES {
            (up, true)
        } else {
            (MAX_EXACT_PHASES, false)
        };
        let i0_beta = bessel_i0(KAISER_BETA);
        let h = |tau: f64| -> f64 {
            let r = tau / reach;
            if r.abs() >= 1.0 {
                return 0.0;
            }
            let w = bessel_i0(KAISER_BETA * (1.0 - r * r).sqrt()) / i0_beta;
            let x = 2.0 * fc * tau;
            let sinc = if x.abs() < 1e-12 {
                1.0
            } else {
                (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
            };
            2.0 * fc * sinc * w
        };
        let mut table = Vec::with_capacity((phases as usize + 1) * taps);
        let mut row = vec![0.0f64; taps];
        for p in 0..=phases {
            let frac = p as f64 / phases as f64;
            // Tap `j` reads input `base - half + 1 + j`, at distance
            // `frac + half - 1 - j` from the output instant.
            for (j, c) in row.iter_mut().enumerate() {
                *c = h(frac + half as f64 - 1.0 - j as f64);
            }
            let sum: f64 = row.iter().sum();
            table.extend(row.iter().map(|c| (c / sum) as f32));
        }
        Self {
            up,
            down,
            half,
            phases,
            exact,
            table,
        }
    }

    #[inline]
    fn taps(&self) -> usize {
        2 * self.half
    }

    /// Input frame `base` and phase of output frame `n`:
    /// `n * down / up = base + phase / up`.
    #[inline]
    fn locate(&self, n: u64) -> (u64, u64) {
        let num = n * self.down;
        (num / self.up, num % self.up)
    }

    /// Output frames whose instant lies inside `frames` input frames.
    #[inline]
    fn out_len(&self, frames: u64) -> u64 {
        (frames * self.up).div_ceil(self.down)
    }

    /// Coefficients for `phase`, either a table row or an interpolation
    /// between two rows written into `scratch` (length `taps`).
    #[inline]
    fn row<'a>(&'a self, phase: u64, scratch: &'a mut [f32]) -> &'a [f32] {
        let taps = self.taps();
        if self.exact {
            let i = phase as usize;
            return &self.table[i * taps..(i + 1) * taps];
        }
        let t = phase as f64 * self.phases as f64 / self.up as f64;
        let i = t as usize;
        let a = (t - i as f64) as f32;
        let r0 = &self.table[i * taps..(i + 1) * taps];
        let r1 = &self.table[(i + 1) * taps..(i + 2) * taps];
        for ((s, c0), c1) in scratch.iter_mut().zip(r0).zip(r1) {
            *s = c0 + (c1 - c0) * a;
        }
        scratch
    }

    /// Output frame `n`, reading input frame `i` (absolute, may lie
    /// outside the data — `fetch` clamps) through `fetch`.
    #[inline]
    fn eval<const C: usize>(
        &self,
        n: u64,
        scratch: &mut [f32],
        fetch: impl Fn(i64) -> [f32; C],
    ) -> [f32; C] {
        let (base, phase) = self.locate(n);
        let first = base as i64 - self.half as i64 + 1;
        let row = self.row(phase, scratch);
        let mut acc = [0.0f32; C];
        for (j, &c) in row.iter().enumerate() {
            let x = fetch(first + j as i64);
            for ch in 0..C {
                acc[ch] += c * x[ch];
            }
        }
        acc
    }
}

/// Integer rate from a caller's `f32` rate; `None` for nonsense.
fn int_rate(rate: f32) -> Option<u64> {
    let r = rate.round();
    (r.is_finite() && r >= 1.0).then_some(r as u64)
}

/// One-shot conversion of `frames` frames of `C`-channel interleaved
/// audio. Identity (or unusable) rates return the input unchanged.
fn resample_interleaved<const C: usize>(
    input: &[f32],
    source_rate: f32,
    target_rate: f32,
) -> Vec<f32> {
    let frames = input.len() / C;
    if frames == 0 {
        return Vec::new();
    }
    let (Some(from), Some(to)) = (int_rate(source_rate), int_rate(target_rate)) else {
        return input[..frames * C].to_vec();
    };
    if from == to {
        return input[..frames * C].to_vec();
    }
    let k = Kernel::new(from, to);
    let out_frames = k.out_len(frames as u64);
    let last = frames as i64 - 1;
    let fetch = |i: i64| -> [f32; C] {
        let i = i.clamp(0, last) as usize * C;
        std::array::from_fn(|ch| input[i + ch])
    };
    let mut scratch = vec![0.0f32; k.taps()];
    let mut out = Vec::with_capacity(out_frames as usize * C);
    for n in 0..out_frames {
        out.extend_from_slice(&k.eval(n, &mut scratch, fetch));
    }
    out
}

/// Band-limited resampler for mono audio (see the module docs).
pub fn resample_mono(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    resample_interleaved::<1>(input, source_rate, target_rate)
}

/// Band-limited resampler for stereo interleaved audio. A trailing
/// half frame is dropped.
pub fn resample_stereo(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    resample_interleaved::<2>(input, source_rate, target_rate)
}

/// Stateful band-limited resampler for stereo interleaved audio, fed in
/// chunks of any size. `process` + a final `flush` reproduce
/// [`resample_stereo`] of the whole stream exactly, whatever the
/// chunking.
///
/// Output frame `n` needs input up to `half_taps` frames past its
/// instant, so `process` holds that many input frames' worth of output
/// back (≈ 1 ms). [`StreamingResampler::flush`] emits it, extending the
/// last frame. `flush` does not end the stream: `process` may continue
/// afterwards and later output stays on the same time grid (the
/// loop-record seam relies on this to cut a take without shifting the
/// next one).
///
/// After [`StreamingResampler::new`] neither `process` nor `flush`
/// allocates or locks, except to grow the caller's output `Vec`.
pub struct StreamingResampler {
    kernel: Option<Kernel>,
    /// The last `taps` input frames, indexed by absolute frame mod `taps`.
    history: Vec<[f32; 2]>,
    /// Interpolated-row scratch (only used by the non-exact table).
    scratch: Vec<f32>,
    /// The very first input frame (left-edge extension).
    first: [f32; 2],
    /// Input frames consumed so far.
    consumed: u64,
    /// Next output frame to emit.
    next_out: u64,
}

impl StreamingResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        let kernel = (source_rate != target_rate && source_rate > 0 && target_rate > 0)
            .then(|| Kernel::new(source_rate as u64, target_rate as u64));
        let taps = kernel.as_ref().map_or(0, Kernel::taps);
        Self {
            kernel,
            history: vec![[0.0; 2]; taps],
            scratch: vec![0.0; taps],
            first: [0.0; 2],
            consumed: 0,
            next_out: 0,
        }
    }

    /// Feed one chunk of stereo-interleaved input; append every output
    /// frame whose filter window is now fully known to `output`.
    pub fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let frames = input.len() / 2;
        if frames == 0 {
            return;
        }
        let Some(k) = self.kernel.as_ref() else {
            output.extend_from_slice(&input[..frames * 2]);
            return;
        };
        if self.consumed == 0 {
            self.first = [input[0], input[1]];
        }
        let start = self.consumed;
        let total = start + frames as u64;
        let (history, first, taps) = (&self.history, self.first, k.taps() as u64);
        let fetch = |i: i64| -> [f32; 2] {
            if i < 0 {
                return first;
            }
            let i = i as u64;
            if i >= start {
                let l = (i - start) as usize * 2;
                [input[l], input[l + 1]]
            } else {
                history[(i % taps) as usize]
            }
        };
        loop {
            let (base, _) = k.locate(self.next_out);
            if base + k.half as u64 >= total {
                break;
            }
            output.extend_from_slice(&k.eval(self.next_out, &mut self.scratch, fetch));
            self.next_out += 1;
        }
        // Keep the chunk's tail for the next call's lookbehind.
        let keep = frames.min(taps as usize);
        for f in frames - keep..frames {
            let abs = start + f as u64;
            self.history[(abs % taps) as usize] = [input[f * 2], input[f * 2 + 1]];
        }
        self.consumed = total;
    }

    /// Emit the held-back output up to the end of the input seen so far,
    /// extending the last input frame over the missing lookahead.
    pub fn flush(&mut self, output: &mut Vec<f32>) {
        let Some(k) = self.kernel.as_ref() else {
            return;
        };
        let total = self.consumed;
        if total == 0 {
            return;
        }
        let (history, first, taps) = (&self.history, self.first, k.taps() as u64);
        let fetch = |i: i64| -> [f32; 2] {
            let i = i.clamp(-1, total as i64 - 1);
            if i < 0 {
                first
            } else {
                history[(i as u64 % taps) as usize]
            }
        };
        let end = k.out_len(total);
        while self.next_out < end {
            output.extend_from_slice(&k.eval(self.next_out, &mut self.scratch, fetch));
            self.next_out += 1;
        }
    }
}
