use std::path::PathBuf;

use resonance_amp::nam::parse::{load_model_from_file, DEFAULT_SAMPLE_RATE};

/// Minimal valid LSTM .nam file body: input 1, hidden 1, one layer.
/// Weights: w_ih(4) + w_hh(4) + b_ih(4) + b_hh(4) + dense(1) + bias(1)
/// + head_scale(1) = 19.
fn minimal_lstm_json(sample_rate_field: Option<&str>) -> String {
    let rate_line = match sample_rate_field {
        Some(v) => format!("\"sample_rate\": {v},"),
        None => String::new(),
    };
    let weights: Vec<String> = (0..19).map(|_| "0.01".to_string()).collect();
    format!(
        r#"{{
            "version": "0.5.2",
            "architecture": "LSTM",
            {rate_line}
            "config": {{"input_size": 1, "hidden_size": 1, "num_layers": 1}},
            "weights": [{}]
        }}"#,
        weights.join(",")
    )
}

/// Write a .nam file to a unique temp path and load it.
fn load_with_rate_field(name: &str, sample_rate_field: Option<&str>) -> f32 {
    let path: PathBuf = std::env::temp_dir().join(format!(
        "resonance_amp_nam_parse_{}_{name}.nam",
        std::process::id()
    ));
    std::fs::write(&path, minimal_lstm_json(sample_rate_field)).unwrap();
    let result = load_model_from_file(path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);
    result.expect("model should load").sample_rate
}

#[test]
fn sample_rate_integer_field_is_parsed() {
    assert_eq!(load_with_rate_field("int", Some("44100")), 44100.0);
}

#[test]
fn sample_rate_float_field_is_parsed() {
    assert_eq!(load_with_rate_field("float", Some("96000.0")), 96000.0);
}

#[test]
fn sample_rate_string_field_is_parsed() {
    assert_eq!(load_with_rate_field("string", Some("\"48000\"")), 48000.0);
}

#[test]
fn missing_sample_rate_defaults_to_48k() {
    assert_eq!(load_with_rate_field("missing", None), DEFAULT_SAMPLE_RATE);
    assert_eq!(DEFAULT_SAMPLE_RATE, 48000.0);
}

#[test]
fn null_sample_rate_defaults_to_48k() {
    assert_eq!(load_with_rate_field("null", Some("null")), DEFAULT_SAMPLE_RATE);
}

#[test]
fn loaded_model_processes_samples() {
    let path: PathBuf = std::env::temp_dir().join(format!(
        "resonance_amp_nam_parse_{}_process.nam",
        std::process::id()
    ));
    std::fs::write(&path, minimal_lstm_json(Some("48000"))).unwrap();
    let loaded = load_model_from_file(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);

    let mut model = loaded.model;
    let out = model.process_sample(0.5);
    assert!(out.is_finite());
}
