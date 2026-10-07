//! Velocity dynamics (vocal-mcp-control-and-quality.md §9, slice 13):
//! velocity drives the bank's energy curve where it has one, else its
//! tension curve, and on every bank a per-note post-vocoder gain of up to
//! ±6 dB (the dynamics gain envelope). Plus the render-unit content key,
//! which follows both new lane params.
//!
//! Hermetic: no model runs. The real-bank render checks are in
//! `vocal_dynamics_render.rs`.

use resonance_app::compose::expression::ExpressionCurves;
use resonance_app::compose::vocal_svs::{
    apply_dynamics_gain, breath_lead_secs, build_segment, dynamics_gain_envelope, resolve_clip,
    split_render_units, validate_for_voicebank, velocity_gain_db, velocity_gain_spans,
    velocity_gain_spans_with_breaths, BankProfile, GainSpan, VoicebankRegistry, VoicebankRoots,
    DYNAMICS_SMOOTH_SEC,
};
use resonance_app::{write_voicebank_fixture, VoicebankFixture};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::{VocalParams, VoicebankId};
use resonance_svs::ds::DsSegment;

const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;
const Q: u64 = TICKS_PER_QUARTER_NOTE;
const BPM: f32 = 120.0;
const SR: u32 = 44_100;

fn params(vb: &str, velocity_dynamics: f32) -> VocalParams {
    VocalParams {
        voicebank: VoicebankId::new(vb),
        velocity_dynamics,
        ..VocalParams::default()
    }
}

fn assign(n: usize, bank: &BankProfile) -> Vec<AssignedSyllable> {
    let draft = vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: n as u8,
        text: vec!["la"; n].join(" "),
        locked: false,
    }];
    let resolved = resolve_clip(&draft, &[], n, &Default::default(), &[], &[]);
    validate_for_voicebank(&resolved, bank).expect("plain English resolves")
}

/// Three legato quarter notes at the given velocities.
fn notes(velocities: [f32; 3]) -> Vec<MidiNote> {
    velocities
        .iter()
        .enumerate()
        .map(|(i, &velocity)| MidiNote {
            note: 64,
            velocity,
            start_tick: i as u64 * Q,
            duration_ticks: Q,
        })
        .collect()
}

fn segment(bank: &BankProfile, p: &VocalParams, velocities: [f32; 3]) -> DsSegment {
    let notes = notes(velocities);
    build_segment(
        &notes,
        p,
        &assign(notes.len(), bank),
        &ExpressionCurves::default(),
        TPQ,
        BPM,
        bank,
    )
}

fn rms_db(x: &[f32]) -> f64 {
    let ms = x.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    10.0 * ms.max(1e-24).log10()
}

#[test]
fn velocity_gain_follows_the_tension_shape() {
    assert_eq!(velocity_gain_db(0.78, 1.0), 0.0, "neutral velocity is unity");
    assert!((velocity_gain_db(1.0, 1.0) - 6.0).abs() < 1e-5);
    assert!((velocity_gain_db(0.56, 1.0) + 6.0).abs() < 1e-5);
    assert!((velocity_gain_db(0.1, 1.0) + 6.0).abs() < 1e-5, "clamped");
    assert!((velocity_gain_db(1.0, 0.5) - 3.0).abs() < 1e-5, "scaled by the amount");
    assert_eq!(velocity_gain_db(1.0, 0.0), 0.0);
}

/// The TIGER case (no tension, no energy): a velocity sweep moves each
/// note's level by its gain, through the post-vocoder envelope alone.
#[test]
fn on_tiger_a_velocity_sweep_changes_rms_per_note() {
    let tiger = BankProfile::not_installed("tiger");
    // No curve reaches TIGER's model: the post gain is the only path.
    let seg = segment(&tiger, &params("tiger", 1.0), [0.56, 0.78, 1.0]);
    assert!(seg.tension.samples.is_empty() && seg.energy.samples.is_empty());

    // A constant-level stand-in for the stitched take: 0.3 s pad, three
    // 0.5 s notes, 0.3 s pad.
    let lead = 0.3;
    let len = ((lead * 2.0 + 1.5) * SR as f64) as usize;
    let mut mono = vec![0.25f32; len];
    let spans = velocity_gain_spans(&notes([0.56, 0.78, 1.0]), TPQ, BPM, 1.0, lead);
    assert!(apply_dynamics_gain(&mut mono, &spans, SR));
    // Guard: every note window holds signal.
    let window = |i: usize| {
        let a = ((lead + i as f64 * 0.5 + 0.05) * SR as f64) as usize;
        let b = ((lead + (i + 1) as f64 * 0.5 - 0.05) * SR as f64) as usize;
        rms_db(&mono[a..b])
    };
    let base = rms_db(&[0.25]);
    let got = [window(0), window(1), window(2)];
    for (g, want) in got.iter().zip([-6.0, 0.0, 6.0]) {
        assert!(*g > -60.0, "not silent: {got:?}");
        assert!((g - base - want).abs() < 0.05, "per-note gain {want} dB: {got:?}");
    }

    // velocity_dynamics 0 leaves the take bit-identical.
    let mut flat = vec![0.25f32; len];
    let spans = velocity_gain_spans(&notes([0.56, 0.78, 1.0]), TPQ, BPM, 0.0, lead);
    assert!(!apply_dynamics_gain(&mut flat, &spans, SR));
    assert!(flat.iter().all(|&s| s == 0.25));
}

#[test]
fn the_gain_envelope_ramps_over_30_ms_at_a_boundary() {
    let spans = [
        GainSpan { start_sec: 0.0, end_sec: 0.5, gain_db: -6.0 },
        GainSpan { start_sec: 0.5, end_sec: f64::INFINITY, gain_db: 6.0 },
    ];
    let env = dynamics_gain_envelope(&spans, SR as usize, SR, DYNAMICS_SMOOTH_SEC).unwrap();
    let db = |t: f64| 20.0 * (env[(t * SR as f64) as usize] as f64).log10();
    assert!((db(0.45) + 6.0).abs() < 1e-3);
    assert!((db(0.55) - 6.0).abs() < 1e-3);
    assert!(db(0.5).abs() < 0.3, "the midpoint of the ramp: {}", db(0.5));
    // Inside the ramp it moves, outside it holds.
    assert!((db(0.49) - db(0.51)).abs() > 1.0);
    assert!((db(0.484) + 6.0).abs() < 1e-3 && (db(0.516) - 6.0).abs() < 1e-3);
    // No step larger than a smooth ramp's per-sample slope.
    let max_step = env.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    assert!(max_step < 0.005, "smoothed, no click (a hard step is 1.5): {max_step}");
}

/// Lilia takes tension, not energy: velocity moves the tension curve
/// (today's path, `tension_velocity_amount`), not energy.
#[test]
fn on_a_bank_with_tension_velocity_changes_the_tension_curve() {
    let lilia = BankProfile::not_installed("lilia");
    let p = params("lilia", 0.5);
    let soft = segment(&lilia, &p, [0.6, 0.6, 0.6]);
    let loud = segment(&lilia, &p, [1.0, 1.0, 1.0]);
    assert!(!soft.tension.samples.is_empty(), "lilia takes tension");
    assert_ne!(soft.tension.samples, loud.tension.samples);
    let mean = |s: &DsSegment| s.tension.samples.iter().sum::<f64>() / s.tension.samples.len() as f64;
    assert!(mean(&loud) > mean(&soft), "louder notes push tension up");
    assert!(soft.energy.samples.is_empty() && loud.energy.samples.is_empty());
}

/// A bank whose acoustic model takes energy gets velocity there, scaled
/// by `velocity_dynamics`.
#[test]
fn on_a_bank_with_energy_velocity_drives_the_energy_curve() {
    let dir = tempfile::tempdir().unwrap();
    write_voicebank_fixture(
        &dir.path().join("loud"),
        &VoicebankFixture {
            use_energy_embed: true,
            ..VoicebankFixture::default()
        },
    );
    let registry = VoicebankRegistry::scan(VoicebankRoots {
        banks: Some(dir.path().to_path_buf()),
        vocoders: None,
        dev_models: Vec::new(),
    });
    let bank = registry.profile(&VoicebankId::new("loud"));
    assert!(bank.is_installed());

    let energy = |amount: f32, vel: [f32; 3]| segment(&bank, &params("loud", amount), vel).energy.samples;
    let soft = energy(1.0, [0.3, 0.3, 0.3]);
    let loud = energy(1.0, [1.0, 1.0, 1.0]);
    assert!(!soft.is_empty());
    assert!(soft.iter().any(|&v| v < -0.9) && soft.iter().all(|&v| v <= 0.0));
    assert!(loud.iter().any(|&v| v > 0.9) && loud.iter().all(|&v| v >= 0.0));
    let half = energy(0.5, [1.0, 1.0, 1.0]);
    let peak = half.iter().cloned().fold(0.0, f64::max);
    assert!((peak - 0.5).abs() < 0.05, "scaled by the amount: {peak}");
    assert!(energy(0.0, [1.0, 1.0, 1.0]).is_empty(), "amount 0 sends no energy");
}

/// Both new lane params are in every unit's content key: changing either
/// re-renders (and moves the lane's plan hash), even where the segment
/// arrays come out the same.
#[test]
fn the_content_key_changes_with_rest_threshold_and_velocity_dynamics() {
    let bank = BankProfile::not_installed("tiger");
    let notes = notes([0.8, 0.8, 0.8]);
    let syl = assign(3, &bank);
    let keys = |p: &VocalParams| -> Vec<u64> {
        split_render_units(&notes, p, &syl, &ExpressionCurves::default(), TPQ, BPM, &bank)
            .iter()
            .map(|u| u.key)
            .collect()
    };
    let base = params("tiger", 0.5);
    let k0 = keys(&base);
    assert_eq!(k0, keys(&base), "stable for the same params");
    let vel = VocalParams { velocity_dynamics: 0.9, ..base.clone() };
    let rest = VocalParams { rest_threshold_beats: 1.0, ..base.clone() };
    // Legato notes: neither change moves the segment on TIGER...
    let seg = |p: &VocalParams| build_segment(&notes, p, &syl, &ExpressionCurves::default(), TPQ, BPM, &bank);
    assert_eq!(seg(&base).ph_dur, seg(&vel).ph_dur);
    assert_eq!(seg(&base).ph_dur, seg(&rest).ph_dur);
    // ...but both move the key.
    assert_ne!(k0, keys(&vel));
    assert_ne!(k0, keys(&rest));
}

/// A written breath is sung at the end of the previous note's slot, but
/// it belongs to the note it leads into: its stretch of the take takes
/// that note's gain, not the previous one's.
#[test]
fn a_breath_takes_the_gain_of_the_note_it_leads_into() {
    let bank = BankProfile::not_installed("tiger");
    let p = params("tiger", 1.0);
    // A one-beat note, a one-beat rest (0.5 s at 120 bpm), then a loud
    // note with a breath: the breath fills the last 0.35 s of the rest.
    let rested = vec![
        MidiNote { note: 64, velocity: 0.56, start_tick: 0, duration_ticks: Q },
        MidiNote { note: 64, velocity: 1.0, start_tick: 2 * Q, duration_ticks: Q },
    ];
    let mut syllables = assign(2, &bank);
    syllables[1].breath_before = true;
    let leads = breath_lead_secs(&rested, &syllables, TPQ, BPM, &p);
    assert_eq!(leads[0], 0.0);
    assert!((leads[1] - 0.35).abs() < 1e-9, "{leads:?}");

    let lead = 0.3;
    let spans = velocity_gain_spans_with_breaths(&rested, &leads, TPQ, BPM, 1.0, lead);
    let onset = lead + 1.0;
    assert!((spans[0].end_sec - (onset - 0.35)).abs() < 1e-9, "{spans:?}");
    assert_eq!(spans[1].start_sec, spans[0].end_sec);
    assert!((spans[1].gain_db - 6.0).abs() < 1e-5, "the breath is loud, like its note");
    // Without the breath the boundary is the onset, as before.
    let plain = velocity_gain_spans(&rested, TPQ, BPM, 1.0, lead);
    assert!((plain[0].end_sec - onset).abs() < 1e-9);

    // Legato: the breath can only take the previous note's tail —
    // 30 % of a 0.5 s quarter.
    let legato = notes([0.56, 1.0, 0.78]);
    let mut syllables = assign(3, &bank);
    syllables[1].breath_before = true;
    let leads = breath_lead_secs(&legato, &syllables, TPQ, BPM, &p);
    assert!((leads[1] - 0.15).abs() < 1e-9, "{leads:?}");
    assert_eq!(leads[2], 0.0);
}
