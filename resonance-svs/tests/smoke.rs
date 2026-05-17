//! Smoke tests for resonance-svs.
//!
//! Pure-Rust parsing checks always run. The end-to-end render test runs only when the
//! environment variable `SVS_POC_VOICEBANK_DIR` is set to a directory containing
//! `dsconfig.yaml`, the acoustic ONNX, `vocoder.yaml`, and the vocoder ONNX. The `.ds`
//! fixture path is taken from `SVS_POC_DS_FILE`. This guards CI from needing voicebank
//! downloads while letting humans verify end-to-end audio quality.

use std::env;
use std::path::PathBuf;

#[test]
fn note_name_parsing_matches_jobsecond_regex() {
    use resonance_svs::ds::note_name_to_midi;
    assert_eq!(note_name_to_midi("C4"), 60);
    assert_eq!(note_name_to_midi("D#4"), 63);
    assert_eq!(note_name_to_midi("Eb4"), 63);
    assert_eq!(note_name_to_midi("A4"), 69);
    assert_eq!(note_name_to_midi("rest"), 0);
    assert_eq!(note_name_to_midi("Bb3"), 58);
    assert_eq!(note_name_to_midi("  G5  "), 79);
}

#[test]
fn sample_curve_resamples_to_target_length() {
    use resonance_svs::ds::SampleCurve;
    let curve = SampleCurve {
        samples: vec![0.0, 1.0, 2.0, 3.0],
        timestep: 0.01,
    };
    let resampled = curve.resample(0.005, 8);
    assert_eq!(resampled.len(), 8);
    // First sample preserved
    assert!((resampled[0] - 0.0).abs() < 1e-6);
    // Midpoint interpolated halfway between samples[0] and samples[1] = 0.5
    assert!((resampled[1] - 0.5).abs() < 1e-6);
}

#[test]
fn parses_minimal_openvpi_style_ds_file() {
    use resonance_svs::ds::load_ds_file;
    let tmp = std::env::temp_dir().join("resonance_svs_sample.ds");
    let body = r#"[
        {
            "offset": 0.0,
            "text": "AP la AP",
            "ph_seq": "AP l a AP",
            "ph_dur": "0.5 0.1 0.4 0.5",
            "ph_num": "1 2 1",
            "note_seq": "rest A4 rest",
            "note_dur": "0.5 0.5 0.5",
            "note_slur": "0 0 0",
            "f0_seq": "440.0 440.0 440.0 440.0",
            "f0_timestep": 0.005
        }
    ]"#;
    std::fs::write(&tmp, body).expect("write fixture");
    let segs = load_ds_file(&tmp).expect("parse");
    assert_eq!(segs.len(), 1);
    let s = &segs[0];
    assert_eq!(s.ph_seq, vec!["AP", "l", "a", "AP"]);
    assert_eq!(s.ph_dur.len(), 4);
    assert_eq!(s.note_seq_midi, vec![0, 69, 0]); // rest=0, A4=69
    assert_eq!(s.f0.samples.len(), 4);
    assert!((s.f0.timestep - 0.005).abs() < 1e-9);
}

#[test]
fn end_to_end_render() {
    let Some(voicebank) = env::var_os("SVS_POC_VOICEBANK_DIR").map(PathBuf::from) else {
        eprintln!("SVS_POC_VOICEBANK_DIR unset; skipping end-to-end render test");
        return;
    };
    let Some(ds_file) = env::var_os("SVS_POC_DS_FILE").map(PathBuf::from) else {
        eprintln!("SVS_POC_DS_FILE unset; skipping end-to-end render test");
        return;
    };
    let acoustic_config = voicebank.join("dsconfig.yaml");
    let vocoder_config = voicebank.join("vocoder.yaml");
    let out = std::env::temp_dir().join("resonance_svs_smoke.wav");

    let args = resonance_svs::pipeline::PipelineArgs {
        ds_file,
        acoustic_config,
        vocoder_config,
        out: out.clone(),
        execution_provider: resonance_svs::stages::common::ExecutionProvider::Cpu,
        device_index: 0,
        speaker: env::var("SVS_POC_SPEAKER").ok(),
        speedup: env::var("SVS_POC_SPEEDUP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20),
        depth: 1000,
    };
    let summary = resonance_svs::pipeline::run(&args).expect("pipeline run failed");
    assert!(summary.total_samples > summary.sample_rate as usize / 2);
    assert!(out.exists(), "WAV should have been written");
    let meta = std::fs::metadata(&out).expect("WAV metadata");
    // RIFF header is 44 bytes; require something audibly non-trivial.
    assert!(meta.len() > 1024, "WAV too small ({} bytes)", meta.len());
}
