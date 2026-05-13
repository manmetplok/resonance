/// Offline wavetable generator.
///
/// This file is NOT declared as a module in `lib.rs` — it is `#[path]`-included
/// from `build.rs` so that wavetable generation happens once at plugin build
/// time. The resulting f32 samples are written to `$OUT_DIR/wavetables.bin`
/// and loaded at runtime by `wavetable::load_bundled()`.
///
/// Keep constants and layout in sync with `wavetable.rs`. The loader validates
/// the header and will panic at plugin init if there is a mismatch.
use std::io::Write;
use std::path::Path;

use resonance_dsp::SimpleRng;

pub const WAVETABLE_SIZE: usize = 2048;
pub const NUM_OCTAVES: usize = 11;
pub const NUM_WAVETABLES: usize = 10;

struct WavetableFrame {
    mip_levels: Vec<Vec<f32>>,
}

struct Wavetable {
    frames: Vec<WavetableFrame>,
}

pub fn write_bundled(sample_rate: f32, out_path: &Path) {
    let tables = generate_all(sample_rate);
    assert_eq!(tables.len(), NUM_WAVETABLES);

    let mut buf: Vec<u8> = Vec::new();
    // Header.
    buf.extend_from_slice(&(WAVETABLE_SIZE as u32).to_le_bytes());
    buf.extend_from_slice(&(NUM_OCTAVES as u32).to_le_bytes());
    buf.extend_from_slice(&(NUM_WAVETABLES as u32).to_le_bytes());
    // Per-table frame counts.
    for table in &tables {
        buf.extend_from_slice(&(table.frames.len() as u32).to_le_bytes());
    }
    // Raw f32 samples, in order: [table][frame][octave][sample].
    for table in &tables {
        for frame in &table.frames {
            assert_eq!(frame.mip_levels.len(), NUM_OCTAVES);
            for mip in &frame.mip_levels {
                assert_eq!(mip.len(), WAVETABLE_SIZE);
                for &s in mip {
                    buf.extend_from_slice(&s.to_le_bytes());
                }
            }
        }
    }

    let mut f = std::fs::File::create(out_path).expect("create wavetables.bin");
    f.write_all(&buf).expect("write wavetables.bin");
}

// ---------------------------------------------------------------------------
// Generation (mirrors wavetable.rs at time of split)
// ---------------------------------------------------------------------------

fn generate_all(sample_rate: f32) -> Vec<Wavetable> {
    let mut tables = Vec::with_capacity(NUM_WAVETABLES);
    tables.push(generate_basic(sample_rate));
    tables.push(generate_saw_stack(sample_rate));
    tables.push(generate_pwm(sample_rate));
    tables.push(generate_formant(sample_rate));
    tables.push(generate_digital(sample_rate));
    tables.push(generate_harmonic_sweep(sample_rate));
    tables.push(generate_metallic(sample_rate));
    tables.push(generate_organ(sample_rate));
    tables.push(generate_noise_cycle(sample_rate));
    tables.push(generate_sync_sweep(sample_rate));
    tables
}

fn frame_from_harmonics(harmonics: &[(usize, f32)], sample_rate: f32) -> WavetableFrame {
    let mut mip_levels = Vec::with_capacity(NUM_OCTAVES);
    for octave in 0..NUM_OCTAVES {
        let freq = 8.175799 * 2.0f32.powi(octave as i32);
        let max_harmonic = (sample_rate / (2.0 * freq)) as usize;
        let mut buffer = vec![0.0f32; WAVETABLE_SIZE];
        for &(h, amp) in harmonics {
            if h == 0 || h > max_harmonic {
                continue;
            }
            let phase_scale = std::f32::consts::TAU * h as f32 / WAVETABLE_SIZE as f32;
            for (i, s) in buffer.iter_mut().enumerate() {
                *s += amp * (i as f32 * phase_scale).sin();
            }
        }
        normalize(&mut buffer);
        mip_levels.push(buffer);
    }
    WavetableFrame { mip_levels }
}

fn frame_from_raw(waveform: &[f32], sample_rate: f32) -> WavetableFrame {
    let n = waveform.len();
    let mut harmonics = Vec::new();
    let max_h = n / 2;
    for h in 1..=max_h {
        let phase_scale = std::f32::consts::TAU * h as f32 / n as f32;
        let mut cos_sum = 0.0f32;
        let mut sin_sum = 0.0f32;
        for (i, &sample) in waveform.iter().enumerate() {
            let angle = i as f32 * phase_scale;
            cos_sum += sample * angle.cos();
            sin_sum += sample * angle.sin();
        }
        let amp = (cos_sum * cos_sum + sin_sum * sin_sum).sqrt() * 2.0 / n as f32;
        let phase = (-cos_sum).atan2(sin_sum);
        if amp > 1e-6 {
            harmonics.push((h, amp, phase));
        }
    }

    let mut mip_levels = Vec::with_capacity(NUM_OCTAVES);
    for octave in 0..NUM_OCTAVES {
        let freq = 8.175799 * 2.0f32.powi(octave as i32);
        let max_harmonic = (sample_rate / (2.0 * freq)) as usize;
        let mut buffer = vec![0.0f32; WAVETABLE_SIZE];
        for &(h, amp, phase) in &harmonics {
            if h > max_harmonic {
                continue;
            }
            let phase_scale = std::f32::consts::TAU * h as f32 / WAVETABLE_SIZE as f32;
            for (i, s) in buffer.iter_mut().enumerate() {
                *s += amp * (i as f32 * phase_scale + phase).sin();
            }
        }
        normalize(&mut buffer);
        mip_levels.push(buffer);
    }
    WavetableFrame { mip_levels }
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

fn generate_basic(sample_rate: f32) -> Wavetable {
    let num_h = WAVETABLE_SIZE / 2;

    let sine = frame_from_harmonics(&[(1, 1.0)], sample_rate);

    let tri_h: Vec<(usize, f32)> = (0..num_h)
        .map(|k| {
            let n = 2 * k + 1;
            let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
            (n, sign / (n as f32 * n as f32))
        })
        .filter(|&(n, _)| n <= num_h)
        .collect();
    let triangle = frame_from_harmonics(&tri_h, sample_rate);

    let saw_h: Vec<(usize, f32)> = (1..=num_h)
        .map(|n| {
            let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
            (n, sign / n as f32)
        })
        .collect();
    let saw = frame_from_harmonics(&saw_h, sample_rate);

    let sq_h: Vec<(usize, f32)> = (0..num_h)
        .map(|k| {
            let n = 2 * k + 1;
            (n, 1.0 / n as f32)
        })
        .filter(|&(n, _)| n <= num_h)
        .collect();
    let square = frame_from_harmonics(&sq_h, sample_rate);

    Wavetable {
        frames: vec![sine, triangle, saw, square],
    }
}

fn generate_saw_stack(sample_rate: f32) -> Wavetable {
    let num_h = WAVETABLE_SIZE / 2;
    let num_frames = 64;
    let frames = (0..num_frames)
        .map(|f| {
            let layers = 1 + f;
            let mut harmonics = vec![0.0f32; num_h + 1];
            for layer in 0..layers {
                let detune = if layers > 1 {
                    (layer as f32 / (layers - 1) as f32) * 0.02 - 0.01
                } else {
                    0.0
                };
                for n in 1..=num_h {
                    let freq_ratio = 1.0 + detune;
                    let effective_n = (n as f32 * freq_ratio).round() as usize;
                    if effective_n >= 1 && effective_n <= num_h {
                        let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
                        harmonics[effective_n] += sign / n as f32;
                    }
                }
            }
            let h_list: Vec<(usize, f32)> = harmonics
                .iter()
                .enumerate()
                .filter(|&(n, &a)| n >= 1 && a.abs() > 1e-8)
                .map(|(n, &a)| (n, a))
                .collect();
            frame_from_harmonics(&h_list, sample_rate)
        })
        .collect();

    Wavetable { frames }
}

fn generate_pwm(sample_rate: f32) -> Wavetable {
    let num_h = WAVETABLE_SIZE / 2;
    let num_frames = 64;
    let frames = (0..num_frames)
        .map(|f| {
            let duty = 0.5 - (f as f32 / (num_frames - 1) as f32) * 0.45;
            let h_list: Vec<(usize, f32)> = (1..=num_h)
                .map(|n| {
                    let amp = (std::f32::consts::PI * n as f32 * duty).sin() * 2.0
                        / (std::f32::consts::PI * n as f32);
                    (n, amp)
                })
                .filter(|&(_, a)| a.abs() > 1e-8)
                .collect();
            frame_from_harmonics(&h_list, sample_rate)
        })
        .collect();

    Wavetable { frames }
}

fn generate_formant(sample_rate: f32) -> Wavetable {
    let vowels: [(f32, f32, f32); 5] = [
        (2.5, 7.0, 12.0),
        (2.0, 10.0, 14.0),
        (1.5, 11.0, 14.0),
        (2.0, 4.0, 12.0),
        (1.5, 3.5, 11.0),
    ];
    let num_h = WAVETABLE_SIZE / 2;
    let num_frames = 64;

    let frames = (0..num_frames)
        .map(|f| {
            let t = f as f32 / (num_frames - 1) as f32;
            let vowel_pos = t * (vowels.len() - 1) as f32;
            let vi = (vowel_pos as usize).min(vowels.len() - 2);
            let frac = vowel_pos - vi as f32;
            let (f1a, f2a, f3a) = vowels[vi];
            let (f1b, f2b, f3b) = vowels[vi + 1];
            let f1 = f1a + (f1b - f1a) * frac;
            let f2 = f2a + (f2b - f2a) * frac;
            let f3 = f3a + (f3b - f3a) * frac;

            let h_list: Vec<(usize, f32)> = (1..=num_h)
                .map(|n| {
                    let nf = n as f32;
                    let a1 = (-((nf - f1) * (nf - f1)) / 2.0).exp();
                    let a2 = (-((nf - f2) * (nf - f2)) / 4.0).exp() * 0.7;
                    let a3 = (-((nf - f3) * (nf - f3)) / 6.0).exp() * 0.4;
                    let amp = a1 + a2 + a3;
                    (n, amp)
                })
                .filter(|&(_, a)| a > 1e-6)
                .collect();
            frame_from_harmonics(&h_list, sample_rate)
        })
        .collect();

    Wavetable { frames }
}

fn generate_digital(sample_rate: f32) -> Wavetable {
    let num_frames = 32;
    let frames = (0..num_frames)
        .map(|f| {
            let t = f as f32 / (num_frames - 1) as f32;
            let levels = (256.0 * (1.0 - t * 0.99)).max(2.0);
            let mut waveform = vec![0.0f32; WAVETABLE_SIZE];
            for (i, slot) in waveform.iter_mut().enumerate() {
                let phase = i as f32 / WAVETABLE_SIZE as f32;
                let sine = (phase * std::f32::consts::TAU).sin();
                *slot = (sine * levels * 0.5).round() / (levels * 0.5);
            }
            frame_from_raw(&waveform, sample_rate)
        })
        .collect();

    Wavetable { frames }
}

fn generate_harmonic_sweep(sample_rate: f32) -> Wavetable {
    let num_frames = 64;
    let frames = (0..num_frames)
        .map(|f| {
            let max_h = 1 + f;
            let h_list: Vec<(usize, f32)> = (1..=max_h).map(|n| (n, 1.0 / n as f32)).collect();
            frame_from_harmonics(&h_list, sample_rate)
        })
        .collect();

    Wavetable { frames }
}

fn generate_metallic(sample_rate: f32) -> Wavetable {
    let num_h = 32;
    let num_frames = 32;
    let frames = (0..num_frames)
        .map(|f| {
            let t = f as f32 / (num_frames - 1) as f32;
            let inharmonicity = t * 0.15;
            let h_list: Vec<(usize, f32)> = (1..=num_h)
                .map(|n| {
                    let nf = n as f32;
                    let stretched = nf * (1.0 + inharmonicity * nf * nf).sqrt();
                    let quantized = stretched.round() as usize;
                    let amp = 1.0 / (1.0 + nf * 0.3);
                    (quantized.max(1), amp)
                })
                .filter(|&(n, _)| n <= WAVETABLE_SIZE / 2)
                .collect();
            frame_from_harmonics(&h_list, sample_rate)
        })
        .collect();

    Wavetable { frames }
}

fn generate_organ(sample_rate: f32) -> Wavetable {
    let drawbar_configs: Vec<[f32; 7]> = vec![
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0],
        [1.0, 0.5, 0.3, 0.0, 0.0, 0.0, 0.0],
        [1.0, 0.7, 0.5, 0.3, 0.0, 0.0, 0.0],
        [1.0, 0.8, 0.6, 0.4, 0.3, 0.0, 0.0],
        [1.0, 1.0, 0.7, 0.5, 0.3, 0.2, 0.0],
        [1.0, 1.0, 0.8, 0.6, 0.4, 0.3, 0.2],
        [0.5, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [0.3, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0],
        [0.0, 0.5, 1.0, 0.0, 1.0, 0.0, 0.5],
        [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0],
        [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.5, 0.7, 1.0, 0.8],
        [0.8, 0.6, 0.4, 0.3, 0.2, 0.1, 0.05],
        [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
    ];
    let harmonic_nums = [1, 2, 3, 4, 5, 6, 8];

    let frames = drawbar_configs
        .iter()
        .map(|config| {
            let h_list: Vec<(usize, f32)> = harmonic_nums
                .iter()
                .zip(config.iter())
                .filter(|&(_, &amp)| amp > 1e-6)
                .map(|(&n, &amp)| (n, amp))
                .collect();
            frame_from_harmonics(&h_list, sample_rate)
        })
        .collect();

    Wavetable { frames }
}

fn generate_noise_cycle(sample_rate: f32) -> Wavetable {
    let mut rng = SimpleRng::new(42);
    let num_frames = 8;
    let frames = (0..num_frames)
        .map(|f| {
            let t = (f as f32 + 1.0) / num_frames as f32;
            let rolloff = 1.0 - t * 0.8;
            let mut waveform = vec![0.0f32; WAVETABLE_SIZE];
            let num_h = WAVETABLE_SIZE / 2;
            for n in 1..=num_h {
                let amp = 1.0 / (n as f32).powf(rolloff);
                let phase = (rng.next_u32() as f32 / u32::MAX as f32) * std::f32::consts::TAU;
                let freq_scale = std::f32::consts::TAU * n as f32 / WAVETABLE_SIZE as f32;
                for (i, s) in waveform.iter_mut().enumerate() {
                    *s += amp * (i as f32 * freq_scale + phase).sin();
                }
            }
            normalize(&mut waveform);
            frame_from_raw(&waveform, sample_rate)
        })
        .collect();

    Wavetable { frames }
}

fn generate_sync_sweep(sample_rate: f32) -> Wavetable {
    let num_frames = 64;
    let frames = (0..num_frames)
        .map(|f| {
            let ratio = 1.0 + f as f32 * 7.0 / (num_frames - 1) as f32;
            let mut waveform = vec![0.0f32; WAVETABLE_SIZE];
            for (i, s) in waveform.iter_mut().enumerate() {
                let phase = (i as f32 / WAVETABLE_SIZE as f32) * ratio;
                let frac = phase - phase.floor();
                *s = 2.0 * frac - 1.0;
            }
            frame_from_raw(&waveform, sample_rate)
        })
        .collect();

    Wavetable { frames }
}
