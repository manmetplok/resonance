//! `spectrum` detail (warmth-width-depth.md §7.1, slice W0): 1/3-octave
//! LTAS, tilt, centroid, low-mid/presence, presence peakiness, air ratio
//! and resonance peaks, against synthetic signals whose answers are known
//! analytically.

mod common;

use common::{coloured_noise, sine_mono};
use resonance_dsp::Biquad;
use resonance_metering::detail::spectrum::{
    third_octave_center_hz, LEVEL_FLOOR_DB, THIRD_OCTAVE_NOMINAL_HZ,
};
use resonance_metering::detail::{spectrum_detail, SpectrumDetail, THIRD_OCTAVE_BANDS};

const SR: f32 = 48_000.0;
/// 2^20 samples ≈ 21.8 s at 48 kHz: enough Welch frames that per-band
/// variance is well under the tolerances below.
const LEN: usize = 1 << 20;

fn pink() -> Vec<f32> {
    coloured_noise(LEN, 1.0, -20.0, 0x5EED_0001)
}

fn measure_mono(x: &[f32]) -> SpectrumDetail {
    spectrum_detail(SR, x, x)
}

fn filtered(x: &[f32], mut filter: Biquad) -> Vec<f32> {
    // Run twice and keep the second pass: the noise is periodic in its
    // length, so this is the filter's steady state with no start-up
    // transient.
    for &s in x {
        filter.process(s);
    }
    x.iter().map(|&s| filter.process(s)).collect()
}

#[test]
fn pink_noise_measures_minus_three_db_per_octave() {
    let d = measure_mono(&pink());
    let tilt = d.tilt_db_per_oct.expect("pink noise has a tilt");
    assert!((tilt - -3.0).abs() <= 0.1, "pink tilt {tilt} dB/oct, want -3.0 ± 0.1");
}

#[test]
fn white_noise_measures_flat() {
    let d = measure_mono(&coloured_noise(LEN, 0.0, -20.0, 0x5EED_0002));
    let tilt = d.tilt_db_per_oct.expect("white noise has a tilt");
    assert!(tilt.abs() <= 0.1, "white tilt {tilt} dB/oct, want 0 ± 0.1");
}

#[test]
fn pink_noise_reads_flat_across_third_octaves() {
    let d = measure_mono(&pink());
    assert_eq!(d.third_octave.len(), THIRD_OCTAVE_BANDS);
    assert_eq!(THIRD_OCTAVE_NOMINAL_HZ.len(), THIRD_OCTAVE_BANDS);
    // 50 Hz .. 16 kHz: equal power per band.
    let mid = &d.third_octave[4..=29];
    let max = mid.iter().copied().fold(f32::MIN, f32::max);
    let min = mid.iter().copied().fold(f32::MAX, f32::min);
    assert!(max - min < 0.6, "pink third-octaves should be flat, spread {} dB: {mid:?}", max - min);
    // The synthesised 1/k spectrum runs from bin 1 to LEN/2, so its total
    // power is the harmonic number H(LEN/2) ≈ ln(LEN/2) + γ, and a
    // 1/3-octave band holds ln(2)/3 of it. A full-scale sine reads 0 dB,
    // hence the +3.01 on a -20 dBFS RMS signal.
    let harmonic = ((LEN / 2) as f32).ln() + 0.577_2;
    let expect = -20.0 + 10.0 * ((2.0f32.ln() / 3.0) / harmonic).log10() + 3.01;
    for &l in mid {
        assert!((l - expect).abs() < 0.6, "band level {l}, want ≈ {expect}");
    }
}

#[test]
fn a_three_db_high_shelf_moves_the_bands_above_its_corner() {
    let dry = pink();
    let mut shelf = Biquad::default();
    shelf.set_high_shelf(SR, 2_000.0, 0.707, 3.0);
    let probe = shelf.clone();
    let wet = filtered(&dry, shelf);

    let before = measure_mono(&dry);
    let after = measure_mono(&wet);
    for i in 0..THIRD_OCTAVE_BANDS {
        let fc = third_octave_center_hz(i);
        let want = 20.0 * probe.magnitude(fc as f32, SR).log10();
        let got = after.third_octave[i] - before.third_octave[i];
        assert!(
            (got - want).abs() < 0.25,
            "band {} Hz moved {got:.2} dB, the shelf's response there is {want:.2} dB",
            THIRD_OCTAVE_NOMINAL_HZ[i]
        );
    }
    // Explicitly: the lows stay put and the top rises by the full 3 dB.
    for i in 0..=13 {
        assert!((after.third_octave[i] - before.third_octave[i]).abs() < 0.1, "band {i} moved");
    }
    for i in 26..=29 {
        assert!((after.third_octave[i] - before.third_octave[i] - 3.0).abs() < 0.2, "band {i}");
    }
    assert!(
        after.tilt_db_per_oct.unwrap() > before.tilt_db_per_oct.unwrap() + 0.2,
        "a treble boost is less tilted"
    );
    assert!(after.centroid_hz.unwrap() > before.centroid_hz.unwrap());
    assert!(after.lowmid_presence_db.unwrap() < before.lowmid_presence_db.unwrap() - 1.0);
    assert!(after.air_ratio_db.unwrap() > before.air_ratio_db.unwrap() + 1.0);
}

#[test]
fn a_sine_lands_in_its_band_at_its_level() {
    let (l, r) = sine_mono(SR, 1_000.0, -12.0, 4.0);
    let d = spectrum_detail(SR, &l, &r);
    let (loudest, level) = d
        .third_octave
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap();
    assert_eq!(THIRD_OCTAVE_NOMINAL_HZ[loudest], 1_000.0);
    assert!((level - -12.0).abs() < 0.2, "a -12 dBFS sine reads {level} dB");
    let centroid = d.centroid_hz.unwrap();
    assert!((centroid - 1_000.0).abs() < 5.0, "centroid {centroid}");
}

#[test]
fn pink_noise_ratios_match_their_bandwidths() {
    let d = measure_mono(&pink());
    // Equal power per octave: ratios are ratios of octave spans.
    let lm = 10.0 * ((500.0f32 / 150.0).ln() / (5_000.0f32 / 2_000.0).ln()).log10();
    let air = 10.0 * ((16_000.0f32 / 8_000.0).ln() / (20_000.0f32 / 20.0).ln()).log10();
    let got_lm = d.lowmid_presence_db.unwrap();
    let got_air = d.air_ratio_db.unwrap();
    assert!((got_lm - lm).abs() < 0.2, "lowmid_presence {got_lm}, want {lm}");
    assert!((got_air - air).abs() < 0.2, "air_ratio {got_air}, want {air}");
    let peaky = d.presence_peakiness_db.unwrap();
    assert!(peaky < 0.5, "pink is even in 2-5 kHz, peakiness {peaky}");
    assert!(d.peaks.is_empty(), "pink noise has no resonances: {:?}", d.peaks);
}

#[test]
fn a_presence_resonance_raises_peakiness_and_is_listed_as_a_peak() {
    let dry = pink();
    let mut bell = Biquad::default();
    bell.set_bell(SR, 3_150.0, 8.0, 9.0);
    let wet = filtered(&dry, bell);
    let before = measure_mono(&dry);
    let after = measure_mono(&wet);
    assert!(
        after.presence_peakiness_db.unwrap() > before.presence_peakiness_db.unwrap() + 3.0,
        "a 3.15 kHz resonance is presence peakiness: {:?} -> {:?}",
        before.presence_peakiness_db,
        after.presence_peakiness_db
    );
    let top = after.peaks.first().expect("the resonance is found");
    assert!(
        (top.freq_hz / 3_150.0 - 1.0).abs() < 0.03,
        "strongest peak at {} Hz, want 3150",
        top.freq_hz
    );
    assert!(top.excess_db > 3.0, "excess {}", top.excess_db);
}

#[test]
fn peaks_are_capped_at_five_and_sorted() {
    let mut x = pink();
    for (i, f) in [110.0f32, 440.0, 1_000.0, 2_500.0, 5_000.0, 9_000.0, 13_000.0].iter().enumerate() {
        let amp = 0.02 * (1.0 + i as f32 * 0.3);
        for (n, s) in x.iter_mut().enumerate() {
            *s += amp * (std::f32::consts::TAU * f * n as f32 / SR).sin();
        }
    }
    let d = measure_mono(&x);
    assert_eq!(d.peaks.len(), 5, "{:?}", d.peaks);
    for pair in d.peaks.windows(2) {
        assert!(pair[0].excess_db >= pair[1].excess_db, "sorted by excess: {:?}", d.peaks);
    }
}

#[test]
fn silence_has_no_ratios_and_floors_every_band() {
    let silence = vec![0.0f32; 48_000];
    let d = spectrum_detail(SR, &silence, &silence);
    assert_eq!(d.third_octave, vec![LEVEL_FLOOR_DB; THIRD_OCTAVE_BANDS]);
    assert_eq!(d.tilt_db_per_oct, None);
    assert_eq!(d.centroid_hz, None);
    assert_eq!(d.lowmid_presence_db, None);
    assert_eq!(d.presence_peakiness_db, None);
    assert_eq!(d.air_ratio_db, None);
    assert!(d.peaks.is_empty());
}

#[test]
fn a_buffer_shorter_than_one_frame_still_reads_its_level() {
    let (l, r) = sine_mono(SR, 1_000.0, -6.0, 0.25);
    let d = spectrum_detail(SR, &l, &r);
    assert!((d.third_octave[17] - -6.0).abs() < 0.5, "{}", d.third_octave[17]);
    let empty: Vec<f32> = Vec::new();
    let d = spectrum_detail(SR, &empty, &empty);
    assert_eq!(d.tilt_db_per_oct, None);
}

#[test]
fn side_content_counts_toward_the_ltas() {
    // Fully anti-phase: the mono sum is silent, the LTAS is not.
    let (l, _) = sine_mono(SR, 1_000.0, -12.0, 2.0);
    let r: Vec<f32> = l.iter().map(|s| -s).collect();
    let d = spectrum_detail(SR, &l, &r);
    assert!((d.third_octave[17] - -12.0).abs() < 0.2);
}

/// A typical mix's low end: −4.5 dB/oct noise, high-passed at 40 Hz with
/// 24 dB/oct (two Butterworth biquads). Its band levels rise out of the
/// filter and fall with the tilt, a broad concave hump around 60 Hz that
/// a mean-of-the-window reference read as a +1.4 dB "resonance".
fn high_passed_mix() -> Vec<f32> {
    let dry = coloured_noise(LEN, 1.5, -20.0, 0x5EED_0045);
    let mut hp1 = Biquad::default();
    hp1.set_high_pass(SR, 40.0, 0.541_196);
    let mut hp2 = Biquad::default();
    hp2.set_high_pass(SR, 40.0, 1.306_563);
    filtered(&filtered(&dry, hp1), hp2)
}

#[test]
fn a_broad_high_passed_low_end_is_not_a_resonance() {
    let d = measure_mono(&high_passed_mix());
    assert!(d.peaks.is_empty(), "a broad hump is no resonance: {:?}", d.peaks);
}

#[test]
fn a_narrow_resonance_on_the_high_passed_low_end_is_still_found() {
    // 1059 Hz sits on a band edge, so its energy splits over two bands.
    for freq in [150.0f32, 1_000.0, 1_059.5, 4_000.0] {
        let mut bell = Biquad::default();
        bell.set_bell(SR, freq, 8.0, 6.0);
        let d = measure_mono(&filtered(&high_passed_mix(), bell));
        assert_eq!(d.peaks.len(), 1, "{freq} Hz: exactly the resonance: {:?}", d.peaks);
        let top = d.peaks[0];
        assert!(
            (top.freq_hz / freq - 1.0).abs() < 0.05,
            "{freq} Hz: found at {} Hz",
            top.freq_hz
        );
        assert!(top.excess_db > 2.0, "{freq} Hz: excess {}", top.excess_db);
    }
}
