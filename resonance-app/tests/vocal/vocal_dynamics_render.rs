//! Rests, breaths and velocity dynamics through the real models
//! (vocal-mcp-control-and-quality.md §8, §9, §13 render tests).
//!
//! Opt-in: every test returns early unless `RESONANCE_SVS_MODELS_DIR`
//! points at the shipped models (`resonance-svs/models`). Each scenario
//! pins its notes, lyrics, tempo and params and checks on its own that
//! what should sound does — a silent render must never pass.

use resonance_app::compose::vocal_svs::{
    build_segment, render_vocal_clip, resolve_clip, validate_for_voicebank, BankProfile,
    SvsRenderCache, VoicebankRegistry, VoicebankRoots,
};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::{VocalParams, VoiceType, VoicebankId};
use resonance_svs::pipeline::{render_segments, PipelineArgs};
use resonance_svs::stages::common::ExecutionProvider;

const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;
const Q: u64 = TICKS_PER_QUARTER_NOTE;
const BPM: f32 = 120.0;
const SR: u32 = 44_100;

fn real_bank(id: &str) -> Option<BankProfile> {
    let dir = std::env::var_os("RESONANCE_SVS_MODELS_DIR").filter(|v| !v.is_empty())?;
    let registry = VoicebankRegistry::scan(VoicebankRoots {
        dev_models: vec![dir.into()],
        ..VoicebankRoots::default()
    });
    let bank = registry.profile(&VoicebankId::new(id));
    bank.is_installed().then_some(bank)
}

fn params(bank: &BankProfile, id: &str, velocity_dynamics: f32) -> VocalParams {
    VocalParams {
        voicebank: VoicebankId::new(id),
        speaker: bank.default_speaker(VoiceType::Alto),
        velocity_dynamics,
        ..VocalParams::default()
    }
}

fn assign(text: &str, notes: usize, bank: &BankProfile) -> Vec<AssignedSyllable> {
    let draft = vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: notes as u8,
        text: text.to_owned(),
        locked: false,
    }];
    let resolved = resolve_clip(&draft, &[], notes, &Default::default(), &[], &[]);
    validate_for_voicebank(&resolved, bank).expect("plain English resolves")
}

fn note(start_tick: u64, duration_ticks: u64, velocity: f32) -> MidiNote {
    MidiNote {
        note: 64,
        velocity,
        start_tick,
        duration_ticks,
    }
}

fn rms_db(x: &[f32]) -> f64 {
    let ms = x.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    10.0 * ms.max(1e-24).log10()
}

/// Left channel of the stereo render between `a` and `b` seconds.
fn window(stereo: &[f32], sr: u32, a: f64, b: f64) -> Vec<f32> {
    let frames = stereo.len() / 2;
    let (a, b) = (
        ((a * sr as f64) as usize).min(frames),
        ((b * sr as f64) as usize).min(frames),
    );
    (a..b).map(|i| stereo[2 * i]).collect()
}

fn render(
    bank: &BankProfile,
    p: &VocalParams,
    notes: &[MidiNote],
    text: &str,
) -> resonance_app::compose::vocal_svs::RenderedVocal {
    render_vocal_clip(
        notes,
        p,
        &assign(text, notes.len(), bank),
        TPQ,
        BPM,
        SR,
        &mut SvsRenderCache::new(),
        bank,
        None,
    )
    .expect("renders")
    .expect("has notes")
}

/// The AP/SP check (spec §8): a two-second rest sung as `AP` comes out of
/// the model as breath noise, as `SP` as near-silence — measured before
/// the rest gate. Lilia 2026-10-07: AP ≈ −51 dBFS, SP ≈ −85 dBFS; TIGER:
/// AP ≈ −64, SP ≈ −78. This is why plain rests and pads are `SP`.
#[test]
fn an_ap_rest_breathes_and_an_sp_rest_does_not() {
    for id in ["lilia", "tiger"] {
        let Some(bank) = real_bank(id) else {
            eprintln!("RESONANCE_SVS_MODELS_DIR unset or {id} missing; skipped");
            return;
        };
        let p = params(&bank, id, 0.5);
        let notes = [note(0, 2 * Q, 0.8), note(6 * Q, 2 * Q, 0.8)];
        let seg = build_segment(
            &notes,
            &p,
            &assign("la la", 2, &bank),
            TPQ,
            BPM,
            &bank,
        );
        let target = bank.render_target(p.speaker.as_deref(), p.voice, None).unwrap();
        let args = PipelineArgs {
            ds_file: Default::default(),
            acoustic_config: target.acoustic_config,
            vocoder_config: target.vocoder_config,
            out: Default::default(),
            execution_provider: ExecutionProvider::Cpu,
            device_index: 0,
            speaker: target.speaker,
            speedup: 20,
            depth: 1000,
            seed: 0,
        };
        let rest_db = |kind: &str| {
            let mut s = seg.clone();
            for ph in s.ph_seq.iter_mut().filter(|ph| *ph == "SP") {
                *ph = kind.to_owned();
            }
            let out = render_segments(std::slice::from_ref(&s), &args).unwrap();
            let sr = out.sample_rate as f64;
            // Pad 0.3 + note 1.0 → the rest runs 1.3..3.3 s.
            let (a, b) = ((1.5 * sr) as usize, (3.1 * sr) as usize);
            let note_db = rms_db(&out.samples[(0.5 * sr) as usize..(1.2 * sr) as usize]);
            assert!(note_db > -40.0, "{id}: the note sings: {note_db:.1} dBFS");
            rms_db(&out.samples[a..b])
        };
        let (ap, sp) = (rest_db("AP"), rest_db("SP"));
        eprintln!("{id}: AP rest {ap:.1} dBFS, SP rest {sp:.1} dBFS");
        assert!(ap > sp + 10.0, "{id}: AP breathes ({ap:.1}) where SP is quiet ({sp:.1})");
    }
}

/// An eighth rest at the default threshold is gated to digital silence;
/// a written breath before the next note is not.
#[test]
fn a_rest_is_silent_and_a_written_breath_is_heard() {
    let Some(bank) = real_bank("lilia") else {
        eprintln!("RESONANCE_SVS_MODELS_DIR unset; skipped");
        return;
    };
    let p = params(&bank, "lilia", 0.5);
    let pad = 0.3;

    // Quarter, eighth rest, quarter: the rest is 0.5..0.75 s after the
    // first onset.
    let out = render(&bank, &p, &[note(0, Q, 0.8), note(Q + Q / 2, Q, 0.8)], "la la");
    let sung = rms_db(&window(&out.samples_stereo, out.sample_rate, pad + 0.1, pad + 0.4));
    assert!(sung > -40.0, "the first note sings: {sung:.1} dBFS");
    // The second `la`'s `l` is sung before its beat (out of its unit's
    // leading pad, ~90 ms), so the silence is the rest's first 0.12 s.
    let rest = window(&out.samples_stereo, out.sample_rate, pad + 0.51, pad + 0.63);
    assert!(rest.iter().all(|&s| s == 0.0), "the eighth rest is silent");

    // A one-beat rest with a breath: 0.15 s of silence, 0.35 s of breath.
    let out = render(&bank, &p, &[note(0, Q, 0.8), note(2 * Q, Q, 0.8)], "la AP la");
    let sung = rms_db(&window(&out.samples_stereo, out.sample_rate, pad + 1.1, pad + 1.4));
    assert!(sung > -40.0, "the second note sings: {sung:.1} dBFS");
    let silence = window(&out.samples_stereo, out.sample_rate, pad + 0.51, pad + 0.64);
    assert!(silence.iter().all(|&s| s == 0.0), "the rest before the breath is silent");
    let breath = rms_db(&window(&out.samples_stereo, out.sample_rate, pad + 0.7, pad + 0.95));
    eprintln!("lilia breath {breath:.1} dBFS, note {sung:.1} dBFS");
    assert!(breath > -70.0, "the breath is heard: {breath:.1} dBFS");
    assert!(breath < sung - 6.0, "and is quieter than singing: {breath:.1} vs {sung:.1}");
}

/// TIGER has neither tension nor energy: the post-vocoder gain alone makes
/// a velocity sweep audible, about ±6 dB per note at full amount.
#[test]
fn on_tiger_a_velocity_sweep_changes_rms_per_note() {
    let Some(bank) = real_bank("tiger") else {
        eprintln!("RESONANCE_SVS_MODELS_DIR unset; skipped");
        return;
    };
    let notes = [note(0, 2 * Q, 0.56), note(2 * Q, 2 * Q, 0.78), note(4 * Q, 2 * Q, 1.0)];
    let per_note = |amount: f32| -> Vec<f64> {
        let out = render(&bank, &params(&bank, "tiger", amount), &notes, "la la la");
        (0..3)
            .map(|i| {
                let a = 0.3 + i as f64 + 0.2;
                rms_db(&window(&out.samples_stereo, out.sample_rate, a, a + 0.6))
            })
            .collect()
    };
    let flat = per_note(0.0);
    let swept = per_note(1.0);
    eprintln!("tiger per-note dBFS: flat {flat:.1?}, swept {swept:.1?}");
    for (i, db) in flat.iter().chain(&swept).enumerate() {
        assert!(*db > -45.0, "note window {i} sings: {db:.1} dBFS");
    }
    // The safety gain may shift the whole take; the spread is the gain.
    let delta: Vec<f64> = swept.iter().zip(&flat).map(|(s, f)| s - f).collect();
    assert!((delta[1] - delta[0] - 6.0).abs() < 1.0, "soft note −6 dB: {delta:.2?}");
    assert!((delta[2] - delta[1] - 6.0).abs() < 1.0, "loud note +6 dB: {delta:.2?}");
}

/// The Dynamics expression curve on TIGER (no energy input) reaches the
/// take as post-render gain (§10): ±12 dB at the curve's ends, `0.5`
/// unity. Velocity dynamics is off so the curve is the only gain; a
/// stepped curve gives the first note 0.25 (−6 dB) and the second 0.75
/// (+6 dB).
#[test]
fn on_tiger_a_dynamics_curve_changes_the_take_through_gain() {
    let Some(bank) = real_bank("tiger") else {
        eprintln!("RESONANCE_SVS_MODELS_DIR unset; skipped");
        return;
    };
    let notes = [note(0, 2 * Q, 0.8), note(2 * Q, 2 * Q, 0.8)];
    let flat = params(&bank, "tiger", 0.0);
    let mut shaped = flat.clone();
    shaped.curves.dynamics = vec![
        resonance_music_theory::VocalCurvePoint {
            beat: 0.0,
            value: 0.25,
            stepped: true,
        },
        resonance_music_theory::VocalCurvePoint {
            beat: 2.0,
            value: 0.75,
            stepped: false,
        },
    ];
    let per_note = |p: &VocalParams| -> Vec<f64> {
        let out = render(&bank, p, &notes, "la la");
        (0..2)
            .map(|i| {
                let a = 0.3 + i as f64 + 0.2;
                rms_db(&window(&out.samples_stereo, out.sample_rate, a, a + 0.6))
            })
            .collect()
    };
    let (plain, curved) = (per_note(&flat), per_note(&shaped));
    eprintln!("tiger per-note dBFS: plain {plain:.1?}, curve {curved:.1?}");
    for (i, db) in plain.iter().chain(&curved).enumerate() {
        assert!(*db > -45.0, "note window {i} sings: {db:.1} dBFS");
    }
    // The safety gain may move the whole take; the spread is the curve.
    let delta: Vec<f64> = curved.iter().zip(&plain).map(|(c, p)| c - p).collect();
    assert!((delta[1] - delta[0] - 12.0).abs() < 1.0, "−6 dB then +6 dB: {delta:.2?}");
}
