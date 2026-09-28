//! Oscillator character: interaction modes (FM, ring, hard sync), per-osc
//! phase warp, and the sub and noise sources.
//!
//! Three kinds of guarantee are pinned here:
//!
//! * **Inert by default.** Every new control at its neutral setting renders
//!   the exact bits of a patch that never touched it — including the cases
//!   that do take the interaction/warp kernel (a warp mode at amount zero,
//!   FM or ring at amount zero) and a noise source at level zero that must
//!   not draw from the shared RNG an S&H LFO also draws from.
//! * **Well-behaved when used.** Every mode renders bounded, finite,
//!   non-silent audio, and the new modulation destinations do what moving
//!   their parameter does.
//! * **Band-limited when used.** Warps are read from a mip level chosen for
//!   their worst-case sweep rate, and the discontinuities of sync and the
//!   stepped warps are polyBLEP-corrected. Both are measured at a high note
//!   the way `mip_aliasing.rs` measures the plain oscillator — energy off
//!   the harmonic series — and held against the same signal made naively.

use resonance_plugin::param::Param;
use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::dsp::modulation::ModDest;
use resonance_wavetable::dsp::oscillator::{phase_inc, plan_tap, read_tap};
use resonance_wavetable::dsp::warp::{Warp, WarpMode};
use resonance_wavetable::dsp::wavetable::load_bundled;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;

/// Basic table: frames sine, triangle, saw, square.
const BASIC: i32 = 0;
const SAW_POS: f32 = 2.0 / 3.0;

const MODE_FM: i32 = 1;
const MODE_RING: i32 = 2;
const MODE_SYNC: i32 = 3;
const SRC_VELOCITY: i32 = 5;
const SRC_LFO1: i32 = 1;

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Two oscillators on the basic table (osc1 saw, osc2 triangle), no filter,
/// no FX, a flat sustain — so nothing but the oscillators shapes the output.
fn bare_params() -> WavetableParams {
    let p = WavetableParams::new();
    p.filter.enabled.set_value(false);
    p.chorus.enabled.set_value(false);
    p.delay.enabled.set_value(false);
    p.distortion.enabled.set_value(false);
    p.osc1.wavetable.set_plain(BASIC as f64);
    p.osc1.position.set_value(SAW_POS);
    p.osc2.wavetable.set_plain(BASIC as f64);
    p.osc2.position.set_value(1.0 / 3.0);
    p.osc1.enabled.set_value(true);
    p.osc2.enabled.set_value(true);
    p.osc1.level.set_value(0.8);
    p.osc2.level.set_value(0.6);
    p.amp_env.attack.set_value(0.001);
    p.amp_env.decay.set_value(0.001);
    p.amp_env.sustain.set_value(1.0);
    p.master_volume.set_value(1.0);
    p
}

/// Render `blocks` blocks of one held note; returns (left, right).
fn render(params: &WavetableParams, note: u8, blocks: usize) -> (Vec<f32>, Vec<f32>) {
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let mut out_l = Vec::with_capacity(blocks * BLOCK);
    let mut out_r = Vec::with_capacity(blocks * BLOCK);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    for b in 0..blocks {
        let on = [NoteEvent::NoteOn {
            note,
            velocity: 1.0,
            timing: 0,
        }];
        let events: &[NoteEvent] = if b == 0 { &on } else { &[] };
        let mut iter = EventIterator::new(events);
        engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter, None);
        out_l.extend_from_slice(&left);
        out_r.extend_from_slice(&right);
    }
    (out_l, out_r)
}

/// A short stereo render as one vector, for bit-exact comparisons.
fn render_bits(params: &WavetableParams, note: u8) -> Vec<u32> {
    let (mut l, r) = render(params, note, 8);
    l.extend_from_slice(&r);
    l.iter().map(|s| s.to_bits()).collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

fn route(p: &WavetableParams, slot: usize, source: i32, dest: ModDest, amount: f32) {
    p.mod_slots[slot].source.set_plain(source as f64);
    p.mod_slots[slot].destination.set_plain(dest as i32 as f64);
    p.mod_slots[slot].amount.set_value(amount);
}

// ---------------------------------------------------------------------------
// Spectrum (the measurement of `mip_aliasing.rs`)
// ---------------------------------------------------------------------------

const N: usize = 1 << 15;
/// Past the note-on and the envelope's approach to sustain.
const SETTLE: usize = 4_800;
const GUARD_BINS: f64 = 10.0;

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

/// Energy off the harmonic series of `f0` relative to the energy on it, in
/// dB. Relative to all harmonics rather than the fundamental alone: a warp
/// or a sync sweep can move most of the energy off the fundamental without
/// aliasing at all.
fn alias_db(x: &[f64], f0: f32) -> f64 {
    let spec = power_spectrum(x);
    let bin_hz = SR as f64 / N as f64;
    let f0_bin = f0 as f64 / bin_hz;
    let mut on = 0.0;
    let mut off = 0.0;
    for (k, &p) in spec.iter().enumerate().skip(GUARD_BINS as usize + 1) {
        let h = (k as f64 / f0_bin).round().max(1.0);
        if (k as f64 - h * f0_bin).abs() <= GUARD_BINS {
            on += p;
        } else {
            off += p;
        }
    }
    assert!(on > 0.0, "{f0} Hz: no harmonic energy");
    10.0 * (off.max(1e-300) / on).log10()
}

/// The steady-state analysis window of a rendered channel.
fn window(x: &[f32]) -> Vec<f64> {
    x[SETTLE..SETTLE + N].iter().map(|&s| s as f64).collect()
}

fn blocks_for_analysis() -> usize {
    (SETTLE + N).div_ceil(BLOCK)
}

fn midi_to_hz(note: u8) -> f32 {
    440.0 * ((note as f32 - 69.0) / 12.0).exp2()
}

// ---------------------------------------------------------------------------
// Inert by default
// ---------------------------------------------------------------------------

#[test]
fn every_new_parameter_defaults_to_its_inert_setting() {
    let p = WavetableParams::new();
    assert_eq!(p.osc_mix.mode.value(), 0, "interaction defaults to Sum");
    assert_eq!(p.osc_mix.amount.value(), 0.0);
    for w in [&p.osc1_warp, &p.osc2_warp] {
        assert_eq!(w.mode.value(), WarpMode::Off as i32);
        assert_eq!(w.amount.value(), 0.0);
    }
    assert_eq!(p.sub.level.value(), 0.0);
    assert_eq!(p.noise.level.value(), 0.0);
    assert_eq!(p.noise.color.value(), 0.0);
}

/// A unison stack, both oscillators heard, with a modulated position so the
/// setups are re-planned every control tick.
fn busy_params() -> WavetableParams {
    let p = bare_params();
    p.unison.voices.set_value(3);
    p.unison.detune.set_value(25.0);
    p.osc2.coarse.set_value(7);
    p.lfo1.depth.set_value(0.7);
    p.lfo1.rate.set_value(3.0);
    route(&p, 0, SRC_LFO1, ModDest::Osc1Position, 0.3);
    p
}

#[test]
fn a_warp_mode_at_amount_zero_renders_the_default_bits() {
    let reference = render_bits(&busy_params(), 60);
    for mode in 1..WarpMode::LABELS.len() as i32 {
        let p = busy_params();
        p.osc1_warp.mode.set_value(mode);
        p.osc2_warp.mode.set_value(mode);
        assert_eq!(
            render_bits(&p, 60),
            reference,
            "{} at amount 0 must be bit-identical to no warp",
            WarpMode::from_int(mode).label()
        );
    }
}

#[test]
fn fm_and_ring_at_amount_zero_render_the_sum_bits() {
    let reference = render_bits(&busy_params(), 60);
    for mode in [MODE_FM, MODE_RING] {
        let p = busy_params();
        p.osc_mix.mode.set_value(mode);
        p.osc_mix.amount.set_value(0.0);
        assert_eq!(render_bits(&p, 60), reference, "mode {mode} at amount 0 must equal Sum");
    }
}

#[test]
fn fm_at_amount_zero_with_osc2_muted_is_osc1_alone() {
    // Osc2 still runs as the (idle) modulator, but must not be heard.
    let reference = busy_params();
    reference.osc2.enabled.set_value(false);
    let fm = busy_params();
    fm.osc2.enabled.set_value(false);
    fm.osc_mix.mode.set_value(MODE_FM);
    assert_eq!(render_bits(&fm, 60), render_bits(&reference, 60));
}

#[test]
fn sub_and_noise_at_level_zero_render_the_default_bits() {
    // An S&H LFO shares the engine RNG with the noise source: a noise
    // source that drew from it at level zero would move the S&H values.
    let base = || {
        let p = busy_params();
        p.lfo2.shape.set_value(4); // S&H
        p.lfo2.depth.set_value(1.0);
        p.lfo2.rate.set_value(40.0);
        route(&p, 1, 2, ModDest::Osc2Position, 0.5);
        p
    };
    let reference = render_bits(&base(), 60);

    let p = base();
    p.sub.waveform.set_value(1);
    p.sub.octave.set_value(1);
    p.noise.noise_type.set_value(1);
    p.noise.color.set_value(0.7);
    assert_eq!(render_bits(&p, 60), reference);
}

// ---------------------------------------------------------------------------
// Well-behaved when used
// ---------------------------------------------------------------------------

fn assert_sane(what: &str, x: &[f32]) {
    assert!(x.iter().all(|s| s.is_finite()), "{what}: non-finite sample");
    let p = peak(x);
    assert!(p > 1e-3, "{what}: rendered silence");
    assert!(p < 4.0, "{what}: peak {p} is out of bounds");
}

#[test]
fn every_interaction_mode_renders_bounded_audio() {
    for mode in 0..4 {
        for amount in [0.25f32, 1.0] {
            for note in [36u8, 72, 108] {
                let p = busy_params();
                p.osc_mix.mode.set_value(mode);
                p.osc_mix.amount.set_value(amount);
                let (l, r) = render(&p, note, 12);
                let what = format!("mode {mode} amount {amount} note {note}");
                assert_sane(&what, &l);
                assert_sane(&what, &r);
            }
        }
    }
}

#[test]
fn every_warp_mode_renders_bounded_audio() {
    for mode in 1..WarpMode::LABELS.len() as i32 {
        for amount in [-1.0f32, -0.3, 0.3, 1.0] {
            for note in [36u8, 72, 108] {
                let p = busy_params();
                p.osc1_warp.mode.set_value(mode);
                p.osc1_warp.amount.set_value(amount);
                p.osc2_warp.mode.set_value(mode);
                p.osc2_warp.amount.set_value(-amount);
                let (l, r) = render(&p, note, 12);
                let what = format!("{} {amount} note {note}", WarpMode::from_int(mode).label());
                assert_sane(&what, &l);
                assert_sane(&what, &r);
            }
        }
    }
}

#[test]
fn warp_and_interaction_combine_without_blowing_up() {
    for mode in 1..4 {
        for warp in 1..WarpMode::LABELS.len() as i32 {
            let p = busy_params();
            p.osc_mix.mode.set_value(mode);
            p.osc_mix.amount.set_value(0.7);
            p.osc1_warp.mode.set_value(warp);
            p.osc1_warp.amount.set_value(0.8);
            p.osc2_warp.mode.set_value(warp);
            p.osc2_warp.amount.set_value(0.5);
            let (l, _) = render(&p, 84, 12);
            assert_sane(&format!("mode {mode} + warp {warp}"), &l);
        }
    }
}

#[test]
fn every_mode_changes_the_sound() {
    let reference = render_bits(&busy_params(), 60);
    for mode in 1..4 {
        let p = busy_params();
        p.osc_mix.mode.set_value(mode);
        p.osc_mix.amount.set_value(0.5);
        assert_ne!(render_bits(&p, 60), reference, "mode {mode} changed nothing");
    }
    for warp in 1..WarpMode::LABELS.len() as i32 {
        let p = busy_params();
        p.osc1_warp.mode.set_value(warp);
        p.osc1_warp.amount.set_value(0.5);
        assert_ne!(render_bits(&p, 60), reference, "warp {warp} changed nothing");
    }
}

#[test]
fn ring_at_full_amount_is_the_product() {
    // Osc2 muted and on osc1's own frame and pitch: full ring is then
    // osc1 times itself, which can never go negative. Any dry osc1 or any
    // audible osc2 leaking through would.
    let p = bare_params();
    p.osc2.enabled.set_value(false);
    p.osc2.position.set_value(SAW_POS);
    p.osc_mix.mode.set_value(MODE_RING);
    p.osc_mix.amount.set_value(1.0);
    let (l, _) = render(&p, 60, 8);
    let tail = &l[2_000..];
    assert!(peak(tail) > 1e-3, "ring rendered silence");
    assert!(
        tail.iter().all(|&s| s >= -1e-6),
        "a saw ring-modulated by itself is a square of the saw: never negative"
    );
}

#[test]
fn sub_sine_sits_an_octave_below_osc1() {
    let p = bare_params();
    p.osc1.enabled.set_value(false);
    p.osc2.enabled.set_value(false);
    p.sub.level.set_value(1.0);
    let (l, _) = render(&p, 69, blocks_for_analysis());
    let x = window(&l);
    let spec = power_spectrum(&x);
    let bin_hz = SR as f64 / N as f64;
    let peak_bin = spec
        .iter()
        .enumerate()
        .skip(2)
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap()
        .0;
    let hz = peak_bin as f64 * bin_hz;
    assert!((hz - 220.0).abs() < 2.0, "sub peak at {hz} Hz, expected 220");
    let off_fundamental: f64 = spec
        .iter()
        .enumerate()
        .skip(2)
        .filter(|(k, _)| (*k as f64 - 220.0 / bin_hz).abs() > GUARD_BINS)
        .map(|(_, p)| p)
        .sum();
    let db = 10.0 * (off_fundamental / spec[peak_bin]).log10();
    assert!(db < -80.0, "sub sine is not clean: {db:.1} dB of other energy");
}

#[test]
fn sub_square_two_octaves_down_is_band_limited() {
    let p = bare_params();
    p.osc1.enabled.set_value(false);
    p.osc2.enabled.set_value(false);
    p.sub.level.set_value(1.0);
    p.sub.waveform.set_value(1);
    p.sub.octave.set_value(1);
    let note = 81; // A5 -> sub at A3, 220 Hz: already a high sub
    let (l, _) = render(&p, note, blocks_for_analysis());
    let f = midi_to_hz(note) / 4.0;
    let db = alias_db(&window(&l), f);
    eprintln!("sub square @ {f} Hz: {db:.1} dB off-harmonic");
    assert!(db < -40.0, "polyBLEP sub square aliases: {db:.1} dB");
}

#[test]
fn noise_sources_are_sane_and_pink_is_darker() {
    let spectrum_tilt = |kind: i32, color: f32| {
        let p = bare_params();
        p.osc1.enabled.set_value(false);
        p.osc2.enabled.set_value(false);
        p.noise.level.set_value(0.5);
        p.noise.noise_type.set_value(kind);
        p.noise.color.set_value(color);
        let (l, r) = render(&p, 60, blocks_for_analysis());
        assert_sane(&format!("noise {kind} color {color}"), &l);
        assert_eq!(l, r, "noise is mono and centred");
        let spec = power_spectrum(&window(&l));
        let bin_hz = SR as f64 / N as f64;
        let band = |lo: f64, hi: f64| -> f64 {
            spec[(lo / bin_hz) as usize..(hi / bin_hz) as usize].iter().sum()
        };
        10.0 * (band(8_000.0, 16_000.0) / band(200.0, 400.0)).log10()
    };
    let white = spectrum_tilt(0, 0.0);
    let pink = spectrum_tilt(1, 0.0);
    let dark = spectrum_tilt(0, -1.0);
    let bright = spectrum_tilt(0, 1.0);
    eprintln!("8-16k vs 200-400 Hz: white {white:.1}, pink {pink:.1}, dark {dark:.1}, bright {bright:.1}");
    // Per band, white noise grows with bandwidth: 8k-16k is 40x wider than
    // 200-400 Hz (+16 dB). Pink is flat per octave (~0 dB).
    assert!((white - 16.0).abs() < 3.0, "white tilt {white:.1} dB");
    assert!(pink.abs() < 4.0, "pink tilt {pink:.1} dB");
    assert!(dark < white - 10.0 && bright > white + 3.0);
}

// ---------------------------------------------------------------------------
// Modulation destinations
// ---------------------------------------------------------------------------

#[test]
fn the_new_destinations_are_labelled_and_available() {
    for (dest, label) in [
        (ModDest::OscModAmount, "Osc Mod Amount"),
        (ModDest::Osc1Warp, "Osc1 Warp"),
        (ModDest::Osc2Warp, "Osc2 Warp"),
    ] {
        assert_eq!(ModDest::from_int(dest as i32) as i32, dest as i32);
        assert_eq!(dest.label(), label);
        assert!(dest.is_available());
    }
}

#[test]
fn modulating_the_new_destinations_equals_moving_the_parameter() {
    // Velocity 1.0 is a constant +1 source, so the comparison is exact.
    type Setter = fn(&WavetableParams, f32);
    let cases: [(ModDest, Setter, f32, f32); 3] = [
        (
            ModDest::OscModAmount,
            |p, v| {
                p.osc_mix.mode.set_value(MODE_FM);
                p.osc_mix.amount.set_value(v);
            },
            0.1,
            0.6,
        ),
        (
            ModDest::Osc1Warp,
            |p, v| {
                p.osc1_warp.mode.set_value(WarpMode::Bend as i32);
                p.osc1_warp.amount.set_value(v);
            },
            -0.2,
            0.3,
        ),
        (
            ModDest::Osc2Warp,
            |p, v| {
                p.osc2_warp.mode.set_value(WarpMode::Pwm as i32);
                p.osc2_warp.amount.set_value(v);
            },
            0.25,
            0.5,
        ),
    ];
    for (dest, set, base, modulation) in cases {
        let modulated = bare_params();
        set(&modulated, base);
        route(&modulated, 0, SRC_VELOCITY, dest, modulation);

        let reference = bare_params();
        set(&reference, base + modulation);

        let unmodulated = bare_params();
        set(&unmodulated, base);

        let a = render_bits(&modulated, 60);
        assert_eq!(a, render_bits(&reference, 60), "{}", dest.label());
        assert_ne!(a, render_bits(&unmodulated, 60), "{} did nothing", dest.label());
    }
}

// ---------------------------------------------------------------------------
// Band-limited when used
// ---------------------------------------------------------------------------

/// Osc1 alone, saw, at `note`.
fn solo_params() -> WavetableParams {
    let p = bare_params();
    p.osc2.enabled.set_value(false);
    p
}

/// The same warped saw as the engine renders, but read from the mip level
/// chosen for the unwarped pitch: what the warp would sound like without
/// the bias (and without any polyBLEP).
fn naive_warp(warp: Warp, f: f32) -> Vec<f64> {
    let tables = load_bundled();
    let table = &tables[BASIC as usize];
    let tap = plan_tap(table, SAW_POS, f, SR);
    let inc = phase_inc(f, SR);
    let mut phase = 0.0f64;
    let mut out = Vec::with_capacity(N);
    for i in 0..SETTLE + N {
        let (q, g) = warp.apply(phase);
        if i >= SETTLE {
            out.push((read_tap(table, &tap, q) * g) as f64);
        }
        phase += inc;
        phase -= phase.floor();
    }
    out
}

/// Figures the band-limiting is held to, measured at 48 kHz as energy off
/// the harmonic series relative to the energy on it. For scale: the plain
/// oscillator reads below -100 dB here, and a textbook polyBLEP square sits
/// near -35 dB at 1 kHz under this (unweighted, whole-band) measure — a
/// two-sample polyBLEP attenuates the partials that fold from just above
/// Nyquist the least, and at a high note those are most of what folds.
///
/// C6 (1047 Hz) is the "high note": the top of a lead's range.
const HIGH_NOTE: u8 = 84;
/// Worst case a warp may leave at [`HIGH_NOTE`].
const WARP_MAX_ALIAS_DB: f64 = -30.0;
/// Worst case hard sync may leave at [`HIGH_NOTE`].
const SYNC_MAX_ALIAS_DB: f64 = -30.0;
/// How much cleaner than the naive signal the engine must be.
const MIN_IMPROVEMENT_DB: f64 = 10.0;

const WARP_CASES: [(WarpMode, f32); 9] = [
    (WarpMode::Bend, 1.0),
    (WarpMode::Bend, -1.0),
    (WarpMode::Bend, 0.4),
    (WarpMode::Mirror, 0.6),
    (WarpMode::Mirror, 1.0),
    (WarpMode::Pwm, 0.7),
    (WarpMode::Formant, 0.5),
    (WarpMode::Formant, 1.0),
    (WarpMode::Quantize, 0.6),
];

/// (engine, naive) off-harmonic figures for osc1's saw under `mode` at
/// `amount`, at `note`.
fn warp_alias(mode: WarpMode, amount: f32, note: u8) -> (f64, f64) {
    let f = midi_to_hz(note);
    let p = solo_params();
    p.osc1_warp.mode.set_value(mode as i32);
    p.osc1_warp.amount.set_value(amount);
    let (l, _) = render(&p, note, blocks_for_analysis());
    let db = alias_db(&window(&l), f);
    let naive = alias_db(&naive_warp(Warp::resolve(mode, amount), f), f);
    eprintln!(
        "{} {amount:+.1} @ {f:.0} Hz: {db:.1} dB off-harmonic (naive {naive:.1} dB)",
        mode.label()
    );
    (db, naive)
}

#[test]
fn warps_are_band_limited_at_a_high_note() {
    for (mode, amount) in WARP_CASES {
        let (db, naive) = warp_alias(mode, amount, HIGH_NOTE);
        assert!(
            db < WARP_MAX_ALIAS_DB,
            "{} {amount} aliases at C6: {db:.1} dB",
            mode.label()
        );
        assert!(
            db < naive - MIN_IMPROVEMENT_DB,
            "{} {amount}: the bias/BLEP bought too little ({db:.1} vs naive {naive:.1} dB)",
            mode.label()
        );
    }
}

/// The looser bound an octave above [`HIGH_NOTE`].
const WARP_MAX_ALIAS_DB_C7: f64 = -25.0;

#[test]
fn warps_stay_bounded_an_octave_higher() {
    // At C7 the steepest settings (Bend at full, Formant) fold audibly even
    // from the top mip level, which holds the fundamental alone: the warp
    // itself writes partials above Nyquist into a sine, and only
    // oversampling removes those. The max-slope bias is not always the
    // better trade there either — Bend -1 on the saw reads slightly cleaner
    // unbiased, because the unbiased level already holds few partials and
    // the bias turns the saw into a sine whose bend then folds. So this
    // octave gets a looser bound and no comparison.
    for (mode, amount) in WARP_CASES {
        let (db, _) = warp_alias(mode, amount, HIGH_NOTE + 12);
        assert!(
            db < WARP_MAX_ALIAS_DB_C7,
            "{} {amount} aliases at C7: {db:.1} dB",
            mode.label()
        );
    }
}

/// Naive hard sync: master at `f`, slave `ratio` times faster, restarted at
/// the master's wrap with no correction. Same slave mip level as the engine
/// (the slave's own pitch) so the only difference is the correction.
fn naive_sync(f: f32, ratio: f32, position: f32) -> Vec<f64> {
    let tables = load_bundled();
    let table = &tables[BASIC as usize];
    let fs = f * ratio;
    let tap = plan_tap(table, position, fs, SR);
    let (inc1, inc2) = (phase_inc(f, SR), phase_inc(fs, SR));
    let (mut p1, mut p2) = (0.0f64, 0.0f64);
    let mut out = Vec::with_capacity(N);
    for i in 0..SETTLE + N {
        if i >= SETTLE {
            out.push(read_tap(table, &tap, p2) as f64);
        }
        p1 += inc1;
        if p1 >= 1.0 {
            p1 -= 1.0;
            p2 = p1 / inc1 * inc2;
        } else {
            p2 += inc2;
            p2 -= p2.floor();
        }
    }
    out
}

#[test]
fn hard_sync_is_band_limited_at_a_high_note() {
    let f = midi_to_hz(HIGH_NOTE);
    // Sine, triangle and saw slaves. The saw is the hardest: its own edge
    // sits at phase 0, so every reset lands mid-edge and cuts the table's
    // band-limited transition in half, which neither correction models.
    for (position, shape) in [(0.0f32, "sine"), (1.0 / 3.0, "triangle"), (SAW_POS, "saw")] {
        for amount in [0.2f32, 0.45] {
            // Osc1 is the (muted) master, osc2 the heard slave.
            let p = bare_params();
            p.osc1.enabled.set_value(false);
            p.osc2.position.set_value(position);
            p.osc_mix.mode.set_value(MODE_SYNC);
            p.osc_mix.amount.set_value(amount);
            let (l, _) = render(&p, HIGH_NOTE, blocks_for_analysis());
            let db = alias_db(&window(&l), f);
            let ratio = (amount * 36.0 / 12.0).exp2();
            let naive = alias_db(&naive_sync(f, ratio, position), f);
            eprintln!(
                "sync {shape} x{ratio:.2} @ {f:.0} Hz: {db:.1} dB off-harmonic (naive {naive:.1} dB)"
            );
            assert!(
                db < SYNC_MAX_ALIAS_DB,
                "{shape} sync aliases at C6 x{ratio:.2}: {db:.1} dB"
            );
            assert!(
                db < naive - MIN_IMPROVEMENT_DB,
                "{shape} sync correction bought too little: {db:.1} vs naive {naive:.1} dB"
            );
        }
    }
}

#[test]
fn sync_follows_the_master_pitch() {
    // Whatever the slave's own pitch, the synced output repeats at the
    // master's period: its energy sits on the master's harmonics.
    let note = 57u8; // 220 Hz
    let p = bare_params();
    p.osc1.enabled.set_value(false);
    p.osc2.position.set_value(1.0 / 3.0);
    p.osc2.coarse.set_value(5); // slave a fourth up: not a harmonic
    p.osc_mix.mode.set_value(MODE_SYNC);
    let (l, _) = render(&p, note, blocks_for_analysis());
    let synced = alias_db(&window(&l), midi_to_hz(note));

    // Without sync the same slave is nowhere near the master's series.
    p.osc_mix.mode.set_value(0);
    let (l, _) = render(&p, note, blocks_for_analysis());
    let free = alias_db(&window(&l), midi_to_hz(note));

    eprintln!("slave a fourth up, synced {synced:.1} dB vs free {free:.1} dB off the master");
    assert!(synced < -40.0, "synced slave is not periodic at the master: {synced:.1} dB");
    assert!(free > 0.0, "the free-running slave should sit off the master's series");
}

// ---------------------------------------------------------------------------
// Alongside the rest of the synth: user tables and filter FM
// ---------------------------------------------------------------------------

#[test]
fn interaction_and_warp_work_on_user_tables() {
    use resonance_plugin::{OutputBuffer, ResonancePlugin};
    use resonance_wavetable::dsp::wavetable::{USER_WAVETABLE_INDEX, WAVETABLE_SIZE};
    use resonance_wavetable::ResonanceWavetable;

    // Two frames rich enough that a wrong table or a no-op mode shows.
    let frames: Vec<f32> = (0..2 * WAVETABLE_SIZE)
        .map(|i| {
            let t = std::f32::consts::TAU * i as f32 / WAVETABLE_SIZE as f32;
            0.6 * t.sin() + 0.3 * (5.0 * t).sin() + 0.1 * (11.0 * t).cos()
        })
        .collect();
    let render_plugin = |set: &dyn Fn(&ResonanceWavetable)| -> Vec<f32> {
        let mut plugin = ResonanceWavetable::new();
        for osc in 0..2 {
            plugin
                .user_wavetables()
                .restore_frames(osc, "", "frames", frames.clone())
                .unwrap();
        }
        plugin.initialize(SR, BLOCK as u32);
        let param = |id: &str| {
            (0..plugin.param_count())
                .map(|i| plugin.param(i))
                .find(|p| p.id() == id)
                .unwrap_or_else(|| panic!("no param `{id}`"))
        };
        param("osc1_wavetable").set_plain(USER_WAVETABLE_INDEX as f64);
        param("osc2_wavetable").set_plain(USER_WAVETABLE_INDEX as f64);
        param("osc2_enabled").set_plain(1.0);
        param("osc2_coarse").set_plain(7.0);
        set(&plugin);
        let mut out = Vec::new();
        let (mut left, mut right) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
        for block in 0..8 {
            let on = [NoteEvent::NoteOn {
                note: 60,
                velocity: 1.0,
                timing: 0,
            }];
            let events: &[NoteEvent] = if block == 0 { &on } else { &[] };
            let mut iter = EventIterator::new(events);
            let mut outs = [OutputBuffer {
                left: &mut left,
                right: &mut right,
            }];
            plugin.process(&mut outs, BLOCK, &mut iter, None);
            out.extend_from_slice(&left);
        }
        assert!(plugin.engine().user_table(0).is_some(), "the user table never landed");
        out
    };
    let set_id = |plugin: &ResonanceWavetable, id: &str, v: f64| {
        (0..plugin.param_count())
            .map(|i| plugin.param(i))
            .find(|p| p.id() == id)
            .unwrap()
            .set_plain(v);
    };

    let reference = render_plugin(&|_| {});
    assert_sane("user tables, Sum", &reference);
    for mode in 1..4 {
        let out = render_plugin(&|p| {
            set_id(p, "osc_mix_mode", mode as f64);
            set_id(p, "osc_mod_amount", 0.5);
        });
        assert_sane(&format!("user tables, mode {mode}"), &out);
        assert_ne!(out, reference, "mode {mode} did nothing on a user table");
    }
    for warp in 1..WarpMode::LABELS.len() {
        let out = render_plugin(&|p| {
            set_id(p, "osc1_warp_mode", warp as f64);
            set_id(p, "osc1_warp_amount", 0.6);
        });
        assert_sane(&format!("user tables, warp {warp}"), &out);
        assert_ne!(out, reference, "warp {warp} did nothing on a user table");
    }
}

#[test]
fn a_muted_modulator_still_drives_filter_fm() {
    // Filter FM reads osc2's raw signal. Under an interaction mode osc2
    // runs while muted, so it keeps modulating the filter; in Sum a muted
    // osc2 is silent everywhere, the filter included.
    let base = |mode: i32| {
        let p = bare_params();
        p.osc2.enabled.set_value(false);
        p.filter.enabled.set_value(true);
        p.filter.cutoff.set_value(800.0);
        p.osc_mix.mode.set_value(mode);
        p
    };
    for (mode, drives) in [(0, false), (MODE_FM, true), (MODE_RING, true), (MODE_SYNC, true)] {
        let dry = base(mode);
        let wet = base(mode);
        wet.filter.fm.set_value(0.8);
        let (a, b) = (render_bits(&dry, 48), render_bits(&wet, 48));
        assert_eq!(
            a != b,
            drives,
            "mode {mode}: filter FM from a muted osc2 should be {}",
            if drives { "active" } else { "silent" }
        );
    }
}
