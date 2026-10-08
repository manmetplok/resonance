//! The voicebank's own duration / pitch / variance models (`bank_timing`,
//! `bank_pitch`): what a render unit hands them, and how the unit keys
//! follow them — all without ONNX. Planning never runs a model, so every
//! scenario here plans against fixture banks whose model files are empty.
//!
//! Real-model checks (alignment, pitch on its notes, repeatability) are at
//! the end and no-op unless `RESONANCE_SVS_MODELS_DIR` is set.

use resonance_app::compose::vocal_svs::{
    build_unit_segment, resolve_clip, split_render_units, validate_for_voicebank, BankProfile,
    UnitSegment, VoicebankRegistry, VoicebankRoots,
};
use resonance_app::{write_voicebank_fixture, VoicebankFixture};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::{VocalCurvePoint, VocalParams, VoicebankId};

const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;
const Q: u64 = TICKS_PER_QUARTER_NOTE;
const BPM: f32 = 120.0;
const PAD: f64 = 0.3;
const EPS: f64 = 1e-9;

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

fn note(pitch: u8, start_beat: f64, beats: f64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick: (start_beat * Q as f64) as u64,
        duration_ticks: (beats * Q as f64) as u64,
    }
}

fn params(bank: &str) -> VocalParams {
    VocalParams {
        voicebank: VoicebankId::new(bank),
        ..VocalParams::default()
    }
}

fn unit(notes: &[MidiNote], text: &str, bank: &BankProfile) -> UnitSegment {
    let p = params(bank.id().as_str());
    build_unit_segment(notes, &p, &assign(text, notes.len(), bank), TPQ, BPM, bank)
}

/// `(symbol, group)` per phoneme of the score.
fn phones(u: &UnitSegment) -> Vec<(String, usize)> {
    u.score
        .phones
        .iter()
        .map(|p| (p.symbol.clone(), p.group))
        .collect()
}

fn ph(list: &[(&str, usize)]) -> Vec<(String, usize)> {
    list.iter().map(|(s, g)| (s.to_string(), *g)).collect()
}

/// A syllable's onset consonants belong to the group before its vowel
/// (sung before the beat); its vowel starts a group on the beat. A
/// consonant–glide–vowel onset (`b r ay`) is no exception: the glide is
/// sung before the beat with the consonant, and the group and the pitch
/// model's note start on the vowel. The score's phonemes are the rule
/// track's, its rule durations add up to the unit.
#[test]
fn onsets_sit_before_the_beat_and_a_glide_onset_too() {
    let bank = BankProfile::not_installed("tiger");
    let notes = [note(62, 0.0, 1.0), note(64, 1.0, 1.0)];
    let u = unit(&notes, "bright stars", &bank);
    assert_eq!(
        phones(&u),
        ph(&[
            ("SP", 0),
            ("b", 0),
            ("r", 0),
            ("ay", 1),
            ("t", 1),
            ("s", 1),
            ("t", 1),
            ("aa", 2),
            ("r", 2),
            ("z", 2),
            ("SP", 3),
        ])
    );
    assert_eq!(u.score.phones.iter().map(|p| &p.symbol).collect::<Vec<_>>(), u.segment.ph_seq.iter().collect::<Vec<_>>());
    let starts: Vec<f64> = u.score.groups.iter().map(|g| g.start_sec).collect();
    assert_eq!(starts[0], 0.0);
    // "bright" lands its vowel on beat 1, PAD into the unit: its group
    // starts there, the glide `r` sung before it.
    let rule_start = |k: usize| u.segment.ph_dur[..k].iter().sum::<f64>();
    assert!((rule_start(3) - PAD).abs() < 1e-6, "the rules put ay on the beat");
    assert!(rule_start(2) < PAD - 0.01, "and r before it");
    assert!((starts[1] - PAD).abs() < 1e-6, "{starts:?}");
    // "stars" lands its vowel on beat 2, the trailing pad starts at the
    // last note's end.
    assert!((starts[2] - (PAD + 0.5)).abs() < 1e-6, "{starts:?}");
    assert!((starts[3] - (PAD + 1.0)).abs() < 1e-6, "{starts:?}");
    assert_eq!(u.score.groups.iter().map(|g| g.midi).collect::<Vec<_>>(), vec![62, 62, 64, 64]);
    let total: f64 = u.score.phones.iter().map(|p| p.rule_sec).sum();
    assert!((total - u.score.total_sec).abs() < EPS);
    assert!((u.score.total_sec - u.segment.ph_dur.iter().sum::<f64>()).abs() < EPS);
    // Two voiced notes for the pitch model, legato: the first ends where
    // the second starts.
    let n = &u.score.notes;
    assert_eq!(n.len(), 2);
    assert!(n.iter().all(|n| n.voiced && !n.slur));
    assert!((n[0].start_sec - PAD).abs() < 1e-6, "the note starts on the beat");
    assert!((n[0].end_sec - n[1].start_sec).abs() < EPS);
    assert!((n[1].start_sec - (PAD + 0.5)).abs() < 1e-6);
}

/// A held note is one vowel to the duration model (the rule track's
/// repeated vowel folds into it, the hold's coda closes the group) but
/// still a note of its own to the pitch model, so it slides to it.
#[test]
fn a_hold_is_one_vowel_and_its_own_note() {
    let bank = BankProfile::not_installed("tiger");
    let notes = [note(60, 0.0, 1.0), note(64, 1.0, 1.0), note(62, 2.0, 1.0)];
    let u = unit(&notes, "sing _ on", &bank);
    assert_eq!(
        phones(&u),
        ph(&[
            ("SP", 0),
            ("s", 0),
            ("ih", 1),
            ("ng", 1),
            ("aa", 2),
            ("n", 2),
            ("SP", 3),
        ])
    );
    assert_eq!(u.score.phones.len() + 1, u.segment.ph_seq.len(), "the held ih folded in");
    let total: f64 = u.score.phones.iter().map(|p| p.rule_sec).sum();
    assert!((total - u.score.total_sec).abs() < EPS);
    let n = &u.score.notes;
    assert_eq!(n.len(), 3);
    assert!(n[1].slur && n[1].voiced);
    assert_eq!(n[1].midi, 64.0);
    assert!((n[1].start_sec - (PAD + 0.5)).abs() < 1e-6);
}

/// A written breath and a rest are groups of their own, anchored where the
/// rules put them; the next syllable's onset follows the breath in its
/// group. The note before them ends where the rest begins.
#[test]
fn breaths_and_rests_are_groups_of_their_own() {
    let bank = BankProfile::not_installed("tiger");
    // A whole-beat gap (a rest at the default ½-beat threshold) with a
    // breath before "sing": one unit, rest then breath.
    let notes = [note(60, 0.0, 1.0), note(62, 2.0, 1.0)];
    let u = unit(&notes, "go AP sing", &bank);
    let symbols: Vec<String> = u.score.phones.iter().map(|p| p.symbol.clone()).collect();
    assert_eq!(symbols, vec!["SP", "g", "ow", "SP", "AP", "s", "ih", "ng", "SP"]);
    let group = |sym: &str, nth: usize| {
        u.score
            .phones
            .iter()
            .filter(|p| p.symbol == sym)
            .nth(nth)
            .unwrap()
            .group
    };
    let rest = group("SP", 1);
    let breath = group("AP", 0);
    assert_eq!(group("s", 0), breath, "the onset follows the breath");
    assert_ne!(rest, breath);
    let rest_start = u.score.groups[rest].start_sec;
    assert!((rest_start - (PAD + 0.5)).abs() < 1e-6, "the rest starts at the note's end");
    assert!((u.score.notes[0].end_sec - rest_start).abs() < EPS);
    assert!(u.score.groups[breath].start_sec > rest_start);
}

fn fixture_registry(dir: &std::path::Path, specs: &[(&str, VoicebankFixture)]) -> VoicebankRegistry {
    for (id, spec) in specs {
        write_voicebank_fixture(&dir.join(id), spec);
    }
    VoicebankRegistry::scan(VoicebankRoots {
        banks: Some(dir.to_path_buf()),
        vocoders: None,
        dev_models: Vec::new(),
    })
}

/// The registry finds a bank's own models; a lane uses the duration
/// model with `bank_timing`, the pitch model with `bank_pitch`, and the
/// variance model with pitch when the acoustic model takes tension.
#[test]
fn a_lane_uses_the_bank_models_its_flags_ask_for() {
    let dir = tempfile::tempdir().unwrap();
    let registry = fixture_registry(
        dir.path(),
        &[
            ("tense", VoicebankFixture { bank_models: true, use_tension_embed: true, ..Default::default() }),
            ("plain", VoicebankFixture { bank_models: true, ..Default::default() }),
            ("bare", VoicebankFixture::default()),
        ],
    );
    let tense = registry.profile(&VoicebankId::new("tense"));
    let all = tense.bank_models();
    assert!(all.duration.is_some() && all.pitch.is_some() && all.variance.is_some());
    let on = tense.active_bank_models(true, true);
    assert_eq!(on, all);
    let timing_only = tense.active_bank_models(true, false);
    assert!(timing_only.duration.is_some() && timing_only.pitch.is_none() && timing_only.variance.is_none());
    let pitch_only = tense.active_bank_models(false, true);
    assert!(pitch_only.duration.is_none() && pitch_only.pitch.is_some() && pitch_only.variance.is_some());
    // No tension input: the variance model has nothing to give.
    let plain = registry.profile(&VoicebankId::new("plain"));
    assert!(plain.active_bank_models(true, true).variance.is_none());
    // No models at all: the flags change nothing.
    let bare = registry.profile(&VoicebankId::new("bare"));
    assert_eq!(bare.active_bank_models(true, true), Default::default());
}

fn keys(notes: &[MidiNote], text: &str, p: &VocalParams, bank: &BankProfile) -> Vec<u64> {
    split_render_units(notes, p, &assign(text, notes.len(), bank), TPQ, BPM, bank)
        .iter()
        .map(|u| u.key)
        .collect()
}

/// Unit keys are planned without running a model (the fixture's model
/// files are empty): they follow the flags where the bank ships the
/// models, and do not where it ships none.
#[test]
fn unit_keys_follow_the_flags_only_where_the_bank_has_the_models() {
    let dir = tempfile::tempdir().unwrap();
    let registry = fixture_registry(
        dir.path(),
        &[
            ("models", VoicebankFixture { bank_models: true, ..Default::default() }),
            ("bare", VoicebankFixture::default()),
        ],
    );
    let notes = [note(60, 0.0, 1.0), note(62, 1.0, 1.0), note(64, 2.0, 1.0)];
    let text = "walk a long";
    let with = |id: &str, timing: bool, pitch: bool| VocalParams {
        bank_timing: timing,
        bank_pitch: pitch,
        ..params(id)
    };

    let models = registry.profile(&VoicebankId::new("models"));
    let all: Vec<Vec<u64>> = [(true, true), (true, false), (false, true), (false, false)]
        .iter()
        .map(|&(t, p)| keys(&notes, text, &with("models", t, p), &models))
        .collect();
    for i in 0..all.len() {
        for j in i + 1..all.len() {
            assert_ne!(all[i], all[j], "flag combinations {i} and {j}");
        }
    }
    let units = split_render_units(&notes, &with("models", true, true), &assign(text, 3, &models), TPQ, BPM, &models);
    assert!(units.iter().all(|u| u.bank_models.duration.is_some() && u.bank_models.pitch.is_some()));

    let bare = registry.profile(&VoicebankId::new("bare"));
    assert_eq!(
        keys(&notes, text, &with("bare", true, true), &bare),
        keys(&notes, text, &with("bare", false, false), &bare),
        "a bank without models renders with the rules either way"
    );
    let rule = split_render_units(&notes, &with("bare", true, true), &assign(text, 3, &bare), TPQ, BPM, &bare);
    assert!(rule.iter().all(|u| u.bank_models == Default::default()));
}

/// With the bank's pitch the synthesized line (vibrato, portamento) never
/// reaches the take, so changing it moves no key; pitch bend rides on the
/// bank's pitch and does. Without it, vibrato moves the key as before.
#[test]
fn with_bank_pitch_vibrato_moves_no_key_but_pitch_bend_does() {
    let dir = tempfile::tempdir().unwrap();
    let registry = fixture_registry(
        dir.path(),
        &[("models", VoicebankFixture { bank_models: true, ..Default::default() })],
    );
    let bank = registry.profile(&VoicebankId::new("models"));
    let notes = [note(60, 0.0, 2.0), note(62, 2.0, 2.0)];
    let text = "oh no";
    let base = params("models");
    let vibrato = VocalParams { vibrato: 0.9, portamento_ms: 120.0, ..base.clone() };
    assert_eq!(keys(&notes, text, &base, &bank), keys(&notes, text, &vibrato, &bank));
    let mut bent = base.clone();
    bent.curves.pitch_bend = vec![
        VocalCurvePoint { beat: 0.0, value: 0.5, stepped: false },
        VocalCurvePoint { beat: 3.0, value: 0.9, stepped: false },
    ];
    assert_ne!(keys(&notes, text, &base, &bank), keys(&notes, text, &bent, &bank));
    let units = split_render_units(&notes, &bent, &assign(text, 2, &bank), TPQ, BPM, &bank);
    assert!(units[0].bend_cents.iter().any(|c| *c > 1.0), "the bend is kept for the bank's pitch");

    let rules = |p: &VocalParams| VocalParams { bank_pitch: false, ..p.clone() };
    assert_ne!(
        keys(&notes, text, &rules(&base), &bank),
        keys(&notes, text, &rules(&vibrato), &bank)
    );
}

/// A unit singing a phoneme the bank's pitch model does not know (TIGER's
/// lacks `io`, `iz`, `rs`) is planned without it: it sings the
/// synthesized line, so its key hashes that line (vibrato moves it); the
/// duration model, which knows the phoneme, still times it. A unit
/// without the phoneme keeps the bank's pitch.
#[test]
fn a_unit_with_a_phoneme_the_pitch_model_lacks_sings_the_rules_pitch_and_its_key_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let registry = fixture_registry(
        dir.path(),
        &[(
            "models",
            VoicebankFixture {
                bank_models: true,
                use_tension_embed: true,
                pitch_model_lacks: vec!["ng".into()],
                ..Default::default()
            },
        )],
    );
    let bank = registry.profile(&VoicebankId::new("models"));
    // Two phrases a rest apart: "sing" (with ng) and "oh".
    let notes = [note(60, 0.0, 1.0), note(62, 4.0, 1.0)];
    let text = "sing oh";
    let base = params("models");
    let units = split_render_units(&notes, &base, &assign(text, 2, &bank), TPQ, BPM, &bank);
    assert_eq!(units.len(), 2);
    let m = &units[0].bank_models;
    assert!(m.duration.is_some(), "the duration model knows ng");
    assert!(m.pitch.is_none() && m.variance.is_none(), "the pitch model does not");
    let m = &units[1].bank_models;
    assert!(m.duration.is_some() && m.pitch.is_some() && m.variance.is_some());

    let vibrato = VocalParams { vibrato: 0.9, portamento_ms: 120.0, ..base.clone() };
    let (a, b) = (keys(&notes, text, &base, &bank), keys(&notes, text, &vibrato, &bank));
    assert_ne!(a[0], b[0], "the rule-pitched unit hears its vibrato");
    assert_eq!(a[1], b[1], "the bank-pitched unit does not");
}

/// Under the bank's pitch the tension contour reads the written pitch, so
/// vibrato and portamento (which are not heard) move no key on a bank
/// whose acoustic model takes tension either; on the rule path they do.
#[test]
fn under_bank_pitch_vibrato_moves_no_key_on_a_bank_with_tension() {
    let dir = tempfile::tempdir().unwrap();
    let registry = fixture_registry(
        dir.path(),
        &[(
            "tense",
            VoicebankFixture {
                bank_models: true,
                use_tension_embed: true,
                ..Default::default()
            },
        )],
    );
    let bank = registry.profile(&VoicebankId::new("tense"));
    assert!(bank.active_bank_models(true, true).variance.is_some());
    let notes = [note(60, 0.0, 2.0), note(67, 2.0, 2.0)];
    let text = "oh no";
    let base = VocalParams {
        tension_contour_amount: 1.0,
        tension_velocity_amount: 1.0,
        ..params("tense")
    };
    let vibrato = VocalParams { vibrato: 0.9, portamento_ms: 150.0, ..base.clone() };
    assert_eq!(keys(&notes, text, &base, &bank), keys(&notes, text, &vibrato, &bank));
    // The contour still leans the tension: a higher note moves the key.
    let higher = [note(60, 0.0, 2.0), note(69, 2.0, 2.0)];
    assert_ne!(keys(&notes, text, &base, &bank), keys(&higher, text, &base, &bank));

    let rules = |p: &VocalParams| VocalParams { bank_pitch: false, ..p.clone() };
    assert_ne!(
        keys(&notes, text, &rules(&base), &bank),
        keys(&notes, text, &rules(&vibrato), &bank)
    );
}

/// Replacing a model file (same path, other contents) moves every unit
/// key once the registry rescans: the acoustic model, the vocoder or one
/// of the bank's own models.
#[test]
fn a_replaced_model_file_moves_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let spec = VoicebankFixture { bank_models: true, ..Default::default() };
    let notes = [note(60, 0.0, 1.0), note(62, 1.0, 1.0)];
    let text = "walk on";
    let keys_now = || {
        let registry = VoicebankRegistry::scan(VoicebankRoots {
            banks: Some(dir.path().to_path_buf()),
            vocoders: None,
            dev_models: Vec::new(),
        });
        let bank = registry.profile(&VoicebankId::new("models"));
        keys(&notes, text, &params("models"), &bank)
    };
    write_voicebank_fixture(&dir.path().join("models"), &spec);
    let before = keys_now();
    assert_eq!(before, keys_now(), "nothing changed, nothing moves");
    let mut seen = vec![before];
    for file in ["acoustic.onnx", "dsvocoder/vocoder.onnx", "dspitch/pitch.onnx"] {
        std::fs::write(dir.path().join("models").join(file), format!("new {file}")).unwrap();
        let after = keys_now();
        assert!(!seen.contains(&after), "replacing {file} moves the keys");
        seen.push(after);
    }
}

// ---- Real models (no-ops unless RESONANCE_SVS_MODELS_DIR is set) ----

mod real {
    use super::*;
    use resonance_app::compose::vocal_svs::{render_vocal_clip, SvsRenderCache};
    use resonance_svs::predict::{midi_to_hz, BankModels, PredictOptions};
    use resonance_svs::stages::common::ExecutionProvider;

    fn registry() -> Option<VoicebankRegistry> {
        std::env::var_os("RESONANCE_SVS_MODELS_DIR")?;
        Some(VoicebankRegistry::scan(VoicebankRoots::for_machine()))
    }

    /// "Walk with me a·long the riv·er", the audition's first line.
    fn line() -> (Vec<MidiNote>, &'static str) {
        let n = |p, s, d| note(p, s, d);
        (
            vec![
                n(57, 0.0, 1.0),
                n(55, 1.0, 0.5),
                n(53, 1.5, 1.5),
                n(53, 3.0, 0.5),
                n(57, 3.5, 1.0),
                n(60, 4.5, 0.5),
                n(62, 5.0, 1.0),
                n(60, 6.0, 1.5),
            ],
            "walk with me a·long the riv·er",
        )
    }

    /// On each shipped bank the duration model's timing fills every group
    /// exactly (vowels on their beats, the unit's length unchanged) and
    /// differs from the rules; the pitch model sings each long note within
    /// 50 cents of it, with no octave jumps; both are repeatable.
    #[test]
    fn the_shipped_banks_models_time_and_pitch_a_line() {
        let Some(registry) = registry() else {
            return;
        };
        let (notes, text) = line();
        for id in ["tiger", "lilia", "meiji"] {
            let bank = registry.profile(&VoicebankId::new(id));
            if !bank.is_installed() {
                continue;
            }
            let p = params(id);
            let units = split_render_units(&notes, &p, &assign(text, notes.len(), &bank), TPQ, BPM, &bank);
            assert_eq!(units.len(), 1);
            let u = &units[0];
            assert!(u.bank_models.duration.is_some() && u.bank_models.pitch.is_some(), "{id} ships both");
            let speaker = bank
                .render_target(p.speaker.as_deref(), p.voice, None)
                .unwrap()
                .speaker;
            let opts = PredictOptions::default();
            let mut models = BankModels::load(&u.bank_models, &opts, ExecutionProvider::Cpu);
            let pred = models.predict(&u.score, speaker.as_deref(), u.key, &opts, &|_| 0.0);
            assert!(pred.timed_by_model, "{id}: duration model ran");
            let total: f64 = pred.ph_dur.iter().sum();
            assert!((total - u.score.total_sec).abs() < 1e-6, "{id}: {total}");
            assert!(pred.ph_dur.iter().all(|d| *d > 0.0));
            // Each group starts on its anchor.
            let mut t = 0.0;
            for (i, ph) in u.score.phones.iter().enumerate() {
                let g = &u.score.groups[ph.group];
                if i > 0 && u.score.phones[i - 1].group != ph.group {
                    assert!((t - g.start_sec).abs() < 1e-6, "{id}: group {} at {t}", ph.group);
                }
                t += pred.ph_dur[i];
            }
            assert_ne!(pred.ph_dur, u.score.rule_durations(), "{id}: not the rule timing");

            let pitch = pred.pitch.as_ref().expect("pitch model ran");
            let fs = pred.frame_sec;
            assert!(pitch.windows(2).all(|w| (w[0] - w[1]).abs() < 6.0), "{id}: no octave jumps");
            // Each note is judged over its vowel: the phoneme starting its
            // group (with the next one when that is a short glide), middle
            // 60 %; consonants carry no pitch worth judging.
            let starts: Vec<f64> = pred
                .ph_dur
                .iter()
                .scan(0.0, |t, d| {
                    let s = *t;
                    *t += d;
                    Some(s)
                })
                .collect();
            for n in u.score.notes.iter().filter(|n| !n.slur && n.end_sec - n.start_sec >= 0.45) {
                let Some(k) = (0..starts.len()).find(|&k| (starts[k] - n.start_sec).abs() < 1e-6) else {
                    panic!("{id}: no phoneme starts note {}", n.midi);
                };
                let mut end = starts[k] + pred.ph_dur[k];
                if pred.ph_dur[k] < 0.06 && k + 1 < starts.len() {
                    end += pred.ph_dur[k + 1];
                }
                let (a, b) = (n.start_sec + 0.2 * (end - n.start_sec), end - 0.2 * (end - n.start_sec));
                let mut frames: Vec<f32> = pitch[(a / fs) as usize..=(b / fs) as usize].to_vec();
                frames.sort_by(|x, y| x.partial_cmp(y).unwrap());
                let median = frames[frames.len() / 2];
                let cents = (median - n.midi) * 100.0;
                assert!(cents.abs() < 50.0, "{id}: note {} sung at {median} ({cents:+.0} cents)", n.midi);
                assert!(midi_to_hz(median as f64) > 60.0);
            }
            let again = models.predict(&u.score, speaker.as_deref(), u.key, &opts, &|_| 0.0);
            assert_eq!(again.ph_dur, pred.ph_dur, "{id}: repeatable timing");
            assert_eq!(again.pitch, pred.pitch, "{id}: repeatable pitch");
        }
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    /// A take sung with the bank's models renders the same samples twice
    /// (fresh caches, so every unit really renders), is not silent, and
    /// keeps a rest between its lines silent.
    #[test]
    fn a_bank_model_take_is_repeatable_audible_and_keeps_its_rest() {
        let Some(registry) = registry() else {
            return;
        };
        let bank = registry.profile(&VoicebankId::new("lilia"));
        if !bank.is_installed() {
            return;
        }
        // Two two-note phrases with a 2-beat rest between them: two units.
        let notes = [note(60, 0.0, 1.0), note(62, 1.0, 1.0), note(60, 4.0, 1.0), note(57, 5.0, 1.0)];
        let text = "sing on and on";
        let p = params("lilia");
        let assigned = assign(text, notes.len(), &bank);
        let render = || {
            render_vocal_clip(&notes, &p, &assigned, TPQ, BPM, 44_100, &mut SvsRenderCache::new(), &bank, None)
                .unwrap()
                .unwrap()
        };
        let a = render();
        let b = render();
        assert_eq!(a.samples_stereo, b.samples_stereo, "bit-identical");
        assert_eq!(a.pre_roll_frames, b.pre_roll_frames);
        // The take starts with the leading pad: the first note sits `PAD`
        // into it (interleaved stereo).
        let frames = |t0: f64, t1: f64| {
            let at = |t: f64| (((PAD + t) * a.sample_rate as f64) as usize * 2).min(a.samples_stereo.len());
            a.samples_stereo[at(t0)..at(t1)].to_vec()
        };
        let sung = rms(&frames(0.1, 0.9));
        let rest = rms(&frames(1.2, 1.8));
        assert!(sung > 0.01, "the first phrase is sung: rms {sung}");
        assert!(rest < sung / 30.0, "the rest is silent: {rest} vs {sung}");
    }

    /// Peak resident memory since the last [`reset_peak_rss`], in MiB
    /// (`VmHWM`), and the current one (`VmRSS`).
    fn rss_mib() -> (f64, f64) {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let field = |name: &str| {
            status
                .lines()
                .find(|l| l.starts_with(name))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f64>().ok())
                .map_or(0.0, |kb| kb / 1024.0)
        };
        (field("VmHWM:"), field("VmRSS:"))
    }

    fn reset_peak_rss() {
        let _ = std::fs::write("/proc/self/clear_refs", "5");
    }

    /// Measurement, by hand (`--ignored --nocapture`, release build): the
    /// wall time and peak RSS of a take's first render and of re-rendering
    /// it after an edit that changes one of its two units, per shipped bank.
    #[test]
    #[ignore]
    fn measure_one_unit_rerender() {
        let Some(registry) = registry() else {
            return;
        };
        let text = "sing on and on";
        for id in ["tiger", "lilia", "meiji"] {
            let bank = registry.profile(&VoicebankId::new(id));
            if !bank.is_installed() {
                continue;
            }
            let p = params(id);
            let assigned = assign(text, 4, &bank);
            // Two phrases a rest apart; an edit moves the last note.
            let take = |last: u8| {
                [note(60, 0.0, 1.0), note(62, 1.0, 1.0), note(60, 4.0, 1.0), note(last, 5.0, 1.0)]
            };
            let mut cache = SvsRenderCache::new();
            let mut render = |notes: &[MidiNote]| {
                reset_peak_rss();
                let t0 = std::time::Instant::now();
                render_vocal_clip(notes, &p, &assigned, TPQ, BPM, 44_100, &mut cache, &bank, None)
                    .unwrap()
                    .unwrap();
                let (peak, now) = rss_mib();
                (t0.elapsed().as_secs_f64(), peak, now)
            };
            let first = render(&take(57));
            let mut edits = Vec::new();
            for last in [59, 57, 59, 57] {
                edits.push(render(&take(last)));
            }
            let n = edits.len() as f64;
            let mean = |f: fn(&(f64, f64, f64)) -> f64| edits.iter().map(f).sum::<f64>() / n;
            eprintln!(
                "MEASURE {id}: first {:.3}s peak {:.0} MiB | \
                 one-unit re-render {:.3}s peak {:.0} MiB rss {:.0} MiB",
                first.0,
                first.1,
                mean(|e| e.0),
                mean(|e| e.1),
                mean(|e| e.2),
            );
        }
    }
}
