//! Band-limiting of the oscillator's mip-level selection.
//!
//! Mip level `k` is generated with partials up to `44100 / (2 * f_k)`,
//! `f_k = 8.1758 * 2^k`, so it is alias-free only for fundamentals up to
//! `f_k` (scaled by `sample_rate / 44100`). The selection used to take
//! `floor(log2(f / f_0))` — the level *below* the playing pitch — and blend
//! in the next one, so almost every note read a table with partials above
//! Nyquist, which folded back as inharmonic "birdies" up to about -26 dB.
//!
//! These tests read a saw straight from the oscillator (no filter, no
//! envelope) across the keyboard, FFT it, and compare the energy that is
//! *not* on a harmonic of the fundamental against the fundamental itself.

use resonance_wavetable::dsp::oscillator::{phase_inc, plan_tap, read_tap, select_mip};
use resonance_wavetable::dsp::wavetable::{load_bundled, Wavetable};

const N: usize = 1 << 15;
/// Skip the first samples so nothing depends on the start phase.
const WARMUP: usize = 64;
/// Bins on either side of a harmonic that count as that harmonic. The
/// 7-term Blackman-Harris main lobe is +-7 bins wide; its side lobes sit
/// near -180 dB, so leakage cannot masquerade as aliasing.
const GUARD_BINS: f64 = 10.0;
/// The acceptance bound from the review finding, for fundamentals from
/// [`INTERP_FLOOR_BELOW_HZ`] up.
const MAX_ALIAS_DB: f64 = -80.0;
/// Below about D2 the selected levels (2 and 3) hold 337-674 partials in a
/// 2048-sample table, and the cubic Hermite read's interpolation images —
/// at `(2048 - h) * f`, folded — set a floor of about -69 dB there
/// (measured -68.6 dB worst at 44.1/48 kHz; clean at 96 kHz, where the
/// images fold onto harmonics). That is the interpolator, not the mip
/// selection: no alias-free level choice changes it short of darkening the
/// bass to a few kHz. It is guarded, not hidden, by [`LOW_NOTE_MAX_DB`].
const INTERP_FLOOR_BELOW_HZ: f32 = 70.0;
const LOW_NOTE_MAX_DB: f64 = -65.0;

/// Basic table: frames sine, triangle, saw, square.
const BASIC: usize = 0;
const SAW_POS: f32 = 2.0 / 3.0;
const SQUARE_POS: f32 = 1.0;

fn render_osc(table: &Wavetable, position: f32, freq: f32, sr: f32) -> Vec<f64> {
    let tap = plan_tap(table, position, freq, sr);
    let inc = phase_inc(freq, sr);
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(N);
    for i in 0..(N + WARMUP) {
        let s = read_tap(table, &tap, phase);
        if i >= WARMUP {
            out.push(s as f64);
        }
        phase += inc;
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
    out
}

/// In-place iterative radix-2 FFT over (re, im).
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let a = start + k;
                let b = a + len / 2;
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Power spectrum (bins 0..=N/2) under a 7-term Blackman-Harris window.
fn power_spectrum(x: &[f64]) -> Vec<f64> {
    const A: [f64; 7] = [
        0.271_051_400_693_42,
        -0.433_297_939_234_48,
        0.218_122_999_543_11,
        -0.065_925_446_388_03,
        0.010_811_742_098_37,
        -0.000_776_584_825_22,
        0.000_013_887_217_35,
    ];
    let n = x.len();
    let mut re: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            let w: f64 = A.iter().enumerate().map(|(k, a)| a * (k as f64 * t).cos()).sum();
            s * w
        })
        .collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    (0..=n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
}

/// Energy off the harmonic series relative to the fundamental, in dB.
fn alias_db(x: &[f64], freq: f32, sr: f32) -> f64 {
    let spec = power_spectrum(x);
    let bin_hz = sr as f64 / N as f64;
    let f0_bin = freq as f64 / bin_hz;
    let mut fundamental = 0.0;
    let mut off_harmonic = 0.0;
    // Skip DC and the lowest bins: the saw's mean and window DC leakage.
    for (k, &p) in spec.iter().enumerate().skip(GUARD_BINS as usize + 1) {
        let h = (k as f64 / f0_bin).round().max(1.0);
        let on_harmonic = (k as f64 - h * f0_bin).abs() <= GUARD_BINS;
        if on_harmonic && h == 1.0 {
            fundamental += p;
        } else if !on_harmonic {
            off_harmonic += p;
        }
    }
    assert!(fundamental > 0.0, "{freq} Hz @ {sr}: no fundamental");
    10.0 * (off_harmonic.max(1e-300) / fundamental).log10()
}

fn midi_to_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// Worst alias figure over a set of frequencies, with the offender.
fn worst(table: &Wavetable, position: f32, freqs: &[f32], sr: f32) -> (f64, f32) {
    let mut worst = (f64::NEG_INFINITY, 0.0);
    for &f in freqs {
        let x = render_osc(table, position, f, sr);
        let peak = x.iter().fold(0.0f64, |m, s| m.max(s.abs()));
        assert!(peak > 0.1, "{f} Hz @ {sr}: oscillator rendered silence");
        let db = alias_db(&x, f, sr);
        if db > worst.0 {
            worst = (db, f);
        }
    }
    worst
}

/// Every key of an 88-key keyboard, plus pitches just above each mip
/// level's own band limit, where the old selection was at its worst.
fn keyboard_freqs() -> Vec<f32> {
    let mut v: Vec<f32> = (21..=108).map(|n| midi_to_hz(n as f32)).collect();
    for k in 2..10 {
        v.push(8.175_799 * (1u32 << k) as f32 * 1.02);
    }
    v
}

fn assert_band_limited(position: f32, shape: &str, sr: f32) {
    let tables = load_bundled();
    let (low, rest): (Vec<f32>, Vec<f32>) =
        keyboard_freqs().into_iter().partition(|&f| f < INTERP_FLOOR_BELOW_HZ);
    for (freqs, limit) in [(rest, MAX_ALIAS_DB), (low, LOW_NOTE_MAX_DB)] {
        let (db, f) = worst(&tables[BASIC], position, &freqs, sr);
        eprintln!("{shape} @ {sr} Hz: worst off-harmonic energy {db:.1} dB at {f:.1} Hz");
        assert!(
            db < limit,
            "{shape} @ {sr} Hz aliases: off-harmonic energy {db:.1} dB re fundamental at \
             {f:.1} Hz (limit {limit} dB)"
        );
    }
}

#[test]
fn saw_is_band_limited_at_44k1() {
    assert_band_limited(SAW_POS, "saw", 44_100.0);
}

#[test]
fn saw_is_band_limited_at_48k() {
    assert_band_limited(SAW_POS, "saw", 48_000.0);
}

#[test]
fn saw_is_band_limited_at_96k() {
    assert_band_limited(SAW_POS, "saw", 96_000.0);
}

#[test]
fn square_is_band_limited_at_48k() {
    assert_band_limited(SQUARE_POS, "square", 48_000.0);
}

/// The review's concrete case: G5 (784 Hz) at 44.1 kHz.
#[test]
fn g5_saw_at_44k1_has_no_birdies() {
    let tables = load_bundled();
    let f = midi_to_hz(79.0);
    let x = render_osc(&tables[BASIC], SAW_POS, f, 44_100.0);
    let db = alias_db(&x, f, 44_100.0);
    assert!(db < MAX_ALIAS_DB, "G5 saw @ 44.1 kHz: off-harmonic energy {db:.1} dB");
}

/// Band-limiting must not throw the top end away: the highest partial kept
/// is never below half of Nyquist (less one harmonic of slack for the top
/// notes, whose levels hold only a couple of partials).
#[test]
fn band_limit_keeps_the_top_end() {
    let tables = load_bundled();
    for sr in [44_100.0f32, 48_000.0] {
        let nyquist = sr as f64 / 2.0;
        for f in keyboard_freqs() {
            let x = render_osc(&tables[BASIC], SAW_POS, f, sr);
            let spec = power_spectrum(&x);
            let bin_hz = sr as f64 / N as f64;
            let fund = spec[(f as f64 / bin_hz).round() as usize];
            // Highest harmonic within 70 dB of the fundamental (a saw's
            // h-th harmonic sits at -20 log10(h) dB, so this is no limit).
            let top = (1..)
                .map(|h| h as f64 * f as f64)
                .take_while(|&hz| hz < nyquist - 10.0 * bin_hz)
                .filter(|&hz| spec[(hz / bin_hz).round() as usize] > fund * 1e-7)
                .last()
                .unwrap_or(0.0);
            assert!(
                top >= 0.45 * nyquist - f as f64,
                "{f:.1} Hz @ {sr}: top partial {top:.0} Hz, over-darkened"
            );
        }
    }
}

/// A higher sample rate never selects a darker level, and between 44.1 and
/// 48 kHz it selects a brighter one somewhere on the keyboard.
#[test]
fn higher_sample_rate_never_selects_darker() {
    let level = |f: f32, sr: f32| {
        let (lo, w) = select_mip(f, sr);
        lo as f32 + w
    };
    let mut brighter_somewhere = false;
    for f in keyboard_freqs() {
        let (l44, l48, l96) = (level(f, 44_100.0), level(f, 48_000.0), level(f, 96_000.0));
        assert!(l48 <= l44 && l96 <= l48, "{f:.1} Hz: levels {l44} / {l48} / {l96}");
        brighter_somewhere |= l48 < l44;
    }
    assert!(brighter_somewhere, "48 kHz never used a brighter level than 44.1 kHz");
}
