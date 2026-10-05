//! Tests for `resonance_metering::decay` (reverb-algorithms.md R0): the
//! harness has to recover known answers from synthetic responses before
//! its numbers on a reverb mean anything.

use resonance_metering::decay::{
    band_decay_times, decay_times, echo_density_profile, energy_decay_curve, late_iacc,
    modal_peakiness_db, mono_fold_db, octave_band, ImpulseReport, ECHO_DENSITY_WINDOW_S,
    MONO_FOLD_FLOOR_DB, OCTAVE_BANDS_HZ, PEAKINESS_START_S,
};

const SR: f32 = 48_000.0;

/// Deterministic standard-normal noise (xorshift64* + Box–Muller).
struct Gauss(u64);

impl Gauss {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        ((v >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    fn next(&mut self) -> f32 {
        let (u1, u2) = (self.uniform(), self.uniform());
        ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
    }

    fn take(&mut self, n: usize) -> Vec<f32> {
        (0..n).map(|_| self.next()).collect()
    }
}

/// Amplitude envelope that falls 60 dB in `t60` seconds.
fn env(i: usize, t60: f32) -> f32 {
    10f32.powf(-3.0 * i as f32 / (t60 * SR))
}

/// Gaussian noise under an exponential decay of `t60`, `len_s` long.
fn decaying_noise(seed: u64, t60: f32, len_s: f32) -> Vec<f32> {
    let n = (len_s * SR) as usize;
    let mut g = Gauss(seed);
    (0..n).map(|i| g.next() * env(i, t60)).collect()
}

fn assert_close(what: &str, got: Option<f32>, want: f32, tol: f32) {
    let got = got.unwrap_or_else(|| panic!("{what}: not measured"));
    let err = (got - want).abs() / want;
    assert!(
        err <= tol,
        "{what}: {got:.4} s vs {want:.4} s ({:.2} %)",
        err * 100.0
    );
}

#[test]
fn broadband_t60_is_recovered_within_two_percent() {
    for (seed, t60) in [(1, 0.5f32), (2, 2.0), (3, 8.0)] {
        let ir = decaying_noise(seed, t60, 1.2 * t60 + 0.2);
        let t = decay_times(&ir, SR);
        assert_close(&format!("T30 @ {t60} s"), t.t30, t60, 0.02);
        assert_close(&format!("T20 @ {t60} s"), t.t20, t60, 0.02);
    }
}

#[test]
fn edt_of_a_pure_exponential_equals_t30() {
    for (seed, t60) in [(11, 0.5f32), (12, 2.0), (13, 8.0)] {
        let ir = decaying_noise(seed, t60, 1.2 * t60 + 0.2);
        let t = decay_times(&ir, SR);
        let t30 = t.t30.expect("t30");
        assert_close(&format!("EDT @ {t60} s"), t.edt, t30, 0.02);
    }
}

/// A floor 50 dB down meets a 2 s decay at 1.67 s, only 15 dB past the
/// end of the T30 fit. Plain Schroeder integration bends the curve there;
/// the Lundeby truncation must not.
#[test]
fn noise_floor_is_truncated_not_integrated() {
    let t60 = 2.0;
    let mut floor = Gauss(99);
    let ir: Vec<f32> = decaying_noise(5, t60, 4.0)
        .into_iter()
        .map(|x| x + 10f32.powf(-50.0 / 20.0) * floor.next())
        .collect();
    let edc = energy_decay_curve(&ir, SR);
    let cross = edc.truncation as f32 / SR;
    assert!(
        (1.4..2.0).contains(&cross),
        "truncation at {cross:.2} s, expected near 1.67 s"
    );
    assert!(
        (-56.0..-44.0).contains(&edc.noise_floor_db),
        "noise floor {:.1} dB",
        edc.noise_floor_db
    );
    assert_close("T30 over a -50 dB floor", edc.t30(), t60, 0.02);
    assert_close("T20 over a -50 dB floor", edc.t20(), t60, 0.02);
}

#[test]
fn leading_silence_does_not_lengthen_edt() {
    let mut ir = vec![0.0f32; (0.05 * SR) as usize];
    ir.extend(decaying_noise(21, 1.0, 1.5));
    let t = decay_times(&ir, SR);
    assert_close("EDT after a 50 ms pre-delay", t.edt, 1.0, 0.03);
    assert_close("T30 after a 50 ms pre-delay", t.t30, 1.0, 0.02);
}

/// Four octave bands, two octaves apart, each its own band-limited noise
/// under its own decay: every band's T30 is its own T60.
#[test]
fn per_band_t60s_are_recovered_within_two_percent() {
    let bands = [
        (125.0f32, 2.0f32),
        (500.0, 1.4),
        (2_000.0, 1.0),
        (8_000.0, 0.6),
    ];
    let n = (2.6 * SR) as usize;
    let mut ir = vec![0.0f32; n];
    for (k, &(fc, t60)) in bands.iter().enumerate() {
        let noise = octave_band(&Gauss(40 + k as u64).take(n), SR, fc);
        for (i, (slot, x)) in ir.iter_mut().zip(noise).enumerate() {
            *slot += x * env(i, t60);
        }
    }
    let measured = band_decay_times(&ir, SR);
    for (fc, t60) in bands {
        let band = measured.iter().find(|b| b.center_hz == fc).unwrap();
        assert_close(&format!("T30 @ {fc} Hz"), band.times.t30, t60, 0.02);
    }
    assert_eq!(measured.len(), OCTAVE_BANDS_HZ.len());
}

#[test]
fn silence_measures_nothing() {
    let zeros = vec![0.0f32; 48_000];
    let t = decay_times(&zeros, SR);
    assert_eq!((t.edt, t.t20, t.t30), (None, None, None));
    let r = ImpulseReport::analyze(&zeros, &zeros, SR);
    assert!(r.finite);
    assert_eq!(r.peakiness_db, None);
    assert_eq!(r.late_iacc, None);
    assert_eq!(r.mono_fold_db, None);
    assert_eq!(r.echo_density.time_to_reach(0.5), None);
}

/// A response too short to fall 35 dB has no T30, rather than a wrong one.
#[test]
fn truncated_response_reports_no_t30() {
    let ir = decaying_noise(7, 8.0, 1.5);
    let t = decay_times(&ir, SR);
    assert_eq!(t.t30, None);
}

#[test]
fn gaussian_noise_has_echo_density_one() {
    let noise = Gauss(3).take(SR as usize);
    let d = echo_density_profile(&noise, SR, ECHO_DENSITY_WINDOW_S);
    let mean = d.mean_between(0.05, 0.95).unwrap();
    assert!((mean - 1.0).abs() < 0.03, "mean density {mean:.3}");
    let min = d.values[50..950]
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min);
    assert!(min > 0.8, "min density {min:.3}");
    // Defined from t = 0 (edge-renormalised window).
    assert!(d.time_to_reach(0.8).unwrap() < 0.005);
}

#[test]
fn sparse_impulse_train_has_low_echo_density() {
    // One click every 5 ms: four per window.
    let mut train = vec![0.0f32; SR as usize];
    for i in (0..train.len()).step_by(240) {
        train[i] = if (i / 240) % 2 == 0 { 1.0 } else { -0.7 };
    }
    let d = echo_density_profile(&train, SR, ECHO_DENSITY_WINDOW_S);
    let mean = d.mean_between(0.05, 0.95).unwrap();
    assert!(mean < 0.1, "mean density {mean:.3}");
    assert_eq!(d.time_to_reach(1.0), None);
}

#[test]
fn density_build_up_is_timed() {
    // Sparse clicks for 50 ms, then noise: density reaches 1 near 50 ms
    // (the 20 ms window blurs it by up to half its length).
    let mut g = Gauss(8);
    let ir: Vec<f32> = (0..SR as usize)
        .map(|i| {
            if i < 2_400 {
                if i % 480 == 0 {
                    1.0
                } else {
                    0.0
                }
            } else {
                g.next()
            }
        })
        .collect();
    let d = echo_density_profile(&ir, SR, ECHO_DENSITY_WINDOW_S);
    let t = d.time_to_reach(0.9).unwrap();
    assert!(
        (0.040..0.062).contains(&t),
        "density 0.9 at {:.1} ms",
        t * 1e3
    );
}

#[test]
fn iacc_of_identical_and_independent_channels() {
    let l = decaying_noise(1, 2.0, 2.0);
    let r = decaying_noise(2, 2.0, 2.0);
    let same = late_iacc(&l, &l, SR, 0.08).unwrap();
    assert!(same > 0.999, "identical IACC {same}");
    let indep = late_iacc(&l, &r, SR, 0.08).unwrap();
    assert!(indep < 0.05, "independent IACC {indep}");
    // A 0.5 ms delay is inside the ±1 ms lag window.
    let mut delayed = vec![0.0f32; 24];
    delayed.extend_from_slice(&l[..l.len() - 24]);
    let lagged = late_iacc(&l, &delayed, SR, 0.08).unwrap();
    assert!(lagged > 0.99, "delayed-copy IACC {lagged}");
}

#[test]
fn mono_fold_of_identical_independent_and_inverted_channels() {
    let l = decaying_noise(1, 2.0, 2.0);
    let r = decaying_noise(2, 2.0, 2.0);
    let inv: Vec<f32> = l.iter().map(|x| -x).collect();
    let same = mono_fold_db(&l, &l, SR, 0.08).unwrap();
    assert!(same.abs() < 1e-4, "identical {same}");
    let indep = mono_fold_db(&l, &r, SR, 0.08).unwrap();
    assert!((indep + 3.01).abs() < 0.2, "independent {indep}");
    assert_eq!(mono_fold_db(&l, &inv, SR, 0.08), Some(MONO_FOLD_FLOOR_DB));
}

#[test]
fn sines_are_peaky_and_noise_is_not() {
    let n = (1.5 * SR) as usize;
    let mut g = Gauss(77);
    let tones: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let s: f32 = [233.0f32, 612.0, 1_490.0, 3_310.0]
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin())
                .sum();
            (s + 0.01 * g.next()) * env(i, 3.0)
        })
        .collect();
    let noise = decaying_noise(78, 3.0, 1.5);
    let (lo, hi) = (100.0, 8_000.0);
    let p_tones = modal_peakiness_db(&tones, SR, PEAKINESS_START_S, lo, hi).unwrap();
    let p_noise = modal_peakiness_db(&noise, SR, PEAKINESS_START_S, lo, hi).unwrap();
    println!("peakiness: tones {p_tones:.1} dB, noise {p_noise:.1} dB");
    assert!(p_tones > 30.0, "tones {p_tones:.1} dB");
    assert!(p_noise < 12.0, "noise {p_noise:.1} dB");
}

#[test]
fn impulse_report_bundles_the_metrics() {
    let l = decaying_noise(31, 1.5, 2.2);
    let r = decaying_noise(32, 1.5, 2.2);
    let rep = ImpulseReport::analyze(&l, &r, SR);
    println!("{}\n{rep}", ImpulseReport::table_header());
    assert!(rep.finite);
    assert_close("report T30", rep.broadband.t30, 1.5, 0.02);
    assert_close("report mid T30", rep.mid_t30(), 1.5, 0.03);
    assert!(rep.late_iacc.unwrap() < 0.05);
    assert!((rep.mono_fold_db.unwrap() + 3.0).abs() < 0.3);
    assert!(rep.echo_density.time_to_reach(0.9).unwrap() < 0.005);
    assert!(rep.rms_2s_dbfs > -30.0 && rep.rms_2s_dbfs < 0.0);
}
