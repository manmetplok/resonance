//! LIB-09: malformed `.ds` input is an error, never a panic or a silent
//! wrong render. No ONNX / voicebank needed.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_svs::ds::{load_ds_file, SampleCurve};
use resonance_svs::pipeline::phonemes_to_tokens;

fn scratch(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resonance_svs_ds_validation_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
}

fn ds_with_f0_timestep(ts: &str) -> String {
    format!(
        r#"[{{"offset": 0.0, "ph_seq": "SP a SP", "ph_dur": "0.1 0.3 0.1",
             "f0_seq": "440 440 440 440", "f0_timestep": {ts}}}]"#
    )
}

#[test]
fn a_valid_timestep_loads() {
    let path = scratch("ok.ds", &ds_with_f0_timestep("0.005"));
    let segs = load_ds_file(&path).expect("valid .ds loads");
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].f0.samples.len(), 4);
}

#[test]
fn a_non_positive_or_non_finite_f0_timestep_is_an_error() {
    for (i, ts) in ["0", "-0.005", "\"0\"", "\"inf\"", "\"NaN\""]
        .into_iter()
        .enumerate()
    {
        let path = scratch(&format!("bad_f0_{i}.ds"), &ds_with_f0_timestep(ts));
        let err = load_ds_file(&path).expect_err(&format!("f0_timestep {ts} must be rejected"));
        assert!(
            format!("{err:#}").contains("f0_timestep"),
            "error for {ts} should name the field: {err:#}"
        );
    }
}

#[test]
fn a_bad_optional_curve_timestep_is_an_error() {
    let body = r#"[{"offset": 0.0, "ph_seq": "SP a SP", "ph_dur": "0.1 0.3 0.1",
        "f0_seq": "440 440", "f0_timestep": 0.005,
        "energy": "0.5 0.5", "energy_timestep": -1}]"#;
    let path = scratch("bad_energy.ds", body);
    let err = load_ds_file(&path).expect_err("negative energy_timestep must be rejected");
    assert!(format!("{err:#}").contains("energy_timestep"), "{err:#}");
}

#[test]
fn resample_never_panics_on_a_degenerate_timestep() {
    for ts in [0.0, -0.01, f64::NAN, f64::INFINITY] {
        let curve = SampleCurve {
            samples: vec![1.0, 2.0],
            timestep: ts,
        };
        let out = curve.resample(0.01, 10);
        assert!(out.is_empty(), "timestep {ts} must resample to nothing, got {out:?}");
    }
    // A huge but finite timestep is legal: the curve just holds its first
    // value. It must not try to allocate `last_time / target` samples.
    let huge = SampleCurve {
        samples: vec![1.0, 2.0],
        timestep: 1e300,
    };
    assert_eq!(huge.resample(0.01, 10), vec![1.0; 10]);
    // And a degenerate TARGET timestep.
    let curve = SampleCurve {
        samples: vec![1.0, 2.0],
        timestep: 0.01,
    };
    for target in [-0.01, f64::NAN, f64::INFINITY] {
        assert!(curve.resample(target, 10).is_empty(), "target {target}");
    }
}

#[test]
fn unknown_phonemes_are_an_error_naming_them() {
    let map: HashMap<String, i64> = [("SP", 0), ("a", 5), ("k", 7)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let ok = phonemes_to_tokens(&map, &["SP".into(), "k".into(), "a".into()]).unwrap();
    assert_eq!(ok, vec![0, 7, 5]);

    let err = phonemes_to_tokens(&map, &["k".into(), "zh".into(), "a".into(), "q".into(), "zh".into()])
        .expect_err("unknown phonemes must not silently become token 0");
    let msg = err.to_string();
    assert!(msg.contains("zh") && msg.contains("q"), "{msg}");
}

/// FU-M6b: a symbol the dictionary lacks is sung as its nearest
/// substitute the dictionary HAS (Lilia `v` → `f`) before it is an error,
/// so a phoneme stream that skipped the app's substitution still renders.
#[test]
fn a_missing_phoneme_with_a_substitute_in_the_dictionary_is_substituted() {
    let map: HashMap<String, i64> = [("SP", 0), ("f", 3), ("a", 5)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let tokens = phonemes_to_tokens(&map, &["v".into(), "a".into()]).unwrap();
    assert_eq!(tokens, vec![3, 5], "v is sung as f");

    let err = phonemes_to_tokens(&map, &["zh".into()]).expect_err("no substitute present");
    let msg = err.to_string();
    assert!(msg.contains("zh") && msg.contains("substitute"), "{msg}");
}

// ---- What the bank's own models are fed (`predict::score`) ----

mod bank_score {
    use resonance_svs::predict::{
        align_durations, fit_frames, pitch_notes, BankModels, BankScore, PredictOptions,
        ScoreGroup, ScoreNote, ScorePhone,
    };

    const FRAME: f64 = 0.01;

    fn phone(symbol: &str, group: usize, rule_sec: f64) -> ScorePhone {
        ScorePhone {
            symbol: symbol.into(),
            language: 0,
            group,
            rule_sec,
        }
    }

    /// `SP s t | aa r | SP`: a pad with the first onset, one note whose
    /// vowel lands at 0.3 s, a trailing pad from 0.8 s; 1.1 s in all.
    fn score() -> BankScore {
        BankScore {
            phones: vec![
                phone("SP", 0, 0.2),
                phone("s", 0, 0.05),
                phone("t", 0, 0.05),
                phone("aa", 1, 0.4),
                phone("r", 1, 0.1),
                phone("SP", 2, 0.3),
            ],
            groups: vec![
                ScoreGroup { start_sec: 0.0, midi: 62 },
                ScoreGroup { start_sec: 0.3, midi: 62 },
                ScoreGroup { start_sec: 0.8, midi: 62 },
            ],
            notes: vec![ScoreNote {
                start_sec: 0.3,
                end_sec: 0.8,
                midi: 62.0,
                slur: false,
                voiced: true,
            }],
            total_sec: 1.1,
            lead_max_sec: 0.25,
        }
    }

    fn close(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 1e-9, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn word_inputs_count_phonemes_and_frames_per_group() {
        let s = score();
        s.validate().unwrap();
        assert_eq!(s.word_div(), vec![3, 2, 1]);
        assert_eq!(s.word_dur(FRAME), vec![30, 50, 30]);
        assert_eq!(s.ph_midi(), vec![62; 6]);
        close(&s.rule_durations(), &[0.2, 0.05, 0.05, 0.4, 0.1, 0.3]);
    }

    #[test]
    fn alignment_keeps_the_onset_before_the_beat_and_fills_every_group() {
        let s = score();
        // Predicted frames: the onset keeps its own length and ends on the
        // beat; the vowel group is stretched to its span; the pad fills.
        let out = align_durations(&s, &[50.0, 10.0, 8.0, 30.0, 10.0, 5.0], FRAME);
        close(&out, &[0.12, 0.10, 0.08, 0.375, 0.125, 0.3]);
        assert!((out.iter().sum::<f64>() - s.total_sec).abs() < 1e-9);
        // An onset longer than the pad allows is shrunk to the cap,
        // leaving the rest of the pad silent.
        let long = align_durations(&s, &[50.0, 30.0, 30.0, 30.0, 10.0, 5.0], FRAME);
        close(&long[..3], &[0.05, 0.125, 0.125]);
    }

    #[test]
    fn a_malformed_score_is_refused() {
        let mut skipped = score();
        skipped.phones[3].group = 2;
        assert!(skipped.validate().is_err(), "a group with no phonemes");
        let mut late = score();
        late.groups[0].start_sec = 0.1;
        assert!(late.validate().is_err(), "group 0 starts at 0");
    }

    #[test]
    fn pitch_notes_pad_with_rests_that_take_their_neighbours_pitch() {
        let mut s = score();
        s.total_sec = 0.8;
        s.notes = vec![
            ScoreNote { start_sec: 0.1, end_sec: 0.3, midi: 60.0, slur: false, voiced: true },
            ScoreNote { start_sec: 0.5, end_sec: 0.7, midi: 64.0, slur: false, voiced: true },
            ScoreNote { start_sec: 0.7, end_sec: 0.75, midi: 65.0, slur: true, voiced: false },
        ];
        let notes = pitch_notes(&s, FRAME, 80);
        let midi: Vec<f32> = notes.iter().map(|n| n.0).collect();
        let rest: Vec<bool> = notes.iter().map(|n| n.1).collect();
        let frames: Vec<i64> = notes.iter().map(|n| n.2).collect();
        // head rest, A, gap, B, held note (voiced like B), tail rest
        assert_eq!(rest, vec![true, false, true, false, false, true]);
        assert_eq!(midi, vec![60.0, 60.0, 60.0, 64.0, 65.0, 65.0]);
        assert_eq!(frames, vec![10, 20, 20, 20, 5, 5]);
        assert_eq!(frames.iter().sum::<i64>(), 80);
    }

    #[test]
    fn frames_are_fitted_to_the_total_from_the_end() {
        let mut f = vec![3, 3, 3];
        fit_frames(&mut f, 11);
        assert_eq!(f, vec![3, 3, 5]);
        let mut f = vec![3, 3, 3];
        fit_frames(&mut f, 2);
        assert_eq!(f, vec![2, 0, 0]);
    }

    /// With no model loaded (a bank without them, or one that failed to
    /// load) a prediction is the rule layout: nothing to re-time or
    /// re-pitch.
    #[test]
    fn without_models_the_prediction_is_the_rule_layout() {
        let s = score();
        let pred = BankModels::default().predict(&s, None, 1, &PredictOptions::default(), &|_| 0.0);
        assert!(!pred.timed_by_model);
        assert!(pred.pitch.is_none());
        close(&pred.ph_dur, &s.rule_durations());
    }
}
