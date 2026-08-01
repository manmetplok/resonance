//! Out-of-process-cycle parameter delivery: the CLAP
//! `clap_plugin_params.flush` entry point.
//!
//! [`ClapInstance::set_param`] only *queues* into `pending_params`; the
//! queue is normally drained inside [`ClapInstance::process_multi`].
//! That is fine while the transport rolls, but the mixer skips the whole
//! arrangement render when the transport is stopped (`mixer::mod`'s
//! `!shared.playing` branch), so a parameter set while stopped would sit
//! in the queue indefinitely — the plugin's DSP keeps the old value and
//! a `save_state()` taken in that window serialises the old value too.
//!
//! CLAP provides `clap_plugin_params.flush(plugin, in, out)` for exactly
//! this case. [`ClapInstance::flush_pending_params`] builds the same
//! param-value event list `process()` would have built and hands it
//! straight to the plugin.
//!
//! # Threading
//!
//! The CLAP contract for `flush` is `[active ? audio-thread :
//! main-thread]`, whose actual requirement is spelled out in the header:
//! *"This method must not be called concurrently to
//! `clap_plugin->process()`"*.
//!
//! This host guarantees that structurally rather than by thread
//! identity. Every [`ClapInstance`] lives inside a
//! `Mutex<SyncClapInstance>` (see [`super::SyncClapInstance`]) and
//! `&mut ClapInstance` is only reachable through that mutex's guard.
//! Every `process()` / `process_multi()` call site takes the same lock
//! first (`mixer::render_core::lock_fx` / `lock_instrument`,
//! `mixer::master`, `mixer::monitor`, `mixer::common`), so holding a
//! `&mut ClapInstance` *is* the proof that no `process()` for this
//! instance is in flight — on the audio thread or on a bounce thread.
//! Taking `&mut self` here therefore inherits the same exclusion the
//! `[audio-thread]` annotation is there to provide.
//!
//! When the plugin is inactive (`self.active == false`, i.e. inside a
//! [`ClapInstance::restart`] / `reload_with_state` activation cycle)
//! CLAP wants `flush` on the main thread; those cycles run on the engine
//! control thread — this host's CLAP main thread, which is also the only
//! caller of `flush_pending_params` — and no `process()` can run at all
//! while deactivated.

use std::ffi::c_void;
use std::ptr;

use clap_sys::events::{
    clap_event_header, clap_event_param_value, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
};

use super::instance::ClapInstance;
use super::process::{
    discard_output_event, mixed_events_get, mixed_events_size, MixedEventListCtx,
};

/// Build one `CLAP_EVENT_PARAM_VALUE` event at time 0. Shared by the
/// `process()` fast path and the flush path so both deliver parameter
/// changes in exactly the same shape.
#[inline]
pub(super) fn param_value_event(param_id: u32, value: f64) -> clap_event_param_value {
    clap_event_param_value {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_param_value>() as u32,
            time: 0,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            type_: CLAP_EVENT_PARAM_VALUE,
            flags: 0,
        },
        param_id,
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    }
}

impl ClapInstance {
    /// True when [`ClapInstance::set_param`] has queued changes the
    /// plugin has not seen yet.
    pub fn has_pending_params(&self) -> bool {
        !self.pending_params.is_empty()
    }

    /// Deliver every queued parameter change to the plugin *now* via
    /// `clap_plugin_params.flush`, instead of waiting for the next
    /// `process()` call.
    ///
    /// Callers must hold the instance's `Mutex<SyncClapInstance>` — see
    /// the module-level threading note; `&mut self` is only reachable
    /// through it, which is what excludes a concurrent `process()`.
    ///
    /// Returns `true` when the queue was handed to the plugin (or was
    /// already empty). Returns `false` when the plugin implements no
    /// usable `params.flush`, in which case the queue is left **intact**
    /// so the next `process()` still applies it. The queue is drained
    /// only on the success path, so a change is never both flushed and
    /// replayed.
    pub fn flush_pending_params(&mut self) -> bool {
        if self.pending_params.is_empty() {
            return true;
        }
        let Some(params) = self.params_ext else {
            return false;
        };
        let Some(flush_fn) = (unsafe { (*params).flush }) else {
            return false;
        };

        // Same pre-allocated scratch the process() path uses; we hold the
        // instance exclusively, so there is no aliasing with it.
        self.param_event_buf.clear();
        self.param_event_buf.extend(
            self.pending_params
                .drain(..)
                .map(|(param_id, value)| param_value_event(param_id, value)),
        );

        let mut event_ctx = MixedEventListCtx {
            param_events: std::mem::take(&mut self.param_event_buf),
            note_events: Vec::new(),
        };

        let in_events = clap_input_events {
            ctx: &mut event_ctx as *mut MixedEventListCtx as *mut c_void,
            size: Some(mixed_events_size),
            get: Some(mixed_events_get),
        };
        let out_events = clap_output_events {
            ctx: ptr::null_mut(),
            try_push: Some(discard_output_event),
        };

        unsafe { flush_fn(self.plugin, &in_events, &out_events) };

        // Reclaim the scratch buffer for reuse (keeps process() allocation-free).
        self.param_event_buf = event_ctx.param_events;
        self.param_event_buf.clear();
        true
    }
}
