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

/// ONNX element types the hand-written models use.
const FLOAT: u64 = 1;
const BOOL: u64 = 9;
const INT64: u64 = 7;
const DOUBLE: u64 = 11;

/// A tensor `ValueInfoProto` of element type `elem` with one symbolic dim
/// per entry of `dims` (a number for a fixed one; none for a scalar).
fn pb_info(name: &str, elem: u64, dims: &[&str]) -> Vec<u8> {
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
    pb_int(&mut tensor, 1, elem);
    pb_len(&mut tensor, 2, &shape);
    let mut ty = Vec::new();
    pb_len(&mut ty, 1, &tensor);
    let mut out = Vec::new();
    pb_len(&mut out, 1, name.as_bytes());
    pb_len(&mut out, 2, &ty);
    out
}

/// A float tensor `ValueInfoProto` (see [`pb_info`]).
fn pb_tensor_info(name: &str, dims: &[&str]) -> Vec<u8> {
    pb_info(name, FLOAT, dims)
}

/// An `AttributeProto`: `int` (type 2), `ints` (7) or `graph` (5).
fn pb_attr_int(name: &str, v: u64) -> Vec<u8> {
    let mut a = Vec::new();
    pb_len(&mut a, 1, name.as_bytes());
    pb_int(&mut a, 3, v);
    pb_int(&mut a, 20, 2);
    a
}

fn pb_attr_graph(name: &str, graph: &[u8]) -> Vec<u8> {
    let mut a = Vec::new();
    pb_len(&mut a, 1, name.as_bytes());
    pb_len(&mut a, 6, graph);
    pb_int(&mut a, 20, 5);
    a
}

fn pb_node(op: &str, inputs: &[&str], outputs: &[&str], attrs: &[Vec<u8>]) -> Vec<u8> {
    let mut node = Vec::new();
    for i in inputs {
        pb_len(&mut node, 1, i.as_bytes());
    }
    for o in outputs {
        pb_len(&mut node, 2, o.as_bytes());
    }
    pb_len(&mut node, 3, format!("{op}_{}", outputs.first().unwrap_or(&"")).as_bytes());
    pb_len(&mut node, 4, op.as_bytes());
    for a in attrs {
        pb_len(&mut node, 5, a);
    }
    node
}

/// A `ModelProto` around `graph` at default-domain opset `opset`.
fn pb_model(graph: &[u8], opset: u64) -> Vec<u8> {
    let mut op = Vec::new();
    pb_len(&mut op, 1, b"");
    pb_int(&mut op, 2, opset);
    let mut model = Vec::new();
    pb_int(&mut model, 1, 8);
    pb_len(&mut model, 8, &op);
    pb_len(&mut model, 7, graph);
    model
}

/// A one-node model: `y = <op>(x)` for a `*Like` op (input `x` of shape
/// `[n]`), or `y = <op>()` with `shape = [2, 3]` for a fixed one.
fn random_model(op: &str) -> Vec<u8> {
    random_model_at(op, 15)
}

fn random_model_at(op: &str, opset: u64) -> Vec<u8> {
    let like = op.ends_with("Like");
    let mut node = Vec::new();
    if like {
        pb_len(&mut node, 1, b"x");
    }
    pb_len(&mut node, 2, b"y");
    pb_len(&mut node, 3, b"rand");
    pb_len(&mut node, 4, op.as_bytes());
    pb_len(&mut node, 5, &pb_attr_int("dtype", 1));
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
    pb_model(&graph, opset)
}

/// `y = Loop(M, "", x)` whose body adds `RandomNormalLike(v)` to the
/// carried `v` each iteration: a random op in a subgraph.
fn loop_model() -> Vec<u8> {
    let mut body = Vec::new();
    pb_len(&mut body, 1, &pb_node("RandomNormalLike", &["v"], &["r"], &[pb_attr_int("dtype", 1)]));
    pb_len(&mut body, 1, &pb_node("Add", &["v", "r"], &["v_out"], &[]));
    pb_len(&mut body, 1, &pb_node("Identity", &["cond_in"], &["cond_out"], &[]));
    pb_len(&mut body, 2, b"body");
    pb_len(&mut body, 11, &pb_info("i", INT64, &[]));
    pb_len(&mut body, 11, &pb_info("cond_in", BOOL, &[]));
    pb_len(&mut body, 11, &pb_tensor_info("v", &["n"]));
    pb_len(&mut body, 12, &pb_info("cond_out", BOOL, &[]));
    pb_len(&mut body, 12, &pb_tensor_info("v_out", &["n"]));
    let mut graph = Vec::new();
    pb_len(&mut graph, 1, &pb_node("Loop", &["M", "", "x"], &["y"], &[pb_attr_graph("body", &body)]));
    pb_len(&mut graph, 2, b"g");
    pb_len(&mut graph, 11, &pb_info("M", INT64, &[]));
    pb_len(&mut graph, 11, &pb_tensor_info("x", &["n"]));
    pb_len(&mut graph, 12, &pb_tensor_info("y", &["n"]));
    pb_model(&graph, 15)
}

/// `c = Cast<double>(x); y = RandomNormalLike(c)` with no `dtype`: the op
/// draws doubles (its input's type), which the scan cannot see (`c` has
/// no declared type), so the rewritten model does not build.
fn double_like_model() -> Vec<u8> {
    let mut graph = Vec::new();
    pb_len(&mut graph, 1, &pb_node("Cast", &["x"], &["c"], &[pb_attr_int("to", DOUBLE)]));
    pb_len(&mut graph, 1, &pb_node("RandomNormalLike", &["c"], &["y"], &[]));
    pb_len(&mut graph, 2, b"g");
    pb_len(&mut graph, 11, &pb_tensor_info("x", &["n"]));
    pb_len(&mut graph, 12, &pb_info("y", DOUBLE, &["n"]));
    pb_model(&graph, 15)
}

/// `y = RandomUniformLike(x)` with no `dtype` over a declared float `x`.
fn untyped_like_model() -> Vec<u8> {
    let mut graph = Vec::new();
    pb_len(&mut graph, 1, &pb_node("RandomUniformLike", &["x"], &["y"], &[]));
    pb_len(&mut graph, 2, b"g");
    pb_len(&mut graph, 11, &pb_tensor_info("x", &["n"]));
    pb_len(&mut graph, 12, &pb_tensor_info("y", &["n"]));
    pb_model(&graph, 15)
}

fn x_input(n: usize) -> Vec<(String, ort::value::Value)> {
    let x = ndarray::Array1::<f32>::zeros(n);
    vec![("x".into(), ort::value::Value::from_array(x).unwrap().into())]
}

fn run_seeded(model: &[u8], x_len: Option<usize>, seed: u64) -> Vec<f32> {
    use resonance_svs::stages::common::{build_session_from, ModelSource, SeededSession};
    use resonance_svs::stages::noise::seed_random_ops;
    use resonance_svs::ExecutionProvider;
    let seeded = seed_random_ops(model).expect("rewrite");
    let bytes = seeded.bytes.expect("rewritten");
    let path = std::path::Path::new("in-memory.onnx");
    let (mut session, _) =
        build_session_from(ModelSource::Bytes { path, bytes: &bytes }, ExecutionProvider::Cpu, 0)
            .expect("the rewritten model loads");
    let mut inputs = x_len.map(x_input).unwrap_or_default();
    inputs.extend(SeededSession::noise_values(&seeded.specs, seed, 8, 1).unwrap());
    let outputs = session.run(inputs).expect("runs");
    let (_, y) = outputs.get("y").unwrap().try_extract_tensor::<f32>().unwrap();
    y.to_vec()
}

/// `model` written to a fresh file, for [`SeededSession::load`].
fn model_file(name: &str, model: &[u8]) -> PathBuf {
    let dir = env::temp_dir().join(format!("resonance-svs-noise-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("model.onnx");
    std::fs::write(&path, model).unwrap();
    path
}

#[test]
fn seeding_replaces_a_random_like_op_with_a_fed_noise_input() {
    use resonance_svs::stages::noise::{noise, seed_random_ops, NoiseDist, NoiseSize, NoiseSpec};
    let model = random_model("RandomNormalLike");
    let seeded = seed_random_ops(&model).unwrap();
    assert!(seeded.repeatable());
    assert_eq!(
        seeded.specs,
        vec![NoiseSpec {
            input: "resonance_noise_0".into(),
            dist: NoiseDist::Normal { mean: 0.0, scale: 1.0 },
            size: NoiseSize::Like,
            count_output: Some("resonance_rn0_n".into()),
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
    use resonance_svs::stages::noise::{seed_random_ops, NoiseDist, NoiseSize};
    let model = random_model("RandomUniform");
    let seeded = seed_random_ops(&model).unwrap();
    assert_eq!(seeded.specs[0].dist, NoiseDist::Uniform { low: 0.0, high: 1.0 });
    assert_eq!(seeded.specs[0].size, NoiseSize::Fixed(6), "shape [2, 3]");
    assert_eq!(seeded.specs[0].count_output, None);
    let y = run_seeded(&model, None, 7);
    assert_eq!(y.len(), 6, "shape [2, 3]");
    assert!(y.iter().all(|v| (0.0..1.0).contains(v)));
    assert_eq!(y, run_seeded(&model, None, 7));

    // A model without random ops is not copied at all.
    let plain = random_model("Identity");
    let seeded = seed_random_ops(&plain).unwrap();
    assert!(seeded.specs.is_empty() && seeded.repeatable());
    assert_eq!(seeded.bytes, None);
}

/// A session feeds a fixed-shape op exactly its size, and a `*Like` op
/// the bound until a run has shown how many values it draws — then
/// exactly that many, for any length (`a · frames + b`). The values are
/// the stream's, whatever the length.
#[test]
fn a_seeded_session_learns_how_much_noise_each_op_draws() {
    use resonance_svs::stages::common::SeededSession;
    use resonance_svs::stages::noise::{noise, NoiseDist};
    use resonance_svs::ExecutionProvider;
    let fixed = SeededSession::load(&model_file("fixed", &random_model("RandomUniform")), ExecutionProvider::Cpu, 0)
        .unwrap();
    assert_eq!(fixed.noise_lengths(400, 5120), vec![6], "fixed shape: exactly its size");

    let path = model_file("like", &random_model("RandomNormalLike"));
    let mut s = SeededSession::load(&path, ExecutionProvider::Cpu, 0).unwrap();
    assert!(s.repeatable);
    assert_eq!(s.noise_lengths(5, 4), vec![5 * 4 + 1024], "unknown: the bound");
    let normal = NoiseDist::Normal { mean: 0.0, scale: 1.0 };
    let run = |s: &mut SeededSession, n: usize| {
        s.run(&x_input(n), 3, n, 4, |out| {
            let (_, y) = out.get("y").unwrap().try_extract_tensor::<f32>()?;
            Ok(y.to_vec())
        })
        .unwrap()
    };
    assert_eq!(run(&mut s, 5), noise(3, 0, 5, normal));
    assert_eq!(s.noise_lengths(5, 4), vec![5]);
    assert_eq!(s.noise_lengths(9, 4), vec![9], "one run: guessed per frame");
    assert_eq!(run(&mut s, 9), noise(3, 0, 9, normal));
    assert_eq!(s.noise_lengths(1000, 4), vec![1000], "two runs: a = 1, b = 0");
    assert_eq!(run(&mut s, 3), noise(3, 0, 3, normal));
}

/// A random op inside a `Loop` body is not rewritten: it is counted, the
/// model loads as it is and says it is not repeatable.
#[test]
fn a_random_op_in_a_loop_body_is_counted_and_the_model_still_loads() {
    use resonance_svs::stages::common::SeededSession;
    use resonance_svs::stages::noise::seed_random_ops;
    use resonance_svs::ExecutionProvider;
    let model = loop_model();
    let seeded = seed_random_ops(&model).unwrap();
    assert_eq!(seeded.unseeded, vec!["RandomNormalLike in a subgraph".to_owned()]);
    assert!(!seeded.repeatable());
    assert!(seeded.specs.is_empty() && seeded.bytes.is_none());

    let mut s = SeededSession::load(&model_file("loop", &model), ExecutionProvider::Cpu, 0).unwrap();
    assert!(!s.repeatable);
    let mut inputs = x_input(4);
    let m = ndarray::Array0::<i64>::from_elem((), 2);
    inputs.push(("M".into(), ort::value::Value::from_array(m).unwrap().into()));
    let y = s
        .run(&inputs, 1, 4, 1, |out| {
            let (_, y) = out.get("y").unwrap().try_extract_tensor::<f32>()?;
            Ok(y.to_vec())
        })
        .unwrap();
    assert_eq!(y.len(), 4);
}

/// A `*Like` op without `dtype` takes its input's type: over a declared
/// float input it is seeded; over a value whose type only the runtime
/// knows (a double, here) the rewritten model does not build, and the
/// file is loaded as it is — it runs, unseeded.
#[test]
fn a_like_op_without_dtype_is_seeded_over_floats_and_falls_back_otherwise() {
    use resonance_svs::stages::common::SeededSession;
    use resonance_svs::stages::noise::seed_random_ops;
    use resonance_svs::ExecutionProvider;
    let untyped = untyped_like_model();
    let seeded = seed_random_ops(&untyped).unwrap();
    assert_eq!(seeded.specs.len(), 1);
    assert!(seeded.repeatable());
    let a = run_seeded(&untyped, Some(6), 5);
    assert_eq!(a, run_seeded(&untyped, Some(6), 5));

    let double = double_like_model();
    assert_eq!(seed_random_ops(&double).unwrap().specs.len(), 1, "the scan cannot tell");
    let mut s = SeededSession::load(&model_file("double", &double), ExecutionProvider::Cpu, 0)
        .expect("falls back to the file as it is");
    assert!(!s.repeatable);
    assert!(s.noise.is_empty());
    let y = s
        .run(&x_input(3), 1, 3, 1, |out| {
            let (_, y) = out.get("y").unwrap().try_extract_tensor::<f64>()?;
            Ok(y.to_vec())
        })
        .unwrap();
    assert_eq!(y.len(), 3);
}

/// Models the rewrite leaves alone: an explicit non-float `dtype`, an
/// opset before 12, weights in external data — each counted. A truncated
/// file is an error from the rewrite, and from the load that falls back
/// (it cannot be read either way), not a panic.
#[test]
fn models_the_rewrite_cannot_seed_are_loaded_unseeded_or_refused_cleanly() {
    use resonance_svs::stages::common::SeededSession;
    use resonance_svs::stages::noise::seed_random_ops;
    use resonance_svs::ExecutionProvider;
    let old = random_model_at("RandomNormalLike", 11);
    let seeded = seed_random_ops(&old).unwrap();
    assert!(seeded.bytes.is_none() && !seeded.repeatable());
    assert!(seeded.unseeded[0].contains("opset"), "{:?}", seeded.unseeded);

    // An initializer whose data lives in an external file.
    let mut tensor = Vec::new();
    pb_int(&mut tensor, 1, 1);
    pb_int(&mut tensor, 2, FLOAT);
    pb_len(&mut tensor, 8, b"w");
    pb_int(&mut tensor, 14, 1);
    let mut graph = Vec::new();
    pb_len(&mut graph, 1, &pb_node("RandomNormalLike", &["x"], &["y"], &[pb_attr_int("dtype", 1)]));
    pb_len(&mut graph, 2, b"g");
    pb_len(&mut graph, 5, &tensor);
    pb_len(&mut graph, 11, &pb_tensor_info("x", &["n"]));
    pb_len(&mut graph, 12, &pb_tensor_info("y", &["n"]));
    let seeded = seed_random_ops(&pb_model(&graph, 15)).unwrap();
    assert!(seeded.bytes.is_none() && !seeded.repeatable());
    assert!(seeded.unseeded[0].contains("external"), "{:?}", seeded.unseeded);

    // An explicit double dtype.
    let mut graph = Vec::new();
    pb_len(&mut graph, 1, &pb_node("RandomNormalLike", &["x"], &["y"], &[pb_attr_int("dtype", DOUBLE)]));
    pb_len(&mut graph, 2, b"g");
    pb_len(&mut graph, 11, &pb_tensor_info("x", &["n"]));
    pb_len(&mut graph, 12, &pb_info("y", DOUBLE, &["n"]));
    let seeded = seed_random_ops(&pb_model(&graph, 15)).unwrap();
    assert!(seeded.bytes.is_none() && !seeded.repeatable());

    let model = random_model("RandomNormalLike");
    let truncated = &model[..model.len() - 7];
    assert!(seed_random_ops(truncated).is_err());
    let err = SeededSession::load(&model_file("truncated", truncated), ExecutionProvider::Cpu, 0)
        .err()
        .expect("an unreadable model is an error");
    assert!(format!("{err:#}").contains("model.onnx"), "{err:#}");
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
