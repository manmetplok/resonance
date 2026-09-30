//! Background model loader thread + model-priming helpers.
//!
//! Kept out of `lib.rs` so the plugin file stays focused on the audio
//! path. The loader's job is:
//!   1. Poll an `AtomicI32` load-request slot that the audio thread
//!      and editor both write to. The value is a **library slot**
//!      (nam-model-library.md §5.1).
//!   2. Resolve the slot to a file through the shared library (a read
//!      lock, on this thread — never the audio thread's business). An
//!      empty slot loads nothing and unloads nothing.
//!   3. Parse the `.nam` file, reset + prime the model with silence so its
//!      internal ring buffers settle to steady state (kills the model-swap
//!      "plop"), and sample its transfer curve for the editor.
//!   4. Publish the result through a [`Mailbox`], then record what plays
//!      in the model reference (what `save_state` persists) and status.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use resonance_plugin::Mailbox;

use crate::model_ref::{ModelRef, ModelState};
use crate::nam::{self, NamInference};
use crate::params::AmpParams;
use crate::viz::{AmpViz, CURVE_POINTS};

/// How many zero samples to run through a fresh model before we hand it
/// to the audio thread. At typical sample rates this is a few tens of
/// milliseconds — enough for WaveNet ring buffers and LSTM cell state
/// to settle to their true steady-state response.
const PRIME_SAMPLES: usize = 2048;

/// Handle returned by `start`. Dropping it or calling `stop` cleanly
/// joins the thread.
pub struct LoaderHandle {
    handle: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl LoaderHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LoaderHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct LoaderDeps {
    pub params: Arc<AmpParams>,
    pub mailbox: Mailbox<Box<dyn NamInference>>,
    pub load_request: Arc<AtomicI32>,
    pub viz: Arc<AmpViz>,
}

/// Spawn the persistent loader thread.
pub fn start(deps: LoaderDeps) -> LoaderHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let handle = std::thread::Builder::new()
        .name("amp-loader".into())
        .spawn(move || loader_loop(deps, stop_clone))
        .expect("failed to spawn amp-loader thread");

    LoaderHandle {
        handle: Some(handle),
        stop,
    }
}

/// Parse, prime and curve-sample the model at `path`, off the audio
/// thread. Shared by the loader thread and `initialize`'s synchronous
/// first load.
///
/// The NAM parser is hardened to return `Err` on hostile/corrupt input
/// rather than panic (see `nam::parse::weights::checked_count` and the
/// WaveNet bounds validation), but `catch_unwind` is defense-in-depth: a
/// panic anywhere in parse-and-prime — this parser, a future one, or the
/// priming/curve-sampling DSP itself — must not take the whole loader
/// thread down for the rest of the session. Without this, one bad file
/// means every *subsequent* model pick silently does nothing, which is
/// much harder to diagnose than a single failed load.
pub fn prepare_model(path: &str, viz: &AmpViz) -> Result<Box<dyn NamInference>, String> {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        nam::parse::load_model_from_file(path).map(|loaded| {
            let mut model = loaded.model;
            note_model_sample_rate(viz, loaded.sample_rate);
            // Reset + prime so the audio thread gets a model that's
            // already at steady state. This is the core "plop" fix —
            // WaveNet and LSTM profiles both emit a small transient
            // for the first few dozen samples as their internal ring
            // buffers fill and biases propagate.
            model.reset();
            prime_model(&mut *model, PRIME_SAMPLES);

            // While we still have exclusive access, sample the
            // transfer curve for the editor. Runs off the audio
            // thread so cost is free.
            let curve = sample_transfer_curve(&mut *model);
            viz.store_transfer_curve(curve);

            // Prime once more so the state is quiet again after the
            // DC ramp excursion, before handing the model over.
            model.reset();
            prime_model(&mut *model, PRIME_SAMPLES);
            model
        })
    }));
    match outcome {
        Ok(result) => result,
        Err(panic) => {
            let msg = panic_message(&panic);
            tracing::error!("panic while loading NAM model {path}: {msg}");
            Err(msg)
        }
    }
}

fn loader_loop(deps: LoaderDeps, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let slot = deps.load_request.swap(-1, Ordering::AcqRel);
        if slot < 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        // Slot → file, under the shared library's read lock. An empty slot
        // means "no model" (§5.1): nothing is loaded, and — unlike the old
        // directory index — nothing is clamped onto the last entry, and
        // what is playing keeps playing.
        let entry = deps.params.library.read().by_slot(slot as u32).cloned();
        let Some(entry) = entry else {
            deps.params.status.lock().state = ModelState::EmptySlot(slot as u32);
            continue;
        };
        let path = entry.path.to_string_lossy().into_owned();

        match prepare_model(&path, &deps.viz) {
            Ok(model) => {
                deps.mailbox.post(model);
                // Recorded after the load, so the reference `save_state`
                // persists is always the model that is actually playing. A
                // blocking lock, not `try_lock`: this is the loader thread,
                // and a lost write here would silently revert the model on
                // the next activation.
                *deps.params.model_ref.lock() = ModelRef::from_entry(&entry);
                let mut st = deps.params.status.lock();
                st.name = entry.name.clone();
                st.id = Some(entry.id.clone());
                st.state = ModelState::Loaded;
                st.external = false;
                st.deleted = false;
                st.notice = None;
            }
            Err(e) => {
                tracing::warn!("failed to load NAM model {path}: {e}");
                deps.params.status.lock().state = ModelState::Error(e);
            }
        }
    }
}


/// Best-effort text for a `catch_unwind` payload. `panic!` payloads are
/// almost always `&str` (a string-literal message) or `String` (a
/// formatted one); anything else surfaces as a generic placeholder rather
/// than failing to report at all.
fn panic_message(payload: &(dyn std::any::Any + Send + 'static)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Record the model's native sample rate for the editor's mismatch
/// warning, and log if it differs from the engine rate (which shifts the
/// amp's frequency response — NAM models have no built-in resampling).
pub fn note_model_sample_rate(viz: &AmpViz, model_rate: f32) {
    viz.store_model_sample_rate(model_rate);
    let (_, engine_rate) = viz.read_sample_rates();
    if engine_rate > 0.0 && (model_rate - engine_rate).abs() > 0.5 {
        eprintln!(
            "Warning: NAM model sample rate {model_rate} Hz differs from engine rate {engine_rate} Hz; the amp's frequency response will be shifted"
        );
    }
}

/// Run `n` zero samples through the model. Used to settle internal
/// state so the audio thread picks it up at steady-state.
pub fn prime_model(model: &mut dyn NamInference, n: usize) {
    for _ in 0..n {
        let _ = model.process_sample(0.0);
    }
}

/// Sample the model's static nonlinear transfer curve at `CURVE_POINTS`
/// input amplitudes from -1.0 to +1.0. For each sample the model is
/// driven with a short DC hold so its internal state adapts, then the
/// steady-state output is recorded. The result is a visual "fingerprint"
/// of the amp profile that the editor can draw on model change.
pub fn sample_transfer_curve(model: &mut dyn NamInference) -> [f32; CURVE_POINTS] {
    // Samples per DC hold. Long enough for WaveNet receptive fields to
    // absorb the new DC level, short enough to keep the whole sweep
    // inside ~a few ms of CPU time.
    const HOLD: usize = 256;

    let mut out = [0.0f32; CURVE_POINTS];
    model.reset();
    for (i, slot) in out.iter_mut().enumerate() {
        let t = i as f32 / (CURVE_POINTS - 1) as f32;
        let x = t * 2.0 - 1.0; // -1..+1
        let mut last = 0.0f32;
        for _ in 0..HOLD {
            last = model.process_sample(x);
        }
        *slot = last;
    }
    out
}
