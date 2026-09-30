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

use crate::model_ref::{resolve_model, ModelRef, ModelState, ModelStatus, Resolved};
use crate::nam::{self, NamInference};
use crate::params::AmpParams;
use crate::viz::{AmpViz, CURVE_POINTS};

/// How many zero samples to run through a fresh model before we hand it
/// to the audio thread. At typical sample rates this is a few tens of
/// milliseconds — enough for WaveNet ring buffers and LSTM cell state
/// to settle to their true steady-state response.
const PRIME_SAMPLES: usize = 2048;

/// Set on a load request raised by `process()` seeing `file_select`
/// change (a param change), as opposed to an explicit pick from the editor.
/// A param change to the slot the current status already accounts for —
/// the slot a missing or external reference was restored at — is not a
/// pick: the loader leaves what the restored state says playing (B2).
pub(crate) const FROM_PARAM: i32 = 1 << 30;

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
        // A state loaded while active (a project's amp restored into a
        // running instance, an undo, a host preset) comes first, and the
        // slot request its `file_select` raised is superseded by it: the
        // reference's content id is the authority, not the slot number.
        let pending = deps.params.pending_ref.lock().take();
        if let Some(reference) = pending {
            deps.load_request.store(-1, Ordering::Release);
            if let Some(model) = apply_reference(&deps.params, &deps.viz, reference, true) {
                deps.mailbox.post(model);
            }
            continue;
        }

        let request = deps.load_request.swap(-1, Ordering::AcqRel);
        if request < 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        let slot = request & !FROM_PARAM;
        if request & FROM_PARAM != 0 && status_accounts_for(&deps.params, slot) {
            continue;
        }
        if let Some(model) = load_slot(&deps.params, &deps.viz, slot as u32) {
            deps.mailbox.post(model);
        }
    }
}

/// Whether the status already describes `file_select == slot`: a restored
/// reference that is missing, or playing from outside the library, at that
/// slot. A param change landing there (the first `process()` after a
/// `load_state` sees the restored `file_select` as a change) must not load
/// whatever the library holds in the slot.
fn status_accounts_for(params: &AmpParams, slot: i32) -> bool {
    let st = params.status.lock();
    match &st.state {
        ModelState::Missing { at_slot, .. } => *at_slot == slot,
        ModelState::Loaded => st.external && st.external_slot == Some(slot),
        _ => false,
    }
}

/// Load library slot `slot`, recording what plays. `None` when nothing is
/// to be installed: an empty slot (loads nothing, unloads nothing), a
/// failed load (what was playing keeps playing), or the model already
/// playing.
fn load_slot(params: &AmpParams, viz: &AmpViz, slot: u32) -> Option<Box<dyn NamInference>> {
    let mut entry = params.library.read().by_slot(slot).cloned();
    if entry.as_ref().is_none_or(|e| !e.path.is_file()) {
        // With no editor open nothing polls the library, so a request for
        // a slot this process has not seen filled (another process
        // downloaded into it), or whose file went away, refreshes it first
        // — throttled, so automation over empty slots does not rescan per
        // step.
        if params.library.rescan_for_miss() {
            entry = params.library.read().by_slot(slot).cloned();
        }
    }
    let Some(entry) = entry else {
        params.status.lock().state = ModelState::EmptySlot(slot);
        return None;
    };
    {
        let st = params.status.lock();
        if st.state == ModelState::Loaded && st.id.as_deref() == Some(entry.id.as_str()) && !st.external {
            // Same bytes already playing (e.g. the slot a state restore
            // just re-derived): nothing to swap.
            return None;
        }
    }
    let path = entry.path.to_string_lossy().into_owned();
    match prepare_model(&path, viz) {
        Ok(model) => {
            // Recorded after the load, so the reference `save_state`
            // persists is always the model that is actually playing.
            *params.model_ref.lock() = ModelRef::from_entry(&entry);
            params.library.set_usage(params.instance_id, Some(&entry.id));
            *params.status.lock() = ModelStatus {
                name: entry.name.clone(),
                id: Some(entry.id.clone()),
                state: ModelState::Loaded,
                ..ModelStatus::default()
            };
            Some(model)
        }
        Err(e) => {
            tracing::warn!("failed to load NAM model {path}: {e}");
            params.status.lock().state = ModelState::Error(e);
            None
        }
    }
}

/// An identity "model": what an active instance swaps to when a restored
/// reference names nothing, or a model that is missing, so what plays is
/// what the saved state says (a clean signal) without dropping the old
/// model on the audio thread — the swap fader retires it to its janitor.
pub(crate) struct Passthrough;

impl NamInference for Passthrough {
    fn process_sample(&mut self, input: f32) -> f32 {
        input
    }

    fn reset(&mut self) {}

    fn is_identity(&self) -> bool {
        true
    }
}

/// Resolve a saved `reference` against the library (nam-model-library.md
/// §5.2), prepare what it names, and record the outcome in `params`: the
/// reference `save_state` persists, the status, the usage count and a
/// re-derived `file_select`. Returns the model to install.
///
/// `active` is whether a model may already be playing: then a reference
/// that resolves to nothing, or to a missing or unloadable model, installs
/// a [`Passthrough`], so what plays matches what is saved; and a reference
/// to the model already playing installs nothing.
///
/// The reference is written only once its outcome is known: after a
/// successful load (filled in from the library entry), or verbatim when
/// the model is missing, so a re-save loses nothing.
pub(crate) fn apply_reference(
    params: &AmpParams,
    viz: &AmpViz,
    reference: ModelRef,
    active: bool,
) -> Option<Box<dyn NamInference>> {
    let at_slot = params.file_select.value();
    let resolved = {
        let lib = params.library.read();
        resolve_model(&reference, &lib, |p| params.library.content_id(p))
    };
    let silence = || -> Option<Box<dyn NamInference>> { active.then(|| Box::new(Passthrough) as _) };
    let (path, entry, notice) = match resolved {
        Resolved::Nothing => {
            *params.model_ref.lock() = reference;
            *params.status.lock() = ModelStatus::default();
            params.library.set_usage(params.instance_id, None);
            return silence();
        }
        Resolved::Missing {
            name,
            path,
            source,
            file_changed,
        } => {
            *params.model_ref.lock() = reference;
            *params.status.lock() = ModelStatus {
                state: ModelState::Missing {
                    name,
                    path,
                    source,
                    file_changed,
                    at_slot,
                },
                ..ModelStatus::default()
            };
            params.library.set_usage(params.instance_id, None);
            return silence();
        }
        Resolved::Load { path, entry } => (path, entry, None),
        Resolved::Relinked { entry } => {
            let notice = format!("Relinked: {} (file had moved)", entry.name);
            (entry.path.clone(), Some(entry), Some(notice))
        }
    };

    let path_str = path.to_string_lossy().into_owned();
    let id = entry
        .as_ref()
        .map(|e| e.id.clone())
        .or_else(|| params.library.content_id(&path));
    // A file with the same bytes as a library entry is that model, wherever
    // it sits; only one the library has nothing like is external.
    let external = entry.is_none();
    let slot = entry.as_ref().and_then(|e| e.slot);

    let already = active && {
        let st = params.status.lock();
        st.state == ModelState::Loaded && st.id.is_some() && st.id == id
    };
    let model = if already {
        None
    } else {
        match prepare_model(&path_str, viz) {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::warn!("failed to load NAM model {path_str}: {e}");
                *params.model_ref.lock() = reference;
                *params.status.lock() = ModelStatus {
                    state: ModelState::Error(e),
                    ..ModelStatus::default()
                };
                params.library.set_usage(params.instance_id, None);
                return silence();
            }
        }
    };

    // What plays now; record it.
    let mut saved = match &entry {
        Some(e) => ModelRef::from_entry(e),
        None => reference.clone(),
    };
    saved.path = path_str;
    if saved.id.is_none() {
        saved.id = id.clone();
    }
    *params.model_ref.lock() = saved;
    if let Some(slot) = slot {
        params.file_select.set_value(slot as i32);
    }
    *params.status.lock() = ModelStatus {
        name: entry
            .as_ref()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| reference.display_name()),
        id: id.clone(),
        state: ModelState::Loaded,
        external,
        external_slot: external.then(|| params.file_select.value()),
        notice,
        ..ModelStatus::default()
    };
    params.library.set_usage(params.instance_id, id.as_deref());
    model
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
