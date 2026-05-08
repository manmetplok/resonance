/// Wavetable oscillator with band-limited mip-map selection and cubic Hermite interpolation.
use crate::wavetable::{Wavetable, NUM_OCTAVES, WAVETABLE_SIZE};

/// Read a wavetable with anti-aliased mip-map crossfading and frame interpolation.
///
/// - `table`: the wavetable to read from
/// - `phase`: oscillator phase (0.0..1.0, f64 for precision)
/// - `position`: wavetable scan position (0.0..1.0)
/// - `freq_hz`: current oscillator frequency (for mip-map selection)
#[inline]
pub fn read_wavetable(table: &Wavetable, phase: f64, position: f32, freq_hz: f32) -> f32 {
    let num_frames = table.frames.len();
    if num_frames == 0 {
        return 0.0;
    }

    // Frame interpolation (position parameter)
    let frame_pos = position.clamp(0.0, 1.0) * (num_frames - 1) as f32;
    let frame_lo = (frame_pos as usize).min(num_frames - 1);
    let frame_frac = frame_pos - frame_lo as f32;
    // Skip the upper-frame fetch when we landed exactly on a frame
    // (no inter-frame interpolation needed). Halves the table reads
    // for static-position presets.
    let frame_hi_needed = frame_frac > 0.0 && frame_lo + 1 < num_frames;

    // Mip-map level selection based on frequency
    let octave_f = if freq_hz > 8.175799 {
        (freq_hz / 8.175799).log2()
    } else {
        0.0
    };
    let oct_lo = (octave_f as usize).min(NUM_OCTAVES - 2);
    let oct_frac = (octave_f - oct_lo as f32).clamp(0.0, 1.0);
    let oct_hi_needed = oct_frac > 0.0;

    let frame_lo_levels = &table.frames[frame_lo].mip_levels;
    let lo = if oct_hi_needed {
        let s00 = cubic_read(&frame_lo_levels[oct_lo], phase);
        let s01 = cubic_read(&frame_lo_levels[oct_lo + 1], phase);
        s00 + oct_frac * (s01 - s00)
    } else {
        cubic_read(&frame_lo_levels[oct_lo], phase)
    };

    if !frame_hi_needed {
        return lo;
    }

    let frame_hi_levels = &table.frames[frame_lo + 1].mip_levels;
    let hi = if oct_hi_needed {
        let s10 = cubic_read(&frame_hi_levels[oct_lo], phase);
        let s11 = cubic_read(&frame_hi_levels[oct_lo + 1], phase);
        s10 + oct_frac * (s11 - s10)
    } else {
        cubic_read(&frame_hi_levels[oct_lo], phase)
    };

    lo + frame_frac * (hi - lo)
}

/// Read a single mip level with cubic Hermite interpolation.
///
/// Every mip level is sized exactly `WAVETABLE_SIZE` (a power of two),
/// which lets the wrap-around become a bitwise AND rather than the
/// `idiv` the slice-len version emitted. With ~16 wrap operations per
/// stereo sample at full polyphony, this is one of the larger
/// arithmetic savings on the synth's hot path.
#[inline]
fn cubic_read(table: &[f32], phase: f64) -> f32 {
    debug_assert_eq!(table.len(), WAVETABLE_SIZE);
    const MASK: usize = WAVETABLE_SIZE - 1;
    let pos = phase * WAVETABLE_SIZE as f64;
    let i = pos as usize;
    let frac = (pos - i as f64) as f32;

    // SAFETY of unchecked indexing: the &MASK reduces every index to
    // 0..WAVETABLE_SIZE, and the debug_assert above pins table.len()
    // at WAVETABLE_SIZE. Bounds-check elimination here is worth it on
    // the synth hot path because LLVM otherwise re-emits four checks
    // per call.
    let s0 = unsafe { *table.get_unchecked(i.wrapping_sub(1) & MASK) };
    let s1 = unsafe { *table.get_unchecked(i & MASK) };
    let s2 = unsafe { *table.get_unchecked((i + 1) & MASK) };
    let s3 = unsafe { *table.get_unchecked((i + 2) & MASK) };

    // Hermite polynomial
    let c0 = s1;
    let c1 = 0.5 * (s2 - s0);
    let c2 = s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3;
    let c3 = 0.5 * (s3 - s0) + 1.5 * (s1 - s2);
    ((c3 * frac + c2) * frac + c1) * frac + c0
}

/// Convert MIDI note (fractional) to frequency in Hz.
///
/// `exp2` rather than `2.0_f32.powf(_)`: powf has to handle a runtime
/// base, which costs ~3× exp2 on x86. With this function in the
/// per-sample-per-unison-per-osc loop, the difference is measurable.
#[inline]
pub fn midi_to_freq(note: f32) -> f32 {
    440.0 * ((note - 69.0) * (1.0 / 12.0)).exp2()
}

/// Compute phase increment for a given frequency and sample rate.
#[inline]
pub fn phase_inc(freq_hz: f32, sample_rate: f32) -> f64 {
    freq_hz as f64 / sample_rate as f64
}
