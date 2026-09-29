//! Audio-input enumeration, off the engine control thread.
//!
//! Enumerating inputs can block for as long as a person takes to answer a
//! dialog: on macOS, cpal's CoreAudio calls wait on the microphone
//! permission prompt the first time a process touches an input device.
//! The app asks for the input list at startup, so running that on the
//! engine thread stalled every command behind it — the CLAP scan never
//! reported, the plugin catalog stayed empty, and no plugin could be
//! added until the prompt was answered.
//!
//! So both consumers — `ListInputDevices` and the external-instrument
//! `CheckExternalInstrumentDevices` — hand the enumeration to one worker
//! thread. Requests that arrive while it runs are coalesced into its next
//! pass, so a stuck enumeration parks one thread, never one per request.
//! The list goes straight out as `InputDevicesListed`; the device check's
//! names come back on the engine inbox, and the check itself runs on the
//! engine thread against the track's configuration at that moment.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::platform;
use crate::types::{AudioEvent, InputDeviceInfo, TrackId};

use super::internal::EngineInternal;
use super::thread::HandlerCtx;

/// What enumerates the inputs: [`platform::enumerate_input_devices`],
/// or a test's stand-in.
pub type InputEnumerator = Arc<dyn Fn() -> (Vec<InputDeviceInfo>, Option<String>) + Send + Sync>;

/// Requests waiting for the worker's next pass.
#[derive(Default)]
struct Pending {
    /// A worker thread is alive and will look at this again before it exits.
    running: bool,
    /// Someone asked for `InputDevicesListed`.
    list: bool,
    /// External-instrument tracks waiting for a device check.
    checks: HashSet<TrackId>,
}

/// The engine thread's handle on the enumeration worker.
pub(crate) struct InputDeviceWorker {
    enumerate: InputEnumerator,
    pending: Arc<Mutex<Pending>>,
}

impl InputDeviceWorker {
    pub(crate) fn new() -> Self {
        Self {
            enumerate: Arc::new(platform::enumerate_input_devices),
            pending: Arc::default(),
        }
    }

    /// Swap the enumerator (tests: one that blocks, or one that reports a
    /// fixed device set).
    pub(crate) fn set_enumerator(&mut self, enumerate: InputEnumerator) {
        self.enumerate = enumerate;
    }

    /// Queue an `InputDevicesListed`. Returns at once.
    pub(crate) fn request_list(&self, ctx: &HandlerCtx) {
        self.request(ctx, |p| p.list = true);
    }

    /// Queue a device check for an external-instrument track. Returns at
    /// once; the result arrives as [`EngineInternal::InputDevicesForCheck`].
    pub(crate) fn request_check(&self, ctx: &HandlerCtx, track_id: TrackId) {
        self.request(ctx, |p| {
            p.checks.insert(track_id);
        });
    }

    fn request(&self, ctx: &HandlerCtx, add: impl FnOnce(&mut Pending)) {
        let mut pending = lock(&self.pending);
        add(&mut pending);
        if pending.running {
            // The live worker picks this up on its next pass.
            return;
        }
        pending.running = true;
        drop(pending);

        let enumerate = Arc::clone(&self.enumerate);
        let worker_pending = Arc::clone(&self.pending);
        let event_tx = ctx.event_tx.clone();
        let shared = Arc::clone(ctx.shared);
        let spawned = std::thread::Builder::new()
            .name("input-devices".into())
            .spawn(move || loop {
                let (list, checks) = {
                    let mut p = lock(&worker_pending);
                    if !p.list && p.checks.is_empty() {
                        p.running = false;
                        return;
                    }
                    (std::mem::take(&mut p.list), std::mem::take(&mut p.checks))
                };
                let (devices, default_name) = run_enumerator(&enumerate);
                if !checks.is_empty() {
                    shared.inbox.post(EngineInternal::InputDevicesForCheck {
                        track_ids: checks,
                        available_inputs: devices.iter().map(|d| d.name.clone()).collect(),
                    });
                }
                if list {
                    let _ = event_tx.send(AudioEvent::InputDevicesListed {
                        devices,
                        default_name,
                    });
                }
            });
        if let Err(e) = spawned {
            // Nothing will serve the queue; leave it for the next request
            // to retry the spawn rather than claiming a worker exists.
            tracing::error!("input devices: could not spawn the enumeration worker: {e}");
            lock(&self.pending).running = false;
        }
    }
}

/// One enumeration pass. A panic in it (a device driver, a cpal bug) is
/// reported and treated as "no inputs" — the worker must survive it, or
/// `running` would stay set and no later request would be served.
fn run_enumerator(enumerate: &InputEnumerator) -> (Vec<InputDeviceInfo>, Option<String>) {
    let mut result = (Vec::new(), None);
    crate::supervise::run_supervised(
        "input-devices",
        || result = enumerate(),
        |message| tracing::error!("{message}"),
    );
    result
}

fn lock(pending: &Mutex<Pending>) -> std::sync::MutexGuard<'_, Pending> {
    pending.lock().unwrap_or_else(|e| e.into_inner())
}
