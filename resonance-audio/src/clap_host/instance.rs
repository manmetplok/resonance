//! `ClapInstance` is one running CLAP plugin instance. The struct lives
//! here; per-concern impl blocks are in sibling modules:
//! - parameter / note queues, transport latching, simple accessors are
//!   below in this file;
//! - GUI extension methods in [`super::gui`];
//! - state extension + reset in [`super::state`];
//! - audio-thread `process` / `process_multi` in [`super::process`].
//!
//! All struct fields are `pub(super)` so sibling impl blocks can reach
//! them without forcing every method through this file.

use std::ffi::CStr;
use std::pin::Pin;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::{clap_event_note, clap_event_param_value};
use clap_sys::ext::audio_ports::{clap_audio_port_info, clap_plugin_audio_ports};
use clap_sys::ext::gui::clap_plugin_gui;
use clap_sys::ext::latency::clap_plugin_latency;
use clap_sys::ext::params::clap_plugin_params;
use clap_sys::ext::state::clap_plugin_state;
use clap_sys::plugin::clap_plugin;

use crate::types::ParamInfo;

use super::HostData;

/// Mutable reference to one stereo output port's buffer. Used by
/// [`ClapInstance::process_multi`] to drive plugins that declare more than
/// one output port (e.g. `resonance-drums` with its per-group outputs).
/// For regular single-output plugins use the shorter [`ClapInstance::process`]
/// convenience wrapper instead.
pub struct StereoBufMut<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

pub struct ClapInstance {
    pub(super) plugin: *const clap_plugin,
    pub(super) host_data: Pin<Box<HostData>>,
    pub(super) active: bool,
    /// A failed (re)activation has been reported to the user and the
    /// instance has not been active since. Keeps a plugin that keeps
    /// requesting restarts from re-sending the same error (ENG-12).
    pub(super) restart_failure_reported: bool,
    pub(super) sample_rate: u32,
    pub(super) params_ext: Option<*const clap_plugin_params>,
    pub(super) state_ext: Option<*const clap_plugin_state>,
    pub(super) audio_ports_ext: Option<*const clap_plugin_audio_ports>,
    pub(super) gui_ext: Option<*const clap_plugin_gui>,
    /// The plugin's `clap.latency` extension, kept so the engine can
    /// re-query after a deactivate → reactivate cycle (doc #260
    /// finding #10). `None` when the plugin doesn't implement it.
    pub(super) latency_ext: Option<*const clap_plugin_latency>,
    /// The plugin's `com.resonance.param-flags` extension, when it is one
    /// of ours: which params its state leaves out. Read by
    /// [`ClapInstance::query_params`]; `None` for third-party plugins.
    pub(super) param_flags_ext: Option<*const resonance_common::param_flags::PluginParamFlags>,
    /// The plugin's `com.resonance.kit-info` extension, when it is one of
    /// our drum plugins: the pads of the kit it plays. Read by
    /// [`ClapInstance::poll_kit_info`]; `None` for every other plugin.
    pub(super) kit_info_ext: Option<*const resonance_common::kit_info::PluginKitInfo>,
    /// The kit info last returned by [`ClapInstance::poll_kit_info`], to
    /// report only a change; `None` until the first read.
    last_kit_info: Option<resonance_common::kit_info::KitInfo>,
    /// Whether [`ClapInstance::poll_kit_info`] has yet to run: the engine
    /// reads the kit info once after creating the instance.
    kit_info_unread: bool,
    /// The plugin's `clap.render` extension: told OFFLINE for the length
    /// of an offline render and REALTIME after
    /// ([`ClapInstance::set_render_mode`]). `None` when the plugin does
    /// not implement it.
    pub(super) render_ext: Option<*const clap_sys::ext::render::clap_plugin_render>,
    /// Whether the plugin was last told to render offline.
    pub(super) render_offline: bool,
    /// True when `gui_create` has been called and `gui_destroy` hasn't yet.
    pub(super) gui_open: bool,
    /// Number of output audio ports as reported by the plugin's audio-ports
    /// extension at activation time. Cached so the mixer can size its
    /// per-port scratch buffers without re-querying on every block.
    /// Always >= 1 because resonance-plugin rejects empty output layouts.
    pub(super) output_port_count: usize,
    /// Number of *input* audio ports the plugin declares. `1` is the
    /// ordinary effect; `2` means it also declares an external sidechain
    /// (key) port, which the host may connect via
    /// [`ClapInstance::process_multi_with_key`]. Instruments report 0.
    /// Cached at activation for the same reason as the output count — the
    /// audio thread must not re-query the extension per block.
    pub(super) input_port_count: usize,
    /// Processing latency in samples, as reported by the plugin's
    /// `clap.latency` extension right after activation (0 if the
    /// extension is absent). Refreshed on every deactivate → reactivate
    /// cycle ([`ClapInstance::restart`] / `reload_with_state`) because
    /// latency may only change while deactivated per the CLAP spec —
    /// and the built-in bridge serves an activation-time cached value
    /// while active (todo #1125).
    pub(super) latency: u32,
    /// Every param id with the value the host last read for it — by
    /// [`ClapInstance::query_params`] or [`ClapInstance::refresh_param_values`]
    /// — so a values rescan re-reads values only and formats only the ones
    /// that moved. Engine thread; the lock is never contended.
    pub(super) param_value_cache: parking_lot::Mutex<Vec<(u32, f64)>>,
    /// Output parameter events the plugin pushed during `process()` or
    /// `params.flush` (its own edits), raw, in order, until the engine
    /// thread folds them ([`ClapInstance::take_param_edits`]).
    /// Pre-allocated; the audio thread never grows it — a full buffer
    /// refuses the push, which a plugin retries later.
    pub(super) out_param_events: Vec<super::params::OutParamEvent>,
    /// Gestures the plugin has opened and not yet closed, with the last
    /// value each carried: a gesture may span many blocks. Engine thread.
    pub(super) open_gestures: Vec<(u32, Option<f64>)>,
    /// Pending parameter changes to send during next process() call.
    pub(super) pending_params: Vec<(u32, f64)>,
    /// Pre-allocated buffer for CLAP parameter events (reused across process() calls).
    pub(super) param_event_buf: Vec<clap_event_param_value>,
    /// Pending note events to send during next process() call.
    /// Each entry: (is_note_on, key, velocity, sample_offset)
    pub(super) pending_notes: Vec<(bool, u8, f32, u32)>,
    /// Note events a previous process() call could not deliver because
    /// their offset lay past that call's `frames_count` (a live note
    /// queued against the whole callback, reaching the head sub-block of
    /// a loop seam). Re-based to the next call's start and delivered
    /// there. Same layout as `pending_notes`. Deliberately untouched by
    /// [`ClapInstance::all_notes_off`]: the seam panics between its two
    /// sub-blocks, and a carried event belongs to the tail, after it.
    pub(super) carried_notes: Vec<(bool, u8, f32, u32)>,
    /// Pre-allocated buffer for CLAP note events (carried + pending, so
    /// sized for two full queues).
    pub(super) note_event_buf: Vec<clap_event_note>,
    /// Frames this instrument still wants processing for while the
    /// transport is stopped: re-armed to `limits::IDLE_HOLD_SECS` by
    /// every live note (so a preview / controller note's release tail
    /// plays out), counted down by `process()`. See
    /// [`ClapInstance::wants_idle_process`] (code review MIX-08).
    pub(super) idle_hold_frames: u32,
    /// Pre-allocated scratch for the CLAP audio output buffer array,
    /// one entry per output port. Reused across every `process_multi`
    /// call so the audio thread never allocates.
    pub(super) audio_out_buffers: Vec<clap_audio_buffer>,
    /// Per-port channel pointer array (2 pointers per port). Each block's
    /// `process_multi` call refreshes these to point at the caller's
    /// supplied slices before handing them to CLAP.
    pub(super) audio_out_ptrs: Vec<[*mut f32; 2]>,
    /// Latched transport state, set by the mixer before each process() call.
    pub(super) transport_bpm: f64,
    pub(super) transport_num: u16,
    pub(super) transport_den: u16,
    pub(super) transport_playing: bool,
    pub(super) transport_pos_beats: f64,
    pub(super) transport_valid: bool,
}

impl ClapInstance {
    /// Build the instance from the parts produced by
    /// [`super::ClapBundle::create_instance`]. Internal use only — kept
    /// in this module so the field invariants stay private.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_parts(
        plugin: *const clap_plugin,
        host_data: Pin<Box<HostData>>,
        sample_rate: u32,
        params_ext: Option<*const clap_plugin_params>,
        state_ext: Option<*const clap_plugin_state>,
        audio_ports_ext: Option<*const clap_plugin_audio_ports>,
        gui_ext: Option<*const clap_plugin_gui>,
        latency_ext: Option<*const clap_plugin_latency>,
        output_port_count: usize,
        input_port_count: usize,
        latency: u32,
        audio_out_buffers: Vec<clap_audio_buffer>,
        audio_out_ptrs: Vec<[*mut f32; 2]>,
    ) -> Self {
        Self {
            plugin,
            host_data,
            active: true,
            restart_failure_reported: false,
            sample_rate,
            params_ext,
            state_ext,
            audio_ports_ext,
            gui_ext,
            latency_ext,
            param_flags_ext: None,
            kit_info_ext: None,
            last_kit_info: None,
            kit_info_unread: true,
            render_ext: None,
            render_offline: false,
            gui_open: false,
            output_port_count,
            input_port_count,
            latency,
            // Pre-size every event buffer at activation so the first
            // process() call after a fresh plugin add doesn't allocate
            // on the audio thread.
            param_value_cache: parking_lot::Mutex::new(Vec::new()),
            out_param_events: Vec::with_capacity(super::params::OUT_PARAM_EVENT_CAPACITY),
            open_gestures: Vec::new(),
            pending_params: Vec::with_capacity(crate::limits::MAX_PENDING_PARAMS),
            param_event_buf: Vec::with_capacity(crate::limits::MAX_PENDING_PARAMS),
            pending_notes: Vec::with_capacity(crate::limits::MAX_PENDING_NOTES),
            carried_notes: Vec::with_capacity(crate::limits::MAX_PENDING_NOTES),
            note_event_buf: Vec::with_capacity(2 * crate::limits::MAX_PENDING_NOTES),
            idle_hold_frames: 0,
            audio_out_buffers,
            audio_out_ptrs,
            transport_bpm: 120.0,
            transport_num: 4,
            transport_den: 4,
            transport_playing: false,
            transport_pos_beats: 0.0,
            transport_valid: false,
        }
    }

    /// Number of output audio ports this plugin declares. Stable for the
    /// lifetime of the instance — use this to size per-port scratch
    /// buffers and (in the app layer) auto-create sub-tracks. Always >= 1.
    pub fn output_port_count(&self) -> usize {
        self.output_port_count
    }

    /// Processing latency in samples (`clap.latency` at the most recent
    /// activation; 0 if the plugin doesn't implement the extension).
    /// Refreshed whenever the instance cycles activation — see the
    /// field doc.
    pub fn latency_samples(&self) -> u32 {
        self.latency
    }

    /// Run the plugin's `on_main_thread` if it asked for it through
    /// `clap_host.request_callback()` since the last call. Engine thread,
    /// under the instance lock, before the other host-request flags are
    /// read — whatever the callback reports (`clap_host_gui.closed`,
    /// `clap_host_latency.changed`) is then picked up in the same poll.
    /// Whether the plugin has asked for `on_main_thread` since the last
    /// [`run_requested_callback`](Self::run_requested_callback).
    pub fn has_requested_callback(&self) -> bool {
        self.host_data
            .callback_requested
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn run_requested_callback(&mut self) {
        use std::sync::atomic::Ordering;
        if !self.host_data.callback_requested.swap(false, Ordering::AcqRel) {
            return;
        }
        // SAFETY: `plugin` is live for the instance's lifetime, and the
        // engine thread is the thread every other main-thread call runs on.
        unsafe {
            if let Some(on_main_thread) = (*self.plugin).on_main_thread {
                on_main_thread(self.plugin);
            }
        }
    }

    /// Consume the host-callback flags set by the plugin: returns true
    /// when it asked for a restart (`clap_host.request_restart`) and/or
    /// signalled a latency change (`clap_host_latency.changed`) since
    /// the last check. Both are honored the same way — the engine
    /// thread runs [`ClapInstance::restart`] at its next safe point,
    /// which re-reads the latency, then republishes PDC.
    pub fn take_host_restart_request(&self) -> bool {
        let (restart, latency) = self.take_host_restart_requests();
        restart || latency
    }

    /// [`Self::take_host_restart_request`], keeping the two flags apart:
    /// `(restart_requested, latency_changed)`. A latency change on a
    /// deactivated instance needs no action — it is read at the next
    /// activation — while a restart request does (FU-M1b).
    pub(crate) fn take_host_restart_requests(&self) -> (bool, bool) {
        use std::sync::atomic::Ordering;
        let restart = self.host_data.restart_requested.swap(false, Ordering::AcqRel);
        let latency = self.host_data.latency_changed.swap(false, Ordering::AcqRel);
        (restart, latency)
    }

    /// Re-read the plugin's reported latency. Only meaningful right
    /// after (re)activation: the CLAP spec defines `latency.get()` for
    /// active plugins, and the built-in bridge serves an
    /// activation-time cached value while active (todo #1125), so a
    /// query without an intervening deactivate → reactivate cycle
    /// returns the stale figure. Called from the activation-cycle paths
    /// in [`super::state`].
    pub(super) fn requery_latency(&mut self) {
        self.latency = unsafe {
            match self.latency_ext.and_then(|ext| (*ext).get) {
                Some(get_fn) => get_fn(self.plugin),
                None => 0,
            }
        };
        // A `changed()` fired during the activation we just completed is
        // captured by the query above; clear it so it doesn't schedule a
        // redundant restart cycle.
        self.host_data
            .latency_changed
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Human-readable name of each output port, as reported by the plugin's
    /// `clap.audio-ports` extension at activation time. Used by the app
    /// layer to name auto-created sub-tracks after their source port
    /// (e.g. "Kick", "Snare", "Overhead"). Falls back to "Out N" if the
    /// plugin doesn't implement the extension or returns empty names.
    pub fn output_port_names(&self) -> Vec<String> {
        let mut names = Vec::with_capacity(self.output_port_count);
        let ports_ext = self.audio_ports_ext;
        unsafe {
            let get_fn = ports_ext.and_then(|ports| (*ports).get);
            for i in 0..self.output_port_count {
                let name = get_fn.and_then(|f| {
                    let mut info = std::mem::MaybeUninit::<clap_audio_port_info>::zeroed();
                    let ok = f(self.plugin, i as u32, false, info.as_mut_ptr());
                    if !ok {
                        return None;
                    }
                    let info = info.assume_init();
                    let cstr = CStr::from_ptr(info.name.as_ptr());
                    let s = cstr.to_string_lossy().into_owned();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                });
                names.push(name.unwrap_or_else(|| format!("Out {}", i + 1)));
            }
        }
        names
    }

    /// The plugin's descriptor id (`com.vendor.plugin`), if it has one.
    /// Allocates; engine side.
    pub fn descriptor_id(&self) -> Option<String> {
        // SAFETY: `plugin` is live for the instance's lifetime, and a
        // CLAP descriptor and its strings outlive the plugin.
        unsafe {
            let desc = (*self.plugin).desc;
            if desc.is_null() || (*desc).id.is_null() {
                return None;
            }
            Some(std::ffi::CStr::from_ptr((*desc).id).to_string_lossy().into_owned())
        }
    }

    /// The id of the plugin's own bypass parameter, if it declares one
    /// (`CLAP_PARAM_IS_BYPASS`).
    ///
    /// A plugin that flags a parameter this way is telling the host "let
    /// me handle bypass myself" — it knows how to fade its own tail out
    /// and, crucially, it keeps reporting the same latency while
    /// bypassed, so the compensation table never has to move. The host
    /// therefore drives this parameter instead of skipping the slot (see
    /// [`super::PluginSlot`]).
    ///
    /// Deliberately *not* filtered by `CLAP_PARAM_IS_HIDDEN`, unlike
    /// [`Self::query_params`]: a bypass parameter is frequently hidden
    /// from the host's generic parameter list precisely because the host
    /// is expected to drive it from its own bypass control.
    ///
    /// Engine-thread call (it walks the plugin's parameter list); the
    /// result is cached in the plugin slot.
    pub fn bypass_param_id(&self) -> Option<u32> {
        let params = self.params_ext?;
        let count_fn = unsafe { (*params).count }?;
        let count = unsafe { count_fn(self.plugin) };
        let get_info = unsafe { (*params).get_info }?;
        for i in 0..count {
            let mut info =
                std::mem::MaybeUninit::<clap_sys::ext::params::clap_param_info>::uninit();
            if !unsafe { get_info(self.plugin, i, info.as_mut_ptr()) } {
                continue;
            }
            let info = unsafe { info.assume_init() };
            if info.flags & clap_sys::ext::params::CLAP_PARAM_IS_BYPASS != 0 {
                return Some(info.id);
            }
        }
        None
    }

    /// Query all parameters from the plugin. Called from the engine thread.
    pub fn query_params(&self) -> Vec<ParamInfo> {
        let params = match self.params_ext {
            Some(p) => p,
            None => return Vec::new(),
        };

        let count = unsafe {
            match (*params).count {
                Some(f) => f(self.plugin),
                None => return Vec::new(),
            }
        };

        let mut result = Vec::with_capacity(count as usize);

        for i in 0..count {
            let mut info =
                std::mem::MaybeUninit::<clap_sys::ext::params::clap_param_info>::uninit();
            let ok = unsafe {
                match (*params).get_info {
                    Some(f) => f(self.plugin, i, info.as_mut_ptr()),
                    None => continue,
                }
            };
            if !ok {
                continue;
            }
            let info = unsafe { info.assume_init() };

            // Get current value
            let mut current = info.default_value;
            if let Some(get_value) = unsafe { (*params).get_value } {
                unsafe { get_value(self.plugin, info.id, &mut current) };
            }

            // Convert name from c_char array
            let name = unsafe {
                CStr::from_ptr(info.name.as_ptr())
                    .to_string_lossy()
                    .to_string()
            };
            let module = unsafe {
                CStr::from_ptr(info.module.as_ptr())
                    .to_string_lossy()
                    .to_string()
            };

            // A hidden parameter is still automatable and still saved —
            // CLAP only asks that it not be *shown* — so it stays in the
            // list, flagged, and the readers that draw a parameter list
            // skip it (ba todo #1290). Dropping it here instead cost the
            // app its value on save and put it out of automation's
            // reach.
            let hidden = info.flags & clap_sys::ext::params::CLAP_PARAM_IS_HIDDEN != 0;
            let stepped = info.flags & clap_sys::ext::params::CLAP_PARAM_IS_STEPPED != 0;
            let automatable =
                info.flags & clap_sys::ext::params::CLAP_PARAM_IS_AUTOMATABLE != 0;
            let read_only = info.flags & clap_sys::ext::params::CLAP_PARAM_IS_READONLY != 0;
            // A read-only output is never the host's to persist either.
            let state_excluded = read_only || self.param_state_excluded(info.id);

            // What the plugin calls this value, and the unit taken off
            // it. `value_to_text` is the only place a unit exists in
            // CLAP — there is no separate field.
            let text = self.param_text(info.id, current).unwrap_or_default();
            let unit = super::param_meta::unit_from_text(&text).to_string();

            // A stepped parameter that names its steps is an
            // enumeration: ask the plugin for each label once, here,
            // rather than leaving every reader to probe for them.
            let choices = if stepped {
                super::param_meta::choice_labels(info.min_value, info.max_value, |v| {
                    self.param_text(info.id, v)
                })
                .unwrap_or_default()
            } else {
                Vec::new()
            };

            result.push(ParamInfo {
                id: info.id,
                name,
                min_value: info.min_value,
                max_value: info.max_value,
                default_value: info.default_value,
                current_value: current,
                text,
                unit,
                stepped,
                choices,
                module,
                hidden,
                automatable,
                read_only,
                state_excluded,
            });
        }

        // What the host now knows: the baseline a values rescan diffs
        // against.
        *self.param_value_cache.lock() = result.iter().map(|p| (p.id, p.current_value)).collect();
        result
    }

    /// The cheap re-read a values rescan asks for (CLAP
    /// `RESCAN_VALUES` / `RESCAN_TEXT`; review finding 7): every param's
    /// value through `get_value`, and the plugin's text only for those
    /// whose value moved since the host last read it — or for all, with
    /// `all_text`. Returns just those params.
    ///
    /// [`Self::query_params`] answers this too, but it walks `get_info`
    /// and, per stepped param, a `value_to_text` per step for its choice
    /// labels — under the instance lock the audio thread abandons a block
    /// rather than wait for. A plugin that reports a load progress every
    /// few percent asks for a rescan each time; a full query for each was
    /// an audible dropout. This walks `get_info` once per instance (the
    /// ids, if no query has listed them yet), and otherwise only
    /// `get_value`, which a CLAP plugin answers from an atomic.
    pub fn refresh_param_values(&self, all_text: bool) -> Vec<crate::types::ParamValueUpdate> {
        let Some(params) = self.params_ext else {
            return Vec::new();
        };
        let Some(get_value) = (unsafe { (*params).get_value }) else {
            return Vec::new();
        };
        let mut cache = self.param_value_cache.lock();
        if cache.is_empty() {
            *cache = self.param_ids().into_iter().map(|id| (id, f64::NAN)).collect();
        }
        let mut changed = Vec::new();
        for (id, seen) in cache.iter_mut() {
            let mut value = 0.0f64;
            // SAFETY: the vtable is the live plugin's; engine thread.
            if !unsafe { get_value(self.plugin, *id, &mut value) } || !value.is_finite() {
                continue;
            }
            if !all_text && value.to_bits() == seen.to_bits() {
                continue;
            }
            *seen = value;
            changed.push(crate::types::ParamValueUpdate {
                id: *id,
                value,
                text: self.param_text(*id, value).unwrap_or_default(),
            });
        }
        changed
    }

    /// Every param id, in the plugin's order: a `get_info` walk, nothing
    /// formatted.
    fn param_ids(&self) -> Vec<u32> {
        let Some(params) = self.params_ext else {
            return Vec::new();
        };
        let (Some(count_fn), Some(get_info)) = (unsafe { (*params).count }, unsafe {
            (*params).get_info
        }) else {
            return Vec::new();
        };
        let count = unsafe { count_fn(self.plugin) };
        (0..count)
            .filter_map(|i| {
                let mut info =
                    std::mem::MaybeUninit::<clap_sys::ext::params::clap_param_info>::uninit();
                // SAFETY: as in `query_params`.
                unsafe { get_info(self.plugin, i, info.as_mut_ptr()) }
                    .then(|| unsafe { info.assume_init() }.id)
            })
            .collect()
    }

    /// Tell the plugin whether it is rendering offline (CLAP `render.set`):
    /// `true` for a bounce, an export, a freeze — no realtime deadline, so
    /// a plugin that cuts corners for time (a streaming sampler dropping a
    /// late disk read) must not — and `false` once the render is over.
    /// Returns whether the plugin took the mode; `false` without the
    /// extension, or when it is already in that mode (nothing sent).
    ///
    /// `[main-thread]` in CLAP: call it outside any audio-thread role,
    /// holding the instance lock (the offline renderers do, before their
    /// first block and after their last).
    pub fn set_render_mode(&mut self, offline: bool) -> bool {
        use clap_sys::ext::render::{CLAP_RENDER_OFFLINE, CLAP_RENDER_REALTIME};
        if self.render_offline == offline {
            return false;
        }
        let Some(set) = self.render_ext.and_then(|ext| unsafe { (*ext).set }) else {
            return false;
        };
        let mode = if offline {
            CLAP_RENDER_OFFLINE
        } else {
            CLAP_RENDER_REALTIME
        };
        // SAFETY: the vtable is the live plugin's; the caller holds the
        // instance exclusively.
        let accepted = unsafe { set(self.plugin, mode) };
        if accepted {
            self.render_offline = offline;
        }
        accepted
    }

    /// Whether the plugin was last told to render offline.
    pub fn render_offline(&self) -> bool {
        self.render_offline
    }

    /// The pads of the kit a drum plugin plays (`com.resonance.kit-info`),
    /// when they changed since the last call — the first call reports
    /// whatever is there. `None` when nothing changed, and always for a
    /// plugin without the extension. `[main-thread]`: the engine calls it
    /// once after creating the instance and after every params rescan the
    /// plugin asks for, which is when the extension's contract says the
    /// pads may have moved.
    pub fn poll_kit_info(&mut self) -> Option<resonance_common::kit_info::KitInfo> {
        self.kit_info_unread = false;
        let ext = self.kit_info_ext?;
        // SAFETY: the vtable is the plugin's, live for the instance's
        // lifetime; this is the main thread.
        let get = unsafe { (*ext).get }?;
        let plugin = self.plugin as *const std::ffi::c_void;
        let mut buf = vec![0u8; 4096];
        // SAFETY: `buf` holds `buf.len()` writable bytes.
        let mut len = unsafe { get(plugin, buf.as_mut_ptr(), buf.len()) };
        if len > buf.len() {
            buf.resize(len, 0);
            // SAFETY: as above, with the size the plugin asked for.
            len = unsafe { get(plugin, buf.as_mut_ptr(), buf.len()) };
            if len > buf.len() {
                return None;
            }
        }
        let info = resonance_common::kit_info::KitInfo::parse(&buf[..len])?;
        if self.last_kit_info.as_ref() == Some(&info) {
            return None;
        }
        self.last_kit_info = Some(info.clone());
        Some(info)
    }

    /// Whether the engine should read the kit info now even without a
    /// params rescan: the instance has the extension and was never read.
    pub fn kit_info_unread(&self) -> bool {
        self.kit_info_ext.is_some() && self.kit_info_unread
    }

    /// Whether the plugin's state leaves `param_id` out
    /// (`com.resonance.param-flags`): `false` for a plugin without the
    /// extension, i.e. every third-party one. Any thread; the answer is
    /// fixed for the instance's lifetime.
    pub fn param_state_excluded(&self, param_id: u32) -> bool {
        let Some(ext) = self.param_flags_ext else {
            return false;
        };
        // SAFETY: the vtable is the plugin's, live for the instance's
        // lifetime; the call is `[thread-safe]`.
        unsafe {
            match (*ext).is_state_excluded {
                Some(f) => f(self.plugin as *const std::ffi::c_void, param_id),
                None => false,
            }
        }
    }

    /// One parameter's `min..=max`, without touching its formatting
    /// (ba todo #1290).
    ///
    /// [`query_params`](Self::query_params) answers this too, but it now
    /// carries a parameter's whole meaning — a `value_to_text` call per
    /// parameter plus a choice-label walk per stepped one — and a caller
    /// that only wants a range would pay all of it and discard it. The
    /// automation snapshot resolves a lane's range while holding the
    /// instance lock that the audio thread abandons a block rather than
    /// wait for, and a breakpoint drag re-resolves per event, so on an
    /// 87-parameter plugin that is a stutter mechanism.
    ///
    /// This walks `get_info` only, allocates nothing, and stops at the
    /// id. Engine/main thread, like the rest of the params extension.
    pub fn param_range(&self, param_id: u32) -> Option<(f64, f64)> {
        let params = self.params_ext?;
        let count_fn = unsafe { (*params).count }?;
        let get_info = unsafe { (*params).get_info }?;
        let count = unsafe { count_fn(self.plugin) };

        for i in 0..count {
            let mut info =
                std::mem::MaybeUninit::<clap_sys::ext::params::clap_param_info>::uninit();
            let ok = unsafe { get_info(self.plugin, i, info.as_mut_ptr()) };
            if !ok {
                continue;
            }
            let info = unsafe { info.assume_init() };
            if info.id == param_id {
                return Some((info.min_value, info.max_value));
            }
        }
        None
    }

    /// The plugin's own rendering of `value` for one parameter — `"40 %"`,
    /// `"-6.0 dB"`, `"Low-pass"` — or `None` when it offers no
    /// conversion (ba todo #1290).
    ///
    /// This is CLAP `value_to_text`, and it is the *only* way to learn
    /// what a number means to the plugin: 100+ `with_value_to_string`
    /// call sites across our own fleet were reachable from third-party
    /// hosts and from nothing in Resonance until this call existed.
    ///
    /// Takes any value, not just the current one, so a caller can ask
    /// "what would this read as" — which is how choice labels are
    /// collected and how the engine echoes fresh text after a write.
    /// Main/engine thread only, like the rest of the params extension.
    pub fn param_text(&self, param_id: u32, value: f64) -> Option<String> {
        let params = self.params_ext?;
        let value_to_text = unsafe { (*params).value_to_text }?;

        // CLAP writes a NUL-terminated string into a caller-owned
        // buffer; 256 bytes is what hosts conventionally offer and far
        // more than a formatted parameter needs.
        let mut buf = [0u8; 256];
        let ok = unsafe {
            value_to_text(
                self.plugin,
                param_id,
                value,
                buf.as_mut_ptr() as *mut std::ffi::c_char,
                buf.len() as u32,
            )
        };
        if !ok {
            return None;
        }
        // Defend against a plugin that fills the buffer without
        // terminating it: bound the scan at the buffer, don't run off it.
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }

    /// Ask the plugin which value `text` names for `param_id` (CLAP
    /// `text_to_value`) — the mirror of [`Self::param_text`]. This is how a
    /// label that is not one of a parameter's enumerated `choices` still
    /// resolves: a model name on the amp's 1000-slot selector, `"-6 dB"`
    /// on a gain (nam-model-library.md §9.2). `None` when the plugin has
    /// no params extension, no `text_to_value`, or does not accept the
    /// text. Main/engine thread only, like the rest of the params
    /// extension.
    pub fn param_from_text(&self, param_id: u32, text: &str) -> Option<f64> {
        let params = self.params_ext?;
        let text_to_value = unsafe { (*params).text_to_value }?;
        // An interior NUL cannot cross the C boundary; nothing a caller
        // means contains one.
        let c_text = std::ffi::CString::new(text).ok()?;
        let mut value = 0.0f64;
        let ok = unsafe { text_to_value(self.plugin, param_id, c_text.as_ptr(), &mut value) };
        (ok && value.is_finite()).then_some(value)
    }

    /// Queue a parameter change to be sent during the next process() call.
    /// Deduplicates by param_id (last value wins) and caps at 128 entries
    /// to prevent unbounded growth when the GUI automates many parameters
    /// between process calls.
    pub fn set_param(&mut self, param_id: u32, value: f64) {
        if let Some(existing) = self
            .pending_params
            .iter_mut()
            .find(|(id, _)| *id == param_id)
        {
            existing.1 = value;
        } else if self.pending_params.len() < crate::limits::MAX_PENDING_PARAMS {
            self.pending_params.push((param_id, value));
        }
    }

    /// Queue a note-on event to be sent during the next process() call.
    /// Dropped when the queue is full — a lost note-on is a missed note,
    /// never a stuck one.
    pub fn queue_note_on(&mut self, key: u8, velocity: f32, sample_offset: u32) {
        if self.pending_notes.len() < crate::limits::MAX_PENDING_NOTES {
            self.pending_notes
                .push((true, key, velocity, sample_offset));
        }
    }

    /// Queue a note-off event to be sent during the next process() call.
    ///
    /// A note-off is never the one dropped at the cap (code review
    /// MIX-08): a full queue evicts its oldest queued note-on instead,
    /// because a dropped note-off leaves a voice sounding forever while a
    /// dropped note-on only loses a note. Only a queue holding nothing but
    /// note-offs drops this one — every key it could release is already
    /// being released. Allocation-free (`Vec::remove` shifts in place).
    pub fn queue_note_off(&mut self, key: u8, sample_offset: u32) {
        if self.pending_notes.len() >= crate::limits::MAX_PENDING_NOTES {
            let Some(oldest_on) = self.pending_notes.iter().position(|n| n.0) else {
                return;
            };
            self.pending_notes.remove(oldest_on);
        }
        self.pending_notes.push((false, key, 0.0, sample_offset));
    }

    /// Re-arm the stopped-transport processing window (see
    /// [`ClapInstance::wants_idle_process`]) for a LIVE note — a
    /// piano-roll preview or a controller key, delivered through
    /// `NoteSink`. Timeline and offline-render notes deliberately don't
    /// arm it: stopping (or finishing an export) must not leave the
    /// song's last voices ringing out of the speakers.
    pub fn arm_idle_hold(&mut self) {
        self.idle_hold_frames = self.sample_rate.saturating_mul(crate::limits::IDLE_HOLD_SECS);
    }

    /// Consume a pending `clap_host.request_process()` (see
    /// `HostData::process_requested`) and, on an active instance, re-arm
    /// the stopped-transport window with it: each request buys
    /// `limits::IDLE_HOLD_SECS` of blocks, and a plugin that needs more
    /// asks again. Returns whether the window was armed. A request on an
    /// inactive instance is consumed and dropped — `process()` would not
    /// run it, so a hold armed now would never count down.
    ///
    /// Audio thread, under the instance lock: one atomic swap, no
    /// allocation. Consuming only under the lock means a block that finds
    /// the slot contended leaves the request for the next one.
    pub fn take_process_request(&mut self) -> bool {
        use std::sync::atomic::Ordering;
        if !self.host_data.process_requested.swap(false, Ordering::AcqRel) {
            return false;
        }
        if !self.active {
            return false;
        }
        self.arm_idle_hold();
        true
    }

    /// Whether this instrument should be processed although the transport
    /// is stopped and nothing monitors its track (code review MIX-08):
    /// note events are waiting, or a live note arrived within the last
    /// `limits::IDLE_HOLD_SECS`, so its voices are still releasing.
    /// Piano-roll preview notes and live MIDI played while stopped used
    /// to sit in the queue unheard, pile up to the cap, and burst on the
    /// next Play.
    /// The hold bounds the cost: an idle instrument stops being processed
    /// a few seconds after its last note.
    pub fn wants_idle_process(&self) -> bool {
        self.idle_hold_frames > 0
            || !self.pending_notes.is_empty()
            || !self.carried_notes.is_empty()
    }

    /// Queue note-off for all 128 MIDI notes (to clear stuck notes).
    /// Clears everything already queued first: a pending note-on must
    /// not fire after a panic (its later sample offset would outlive
    /// the offs), and pending note-offs are superseded by the full
    /// sweep below. Clearing also guarantees all 128 offs always fit
    /// without reallocating `pending_notes` on the audio thread
    /// (e.g. loop-seam panic during a live MIDI burst). Events carried
    /// past an earlier sub-block (`carried_notes`) are kept: they are
    /// timed after the seam this panic marks.
    pub fn all_notes_off(&mut self) {
        const _: () = assert!(crate::limits::MAX_PENDING_NOTES >= 128);
        self.pending_notes.clear();
        for key in 0..=127u8 {
            self.pending_notes.push((false, key, 0.0, 0));
        }
    }

    /// [`Self::all_notes_off`] for a transport Stop / relocate panic
    /// (FU-F2a): also drops the events carried past an earlier
    /// sub-block. Those are timed after a *seam*, which is why the seam
    /// panic keeps them; after a Stop or a seek there is no "after" for
    /// them to belong to, and a carried note-on delivered behind the
    /// 128 offs would hang. Allocation-free.
    pub fn all_notes_off_and_drop_carried(&mut self) {
        self.carried_notes.clear();
        self.all_notes_off();
    }

    /// Read-only view of the pending note queue. Test surface only —
    /// see `tests/clap_host/clap_all_notes_off.rs`.
    #[doc(hidden)]
    pub fn __pending_notes_for_test(&self) -> &[(bool, u8, f32, u32)] {
        &self.pending_notes
    }

    /// CLAP `clap_plugin.reset()`: clear every buffer and kill every
    /// voice (reverb / delay tails, envelopes, filter state) without
    /// touching parameter values (code review ENG-04). Also drops the
    /// host-side note queues, so a note queued by live playback can't
    /// fire into whatever runs next. `[audio-thread & active]`: the
    /// caller holds the instance mutex, so no `process()` is in flight;
    /// the offline renderers call it as their audio thread before each
    /// pass. A no-op on an inactive instance.
    pub fn reset(&mut self) {
        if !self.active {
            return;
        }
        self.pending_notes.clear();
        self.carried_notes.clear();
        self.idle_hold_frames = 0;
        // SAFETY: `self.plugin` is the live plugin this instance owns;
        // `active` holds, and `&mut self` means no process() runs.
        if let Some(reset) = unsafe { (*self.plugin).reset } {
            // `[audio-thread]` in CLAP; see `AudioThreadScope`.
            let _audio = super::thread_check::AudioThreadScope::enter();
            unsafe { reset(self.plugin) };
        }
    }

    /// Latch the current transport state so the next process() call can
    /// forward it to the plugin via `clap_event_transport`.
    pub fn set_transport(&mut self, bpm: f64, num: u16, den: u16, playing: bool, pos_beats: f64) {
        self.transport_bpm = bpm;
        self.transport_num = num;
        self.transport_den = den;
        self.transport_playing = playing;
        self.transport_pos_beats = pos_beats;
        self.transport_valid = true;
    }
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
        // Tear down any open GUI first so the plugin can release its editor
        // thread before the rest of the plugin goes away.
        let _ = self.close_gui();
        if self.active {
            if let Some(stop) = unsafe { (*self.plugin).stop_processing } {
                // `[audio-thread]` in CLAP; see `AudioThreadScope`.
                let _audio = super::thread_check::AudioThreadScope::enter();
                unsafe { stop(self.plugin) };
            }
            if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
                unsafe { deactivate(self.plugin) };
            }
        }
        if let Some(destroy) = unsafe { (*self.plugin).destroy } {
            unsafe { destroy(self.plugin) };
        }
    }
}
