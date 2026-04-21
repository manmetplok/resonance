//! UI-side wavetable shape generation.
//!
//! We deliberately do NOT reuse the audio engine's `generate_all()` here —
//! it computes 11 mip-mapped octaves per frame with up to thousands of
//! harmonics, which takes seconds on the UI thread even in release builds.
//! For display we only need a single 256-sample representative waveform per
//! frame, so we use lightweight purpose-built generators matched to the
//! character of each wavetable (not cycle-accurate to the audio-thread
//! version, but visually faithful).
//!
//! Results are cached in a `OnceLock` so the cost is paid once per
//! (wavetable, frame) combination.

use std::f32::consts::TAU;
use std::sync::OnceLock;

const N_POINTS: usize = 256;

/// Per-wavetable frame count. Matches the audio-thread layout so selection
/// UIs stay in sync.
const FRAME_COUNTS: [usize; 10] = [
    4,  // Basic
    64, // Saw Stack
    64, // PWM
    64, // Formant
    32, // Digital
    64, // Harmonic Sweep
    32, // Metallic
    16, // Organ
    8,  // Noise Cycle
    64, // Sync Sweep
];

pub const WAVETABLE_NAMES: [&str; 10] = [
    "Basic",
    "Saw Stack",
    "PWM",
    "Formant",
    "Digital",
    "Harmonic Sweep",
    "Metallic",
    "Organ",
    "Noise Cycle",
    "Sync Sweep",
];

pub fn wavetable_name(index: usize) -> &'static str {
    WAVETABLE_NAMES.get(index).copied().unwrap_or("(unknown)")
}

pub fn frame_count(wt_index: usize) -> usize {
    FRAME_COUNTS.get(wt_index).copied().unwrap_or(0)
}

/// Returns a representative waveform for (wavetable, frame).
/// Cached on first access.
pub fn display_samples(wt_index: usize, frame_index: usize, n_points: usize) -> Vec<f32> {
    let out = cached_frame(wt_index, frame_index);
    if n_points == N_POINTS {
        out.to_vec()
    } else {
        // Resample to the requested length by index stride.
        let stride = (N_POINTS as f32 / n_points as f32).max(1.0);
        (0..n_points)
            .map(|i| {
                let src_idx = ((i as f32) * stride) as usize;
                out[src_idx.min(N_POINTS - 1)]
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Generation cache
// ---------------------------------------------------------------------------

type FrameBuf = [f32; N_POINTS];

fn cached_frame(wt_index: usize, frame_index: usize) -> &'static FrameBuf {
    static CACHE: OnceLock<Vec<Vec<FrameBuf>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        let mut out: Vec<Vec<FrameBuf>> = Vec::with_capacity(10);
        for (wt, &n) in FRAME_COUNTS.iter().enumerate() {
            let mut frames: Vec<FrameBuf> = Vec::with_capacity(n);
            for f in 0..n {
                frames.push(generate_frame(wt, f, n));
            }
            out.push(frames);
        }
        out
    });
    let wt = wt_index.min(cache.len() - 1);
    let f = frame_index.min(cache[wt].len() - 1);
    &cache[wt][f]
}

fn generate_frame(wt: usize, frame: usize, frame_count: usize) -> FrameBuf {
    let mut buf = [0.0f32; N_POINTS];
    match wt {
        0 => basic(&mut buf, frame),
        1 => saw_stack(&mut buf, frame, frame_count),
        2 => pwm(&mut buf, frame, frame_count),
        3 => formant(&mut buf, frame, frame_count),
        4 => digital(&mut buf, frame, frame_count),
        5 => harmonic_sweep(&mut buf, frame, frame_count),
        6 => metallic(&mut buf, frame, frame_count),
        7 => organ(&mut buf, frame, frame_count),
        8 => noise_cycle(&mut buf, frame, frame_count),
        9 => sync_sweep(&mut buf, frame, frame_count),
        _ => {}
    }
    normalize(&mut buf);
    buf
}

// ---------------------------------------------------------------------------
// Per-wavetable generators. Lightweight, no band-limiting.
// ---------------------------------------------------------------------------

fn basic(buf: &mut FrameBuf, frame: usize) {
    match frame {
        0 => {
            for (i, s) in buf.iter_mut().enumerate() {
                *s = (i as f32 / N_POINTS as f32 * TAU).sin();
            }
        }
        1 => {
            for (i, s) in buf.iter_mut().enumerate() {
                let t = i as f32 / N_POINTS as f32;
                *s = if t < 0.25 {
                    t * 4.0
                } else if t < 0.75 {
                    2.0 - t * 4.0
                } else {
                    t * 4.0 - 4.0
                };
            }
        }
        2 => {
            // Band-limited-ish saw via 32 harmonics.
            sum_harmonics(
                buf,
                |h| -2.0 / (TAU * h as f32) * (-1.0_f32).powi(h as i32 + 1),
                32,
            );
        }
        _ => {
            // Square via odd harmonics.
            sum_harmonics(
                buf,
                |h| {
                    if h % 2 == 0 {
                        0.0
                    } else {
                        4.0 / (TAU * h as f32)
                    }
                },
                32,
            );
        }
    }
}

fn saw_stack(buf: &mut FrameBuf, frame: usize, frame_count: usize) {
    // Frame 0: single saw. Frame N-1: many overlapping detuned saws.
    let t = frame as f32 / (frame_count - 1).max(1) as f32;
    let layers = (1.0 + t * 6.0) as usize; // 1..7 layers
    for l in 0..layers {
        let detune = (l as f32 - layers as f32 / 2.0) * 0.02;
        for (i, s) in buf.iter_mut().enumerate() {
            let phase = (i as f32 / N_POINTS as f32 + detune) % 1.0;
            let saw = 2.0 * phase - 1.0;
            *s += saw / layers as f32;
        }
    }
}

fn pwm(buf: &mut FrameBuf, frame: usize, frame_count: usize) {
    let duty = 0.5 - (frame as f32 / (frame_count - 1).max(1) as f32) * 0.45;
    for (i, s) in buf.iter_mut().enumerate() {
        let t = i as f32 / N_POINTS as f32;
        *s = if t < duty { 1.0 } else { -1.0 };
    }
}

fn formant(buf: &mut FrameBuf, frame: usize, frame_count: usize) {
    // Five vowel formant spectra, interpolated via a triangle through A-E-I-O-U.
    let vowels: [[(usize, f32); 4]; 5] = [
        [(1, 1.0), (5, 0.6), (11, 0.4), (16, 0.2)],  // A
        [(1, 1.0), (4, 0.7), (14, 0.6), (22, 0.25)], // E
        [(1, 1.0), (3, 0.5), (17, 0.7), (24, 0.3)],  // I
        [(1, 1.0), (2, 0.8), (6, 0.4), (13, 0.2)],   // O
        [(1, 1.0), (2, 0.9), (5, 0.3), (10, 0.15)],  // U
    ];
    let t = (frame as f32 / (frame_count - 1).max(1) as f32) * 4.0;
    let lo = t.floor() as usize;
    let frac = t - lo as f32;
    let hi = (lo + 1).min(4);
    let a = &vowels[lo];
    let b = &vowels[hi];
    for k in 0..4 {
        let amp = a[k].1 * (1.0 - frac) + b[k].1 * frac;
        let h = a[k].0;
        for (i, s) in buf.iter_mut().enumerate() {
            *s += amp * (i as f32 / N_POINTS as f32 * TAU * h as f32).sin();
        }
    }
}

fn digital(buf: &mut FrameBuf, frame: usize, frame_count: usize) {
    // Sine morphing into a bitcrushed version.
    let levels = 2_usize.pow(8 - (frame as u32 * 7 / (frame_count as u32 - 1).max(1)).min(7));
    for (i, s) in buf.iter_mut().enumerate() {
        let v = (i as f32 / N_POINTS as f32 * TAU).sin();
        let q = (v * levels as f32).round() / levels as f32;
        *s = q;
    }
}

fn harmonic_sweep(buf: &mut FrameBuf, frame: usize, frame_count: usize) {
    let max_h = 1 + (frame * 32 / frame_count.max(1));
    sum_harmonics(
        buf,
        |h| if h <= max_h { 1.0 / h as f32 } else { 0.0 },
        max_h.max(1),
    );
}

fn metallic(buf: &mut FrameBuf, frame: usize, frame_count: usize) {
    // Harmonic -> inharmonic (shift odd harmonics by a ratio).
    let ratio = 1.0 + (frame as f32 / frame_count.max(1) as f32) * 0.4;
    for h in 1..=12 {
        let freq_mult = if h % 2 == 0 {
            h as f32
        } else {
            h as f32 * ratio
        };
        for (i, s) in buf.iter_mut().enumerate() {
            *s += (i as f32 / N_POINTS as f32 * TAU * freq_mult).sin() / h as f32;
        }
    }
}

fn organ(buf: &mut FrameBuf, frame: usize, frame_count: usize) {
    // Hammond-ish drawbar blend.
    let weights = [1.0, 0.8, 0.6, 0.4, 0.3, 0.2, 0.15];
    let t = frame as f32 / frame_count.max(1) as f32;
    for (h_idx, w) in weights.iter().enumerate() {
        let h = match h_idx {
            0 => 1.0,
            1 => 2.0,
            2 => 3.0,
            3 => 4.0,
            4 => 6.0,
            5 => 8.0,
            _ => 16.0,
        };
        let amp = w * (1.0 - (1.0 - t).powi(h_idx as i32 + 1));
        for (i, s) in buf.iter_mut().enumerate() {
            *s += amp * (i as f32 / N_POINTS as f32 * TAU * h).sin();
        }
    }
}

fn noise_cycle(buf: &mut FrameBuf, frame: usize, _frame_count: usize) {
    // Deterministic PRNG seeded per-frame.
    let mut state: u32 = 0x12345u32.wrapping_mul(frame as u32 + 1);
    for s in buf.iter_mut() {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        *s = ((state >> 16) as f32 / 32768.0) - 1.0;
    }
    // Apply a simple low-pass proportional to frame index so later frames are smoother.
    let lp = 0.7;
    let mut prev = 0.0;
    for s in buf.iter_mut() {
        prev = prev * lp + *s * (1.0 - lp);
        *s = prev;
    }
}

fn sync_sweep(buf: &mut FrameBuf, frame: usize, frame_count: usize) {
    // Hard-sync emulation via phase compression.
    let ratio = 1.0 + (frame as f32 / frame_count.max(1) as f32) * 6.0;
    for (i, s) in buf.iter_mut().enumerate() {
        let t = i as f32 / N_POINTS as f32;
        let p = (t * ratio) % 1.0;
        *s = 2.0 * p - 1.0;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Fill `buf` with `sum_{h=1..=max_h} amp(h) * sin(2π * h * t)`.
fn sum_harmonics<F: Fn(usize) -> f32>(buf: &mut FrameBuf, amp: F, max_h: usize) {
    for (i, s) in buf.iter_mut().enumerate() {
        let t = i as f32 / N_POINTS as f32;
        let mut acc = 0.0f32;
        for h in 1..=max_h {
            let a = amp(h);
            if a != 0.0 {
                acc += a * (t * TAU * h as f32).sin();
            }
        }
        *s = acc;
    }
}

fn normalize(buf: &mut FrameBuf) {
    let mut peak = 0.0f32;
    for s in buf.iter() {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
    }
    if peak > 0.001 {
        let inv = 1.0 / peak;
        for s in buf.iter_mut() {
            *s *= inv;
        }
    }
}
