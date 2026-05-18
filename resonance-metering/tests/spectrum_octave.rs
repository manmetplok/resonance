use resonance_metering::spectrum::octave::{OctaveTable, HIGH_HZ, LOW_HZ, NUM_OCTAVE_BINS};

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
