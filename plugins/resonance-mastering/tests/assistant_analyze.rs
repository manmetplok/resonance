use resonance_mastering::assistant::analyze::run;

#[test]
fn silence_produces_low_floor() {
    let l = vec![0.0_f32; 48_000];
    let r = vec![0.0_f32; 48_000];
    let result = run(48_000.0, &l, &r);
    assert!(result.integrated_lufs < -60.0);
    assert!(result.true_peak_dbtp < -100.0);
    for v in &result.spectrum_db {
        assert!(*v < -60.0, "silent spectrum bin = {v}");
    }
}

#[test]
fn sine_at_minus_23_dbfs_reads_minus_23_lufs() {
    let sr = 48_000.0_f32;
    let n = (sr * 3.0) as usize;
    let amp = 10.0_f32.powf(-23.0 / 20.0);
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * 1000.0 * std::f32::consts::TAU).sin() * amp;
        l[i] = s;
        r[i] = s;
    }
    let result = run(sr, &l, &r);
    assert!((result.integrated_lufs - -23.0).abs() < 0.3);
    assert!(result.duration_s > 2.9);
}
