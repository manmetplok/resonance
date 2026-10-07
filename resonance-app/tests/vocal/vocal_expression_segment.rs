//! Feeding the lane's expression curves (`VocalParams::curves`,
//! vocal-mcp-control-and-quality.md §10) into `DsSegment` building.
//!
//! Dynamics (→ energy), tension and breathiness reach the model only on
//! voicebanks whose acoustic model accepts them — no shipped bank takes
//! energy or breathiness, Lilia and Meiji take tension; pitch bend shifts
//! the f0 curve on every bank; an empty curve leaves the auto-derived
//! result untouched; and curve points are section-relative beats, so a
//! note samples the curve where it sits in the section.

use resonance_app::compose::vocal_svs::{build_segment, BankProfile, CurveKind};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::g2p::{
    AssignedSyllable, G2pSource, PhonemeProvenance, SyllableStress,
};
use resonance_music_theory::{VocalCurvePoint, VocalParams, VoicebankId};

const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;
const BPM: f32 = 120.0;

fn params(vb: &str) -> VocalParams {
    VocalParams {
        voicebank: VoicebankId::new(vb),
        // A non-zero baseline tension so the auto-derived tension curve is
        // present (non-default) on supporting voicebanks.
        tension: 0.3,
        ..VocalParams::default()
    }
}

fn syl(phonemes: &[&'static str], syllable_index: usize) -> AssignedSyllable {
    AssignedSyllable {
        label: phonemes.concat(),
        phonemes: phonemes.to_vec(),
        is_slur: false,
        is_word_end: true,
        syllable_index,
        stress: SyllableStress::None,
        provenance: PhonemeProvenance::Auto,
        source: G2pSource::Cmu,
        breath_before: false,
        syllable_warning: false,
    }
}

/// One sustained note at section beat `beat` — long enough that the f0
/// grid has plenty of frames.
fn one_note_at(beat: u64) -> (Vec<MidiNote>, Vec<AssignedSyllable>) {
    let notes = vec![MidiNote {
        note: 64,
        velocity: 0.8,
        start_tick: beat * TICKS_PER_QUARTER_NOTE,
        duration_ticks: TICKS_PER_QUARTER_NOTE * 2,
    }];
    let assigned = vec![syl(&["s", "ow"], 0)];
    (notes, assigned)
}

fn point(beat: f64, value: f32) -> VocalCurvePoint {
    VocalCurvePoint {
        beat,
        value,
        stepped: false,
    }
}

/// Params whose `kind` curve is one point at normalized `value` (held
/// across the whole section).
fn flat(vb: &str, kind: CurveKind, value: f32) -> VocalParams {
    let mut p = params(vb);
    *p.curves.get_mut(kind.index()).unwrap() = vec![point(0.0, value)];
    p
}

fn build_at(p: &VocalParams, beat: u64) -> resonance_svs::ds::DsSegment {
    let (notes, assigned) = one_note_at(beat);
    build_segment(&notes, p, &assigned, TPQ, BPM, &BankProfile::not_installed(p.voicebank.as_str()))
}

fn build(p: &VocalParams) -> resonance_svs::ds::DsSegment {
    build_at(p, 0)
}

// ---------------------------------------------------------------------------
// Empty curves leave the auto-derived result untouched
// ---------------------------------------------------------------------------

#[test]
fn empty_curves_leave_energy_and_breathiness_default() {
    let seg = build(&params("lilia"));
    assert!(seg.energy.is_empty(), "energy should be default with no curve");
    assert!(seg.breathiness.is_empty(), "breathiness should be default with no curve");
}

#[test]
fn a_dynamics_curve_does_not_touch_tension_or_f0() {
    let seg_default = build(&params("lilia"));
    assert!(
        !seg_default.tension.is_empty(),
        "auto tension should be present on a supporting voicebank"
    );
    let dyn_edited = build(&flat("lilia", CurveKind::Dynamics, 0.9));
    assert_eq!(seg_default.tension.samples, dyn_edited.tension.samples);
    assert_eq!(seg_default.f0.samples, dyn_edited.f0.samples);
}

// ---------------------------------------------------------------------------
// Dynamics / Breathiness: no shipped acoustic model takes them
// ---------------------------------------------------------------------------

#[test]
fn dynamics_curve_sends_no_energy_on_any_shipped_voicebank() {
    // No shipped bank sets `use_energy_embed`: the curve becomes a
    // post-render gain instead (see `vocal_curve_dynamics_render`).
    for vb in ["tiger", "lilia", "meiji"] {
        let seg = build(&flat(vb, CurveKind::Dynamics, 1.0));
        assert!(seg.energy.is_empty(), "{vb:?} must leave energy default");
    }
}

#[test]
fn breathiness_curve_is_a_no_op_on_every_shipped_voicebank() {
    for vb in ["tiger", "lilia", "meiji"] {
        let seg = build(&flat(vb, CurveKind::Breathiness, 1.0));
        assert!(seg.breathiness.is_empty(), "{vb:?} must leave breathiness default");
    }
}

// ---------------------------------------------------------------------------
// Tension honoured only on supporting voicebanks; value mapping
// ---------------------------------------------------------------------------

#[test]
fn tension_curve_honoured_on_supporting_voicebank() {
    for vb in ["lilia", "meiji"] {
        let seg = build(&flat(vb, CurveKind::Tension, 1.0));
        assert!(!seg.tension.is_empty(), "{vb:?}");
        assert_eq!(seg.tension.samples.len(), seg.f0.samples.len());
        for v in &seg.tension.samples {
            assert!((*v - 1.0).abs() < 1e-6, "tension frame {v} should map to +1.0");
        }
    }
}

#[test]
fn tension_midpoint_maps_to_model_neutral() {
    let seg = build(&flat("lilia", CurveKind::Tension, 0.5));
    assert!(!seg.tension.is_empty());
    for v in &seg.tension.samples {
        assert!((*v).abs() < 1e-6, "tension frame {v} should map to 0.0");
    }
}

#[test]
fn unsupported_tension_curve_leaves_field_default() {
    let seg = build(&flat("tiger", CurveKind::Tension, 1.0));
    assert!(seg.tension.is_empty(), "TIGER must leave tension default");
}

// ---------------------------------------------------------------------------
// Pitch bend → additive cents offset on f0
// ---------------------------------------------------------------------------

#[test]
fn pitch_bend_curve_shifts_f0_up() {
    let baseline = build(&params("lilia"));
    // Normalized 1.0 = +50 cents: every voiced frame rises by 2^(50/1200).
    let bent = build(&flat("lilia", CurveKind::PitchBend, 1.0));
    assert_eq!(baseline.f0.samples.len(), bent.f0.samples.len());
    let factor = 2.0_f64.powf(50.0 / 1200.0);
    let mut shifted = 0;
    for (b, e) in baseline.f0.samples.iter().zip(&bent.f0.samples) {
        if *b > 0.0 {
            assert!((e / b - factor).abs() < 1e-6, "expected ~{factor}x shift");
            shifted += 1;
        }
    }
    assert!(shifted > 0, "expected some voiced frames to shift");
}

#[test]
fn pitch_bend_curve_shifts_f0_down() {
    let baseline = build(&params("lilia"));
    let bent = build(&flat("lilia", CurveKind::PitchBend, 0.0));
    for (b, e) in baseline.f0.samples.iter().zip(&bent.f0.samples) {
        if *b > 0.0 {
            assert!(*e < *b, "negative bend should lower f0");
        }
    }
}

#[test]
fn an_empty_pitch_bend_leaves_f0_identical() {
    let baseline = build(&params("tiger"));
    let other = build(&flat("tiger", CurveKind::Dynamics, 0.5));
    assert_eq!(baseline.f0.samples, other.f0.samples);
}

// ---------------------------------------------------------------------------
// Section-relative time
// ---------------------------------------------------------------------------

#[test]
fn a_note_samples_the_curve_at_its_section_beat() {
    // Pitch bend: neutral until beat 4, then +50 cents (stepped jump).
    let mut p = params("lilia");
    p.curves.pitch_bend = vec![
        VocalCurvePoint {
            beat: 0.0,
            value: 0.5,
            stepped: true,
        },
        point(4.0, 1.0),
    ];
    let mut plain = params("lilia");
    plain.curves = Default::default();
    let factor = 2.0_f64.powf(50.0 / 1200.0);
    // The 0.3 s lead pad is 0.6 beats at 120 bpm; the note itself (beats
    // 4..6) sits in the +50 cents stretch.
    let (pad_frames, note_frames) = (60, 200);
    let late = build_at(&p, 4);
    let late_plain = build_at(&plain, 4);
    for i in pad_frames + 10..pad_frames + note_frames - 10 {
        let ratio = late.f0.samples[i] / late_plain.f0.samples[i];
        assert!((ratio - factor).abs() < 1e-6, "frame {i}: {ratio}");
    }
    // The same note at beat 0 is before the bend: f0 untouched.
    let early = build_at(&p, 0);
    let early_plain = build_at(&plain, 0);
    for i in pad_frames + 10..pad_frames + note_frames - 10 {
        assert_eq!(early.f0.samples[i], early_plain.f0.samples[i], "frame {i}");
    }
}

// ---------------------------------------------------------------------------
// Dynamics on a bank without energy → post-render gain
// ---------------------------------------------------------------------------

#[test]
fn dynamics_on_a_bank_without_energy_is_gain_the_take_key_sees() {
    use resonance_app::compose::vocal_svs::{split_render_units, DYNAMICS_CURVE_MAX_DB};
    let (notes, assigned) = one_note_at(0);
    let bank = BankProfile::not_installed("tiger");
    let units = |p: &VocalParams| split_render_units(&notes, p, &assigned, TPQ, BPM, &bank);
    let plain = units(&params("tiger"));
    let loud = units(&flat("tiger", CurveKind::Dynamics, 1.0));
    assert!(plain[0].curve_gain.is_empty(), "no curve, no gain");
    assert!(!loud[0].curve_gain.is_empty());
    assert!(loud[0]
        .curve_gain
        .iter()
        .all(|s| (s.gain_db - DYNAMICS_CURVE_MAX_DB).abs() < 1e-4));
    // The gain is applied after the cache: same cache key, new state key.
    assert_eq!(plain[0].key, loud[0].key);
    assert_ne!(plain[0].state_key, loud[0].state_key);
    // Neutral is unity: no gain spans at all would also do, but 0 dB is fine.
    let neutral = units(&flat("tiger", CurveKind::Dynamics, 0.5));
    assert!(neutral[0].curve_gain.iter().all(|s| s.gain_db == 0.0));
}

#[test]
fn gain_layers_add_in_db() {
    use resonance_app::compose::vocal_svs::{dynamics_gain_envelope_layers, GainSpan};
    let a = [GainSpan {
        start_sec: 0.0,
        end_sec: f64::INFINITY,
        gain_db: 6.0,
    }];
    let b = [GainSpan {
        start_sec: 0.0,
        end_sec: f64::INFINITY,
        gain_db: -3.0,
    }];
    let env = dynamics_gain_envelope_layers(&[&a, &b], 100, 1000, 0.0).unwrap();
    let want = 10f32.powf(3.0 / 20.0);
    assert!(env.iter().all(|g| (g - want).abs() < 1e-5));
}
