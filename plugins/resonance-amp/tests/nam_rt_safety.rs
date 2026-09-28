//! Real-time-safety audit of the NAM inference hot path (ba todo #1113):
//! `process_sample` / `process_block` must not touch the heap for ANY loadable model — the
//! full A2 feature set (bottleneck, blended gating, grouped convs,
//! head1x1, all FiLM sites, windowed heads, condition_dsp, slimmable
//! full-slice) and the legacy A1 path alike. All scratch is preallocated
//! at construction.
//!
//! The check counts allocator calls with a wrapping global allocator,
//! armed only on the thread running the model (NAM inference is
//! single-threaded: `process_*` never hands work to another thread). A
//! process-wide count used to flake under load: once the test ran past
//! 60 s, libtest's main thread allocated five times to collect and print
//! its "has been running for over 60 seconds" notice, landing inside the
//! measured window. This file still holds a single test, so nothing else
//! in the binary shares the counter.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAllocator;

static ALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn count() {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Allocator calls made on THIS thread while running `f`.
fn allocs_during(f: impl FnOnce()) -> usize {
    let before = ALLOC_CALLS.load(Ordering::Relaxed);
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    ALLOC_CALLS.load(Ordering::Relaxed) - before
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
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

fn lstm_fixture_path() -> String {
    format!(
        "{}/tests/fixtures/lstm/lstm.nam",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// An A1-shaped model (no A2 markers), so the fast-activation A1 flavor
/// is audited alongside the exact-flavor A2 fixtures (both share the
/// reference forward structure since ba todo #1116).
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
    // The recurrent architecture too (real NAM LSTM export, ba todo #1115).
    let lstm = load_model_from_file(&lstm_fixture_path()).expect("LSTM fixture loads");
    models.push(("lstm.nam".to_string(), lstm.model));

    for (name, model) in &mut models {
        // Warm up (also covers reset + the first samples, which fill the
        // ring buffers' write paths).
        model.reset();
        for _ in 0..64 {
            model.process_sample(0.0);
        }

        let mut acc = 0.0f32;
        let calls = allocs_during(|| {
            for n in 0..4096 {
                let x = ((n as f32) * 0.013).sin() * 0.4;
                acc += model.process_sample(x);
            }
        });
        assert!(acc.is_finite(), "{name}: output must stay finite");
        assert_eq!(
            calls, 0,
            "{name}: process_sample must not touch the allocator (counted {calls} calls over 4096 samples)"
        );

        // The block path the amp actually runs, at a host-sized block
        // and one past the WaveNet's internal chunk; long enough for
        // every layer history to rewind many times. A history rewinds
        // every max(lookback, 256) frames, and the longest lookback in
        // these fixtures is 1195 (A2.nam), so 8 x 2000 frames rewind each
        // layer at least 13 times, at shifting alignments.
        let input: Vec<f32> = (0..1000).map(|n| ((n as f32) * 0.013).sin() * 0.4).collect();
        let mut output = vec![0.0f32; input.len()];
        let calls = allocs_during(|| {
            for _ in 0..8 {
                for block in [128, 1000] {
                    for (i, o) in input.chunks(block).zip(output.chunks_mut(block)) {
                        model.process_block(i, o);
                    }
                }
            }
        });
        assert!(output.iter().all(|v| v.is_finite()), "{name}: block output must stay finite");
        assert_eq!(
            calls, 0,
            "{name}: process_block must not touch the allocator (counted {calls} calls)"
        );
    }
}
