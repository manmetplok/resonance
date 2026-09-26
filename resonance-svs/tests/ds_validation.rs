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
