/// Wavetable oscillator with band-limited mip-map selection and cubic Hermite interpolation.
use crate::wavetable::{Wavetable, NUM_OCTAVES};

/// Read a wavetable with anti-aliased mip-map crossfading and frame interpolation.
///
/// - `table`: the wavetable to read from
/// - `phase`: oscillator phase (0.0..1.0, f64 for precision)
/// - `position`: wavetable scan position (0.0..1.0)
/// - `freq_hz`: current oscillator frequency (for mip-map selection)
pub fn read_wavetable(table: &Wavetable, phase: f64, position: f32, freq_hz: f32) -> f32 {
    let num_frames = table.frames.len();
    if num_frames == 0 {
        return 0.0;
    }

    // Frame interpolation (position parameter)
    let frame_pos = position.clamp(0.0, 1.0) * (num_frames - 1) as f32;
    let frame_lo = (frame_pos as usize).min(num_frames - 1);
    let frame_hi = (frame_lo + 1).min(num_frames - 1);
    let frame_frac = frame_pos - frame_lo as f32;

    // Mip-map level selection based on frequency
    let octave_f = if freq_hz > 8.175799 {
        (freq_hz / 8.175799).log2()
    } else {
        0.0
    };
    let oct_lo = (octave_f as usize).min(NUM_OCTAVES - 2);
    let oct_hi = oct_lo + 1;
    let oct_frac = (octave_f - oct_lo as f32).clamp(0.0, 1.0);

    // 4 lookups: 2 frames x 2 mip levels, bilinear interpolation
    let s00 = cubic_read(&table.frames[frame_lo].mip_levels[oct_lo], phase);
    let s01 = cubic_read(&table.frames[frame_lo].mip_levels[oct_hi], phase);
    let s10 = cubic_read(&table.frames[frame_hi].mip_levels[oct_lo], phase);
    let s11 = cubic_read(&table.frames[frame_hi].mip_levels[oct_hi], phase);

    let lo = s00 + oct_frac * (s01 - s00);
    let hi = s10 + oct_frac * (s11 - s10);
    lo + frame_frac * (hi - lo)
}

/// Read a single mip level with cubic Hermite interpolation.
fn cubic_read(table: &[f32], phase: f64) -> f32 {
    let len = table.len();
    let pos = phase * len as f64;
    let i = pos as usize;
    let frac = (pos - i as f64) as f32;

    let s0 = table[(i + len - 1) % len];
    let s1 = table[i % len];
    let s2 = table[(i + 1) % len];
    let s3 = table[(i + 2) % len];

    // Hermite polynomial
    let c0 = s1;
    let c1 = 0.5 * (s2 - s0);
    let c2 = s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3;
    let c3 = 0.5 * (s3 - s0) + 1.5 * (s1 - s2);
    ((c3 * frac + c2) * frac + c1) * frac + c0
}

/// Convert MIDI note (fractional) to frequency in Hz.
#[inline]
pub fn midi_to_freq(note: f32) -> f32 {
    440.0 * 2.0f32.powf((note - 69.0) / 12.0)
}

/// Compute phase increment for a given frequency and sample rate.
#[inline]
pub fn phase_inc(freq_hz: f32, sample_rate: f32) -> f64 {
    freq_hz as f64 / sample_rate as f64
}
