//! Fractional-octave log binning for FFT magnitudes.
//!
//! The analyzer publishes a 1/6-octave band curve (20 Hz to 20 kHz) which
//! is well suited to comparing against tonal-balance target curves and
//! draws compactly at ~60 bars. Each target band gets the max of the
//! FFT bins whose centre frequency falls inside it, or an interpolated
//! value at the boundary if the FFT resolution is coarser than the band.

/// Number of 1/6-octave bands spanning the audible range.
/// `log2(20000/20) * 6 ≈ 60`.
pub const NUM_OCTAVE_BINS: usize = 60;
/// Lowest band centre frequency in Hz.
pub const LOW_HZ: f32 = 20.0;
/// Highest band centre frequency in Hz.
pub const HIGH_HZ: f32 = 20_000.0;

/// Table mapping a target octave band index to `(freq_low, freq_high)`
/// in Hz. Constructed once at analyzer startup so the per-FFT hot loop
/// is a simple scan.
pub struct OctaveTable {
    pub edges: Vec<f32>, // length NUM_OCTAVE_BINS + 1
}

impl OctaveTable {
    pub fn new() -> Self {
        let ratio = (HIGH_HZ / LOW_HZ).powf(1.0 / NUM_OCTAVE_BINS as f32);
        let edges = (0..=NUM_OCTAVE_BINS)
            .map(|i| LOW_HZ * ratio.powi(i as i32))
            .collect();
        Self { edges }
    }

    /// Centre frequency of a given band.
    pub fn center(&self, band: usize) -> f32 {
        (self.edges[band] * self.edges[band + 1]).sqrt()
    }

    /// Convert a magnitudes-in-dB array (one entry per FFT bin, linear in
    /// frequency) into `NUM_OCTAVE_BINS` 1/6-octave maxes.
    ///
    /// `mag_db[k]` corresponds to frequency `k * sample_rate / fft_size`.
    /// Each target band takes the max over the FFT bins whose centre
    /// frequency falls within the band edges. Bands with no FFT bins
    /// (i.e. below the first positive bin) are filled with the nearest
    /// neighbour value.
    pub fn aggregate(&self, mag_db: &[f32], sample_rate: f32, out: &mut [f32], floor_db: f32) {
        debug_assert_eq!(out.len(), NUM_OCTAVE_BINS);
        let fft_size = mag_db.len() * 2; // we got positive-frequency half
        let bin_hz = sample_rate / fft_size as f32;
        let max_k = mag_db.len();

        for (band, slot) in out.iter_mut().enumerate().take(NUM_OCTAVE_BINS) {
            let f_low = self.edges[band];
            let f_high = self.edges[band + 1];
            let k_low = ((f_low / bin_hz).floor() as isize).max(0) as usize;
            let k_high_excl = ((f_high / bin_hz).ceil() as usize + 1).min(max_k);

            if k_low >= k_high_excl {
                // No FFT bin falls inside this band; fall back to the
                // nearest bin centred inside the band range.
                let k = ((self.center(band) / bin_hz).round() as usize).min(max_k - 1);
                *slot = mag_db[k].max(floor_db);
                continue;
            }
            let mut peak = floor_db;
            for &v in &mag_db[k_low..k_high_excl] {
                if v > peak {
                    peak = v;
                }
            }
            *slot = peak;
        }
    }
}

impl Default for OctaveTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_covers_full_range() {
        let t = OctaveTable::new();
        assert_eq!(t.edges.len(), NUM_OCTAVE_BINS + 1);
        assert!((t.edges[0] - LOW_HZ).abs() < 0.1);
        assert!((t.edges[NUM_OCTAVE_BINS] - HIGH_HZ).abs() < 0.5);
    }

    #[test]
    fn aggregate_finds_tone_bin_peak() {
        // 8192-point FFT at 48 kHz: bin width ~5.86 Hz.
        let fft_half = 4096usize;
        let sr = 48_000.0_f32;
        let mut mag_db = vec![-96.0_f32; fft_half];
        // Plant a peak at 1 kHz.
        let k = (1000.0 / (sr / (fft_half as f32 * 2.0))) as usize;
        mag_db[k] = 0.0;

        let t = OctaveTable::new();
        let mut out = vec![0.0_f32; NUM_OCTAVE_BINS];
        t.aggregate(&mag_db, sr, &mut out, -96.0);

        // At least one band should carry the 0 dB peak.
        let max_out = out.iter().copied().fold(-200.0_f32, f32::max);
        assert!((max_out - 0.0).abs() < 0.01, "max = {max_out}");
    }

    #[test]
    fn out_of_range_bands_fall_back_to_nearest() {
        let t = OctaveTable::new();
        let mut out = vec![0.0_f32; NUM_OCTAVE_BINS];
        // Empty-ish FFT, just has one bin with a value.
        let mag_db = vec![-40.0_f32; 4096];
        t.aggregate(&mag_db, 48_000.0, &mut out, -96.0);
        for v in &out {
            assert!(*v >= -96.0 && *v <= -40.0);
        }
    }
}
