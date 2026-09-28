//! Runtime-built wavetables: the band-limited mip pyramid for a table the
//! user imported, laid out exactly like the bundle so the oscillator reads
//! it with the same code.
//!
//! The bundled tables are built by `wavetable_gen.rs` at plugin build time
//! with a direct DFT — `frame_from_raw` analyses every harmonic of a frame
//! and resynthesises each mip level partial by partial, about 25 M `sin`
//! calls per frame. That is fine once per build and hopeless for a 256-frame
//! import, so this is the same construction on an FFT: analyse the frame
//! once, and per mip level keep the bins up to that level's band limit
//! (`TABLE_SAMPLE_RATE / (2 * f_k)`, the formula `frame_from_raw` uses),
//! inverse-transform and normalise to unit peak. Same partials, same
//! amplitude floor, same per-level normalisation — the tests check a user
//! table built from a bundled frame lands on the bundled mips. The build-time
//! path is deliberately left alone: sharing it would change the bundled
//! tables' bits and with them every golden render.
//!
//! Everything here allocates and is meant for a loader thread; the audio
//! thread only ever receives a finished [`UserTable`].

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::dsp::wavetable::{Wavetable, FRAME_STRIDE, NUM_OCTAVES, WAVETABLE_SIZE};

/// Most frames a user table may hold (the Serum convention). Longer imports
/// are thinned evenly across the file rather than truncated, so the whole
/// morph survives.
pub const MAX_USER_FRAMES: usize = 256;

/// Sample rate the mip levels are band-limited for. Must match `build.rs`
/// and the oscillator's `TABLE_SAMPLE_RATE`, which rescales the selection to
/// the running rate.
const TABLE_SAMPLE_RATE: f32 = 44_100.0;

/// Lowest mip level's design pitch (C-1), as in `wavetable_gen.rs`.
const MIP_BASE_HZ: f32 = 8.175799;

/// A partial quieter than this (as a sine amplitude) is numerical dust, and is
/// dropped — the threshold `frame_from_raw` applies.
const PARTIAL_FLOOR: f32 = 1e-6;

/// An imported wavetable's mip data: `num_frames × NUM_OCTAVES ×
/// WAVETABLE_SIZE` samples, owned. The engine reads it through a
/// [`Wavetable`] view (see [`UserTable::view`]).
pub struct UserTable {
    data: Box<[f32]>,
    num_frames: usize,
}

impl UserTable {
    /// Build the mip pyramid for `frames`, a whole number of
    /// `WAVETABLE_SIZE`-sample single cycles (1..=[`MAX_USER_FRAMES`]).
    pub fn build(frames: &[f32]) -> Result<Self, String> {
        if frames.is_empty() || frames.len() % WAVETABLE_SIZE != 0 {
            return Err(format!(
                "a wavetable is a whole number of {WAVETABLE_SIZE}-sample frames, got {} samples",
                frames.len()
            ));
        }
        let num_frames = frames.len() / WAVETABLE_SIZE;
        if num_frames > MAX_USER_FRAMES {
            return Err(format!(
                "{num_frames} frames is more than the {MAX_USER_FRAMES} a wavetable holds"
            ));
        }
        if frames.iter().any(|s| !s.is_finite()) {
            return Err("the wavetable holds non-finite samples".to_string());
        }

        let n = WAVETABLE_SIZE;
        let mut planner = FftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(n);
        let inverse = planner.plan_fft_inverse(n);
        let mut spectrum = vec![Complex::new(0.0f32, 0.0); n];
        let mut level = vec![Complex::new(0.0f32, 0.0); n];
        let mut data = vec![0.0f32; num_frames * FRAME_STRIDE].into_boxed_slice();

        // Highest harmonic each level keeps. The two lowest levels reach
        // the table's own Nyquist bin (harmonic `n / 2`), as the bundled
        // ones do.
        let limits: [usize; NUM_OCTAVES] = std::array::from_fn(|octave| {
            let freq = MIP_BASE_HZ * 2.0f32.powi(octave as i32);
            ((TABLE_SAMPLE_RATE / (2.0 * freq)) as usize).min(n / 2)
        });

        for (f, frame) in frames.chunks_exact(n).enumerate() {
            for (c, &s) in spectrum.iter_mut().zip(frame) {
                *c = Complex::new(s, 0.0);
            }
            forward.process(&mut spectrum);
            // DC goes, and so does every partial below the floor.
            spectrum[0] = Complex::new(0.0, 0.0);
            for h in 1..=n / 2 {
                if spectrum[h].norm() * 2.0 / n as f32 <= PARTIAL_FLOOR {
                    spectrum[h] = Complex::new(0.0, 0.0);
                    spectrum[n - h] = Complex::new(0.0, 0.0);
                }
            }

            for (octave, &limit) in limits.iter().enumerate() {
                // Positive harmonics 1..=pos and their mirrors; the Nyquist
                // bin is its own mirror.
                let pos = limit.min(n / 2 - 1);
                level.fill(Complex::new(0.0, 0.0));
                level[1..=pos].copy_from_slice(&spectrum[1..=pos]);
                level[n - pos..].copy_from_slice(&spectrum[n - pos..]);
                if limit == n / 2 {
                    level[n / 2] = spectrum[n / 2];
                }
                inverse.process(&mut level);

                let off = f * FRAME_STRIDE + octave * n;
                let out = &mut data[off..off + n];
                for (o, c) in out.iter_mut().zip(&level) {
                    *o = c.re;
                }
                normalize(out);
            }
        }

        Ok(Self { data, num_frames })
    }

    #[inline]
    pub fn num_frames(&self) -> usize {
        self.num_frames
    }

    /// The oscillator's view of this table.
    ///
    /// # Safety
    ///
    /// The view borrows this table's storage without a lifetime: it must not
    /// be read after this `UserTable` is dropped. (Moving the `UserTable` —
    /// e.g. its `Box` through a channel — does not move the storage.)
    pub unsafe fn view(&self) -> Wavetable {
        // SAFETY: forwarded to the caller.
        unsafe { Wavetable::from_raw_parts(&self.data, self.num_frames) }
    }
}

/// Resample one single cycle of any length to `WAVETABLE_SIZE` samples.
///
/// A cycle is periodic, so this is exact rather than approximate: transform
/// the cycle, keep the partials a `WAVETABLE_SIZE` table can hold, and
/// inverse-transform at the new length. DC and the source's Nyquist bin are
/// dropped (the table is DC-free and the mip builder would drop them anyway).
pub fn resample_cycle(cycle: &[f32]) -> Vec<f32> {
    let len = cycle.len();
    let n = WAVETABLE_SIZE;
    if len == n {
        return cycle.to_vec();
    }
    if len < 2 {
        return vec![0.0; n];
    }
    let mut planner = FftPlanner::<f32>::new();
    let mut src: Vec<Complex<f32>> = cycle.iter().map(|&s| Complex::new(s, 0.0)).collect();
    planner.plan_fft_forward(len).process(&mut src);

    let top = ((len - 1) / 2).min(n / 2 - 1);
    let scale = 1.0 / len as f32;
    let mut dst = vec![Complex::new(0.0f32, 0.0); n];
    for k in 1..=top {
        dst[k] = src[k] * scale;
        dst[n - k] = src[len - k] * scale;
    }
    planner.plan_fft_inverse(n).process(&mut dst);
    dst.iter().map(|c| c.re).collect()
}

fn normalize(buffer: &mut [f32]) {
    let peak = buffer.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak > 0.0 {
        let inv = 1.0 / peak;
        for s in buffer.iter_mut() {
            *s *= inv;
        }
    }
}
