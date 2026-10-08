//! Smoke tests for resonance-svs.
//!
//! Pure-Rust parsing checks always run. The end-to-end render test is `#[ignore]`d: it
//! needs a real voicebank directory (`SVS_POC_VOICEBANK_DIR`, containing `dsconfig.yaml`,
//! the acoustic ONNX, `vocoder.yaml`, and the vocoder ONNX) and a `.ds` fixture
//! (`SVS_POC_DS_FILE`). Run it by hand with `-- --ignored` once those are set; without
//! them it panics rather than silently reporting green. This guards CI from needing
//! voicebank downloads while letting humans verify end-to-end audio quality.

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
fn execution_provider_names_round_trip_and_rocm_means_migraphx() {
    use resonance_svs::ExecutionProvider;
    for ep in [
        ExecutionProvider::Cpu,
        ExecutionProvider::Cuda,
        ExecutionProvider::MiGraphX,
    ] {
        assert_eq!(ExecutionProvider::parse(ep.as_str()), ep);
    }
    // ONNX Runtime 1.23 removed the ROCm provider; the old names select
    // its replacement rather than silently meaning CPU.
    assert_eq!(ExecutionProvider::parse("rocm"), ExecutionProvider::MiGraphX);
    assert_eq!(ExecutionProvider::parse("HIP"), ExecutionProvider::MiGraphX);
    assert_eq!(ExecutionProvider::parse("bogus"), ExecutionProvider::Cpu);
}

/// A GPU fallback is remembered so later renders go straight to CPU — but
/// only when the model itself loads on CPU. A missing model fails on
/// every provider and must not switch the GPU off for the process.
#[test]
fn a_missing_model_does_not_mark_the_gpu_provider_failed() {
    use resonance_svs::stages::common::{build_session_reporting, provider_failed};
    use resonance_svs::ExecutionProvider;
    let missing = std::path::Path::new("/nonexistent/resonance-svs-test/acoustic.onnx");
    assert!(build_session_reporting(missing, ExecutionProvider::Cuda, 7).is_err());
    assert!(!provider_failed(ExecutionProvider::Cuda, 7));
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
#[ignore = "needs a voicebank: set SVS_POC_VOICEBANK_DIR and SVS_POC_DS_FILE, run with -- --ignored"]
fn end_to_end_render() {
    let voicebank = env::var_os("SVS_POC_VOICEBANK_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set SVS_POC_VOICEBANK_DIR to a voicebank directory containing dsconfig.yaml, the acoustic ONNX, vocoder.yaml, and the vocoder ONNX"));
    let ds_file = env::var_os("SVS_POC_DS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set SVS_POC_DS_FILE to a .ds fixture path"));
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
        seed: 0,
    };
    let summary = resonance_svs::pipeline::run(&args).expect("pipeline run failed");
    // The acoustic session's backend is recorded once it exists.
    assert_eq!(
        resonance_svs::active_backend(),
        Some(resonance_svs::ExecutionProvider::Cpu)
    );
    assert!(summary.total_samples > summary.sample_rate as usize / 2);
    assert!(out.exists(), "WAV should have been written");
    let meta = std::fs::metadata(&out).expect("WAV metadata");
    // RIFF header is 44 bytes; require something audibly non-trivial.
    assert!(meta.len() > 1024, "WAV too small ({} bytes)", meta.len());
}

// ---- Seeded noise (`stages::noise`) ----
//
// A tiny ONNX model is written by hand (protobuf wire format) so the
// rewrite and the session it builds are checked without a voicebank.

fn pb_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn pb_len(out: &mut Vec<u8>, num: u64, bytes: &[u8]) {
    pb_varint(out, num << 3 | 2);
    pb_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn pb_int(out: &mut Vec<u8>, num: u64, v: u64) {
    pb_varint(out, num << 3);
    pb_varint(out, v);
}

/// A float tensor `ValueInfoProto` with one symbolic dim per entry of `dims`
/// (a number for a fixed one).
fn pb_tensor_info(name: &str, dims: &[&str]) -> Vec<u8> {
    let mut shape = Vec::new();
    for d in dims {
        let mut dim = Vec::new();
        match d.parse::<u64>() {
            Ok(n) => pb_int(&mut dim, 1, n),
            Err(_) => pb_len(&mut dim, 2, d.as_bytes()),
        }
        pb_len(&mut shape, 1, &dim);
    }
    let mut tensor = Vec::new();
    pb_int(&mut tensor, 1, 1);
    pb_len(&mut tensor, 2, &shape);
    let mut ty = Vec::new();
    pb_len(&mut ty, 1, &tensor);
    let mut out = Vec::new();
    pb_len(&mut out, 1, name.as_bytes());
    pb_len(&mut out, 2, &ty);
    out
}

/// A one-node model: `y = <op>(x)` for a `*Like` op (input `x` of shape
/// `[n]`), or `y = <op>()` with `shape = [2, 3]` for a fixed one.
fn random_model(op: &str) -> Vec<u8> {
    let like = op.ends_with("Like");
    let mut node = Vec::new();
    if like {
        pb_len(&mut node, 1, b"x");
    }
    pb_len(&mut node, 2, b"y");
    pb_len(&mut node, 3, b"rand");
    pb_len(&mut node, 4, op.as_bytes());
    let mut dtype = Vec::new();
    pb_len(&mut dtype, 1, b"dtype");
    pb_int(&mut dtype, 3, 1);
    pb_int(&mut dtype, 20, 2);
    pb_len(&mut node, 5, &dtype);
    if !like {
        let mut shape = Vec::new();
        pb_len(&mut shape, 1, b"shape");
        pb_int(&mut shape, 8, 2);
        pb_int(&mut shape, 8, 3);
        pb_int(&mut shape, 20, 7);
        pb_len(&mut node, 5, &shape);
    }
    let mut graph = Vec::new();
    pb_len(&mut graph, 1, &node);
    pb_len(&mut graph, 2, b"g");
    if like {
        pb_len(&mut graph, 11, &pb_tensor_info("x", &["n"]));
        pb_len(&mut graph, 12, &pb_tensor_info("y", &["n"]));
    } else {
        pb_len(&mut graph, 12, &pb_tensor_info("y", &["2", "3"]));
    }
    let mut opset = Vec::new();
    pb_len(&mut opset, 1, b"");
    pb_int(&mut opset, 2, 15);
    let mut model = Vec::new();
    pb_int(&mut model, 1, 8);
    pb_len(&mut model, 8, &opset);
    pb_len(&mut model, 7, &graph);
    model
}

fn run_seeded(model: &[u8], x_len: Option<usize>, seed: u64) -> Vec<f32> {
    use resonance_svs::stages::common::{build_session_from, ModelSource, SeededSession};
    use resonance_svs::stages::noise::seed_random_ops;
    use resonance_svs::ExecutionProvider;
    let (bytes, specs) = seed_random_ops(model).expect("rewrite");
    let path = std::path::Path::new("in-memory.onnx");
    let (mut session, _) =
        build_session_from(ModelSource::Bytes { path, bytes: &bytes }, ExecutionProvider::Cpu, 0)
            .expect("the rewritten model loads");
    let mut inputs: Vec<(String, ort::value::Value)> = Vec::new();
    if let Some(n) = x_len {
        let x = ndarray::Array1::<f32>::zeros(n);
        inputs.push(("x".into(), ort::value::Value::from_array(x).unwrap().into()));
    }
    inputs.extend(SeededSession::noise_values(&specs, seed, 8, 1).unwrap());
    let outputs = session.run(inputs).expect("runs");
    let (_, y) = outputs.get("y").unwrap().try_extract_tensor::<f32>().unwrap();
    y.to_vec()
}

#[test]
fn seeding_replaces_a_random_like_op_with_a_fed_noise_input() {
    use resonance_svs::stages::noise::{noise, seed_random_ops, NoiseDist, NoiseSpec};
    let model = random_model("RandomNormalLike");
    let (_, specs) = seed_random_ops(&model).unwrap();
    assert_eq!(
        specs,
        vec![NoiseSpec {
            input: "resonance_noise_0".into(),
            dist: NoiseDist::Normal { mean: 0.0, scale: 1.0 },
        }]
    );
    // The op now draws the first `n` values of the seeded stream, the
    // same ones on every run and in every session, other ones under
    // another seed.
    let a = run_seeded(&model, Some(5), 42);
    let b = run_seeded(&model, Some(5), 42);
    let c = run_seeded(&model, Some(5), 43);
    assert_eq!(a.len(), 5);
    assert_eq!(a, b, "same seed, same noise");
    assert_ne!(a, c, "another seed, other noise");
    let stream = noise(42, 0, 5, NoiseDist::Normal { mean: 0.0, scale: 1.0 });
    assert_eq!(a, stream);
}

#[test]
fn seeding_handles_a_fixed_shape_random_op_and_leaves_other_models_alone() {
    use resonance_svs::stages::noise::{seed_random_ops, NoiseDist};
    let model = random_model("RandomUniform");
    let (_, specs) = seed_random_ops(&model).unwrap();
    assert_eq!(specs[0].dist, NoiseDist::Uniform { low: 0.0, high: 1.0 });
    let y = run_seeded(&model, None, 7);
    assert_eq!(y.len(), 6, "shape [2, 3]");
    assert!(y.iter().all(|v| (0.0..1.0).contains(v)));
    assert_eq!(y, run_seeded(&model, None, 7));

    // A model without random ops comes back byte for byte.
    let plain = random_model("Identity");
    let (bytes, specs) = seed_random_ops(&plain).unwrap();
    assert!(specs.is_empty());
    assert_eq!(bytes, plain);
}

#[test]
fn seeded_noise_is_prefix_stable_and_shaped_like_its_distribution() {
    use resonance_svs::stages::noise::{noise, NoiseDist};
    let normal = NoiseDist::Normal { mean: 0.0, scale: 1.0 };
    let long = noise(9, 2, 20_000, normal);
    // The values do not depend on how many were asked for (inputs are
    // over-allocated), and each stream of a seed is its own.
    assert_eq!(&long[..333], &noise(9, 2, 333, normal)[..]);
    assert_ne!(&long[..64], &noise(9, 3, 64, normal)[..]);
    let mean = long.iter().map(|v| *v as f64).sum::<f64>() / long.len() as f64;
    let var = long.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / long.len() as f64;
    assert!(mean.abs() < 0.03, "mean {mean}");
    assert!((var.sqrt() - 1.0).abs() < 0.03, "std {}", var.sqrt());
    let shifted = noise(9, 2, 1000, NoiseDist::Normal { mean: 5.0, scale: 0.1 });
    assert!(shifted.iter().all(|v| (4.0..6.0).contains(v)));
    let uniform = noise(1, 0, 5000, NoiseDist::Uniform { low: -2.0, high: 3.0 });
    assert!(uniform.iter().all(|v| (-2.0..3.0).contains(v)));
}
