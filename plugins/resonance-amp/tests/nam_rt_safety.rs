//! Real-time-safety audit of the NAM inference hot path (ba todo #1113):
//! `process_sample` must not touch the heap for ANY loadable model — the
//! full A2 feature set (bottleneck, blended gating, grouped convs,
//! head1x1, all FiLM sites, windowed heads, condition_dsp, slimmable
//! full-slice) and the legacy A1 path alike. All scratch is preallocated
//! at construction.
//!
//! The check counts allocator calls with a wrapping global allocator.
//! This file deliberately contains a SINGLE test: the counter is global,
//! so concurrent tests in the same binary would pollute it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAllocator;

static ALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

use resonance_amp::nam::parse::load_model_from_file;
use resonance_amp::nam::NamInference;

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/a2/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// A legacy A1-shaped model (no A2 markers), so the historical engine path
/// is audited alongside the reference-semantics one.
fn write_legacy_a1_model() -> std::path::PathBuf {
    // rechannel 2 + 2 layers x (conv 12 + bias 2 + mixin 2 + layer1x1 4+2)
    // + head_rechannel 2 + head bias 1 + head_scale 1 = 50.
    let weights: Vec<String> = (0..50)
        .map(|i| format!("{}", 0.01 + (i as f32) * 0.003))
        .collect();
    let body = format!(
        r#"{{
            "architecture": "WaveNet",
            "sample_rate": 48000,
            "config": {{
                "layers": [{{
                    "input_size": 1, "condition_size": 1, "head_size": 1,
                    "channels": 2, "dilations": [1, 2], "kernel_size": 3,
                    "activation": "Tanh", "gated": false, "head_bias": true
                }}],
                "head": null,
                "head_scale": 0.02
            }},
            "weights": [{}]
        }}"#,
        weights.join(",")
    );
    let path = std::env::temp_dir().join(format!(
        "resonance_amp_rt_safety_a1_{}.nam",
        std::process::id()
    ));
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn process_sample_never_allocates_across_all_model_kinds() {
    // Model construction MAY allocate; audit only the processing loop.
    let legacy_path = write_legacy_a1_model();
    let mut models: Vec<(String, Box<dyn NamInference>)> = Vec::new();
    for name in [
        "A2.nam",
        "slimmable_wavenet.nam",
        "wavenet_a2_max.nam",
        "wavenet_condition_dsp.nam",
    ] {
        let loaded = load_model_from_file(&fixture_path(name))
            .unwrap_or_else(|e| panic!("failed to load {name}: {e}"));
        models.push((name.to_string(), loaded.model));
    }
    let legacy = load_model_from_file(legacy_path.to_str().unwrap())
        .expect("legacy A1 model loads");
    models.push(("legacy A1".to_string(), legacy.model));
    let _ = std::fs::remove_file(&legacy_path);

    for (name, model) in &mut models {
        // Warm up (also covers reset + the first samples, which fill the
        // ring buffers' write paths).
        model.reset();
        for _ in 0..64 {
            model.process_sample(0.0);
        }

        let before = ALLOC_CALLS.load(Ordering::Relaxed);
        let mut acc = 0.0f32;
        for n in 0..4096 {
            let x = ((n as f32) * 0.013).sin() * 0.4;
            acc += model.process_sample(x);
        }
        let after = ALLOC_CALLS.load(Ordering::Relaxed);
        assert!(acc.is_finite(), "{name}: output must stay finite");
        assert_eq!(
            after - before,
            0,
            "{name}: process_sample must not touch the allocator (counted {} calls over 4096 samples)",
            after - before
        );
    }
}
