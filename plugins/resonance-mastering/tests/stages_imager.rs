use resonance_mastering::stages::imager::{Imager, ImagerConfig};

#[test]
fn disabled_passes_audio_unchanged() {
    let mut im = Imager::new(48_000.0);
    let mut l = vec![0.3, -0.4, 0.5, -0.6];
    let mut r = vec![0.2, -0.3, 0.4, -0.5];
    let el = l.clone();
    let er = r.clone();
    im.process_stereo(&mut l, &mut r, &ImagerConfig::default());
    assert_eq!(l, el);
    assert_eq!(r, er);
}

#[test]
fn width_one_is_identity() {
    let mut im = Imager::new(48_000.0);
    let mut l = vec![0.3_f32, -0.4, 0.5, -0.6];
    let mut r = vec![0.2_f32, -0.3, 0.4, -0.5];
    let el = l.clone();
    let er = r.clone();
    im.process_stereo(
        &mut l,
        &mut r,
        &ImagerConfig {
            enabled: true,
            width: 1.0,
            side_hpf_on: false,
            side_hpf_hz: 120.0,
        },
    );
    for (a, b) in l.iter().zip(el.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
    for (a, b) in r.iter().zip(er.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}

#[test]
fn width_zero_collapses_to_mono() {
    let mut im = Imager::new(48_000.0);
    // L and R start different but should both become 0.5*(L+R).
    let mut l = vec![0.4_f32, -0.6, 0.8, 0.0];
    let mut r = vec![0.0_f32, 0.0, -0.2, 0.4];
    let expected_mono: Vec<f32> = l.iter().zip(r.iter()).map(|(a, b)| 0.5 * (a + b)).collect();
    im.process_stereo(
        &mut l,
        &mut r,
        &ImagerConfig {
            enabled: true,
            width: 0.0,
            side_hpf_on: false,
            side_hpf_hz: 120.0,
        },
    );
    for (i, (a, b)) in l.iter().zip(expected_mono.iter()).enumerate() {
        assert!((a - b).abs() < 1e-6, "left[{i}] {a} vs {b}");
    }
    for (i, (a, b)) in r.iter().zip(expected_mono.iter()).enumerate() {
        assert!((a - b).abs() < 1e-6, "right[{i}] {a} vs {b}");
    }
}

#[test]
fn side_hpf_removes_low_frequencies_from_side_channel() {
    // Build a 50 Hz anti-phase signal (pure side content).
    // After side HPF at 200 Hz the side should be heavily
    // attenuated, so L and R converge to the mono sum (= 0).
    let sr = 48_000.0_f32;
    let mut im = Imager::new(sr);
    let n = 4096;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * 50.0 * std::f32::consts::TAU).sin() * 0.5;
        l[i] = s;
        r[i] = -s;
    }
    im.process_stereo(
        &mut l,
        &mut r,
        &ImagerConfig {
            enabled: true,
            width: 1.0,
            side_hpf_on: true,
            side_hpf_hz: 200.0,
        },
    );
    // Look at the settled tail.
    let tail = &l[n / 2..];
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(peak < 0.05, "low-freq side peak = {peak}");
}
