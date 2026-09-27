//! Character filter models and audio-rate filter FM.
//!
//! The clean SVF's output is pinned by `render_block_regression.rs` and
//! `null_test.rs`, whose goldens the models must leave untouched. This file
//! covers what is new: every model stays finite and bounded under the worst
//! settings at every common sample rate, the resonant models self-oscillate
//! at their cutoff, and filter FM costs nothing — and changes nothing — at
//! zero.

use resonance_plugin::param::Param;
use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::dsp::filter::FilterType;
use resonance_wavetable::dsp::filter_models::{
    exp2_fast, response_db, tan_fast, CharacterFilter, FilterModel, SAT_HEADROOM,
};
use resonance_wavetable::dsp::modulation::ModDest;
use resonance_wavetable::params::WavetableParams;

const CHARACTER_MODELS: [FilterModel; 4] = [
    FilterModel::Ladder,
    FilterModel::Diode,
    FilterModel::Ms20,
    FilterModel::NonlinearSvf,
];
const TYPES: [FilterType; 4] = [
    FilterType::Lowpass,
    FilterType::Highpass,
    FilterType::Bandpass,
    FilterType::Notch,
];
const RATES: [f32; 4] = [44_100.0, 48_000.0, 96_000.0, 192_000.0];

/// Deterministic xorshift noise in [-1, 1].
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

#[test]
fn new_params_default_to_the_original_filter() {
    let p = WavetableParams::new();
    assert_eq!(p.filter.model.value(), FilterModel::Clean as i32);
    assert_eq!(p.filter.fm.value(), 0.0);
    assert_eq!(p.filter.model.id(), "filter_model");
    assert_eq!(p.filter.fm.id(), "filter_fm");
}

#[test]
fn model_labels_cover_the_parameter_range() {
    let p = WavetableParams::new();
    assert_eq!(p.filter.model.min_plain(), 0.0);
    assert_eq!(
        p.filter.model.max_plain(),
        (FilterModel::LABELS.len() - 1) as f64
    );
    for (i, label) in FilterModel::LABELS.iter().enumerate() {
        let m = FilterModel::from_int(i as i32);
        assert_eq!(m as usize, i);
        assert_eq!(&m.label(), label);
        assert_eq!(&p.filter.model.display(i as f64), label);
    }
}

#[test]
fn filter_fm_is_a_mod_destination() {
    let d = ModDest::from_int(ModDest::FilterFm as i32);
    assert!(d == ModDest::FilterFm);
    assert_eq!(d.label(), "Filter FM");
    assert!(d.is_available());
}

#[test]
fn only_the_diode_ladder_falls_back_to_lowpass() {
    for model in [
        FilterModel::Clean,
        FilterModel::Ladder,
        FilterModel::Ms20,
        FilterModel::NonlinearSvf,
    ] {
        for t in TYPES {
            assert_eq!(model.effective_type(t), t, "{model:?} {t:?}");
        }
    }
    for t in TYPES {
        assert_eq!(FilterModel::Diode.effective_type(t), FilterType::Lowpass);
    }
}

// ---------------------------------------------------------------------------
// Stability
// ---------------------------------------------------------------------------

/// Max resonance, max drive, a hot noisy input, and the cutoff jumping
/// between random extremes — every sample for one pass, every 16 samples
/// (the control rate) for another. Nothing may go non-finite, and the output
/// stays within a small multiple of the saturator headroom.
#[test]
fn every_model_is_bounded_under_abuse_at_every_rate() {
    for sr in RATES {
        for model in CHARACTER_MODELS {
            for t in TYPES {
                for interval in [1usize, 16] {
                    let mut f = CharacterFilter::new();
                    let mut noise = Noise(0x1234_5678);
                    let mut peak = 0.0f32;
                    for n in 0..(sr as usize / 4) {
                        if n % interval == 0 {
                            let cutoff = 20.0 * 1000.0f32.powf((noise.next() + 1.0) * 0.5);
                            f.set_coeffs(model, cutoff, 1.0, sr, 1.0);
                        }
                        let x = 2.0 * noise.next();
                        let y = f.process(x, t);
                        assert!(y.is_finite(), "{model:?} {t:?} @ {sr}: non-finite at {n}");
                        peak = peak.max(y.abs());
                    }
                    assert!(
                        peak < 20.0 * SAT_HEADROOM,
                        "{model:?} {t:?} @ {sr} Hz (interval {interval}): peak {peak}"
                    );
                }
            }
        }
    }
}

/// Audio-rate cutoff modulation through `set_g` — the FM path — with the
/// gain swinging across the whole range every sample.
#[test]
fn every_model_is_bounded_under_audio_rate_set_g() {
    for sr in RATES {
        for model in CHARACTER_MODELS {
            let mut f = CharacterFilter::new();
            f.set_coeffs(model, 1000.0, 1.0, sr, 1.0);
            let w_min = std::f32::consts::PI * 20.0 / sr;
            let w_max = std::f32::consts::PI * 0.49;
            let mut noise = Noise(0x9e37_79b9);
            for n in 0..(sr as usize / 4) {
                let w = w_min + (w_max - w_min) * (noise.next() * 0.5 + 0.5);
                f.set_g(tan_fast(w));
                let y = f.process(2.0 * noise.next(), FilterType::Lowpass);
                assert!(y.is_finite(), "{model:?} @ {sr}: non-finite at {n}");
                assert!(y.abs() < 20.0 * SAT_HEADROOM, "{model:?} @ {sr}: {y}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Self-oscillation
// ---------------------------------------------------------------------------

/// Ring a filter with one impulse at full resonance and measure what is
/// left a second later: RMS and zero-crossing frequency.
fn ring(model: FilterModel, cutoff: f32, sr: f32) -> (f32, f32) {
    let mut f = CharacterFilter::new();
    f.set_coeffs(model, cutoff, 1.0, sr, 0.0);
    let total = sr as usize * 2;
    let tail = sr as usize / 2;
    let mut out = Vec::with_capacity(total);
    for n in 0..total {
        let x = if n == 0 { 1.0 } else { 0.0 };
        out.push(f.process(x, FilterType::Lowpass));
    }
    let tail = &out[total - tail..];
    let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
    let crossings = tail
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    let freq = crossings as f32 / 2.0 / (tail.len() as f32 / sr);
    (rms, freq)
}

#[test]
fn full_resonance_self_oscillates_at_the_cutoff() {
    for sr in [48_000.0, 192_000.0] {
        for model in CHARACTER_MODELS {
            for cutoff in [220.0, 1000.0, 4000.0] {
                let (rms, freq) = ring(model, cutoff, sr);
                assert!(
                    rms > 0.05,
                    "{model:?} @ {cutoff} Hz / {sr}: did not sustain (rms {rms})"
                );
                assert!(
                    rms < SAT_HEADROOM * 4.0,
                    "{model:?} @ {cutoff} Hz / {sr}: runaway (rms {rms})"
                );
                let ratio = freq / cutoff;
                assert!(
                    (0.85..1.15).contains(&ratio),
                    "{model:?} @ {cutoff} Hz / {sr}: oscillates at {freq} Hz"
                );
            }
        }
    }
}

#[test]
fn below_the_threshold_the_ring_dies_away() {
    for model in CHARACTER_MODELS {
        let mut f = CharacterFilter::new();
        f.set_coeffs(model, 1000.0, 0.7, 48_000.0, 0.0);
        let mut last = 0.0f32;
        for n in 0..96_000 {
            let x = if n == 0 { 1.0 } else { 0.0 };
            let y = f.process(x, FilterType::Lowpass);
            if n > 90_000 {
                last = last.max(y.abs());
            }
        }
        assert!(last < 1e-4, "{model:?} still ringing at 70 % resonance: {last}");
    }
}

// ---------------------------------------------------------------------------
// Engine: model selection and filter FM
// ---------------------------------------------------------------------------

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// Render `blocks` blocks of a two-note chord (released halfway) and return
/// the interleaved-by-block stereo output.
fn render(params: &WavetableParams, sr: f32, blocks: usize) -> Vec<f32> {
    let mut engine = SynthEngine::new();
    engine.initialize(sr);
    let mut out = Vec::with_capacity(blocks * BLOCK * 2);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    for block in 0..blocks {
        let mut events = Vec::new();
        if block == 0 {
            for (i, note) in [45u8, 57].into_iter().enumerate() {
                events.push(NoteEvent::NoteOn {
                    note,
                    velocity: 0.9,
                    timing: 3 + i as u32 * 17,
                });
            }
        }
        if block == blocks / 2 {
            for note in [45u8, 57] {
                events.push(NoteEvent::NoteOff { note, timing: 5 });
            }
        }
        let mut iter = EventIterator::new(&events);
        engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter, None);
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
    }
    out
}

/// A patch with osc 2 on (the FM modulator), the filter mid-way and a moving
/// cutoff, so filter FM and the models both have something to act on.
fn fm_patch() -> WavetableParams {
    let p = WavetableParams::new();
    p.osc2.enabled.set_value(true);
    p.osc2.coarse.set_value(7);
    p.osc2.level.set_value(0.4);
    p.unison.voices.set_value(3);
    p.filter.cutoff.set_value(900.0);
    p.filter.resonance.set_value(0.5);
    p.filter.env_depth.set_value(0.4);
    p.filter.drive.set_value(0.3);
    p.lfo1.depth.set_value(0.6);
    p.mod_slots[0].source.set_value(1); // LFO 1
    p.mod_slots[0].destination.set_value(5); // cutoff
    p.mod_slots[0].amount.set_value(0.3);
    p
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|s| s.to_bits()).collect()
}

/// Zero FM through a routing that nets out at or below zero takes exactly
/// the no-FM path: the clamped depth is 0, so the per-sample coefficient
/// update never runs. Checked for the clean SVF and every character model.
#[test]
fn zero_fm_is_bit_identical_to_no_fm() {
    for model in [FilterModel::Clean]
        .into_iter()
        .chain(CHARACTER_MODELS)
    {
        let plain = fm_patch();
        plain.filter.model.set_value(model as i32);
        let reference = render(&plain, SR, 24);

        // A negative-only routing into Filter FM: clamped to zero depth.
        let routed = fm_patch();
        routed.filter.model.set_value(model as i32);
        routed.mod_slots[1].source.set_value(5); // velocity: 0.9 -> +0.8
        routed.mod_slots[1].destination.set_value(ModDest::FilterFm as i32);
        routed.mod_slots[1].amount.set_value(-1.0);
        assert!(
            bits(&render(&routed, SR, 24)) == bits(&reference),
            "{model:?}: a zero-depth FM routing changed the output"
        );
    }
}

#[test]
fn filter_fm_changes_the_sound() {
    for model in [FilterModel::Clean]
        .into_iter()
        .chain(CHARACTER_MODELS)
    {
        let dry = fm_patch();
        dry.filter.model.set_value(model as i32);
        let wet = fm_patch();
        wet.filter.model.set_value(model as i32);
        wet.filter.fm.set_value(0.5);
        let (a, b) = (render(&dry, SR, 16), render(&wet, SR, 16));
        let diff = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        assert!(diff > 1e-3, "{model:?}: filter FM did nothing ({diff})");
    }
}

#[test]
fn the_fm_mod_destination_drives_fm() {
    let base = fm_patch();
    let routed = fm_patch();
    routed.mod_slots[1].source.set_value(5); // velocity: positive at 0.9
    routed.mod_slots[1].destination.set_value(ModDest::FilterFm as i32);
    routed.mod_slots[1].amount.set_value(1.0);
    assert!(bits(&render(&routed, SR, 12)) != bits(&render(&base, SR, 12)));
}

/// The whole voice path — every model, every type, full resonance, full
/// drive, full FM with a mod-matrix sweep of the cutoff — at every rate.
#[test]
fn engine_is_bounded_with_everything_maxed() {
    for sr in RATES {
        for model in CHARACTER_MODELS {
            for t in TYPES {
                let p = fm_patch();
                p.filter.model.set_value(model as i32);
                p.filter.filter_type.set_value(t as i32);
                p.filter.resonance.set_value(1.0);
                p.filter.drive.set_value(1.0);
                p.filter.fm.set_value(1.0);
                p.filter.cutoff.set_value(20000.0);
                p.lfo1.rate.set_value(20.0);
                p.lfo1.depth.set_value(1.0);
                p.mod_slots[0].amount.set_value(-1.0);
                let out = render(&p, sr, 16);
                assert!(
                    out.iter().all(|s| s.is_finite()),
                    "{model:?} {t:?} @ {sr}: non-finite output"
                );
                let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                assert!(peak < 50.0, "{model:?} {t:?} @ {sr}: peak {peak}");
            }
        }
    }
}

/// Changing the model between blocks restarts the filters from rest rather
/// than resuming stale state, and never produces a non-finite sample.
#[test]
fn switching_models_mid_note_is_clean() {
    let p = fm_patch();
    p.filter.resonance.set_value(0.95);
    let mut engine = SynthEngine::new();
    engine.initialize(SR);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    for block in 0..40 {
        let events = if block == 0 {
            vec![NoteEvent::NoteOn {
                note: 48,
                velocity: 1.0,
                timing: 0,
            }]
        } else {
            Vec::new()
        };
        p.filter.model.set_value((block / 4 % 5) as i32);
        let mut iter = EventIterator::new(&events);
        engine.render_block(&mut left, &mut right, BLOCK, &p, &mut iter, None);
        assert!(
            left.iter().chain(&right).all(|s| s.is_finite() && s.abs() < 50.0),
            "block {block}"
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers and the editor's response curve
// ---------------------------------------------------------------------------

#[test]
fn fast_tan_and_exp2_are_accurate() {
    let mut x = 1e-4f32;
    while x < std::f32::consts::PI * 0.49 {
        let rel = (tan_fast(x) - x.tan()).abs() / x.tan();
        // Tight through the audio band, looser only near the 0.49·π clamp.
        let bound = if x < 1.2 { 1e-5 } else { 2e-2 };
        assert!(rel < bound, "tan_fast({x}): rel err {rel}");
        x += 1e-3;
    }
    let mut e = -8.0f32;
    while e <= 8.0 {
        let rel = (exp2_fast(e) - e.exp2()).abs() / e.exp2();
        assert!(rel < 1e-4, "exp2_fast({e}): rel err {rel}");
        e += 0.01;
    }
}

#[test]
fn response_curve_is_finite_for_every_model() {
    for model in CHARACTER_MODELS {
        for t in TYPES {
            for reso in [0.0, 0.5, 0.9, 1.0] {
                for i in 0..128 {
                    let freq = 20.0 * 1000.0f32.powf(i as f32 / 127.0);
                    let db = response_db(model, t, freq, 1000.0, reso);
                    assert!(db.is_finite(), "{model:?} {t:?} r={reso} f={freq}: {db}");
                }
            }
            // At zero resonance a lowpass passes the bass and a highpass
            // passes the top — the curve is the right way up.
            let low = response_db(model, t, 20.0, 1000.0, 0.0);
            let high = response_db(model, t, 19_000.0, 1000.0, 0.0);
            match model.effective_type(t) {
                FilterType::Lowpass => assert!(low > high + 12.0, "{model:?} {t:?}"),
                FilterType::Highpass => assert!(high > low + 12.0, "{model:?} {t:?}"),
                _ => {}
            }
        }
    }
}
