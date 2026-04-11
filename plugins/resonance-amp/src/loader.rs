//! Background model loader thread + model-priming helpers.
//!
//! Kept out of `lib.rs` so the plugin file stays focused on the audio
//! path. The loader's job is:
//!   1. Poll an `AtomicI32` load-request slot that the audio thread
//!      and editor both write to.
//!   2. Parse the requested `.nam` file on a dedicated thread.
//!   3. Reset + prime the model with silence so its internal ring
//!      buffers settle to steady state (kills the model-swap "plop").
//!   4. Sample the model's nonlinear transfer curve for the editor
//!      visualisation.
//!   5. Publish the result through a `Mutex<Option<...>>` mailbox and
//!      update `viz`/`model_name` so the UI can pick it up.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;

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
    pub mailbox: Arc<Mutex<Option<Box<dyn NamInference>>>>,
    pub model_name: Arc<Mutex<String>>,
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

fn loader_loop(deps: LoaderDeps, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let idx = deps.load_request.swap(-1, Ordering::AcqRel);
        if idx < 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        let path = {
            let list = deps.params.file_list.lock();
            if list.is_empty() {
                continue;
            }
            let clamped = (idx as usize).min(list.len() - 1);
            let p = list[clamped].clone();
            drop(list);
            if let Some(mut mp) = deps.params.model_path.try_lock() {
                *mp = p.clone();
            }
            p
        };

        match nam::parse::load_model_from_file(&path) {
            Ok(mut model) => {
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
                deps.viz.store_transfer_curve(curve);

                // Prime once more so the state is quiet again after the
                // DC ramp excursion, before handing the model over.
                model.reset();
                prime_model(&mut *model, PRIME_SAMPLES);

                let name = Path::new(&path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                *deps.model_name.lock() = name;
                *deps.mailbox.lock() = Some(model);
            }
            Err(e) => {
                eprintln!("Failed to load NAM model: {e}");
                *deps.model_name.lock() = format!("Error: {e}");
            }
        }
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
