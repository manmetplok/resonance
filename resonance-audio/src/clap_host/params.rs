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
use super::process::{mixed_events_get, mixed_events_size, MixedEventListCtx};

/// Room for the output parameter events of a poll interval: three per
/// announced edit (begin, value, end) for well over a hundred edits.
pub(super) const OUT_PARAM_EVENT_CAPACITY: usize = 512;

/// One output parameter event as the plugin pushed it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OutParamEvent {
    pub kind: OutParamKind,
    pub param_id: u32,
    pub value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutParamKind {
    GestureBegin,
    Value,
    GestureEnd,
}

/// `clap_output_events.try_push` for `process()` and `params.flush`:
/// keeps the parameter events (value, gesture begin / end) in the
/// instance's pre-allocated buffer and accepts — drops — everything else,
/// as before. Allocation-free: a full buffer refuses the event (`false`),
/// which tells the plugin to report it again later.
///
/// # Safety
/// `list.ctx` is a `*mut Vec<OutParamEvent>` valid for the call, and
/// `event` a valid CLAP event header.
pub(super) unsafe extern "C" fn collect_output_event(
    list: *const clap_output_events,
    event: *const clap_event_header,
) -> bool {
    use clap_sys::events::{
        clap_event_param_gesture, CLAP_EVENT_PARAM_GESTURE_BEGIN, CLAP_EVENT_PARAM_GESTURE_END,
    };
    if list.is_null() || event.is_null() || (*list).ctx.is_null() {
        return true;
    }
    let header = &*event;
    if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
        return true;
    }
    let parsed = match header.type_ {
        CLAP_EVENT_PARAM_VALUE
            if header.size as usize >= std::mem::size_of::<clap_event_param_value>() =>
        {
            let e = &*(event as *const clap_event_param_value);
            OutParamEvent {
                kind: OutParamKind::Value,
                param_id: e.param_id,
                value: e.value,
            }
        }
        CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END
            if header.size as usize >= std::mem::size_of::<clap_event_param_gesture>() =>
        {
            let e = &*(event as *const clap_event_param_gesture);
            OutParamEvent {
                kind: if header.type_ == CLAP_EVENT_PARAM_GESTURE_BEGIN {
                    OutParamKind::GestureBegin
                } else {
                    OutParamKind::GestureEnd
                },
                param_id: e.param_id,
                value: 0.0,
            }
        }
        _ => return true,
    };
    let buf = &mut *((*list).ctx as *mut Vec<OutParamEvent>);
    if buf.len() >= buf.capacity() {
        return false;
    }
    buf.push(parsed);
    true
}

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
        self.flush_params_now()
    }

    /// Run the `params.flush` the plugin asked for
    /// (`clap_host_params.request_flush`), if it did: that is how a plugin
    /// with its transport stopped — no `process()` coming — delivers the
    /// output events of a param it changed itself. Carries any queued host
    /// changes too. Engine thread, under the instance lock.
    pub fn service_flush_request(&mut self) {
        if self
            .host_data
            .flush_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            self.flush_params_now();
        }
    }

    /// Bring what the plugin reports in step with what it plays, before
    /// the host reads it back — `state.save`, `get_value` (code review
    /// HOST-01).
    ///
    /// A plugin's editor writes its params directly, and a plugin that
    /// mirrors its values for the main thread (every Resonance plugin
    /// does, while active) refreshes that mirror only at a `process()` or
    /// a `params.flush`. With the transport stopped no `process()` runs,
    /// so this calls `params.flush` — carrying any queued host changes,
    /// and collecting what the plugin announces for the next
    /// [`Self::take_param_edits`]. A no-op for a plugin without one.
    ///
    /// Engine thread, under the instance lock (see the module doc).
    pub fn sync_plugin_values(&mut self) {
        self.flush_params_now();
    }

    /// Fold the output parameter events the plugin pushed since the last
    /// call into edits: a gesture is reported once, when it ends, with the
    /// last value it carried; a value outside any gesture is reported as
    /// it is (the last one per param, if several). Each comes with the
    /// plugin's text for it. Engine thread.
    pub fn take_param_edits(&mut self) -> Vec<crate::types::PluginParamEdit> {
        let mut edits: Vec<crate::types::PluginParamEdit> = Vec::new();
        let raw = std::mem::take(&mut self.out_param_events);
        for event in &raw {
            let id = event.param_id;
            let open = self.open_gestures.iter().position(|(g, _)| *g == id);
            match (event.kind, open) {
                (OutParamKind::GestureBegin, None) => self.open_gestures.push((id, None)),
                (OutParamKind::GestureBegin, Some(_)) => {}
                (OutParamKind::Value, Some(i)) => self.open_gestures[i].1 = Some(event.value),
                (OutParamKind::Value, None) => {
                    edits.retain(|e| e.gesture || e.param_id != id);
                    edits.push(crate::types::PluginParamEdit {
                        param_id: id,
                        value: event.value,
                        text: String::new(),
                        gesture: false,
                    });
                }
                (OutParamKind::GestureEnd, Some(i)) => {
                    let (_, last) = self.open_gestures.swap_remove(i);
                    if let Some(value) = last {
                        edits.push(crate::types::PluginParamEdit {
                            param_id: id,
                            value,
                            text: String::new(),
                            gesture: true,
                        });
                    }
                }
                (OutParamKind::GestureEnd, None) => {}
            }
        }
        // Hand the buffer back, emptied, with its capacity: the audio
        // thread must never be the one to allocate it.
        self.out_param_events = raw;
        self.out_param_events.clear();
        for edit in &mut edits {
            edit.text = self.param_text(edit.param_id, edit.value).unwrap_or_default();
        }
        edits
    }

    /// `clap_plugin_params.flush` with whatever is queued (possibly
    /// nothing), collecting the plugin's output parameter events.
    fn flush_params_now(&mut self) -> bool {
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
            ctx: &mut self.out_param_events as *mut Vec<OutParamEvent> as *mut c_void,
            try_push: Some(collect_output_event),
        };

        {
            // `params.flush` is `[active ? audio-thread : main-thread]`;
            // see `AudioThreadScope`.
            let _audio = self
                .active
                .then(super::thread_check::AudioThreadScope::enter);
            unsafe { flush_fn(self.plugin, &in_events, &out_events) };
        }

        // Reclaim the scratch buffer for reuse (keeps process() allocation-free).
        self.param_event_buf = event_ctx.param_events;
        self.param_event_buf.clear();
        true
    }
}
