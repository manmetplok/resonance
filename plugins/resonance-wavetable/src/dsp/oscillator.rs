/// Wavetable oscillator with band-limited mip-map selection and cubic Hermite
/// interpolation (6-point Lagrange on the dense bass levels).
///
/// The read is split into two halves:
///
/// * [`plan_tap`] resolves *which* mip levels and frames to blend and with
///   what weights. Its inputs — the scan position and the oscillator
///   frequency — are control-rate quantities (they change only when the mod
///   matrix ticks or portamento moves the pitch), yet it is where the
///   expensive `log2` lives.
/// * [`read_tap`] does the per-sample work: two to four interpolated reads
///   at the current phase, blended with the precomputed weights.
///
/// The caller ([`crate::dsp::render`]) caches a [`TableTap`] per unison
/// sub-oscillator and only re-plans when its inputs actually change, which
/// takes `log2` (and, via the caller, `exp2`/`sin`/`cos`) off the per-sample
/// path entirely. `plan_tap` + `read_tap` compute exactly the same arithmetic
/// the old fused `read_wavetable` did, in the same order, so output is
/// bit-identical.
use crate::dsp::wavetable::{Wavetable, NUM_OCTAVES, WAVETABLE_SIZE};

/// Lowest MIDI note's frequency (C-1); the reference for mip-level selection.
const MIP_BASE_HZ: f32 = 8.175799;

/// Resolved read plan for one oscillator: the frames and mip levels to blend
/// and the crossfade weights between them. Valid until the scan position or
/// the oscillator frequency changes.
#[derive(Clone, Copy, Default)]
pub struct TableTap {
    /// Lower frame index; `frame_lo + 1` is used when `frame_hi_needed`.
    pub frame_lo: u32,
    /// Lower mip level; `oct_lo + 1` is used when `oct_hi_needed`.
    pub oct_lo: u32,
    pub frame_frac: f32,
    pub oct_frac: f32,
    pub frame_hi_needed: bool,
    pub oct_hi_needed: bool,
    /// Read with the 6-point Lagrange kernel instead of cubic Hermite:
    /// set for the dense bass levels (see [`HQ_INTERP_MAX_LEVEL`]).
    pub hq_interp: bool,
    /// False when the table has no frames at all; [`read_tap`] returns 0.
    pub valid: bool,
}

/// Sample rate the bundled mip levels are band-limited for (see `build.rs`):
/// level `k` holds partials up to `TABLE_SAMPLE_RATE / (2 * f_k)`, with
/// `f_k = MIP_BASE_HZ * 2^k`.
const TABLE_SAMPLE_RATE: f32 = 44_100.0;

/// Highest sample rate the selection scales its band limit to. Above it the
/// selection stays where it is at 48 kHz: the extra levels would only add
/// partials above 24 kHz, which nobody hears, and would read denser tables
/// whose cubic-interpolation images are the one remaining source of
/// inharmonic energy at low notes.
const MAX_BAND_SAMPLE_RATE: f32 = 48_000.0;

/// Highest mip level read with the 6-point Lagrange kernel. Levels up to
/// here hold more than 256 partials in a 2048-sample table (level 3: 337,
/// level 2: 674), where a 4-point cubic Hermite read's images — at
/// `(2048 - h) * f`, folded at the output rate — left a -69 dB inharmonic
/// floor on bass notes at 44.1/48 kHz (FU-G2a); Lagrange takes it below
/// -80 dB. Sparser levels keep the cheaper Hermite read, whose images are
/// already under -80 dB there, so the extra reads cost only bass notes.
const HQ_INTERP_MAX_LEVEL: usize = 3;

/// Width, in octaves, of the crossfade into the next (darker) level at the
/// top of each level's range. Keeps the timbre continuous across a level
/// boundary — during a glide or a pitch bend — instead of stepping.
const MIP_CROSSFADE_OCTAVES: f32 = 0.25;

/// Pick the mip levels for a fundamental of `freq_hz` at `sample_rate`:
/// `(level, weight)` — read `level`, blended with `level + 1` by `weight`.
///
/// Both levels are always band-limited for the playing frequency: no partial
/// lands above Nyquist. With `x = log2(f / f_0)` scaled to the table's
/// sample rate, level `L` is alias-free iff `L >= x`, so the selection rounds
/// *up* to `ceil(x)` and, over the last [`MIP_CROSSFADE_OCTAVES`] below each
/// boundary, fades into `ceil(x) + 1` — which is where the next octave's
/// selection starts, so the weight is continuous in frequency.
///
/// This used to be `floor(x)` blended toward `floor(x) + 1` by the fraction,
/// i.e. the level *below* the playing pitch, whose top partials reach up to
/// twice Nyquist and fold back as inharmonic "birdies" (review finding
/// DSP-03). The price of alias-free selection is bandwidth: the top partial
/// now sits between half and all of Nyquist rather than above it.
#[inline]
pub fn select_mip(freq_hz: f32, sample_rate: f32) -> (usize, f32) {
    let band_sr = sample_rate.min(MAX_BAND_SAMPLE_RATE);
    // Frequency as seen by the tables: at a higher sample rate a level stays
    // alias-free up to a proportionally higher fundamental.
    let ratio = freq_hz * (TABLE_SAMPLE_RATE / band_sr) / MIP_BASE_HZ;
    let x = if ratio > 0.0 { ratio.log2() } else { f32::NEG_INFINITY };

    let top = NUM_OCTAVES - 1;
    let lo = x.ceil().clamp(0.0, top as f32);
    let weight = ((x - lo) / MIP_CROSSFADE_OCTAVES + 1.0).clamp(0.0, 1.0);
    let lo = lo as usize;
    if lo >= top {
        // Above the top level's range nothing darker exists. The top level
        // (11, design pitch C10 at 44.1 kHz) holds the fundamental alone,
        // so it stays alias-free until the fundamental itself passes
        // Nyquist. It used to be level 10, whose second partial folded
        // from about 11 kHz up (FU-G2b).
        (top, 0.0)
    } else {
        (lo, weight)
    }
}

/// Resolve the mip/frame blend for a given scan position and frequency.
///
/// - `table`: the wavetable to read from
/// - `position`: wavetable scan position (0.0..1.0)
/// - `freq_hz`: current oscillator frequency (for mip-map selection)
/// - `sample_rate`: the rate the oscillator runs at (for the band limit)
#[inline]
pub fn plan_tap(table: &Wavetable, position: f32, freq_hz: f32, sample_rate: f32) -> TableTap {
    let num_frames = table.num_frames();
    if num_frames == 0 {
        return TableTap::default();
    }

    // Frame interpolation (position parameter)
    let frame_pos = position.clamp(0.0, 1.0) * (num_frames - 1) as f32;
    let frame_lo = (frame_pos as usize).min(num_frames - 1);
    let frame_frac = frame_pos - frame_lo as f32;
    // Skip the upper-frame fetch when we landed exactly on a frame
    // (no inter-frame interpolation needed). Halves the table reads
    // for static-position presets.
    let frame_hi_needed = frame_frac > 0.0 && frame_lo + 1 < num_frames;

    let (oct_lo, oct_frac) = select_mip(freq_hz, sample_rate);
    let oct_hi_needed = oct_frac > 0.0 && oct_lo + 1 < NUM_OCTAVES;

    TableTap {
        frame_lo: frame_lo as u32,
        oct_lo: oct_lo as u32,
        frame_frac,
        oct_frac,
        frame_hi_needed,
        oct_hi_needed,
        hq_interp: oct_lo <= HQ_INTERP_MAX_LEVEL,
        valid: true,
    }
}

/// Sample a planned tap at `phase` (0.0..1.0).
///
/// The phase-to-index conversion is done once and shared by all four possible
/// reads; the old code recomputed it inside every `cubic_read`.
#[inline]
pub fn read_tap(table: &Wavetable, tap: &TableTap, phase: f64) -> f32 {
    if !tap.valid {
        return 0.0;
    }

    let idx = PhaseIndex::new(phase);
    // One well-predicted branch per sample: `hq_interp` is fixed per tap.
    let read = |t: &[f32; WAVETABLE_SIZE]| {
        if tap.hq_interp {
            idx.read6(t)
        } else {
            idx.read(t)
        }
    };
    let frame_lo = tap.frame_lo as usize;
    let oct_lo = tap.oct_lo as usize;

    let lo = if tap.oct_hi_needed {
        let s00 = read(table.mip(frame_lo, oct_lo));
        let s01 = read(table.mip(frame_lo, oct_lo + 1));
        s00 + tap.oct_frac * (s01 - s00)
    } else {
        read(table.mip(frame_lo, oct_lo))
    };

    if !tap.frame_hi_needed {
        return lo;
    }

    let hi = if tap.oct_hi_needed {
        let s10 = read(table.mip(frame_lo + 1, oct_lo));
        let s11 = read(table.mip(frame_lo + 1, oct_lo + 1));
        s10 + tap.oct_frac * (s11 - s10)
    } else {
        read(table.mip(frame_lo + 1, oct_lo))
    };

    lo + tap.frame_frac * (hi - lo)
}

/// Phase resolved to a sample index plus fraction, shared across the up-to-four
/// mip reads a single oscillator sample performs.
struct PhaseIndex {
    i: usize,
    frac: f32,
}

impl PhaseIndex {
    #[inline]
    fn new(phase: f64) -> Self {
        let pos = phase * WAVETABLE_SIZE as f64;
        let i = pos as usize;
        Self {
            i,
            frac: (pos - i as f64) as f32,
        }
    }

    /// Read a single mip level with cubic Hermite interpolation.
    ///
    /// The table is a fixed-size array of a power-of-two length, so the
    /// wrap-around masks below are provably in bounds: no bounds checks, no
    /// length assert, and the wrap is an `and` rather than the `idiv` a
    /// runtime-length slice would need.
    #[inline]
    fn read(&self, table: &[f32; WAVETABLE_SIZE]) -> f32 {
        const MASK: usize = WAVETABLE_SIZE - 1;
        let i = self.i;
        let frac = self.frac;

        let s0 = table[i.wrapping_sub(1) & MASK];
        let s1 = table[i & MASK];
        let s2 = table[(i + 1) & MASK];
        let s3 = table[(i + 2) & MASK];

        // Hermite polynomial
        let c0 = s1;
        let c1 = 0.5 * (s2 - s0);
        let c2 = s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3;
        let c3 = 0.5 * (s3 - s0) + 1.5 * (s1 - s2);
        ((c3 * frac + c2) * frac + c1) * frac + c0
    }

    /// Read a single mip level with 6-point Lagrange interpolation; same
    /// provably-in-bounds power-of-two wrap as [`Self::read`].
    #[inline]
    fn read6(&self, table: &[f32; WAVETABLE_SIZE]) -> f32 {
        const MASK: usize = WAVETABLE_SIZE - 1;
        let i = self.i;
        resonance_dsp::lagrange6(
            table[i.wrapping_sub(2) & MASK],
            table[i.wrapping_sub(1) & MASK],
            table[i & MASK],
            table[(i + 1) & MASK],
            table[(i + 2) & MASK],
            table[(i + 3) & MASK],
            self.frac,
        )
    }
}

/// Convenience wrapper that plans and reads in one go. Kept for tests and
/// non-realtime callers (viz); the audio path uses the split form so the
/// planning stays off the per-sample loop.
#[inline]
pub fn read_wavetable(
    table: &Wavetable,
    phase: f64,
    position: f32,
    freq_hz: f32,
    sample_rate: f32,
) -> f32 {
    read_tap(table, &plan_tap(table, position, freq_hz, sample_rate), phase)
}

/// Convert MIDI note (fractional) to frequency in Hz.
///
/// `exp2` rather than `2.0_f32.powf(_)`: powf has to handle a runtime
/// base, which costs ~3× exp2 on x86.
#[inline]
pub fn midi_to_freq(note: f32) -> f32 {
    440.0 * ((note - 69.0) * (1.0 / 12.0)).exp2()
}

/// Compute phase increment for a given frequency and sample rate.
#[inline]
pub fn phase_inc(freq_hz: f32, sample_rate: f32) -> f64 {
    freq_hz as f64 / sample_rate as f64
}
