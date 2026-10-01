//! Minimal CLAP plugin host. Loads `.clap` shared libraries, instantiates
//! plugins, and runs them through the audio callback.
//!
//! Submodules by concern:
//! - [`bundle`]: load a `.clap` library, walk its plugin factory.
//! - [`instance`]: per-plugin lifecycle, parameter / note queues,
//!   transport latching, the [`StereoBufMut`] borrow type used to
//!   pass per-port output slices into the audio thread.
//! - [`process`]: the audio-thread fast path ([`ClapInstance::process`]
//!   single-output wrapper and [`ClapInstance::process_multi`]).
//! - [`param_meta`]: what a parameter *means* — its unit, its choice
//!   labels — read back out of the plugin's own formatter.
//! - [`params`]: `clap_plugin_params.flush` — delivers queued parameter
//!   changes when no `process()` call is coming (transport stopped).
//! - [`state`]: CLAP state extension (save / load / reload / reset).
//! - [`gui`]: CLAP GUI extension (open / close the editor window).
//! - host callbacks (in this file): the `clap_host` vtable we hand back
//!   to plugins. Mostly no-ops, with two exceptions serviced by the
//!   engine thread (doc #260 finding #10): `clap_host_latency.changed()`
//!   and `request_restart()` both flag the instance so the engine
//!   deactivates → reactivates it at the next safe point and re-reads
//!   its latency. `clap_host_gui.closed()` (ba todo #1347) is latched
//!   the same way: the engine's poll finishes the teardown and reports
//!   `AudioEvent::PluginEditorState` so a window the user closed from
//!   its own titlebar stops reading as open.

mod bundle;
pub mod discovery;
mod gui;
mod instance;
mod param_meta;
mod params;
mod process;
mod preset_state;
mod state;
pub(crate) mod thread_check;
mod thread_pool;

pub use bundle::ClapBundle;
pub use bundle::bundle_binary_path;
pub use bundle::ClapBundleError;
pub use instance::{ClapInstance, StereoBufMut};
pub use bundle::DiscoveryFactory;
pub use preset_state::PresetHostReport;

/// What a plugin's params rescan asks the host to re-read
/// ([`ClapInstance::take_params_refresh`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamsRefresh {
    /// Nothing pending.
    None,
    /// Values moved (CLAP `RESCAN_VALUES`): re-read every value and the
    /// text of each that moved — or of every one, when `all_text` (CLAP
    /// `RESCAN_TEXT`: the plugin's formatting itself changed). See
    /// [`ClapInstance::refresh_param_values`].
    Values { all_text: bool },
    /// Anything may have changed (`RESCAN_INFO` / `ALL`, or a load's
    /// second look): the full [`ClapInstance::query_params`].
    Full,
}
pub use param_meta::{choice_labels, label_round_trips, unit_from_text, MAX_CHOICE_STEPS};

use std::ffi::{c_char, c_void, CStr};
use std::pin::Pin;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicUsize, Ordering};

use clap_sys::ext::gui::{clap_host_gui, CLAP_EXT_GUI};
use clap_sys::ext::latency::{clap_host_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::thread_check::CLAP_EXT_THREAD_CHECK;
use clap_sys::ext::thread_pool::CLAP_EXT_THREAD_POOL;
use clap_sys::host::clap_host;
use clap_sys::version::CLAP_VERSION;
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::bypass::{BypassFade, FadeStage};
use crate::types::PluginInstanceId;

/// `min_frames_count` every plugin is activated with, at creation and on
/// every re-activation. 1, because that is what the host really sends:
/// the live callback splits a buffer that crosses a loop seam into head
/// and tail sub-blocks of any length (ENG-10). CLAP requires
/// `min_frames_count <= frames_count` on every `process()`.
pub(super) const ACTIVATE_MIN_FRAMES: u32 = 1;
/// `max_frames_count` every plugin is activated with.
pub(super) const ACTIVATE_MAX_FRAMES: u32 = 8192;

// ---------------------------------------------------------------------------
// Host callbacks
// ---------------------------------------------------------------------------

pub(super) struct HostData {
    pub clap_host: clap_host,
    /// `clap_host_latency` vtable served from `host_get_extension`.
    /// Lives inside the pinned `HostData` so the pointer we hand the
    /// plugin stays stable for the instance's lifetime.
    pub latency_ext: clap_host_latency,
    /// Set by `clap_host_latency.changed()`: the plugin's reported
    /// latency is out of date. Because the bridge serves an
    /// activation-time cached value while active (todo #1125), acting
    /// on this means a deactivate → reactivate cycle, same as a
    /// restart request. Consumed by
    /// [`ClapInstance::take_host_restart_request`].
    pub latency_changed: AtomicBool,
    /// Set by `clap_host.request_restart()`: the plugin asks for a
    /// deactivate → reactivate cycle. Consumed by
    /// [`ClapInstance::take_host_restart_request`].
    pub restart_requested: AtomicBool,
    /// Set by `clap_host.request_callback()`: the plugin wants its
    /// `on_main_thread` run. Consumed by
    /// [`ClapInstance::run_requested_callback`] on the engine thread,
    /// which is the CLAP main thread for our instances. The built-in
    /// bridge relies on it to deliver `clap_host_gui.closed()` for an
    /// editor the user closed from its own titlebar (PLG-01).
    pub callback_requested: AtomicBool,
    /// `clap_host_gui` vtable served from `host_get_extension`. Lives
    /// inside the pinned `HostData` for the same reason as
    /// `latency_ext`: the pointer we hand the plugin must stay valid
    /// for the instance's lifetime.
    pub gui_ext: clap_host_gui,
    /// Set by `clap_host_gui.closed()`: the plugin's editor window went
    /// away without the host asking — in practice the user closed the
    /// floating window from its own titlebar (ba todo #1347). Consumed
    /// by [`ClapInstance::take_gui_closed`] on the engine thread, which
    /// turns it into `AudioEvent::PluginEditorState`.
    gui_closed: AtomicBool,
    /// Companion to `gui_closed`: the `was_destroyed` argument of the
    /// notification.
    ///
    /// INFORMATIONAL ONLY — it does not decide the teardown, and an
    /// earlier version of this comment said the opposite. `clap/ext/gui.h`
    /// on `clap_host_gui.closed`: "If was_destroyed is true, then the host
    /// must call clap_plugin_gui->destroy() to acknowledge the gui
    /// destruction." The flag describes the WINDOW, not the plugin's gui
    /// object, which the plugin never frees itself. So the host calls
    /// `destroy` either way (ba todo #1347).
    ///
    /// Written before `gui_closed` is set and read after it is taken, so
    /// the release/acquire pair on `gui_closed` publishes it.
    gui_closed_was_destroyed: AtomicBool,
    /// The instance this host serves, from the moment it is created — the
    /// `thread-pool` extension hands it back to the plugin's `exec`.
    pub(super) plugin: std::sync::atomic::AtomicPtr<clap_sys::plugin::clap_plugin>,
    /// The plugin's `clap_plugin_thread_pool.exec`, queried after `init`.
    pub(super) thread_pool_exec:
        std::sync::OnceLock<unsafe extern "C" fn(*const clap_sys::plugin::clap_plugin, u32)>,
    /// True while the plugin is inside `process()`: the only time CLAP
    /// lets it use the host's thread pool.
    pub(super) in_process: AtomicBool,
    /// `clap_host_preset_load` vtable served from `host_get_extension`.
    pub(super) preset_load_ext: clap_sys::ext::preset_load::clap_host_preset_load,
    /// `com.resonance.preset-session` vtable, likewise.
    pub(super) preset_session_ext: resonance_common::preset_session::HostPresetSession,
    /// What the plugin reported about its preset (`loaded`, `on_error`,
    /// the identity report), queued by the main-thread callbacks and
    /// drained by `ClapInstance::take_preset_reports`.
    pub(super) preset_reports: Mutex<Vec<preset_state::PresetHostReport>>,
    /// `clap_host_params` vtable (rescan / clear / request_flush).
    pub(super) params_ext: clap_sys::ext::params::clap_host_params,
    /// The app's param mirror should be re-read: the CLAP rescan flags the
    /// plugin asked for (`VALUES`, `TEXT`, `INFO`, `ALL`), OR-ed, or `ALL`
    /// from a preset load that wants a second look after the next block.
    /// Consumed by `ClapInstance::take_params_refresh`, which decides how
    /// much to re-read.
    pub(super) params_refresh: std::sync::atomic::AtomicU32,
    /// Set by `clap_host_params.request_flush()`: the plugin has output
    /// events to deliver (a param it changed itself) and wants a
    /// `params.flush` even if no `process()` is coming — a stopped
    /// transport runs none. Consumed by
    /// [`ClapInstance::service_flush_request`] on the engine thread.
    pub(super) flush_requested: AtomicBool,
}

impl HostData {
    /// Consume a pending `clap_host_gui.closed()` notification.
    /// `Some(was_destroyed)` when one had fired since the last call.
    pub(super) fn take_gui_closed(&self) -> Option<bool> {
        if self.gui_closed.swap(false, Ordering::AcqRel) {
            Some(self.gui_closed_was_destroyed.load(Ordering::Acquire))
        } else {
            None
        }
    }
}

/// Recover the `HostData` behind a `clap_host` pointer handed back by a
/// plugin. Returns `None` for null / not-yet-wired pointers so a
/// misbehaving plugin calling into us mid-construction can't crash.
pub(super) unsafe fn host_data_from<'a>(host: *const clap_host) -> Option<&'a HostData> {
    if host.is_null() {
        return None;
    }
    let data = (*host).host_data as *const HostData;
    data.as_ref()
}

unsafe extern "C" fn host_get_extension(
    host: *const clap_host,
    extension_id: *const c_char,
) -> *const c_void {
    let Some(data) = host_data_from(host) else {
        return ptr::null();
    };
    if extension_id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(extension_id).to_bytes();
    if id == CLAP_EXT_LATENCY.to_bytes() {
        return &data.latency_ext as *const clap_host_latency as *const c_void;
    }
    if id == CLAP_EXT_GUI.to_bytes() {
        return &data.gui_ext as *const clap_host_gui as *const c_void;
    }
    if id == CLAP_EXT_THREAD_CHECK.to_bytes() {
        return thread_check::host_thread_check_ptr();
    }
    if id == CLAP_EXT_THREAD_POOL.to_bytes() {
        return thread_pool::host_thread_pool_ptr();
    }
    if id == clap_sys::ext::preset_load::CLAP_EXT_PRESET_LOAD.to_bytes()
        || id == clap_sys::ext::preset_load::CLAP_EXT_PRESET_LOAD_COMPAT.to_bytes()
    {
        return &data.preset_load_ext as *const _ as *const c_void;
    }
    if id == clap_sys::ext::params::CLAP_EXT_PARAMS.to_bytes() {
        return &data.params_ext as *const _ as *const c_void;
    }
    if id == resonance_common::preset_session::EXTENSION_ID.to_bytes() {
        return &data.preset_session_ext as *const _ as *const c_void;
    }
    ptr::null()
}

/// `clap_host_latency.changed()` — the plugin's latency is stale.
/// Callable per spec on the main thread (incl. during activation); we
/// only flip an atomic flag here, the actual reactivate + re-query runs
/// on the engine thread at the next loop iteration.
unsafe extern "C" fn host_latency_changed(host: *const clap_host) {
    if let Some(data) = host_data_from(host) {
        data.latency_changed.store(true, Ordering::Release);
    }
}

/// `clap_host.request_restart()` — schedule a deactivate → reactivate
/// cycle. Flag only; serviced by the engine thread.
unsafe extern "C" fn host_request_restart(host: *const clap_host) {
    if let Some(data) = host_data_from(host) {
        data.restart_requested.store(true, Ordering::Release);
    }
}

/// `clap_host_gui.closed()` — the plugin's editor window is gone
/// without the host having asked (ba todo #1347): the user closed the
/// floating window from its own titlebar, or the plugin tore its editor
/// down itself. Main-thread callback; we only latch the flag, the
/// engine thread's `poll_plugin_host_requests` performs the teardown —
/// `destroy` unconditionally, see `gui_closed_was_destroyed` — and emits
/// the event.
unsafe extern "C" fn host_gui_closed(host: *const clap_host, was_destroyed: bool) {
    if let Some(data) = host_data_from(host) {
        data.gui_closed_was_destroyed
            .store(was_destroyed, Ordering::Release);
        data.gui_closed.store(true, Ordering::Release);
    }
}

/// `clap_host_gui.resize_hints_changed()` — every editor we open is a
/// floating top-level window the plugin sizes itself, so there are no
/// host-side hints to invalidate.
unsafe extern "C" fn host_gui_resize_hints_changed(_host: *const clap_host) {}

/// `clap_host_gui.request_resize()` — only meaningful for embedded
/// windows, whose parent the host owns. We embed nothing (floating
/// only), so refuse rather than claim a resize we cannot perform.
unsafe extern "C" fn host_gui_request_resize(
    _host: *const clap_host,
    _width: u32,
    _height: u32,
) -> bool {
    false
}

/// `clap_host_gui.request_show()` / `request_hide()` — the plugin asking
/// the host to show/hide its editor. The host drives visibility from
/// `AudioCommand::Open/ClosePluginEditor`; honouring a plugin-initiated
/// show here would open a window the app does not know about (and the
/// GUI ext must be driven from the main thread, which this callback is
/// not guaranteed to be). Refuse.
unsafe extern "C" fn host_gui_request_show(_host: *const clap_host) -> bool {
    false
}

unsafe extern "C" fn host_gui_request_hide(_host: *const clap_host) -> bool {
    false
}

/// `clap_host_params.rescan` — `[main-thread]`. A values (or text)
/// rescan is what a plugin sends after changing params itself (a preset it
/// loaded, a progress it moved); the mirror re-reads them on the next
/// host-request poll — the values only, unless the plugin says more
/// changed (`ClapInstance::take_params_refresh`). Adding or removing
/// params (`ALL`) needs a re-instantiation the host does not do live; what
/// it can re-read, it does.
unsafe extern "C" fn host_params_rescan(host: *const clap_host, flags: u32) {
    use clap_sys::ext::params::{
        CLAP_PARAM_RESCAN_ALL, CLAP_PARAM_RESCAN_INFO, CLAP_PARAM_RESCAN_TEXT,
        CLAP_PARAM_RESCAN_VALUES,
    };
    let known = CLAP_PARAM_RESCAN_VALUES
        | CLAP_PARAM_RESCAN_TEXT
        | CLAP_PARAM_RESCAN_INFO
        | CLAP_PARAM_RESCAN_ALL;
    if flags & known == 0 {
        return;
    }
    if let Some(data) = host_data_from(host) {
        data.params_refresh.fetch_or(flags & known, Ordering::AcqRel);
    }
}

unsafe extern "C" fn host_params_clear(_host: *const clap_host, _param_id: u32, _flags: u32) {}

/// `clap_host_params.request_flush` — `[thread-safe]`. Latched; the
/// engine thread's poll runs the flush (under the instance lock, which
/// excludes a concurrent `process()`).
unsafe extern "C" fn host_params_request_flush(host: *const clap_host) {
    if let Some(data) = host_data_from(host) {
        data.flush_requested.store(true, Ordering::Release);
    }
}

unsafe extern "C" fn host_request_process(_host: *const clap_host) {}
unsafe extern "C" fn host_request_callback(host: *const clap_host) {
    if let Some(data) = host_data_from(host) {
        data.callback_requested.store(true, Ordering::Release);
    }
}

pub(super) fn create_host_data() -> Pin<Box<HostData>> {
    let mut host_data = Box::pin(HostData {
        clap_host: clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: c"Resonance".as_ptr(),
            vendor: c"Resonance".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.1.0".as_ptr(),
            get_extension: Some(host_get_extension),
            request_restart: Some(host_request_restart),
            request_process: Some(host_request_process),
            request_callback: Some(host_request_callback),
        },
        latency_ext: clap_host_latency {
            changed: Some(host_latency_changed),
        },
        latency_changed: AtomicBool::new(false),
        restart_requested: AtomicBool::new(false),
        callback_requested: AtomicBool::new(false),
        gui_ext: clap_host_gui {
            resize_hints_changed: Some(host_gui_resize_hints_changed),
            request_resize: Some(host_gui_request_resize),
            request_show: Some(host_gui_request_show),
            request_hide: Some(host_gui_request_hide),
            closed: Some(host_gui_closed),
        },
        gui_closed: AtomicBool::new(false),
        gui_closed_was_destroyed: AtomicBool::new(false),
        plugin: std::sync::atomic::AtomicPtr::new(ptr::null_mut()),
        thread_pool_exec: std::sync::OnceLock::new(),
        in_process: AtomicBool::new(false),
        preset_load_ext: preset_state::host_preset_load_ext(),
        preset_session_ext: preset_state::host_preset_session_ext(),
        preset_reports: Mutex::new(Vec::new()),
        params_ext: clap_sys::ext::params::clap_host_params {
            rescan: Some(host_params_rescan),
            clear: Some(host_params_clear),
            request_flush: Some(host_params_request_flush),
        },
        params_refresh: std::sync::atomic::AtomicU32::new(0),
        flush_requested: AtomicBool::new(false),
    });
    let ptr = &*host_data as *const HostData as *mut c_void;
    unsafe {
        let host_data_mut = Pin::get_unchecked_mut(host_data.as_mut());
        host_data_mut.clap_host.host_data = ptr;
    }
    host_data
}

/// Test-only: build a [`ClapInstance`] around a raw `clap_plugin`
/// produced by `build` (which receives the host vtable pointer exactly
/// like a factory's `create_plugin` would). Runs the same
/// init / extension-query / activate / latency-query / start sequence
/// as [`ClapBundle::create_instance`], so integration tests can drive
/// the host-side latency machinery against a hand-rolled fake plugin
/// without a shared library.
#[doc(hidden)]
pub fn __instance_from_raw_for_test(
    build: impl FnOnce(*const clap_host) -> *const clap_sys::plugin::clap_plugin,
    sample_rate: u32,
) -> Result<ClapInstance, bundle::ClapBundleError> {
    let host_data = create_host_data();
    let host_ptr = &host_data.clap_host as *const clap_host;
    let plugin = build(host_ptr);
    if plugin.is_null() {
        return Err(bundle::ClapBundleError::TestPluginNull);
    }
    bundle::build_instance(plugin, host_data, sample_rate)
}

// ---------------------------------------------------------------------------
// SyncClapInstance — Send + Sync wrapper
// ---------------------------------------------------------------------------

/// Wrapper that makes [`ClapInstance`] `Send + Sync`.
///
/// SAFETY: This is justified by the CLAP threading contract:
/// - Lifecycle methods (create/activate/destroy) are called from the engine thread only
/// - process() is called from the audio callback thread only
/// - set_param() is called from the engine thread, pending_params consumed by process()
/// - flush_pending_params() (`clap_plugin_params.flush`) may only run
///   while no process() call is in flight for the same instance. The
///   mutex around this wrapper is what enforces that: `&mut ClapInstance`
///   is unreachable without its guard, and every process() call site
///   takes the same lock. See `clap_host::params` for the full argument.
pub struct SyncClapInstance(pub ClapInstance);

unsafe impl Send for SyncClapInstance {}
unsafe impl Sync for SyncClapInstance {}

// ---------------------------------------------------------------------------
// PluginSlot — one live instance plus the chain state around it
// ---------------------------------------------------------------------------

/// One slot of an insert chain: the plugin instance, plus the per-slot
/// state the mixer needs to reach *without* taking the instance's lock —
/// its [`BypassFade`] and whether the plugin declares a bypass parameter
/// of its own (ba doc #275 finding X3).
///
/// Derefs to the instance mutex, so every existing `try_lock()` /
/// `lock_fx(slot)` call site reads exactly as it did when the engine's
/// plugin map held bare `Mutex<SyncClapInstance>` values.
pub struct PluginSlot {
    instance: Mutex<SyncClapInstance>,
    /// Per-slot bypass, independent of the chain-level one that bypasses
    /// every slot at once.
    pub bypass: BypassFade,
    /// The plugin's own bypass parameter (`CLAP_PARAM_IS_BYPASS`), read
    /// once at instantiation. When present the host does **not** skip the
    /// slot: it drives this parameter and lets the plugin bypass itself,
    /// which is the only way a latency-carrying plugin can be bypassed
    /// without changing the chain's latency.
    pub bypass_param: Option<u32>,
    /// The bypass value last handed to `bypass_param`: `0` = engaged,
    /// `1` = bypassed, `-1` = never sent. Keeps the render path from
    /// re-queuing an unchanged parameter every single block.
    own_bypass_sent: AtomicI8,
    /// The render pool must process this plugin on the audio thread
    /// only (realtime-multithreading.md §4.6): a hand-maintained escape
    /// hatch for a third-party plugin found to misbehave when its
    /// `process()` moves between threads. See [`serial_only_plugin`].
    pub serial_only: bool,
}

/// Plugin ids known to need a fixed audio thread. Empty: none of ours
/// hold thread-affine state, and no third-party plugin has been caught
/// needing it yet. `RESONANCE_SERIAL_ONLY_PLUGINS` (comma-separated ids)
/// adds more without a rebuild.
const SERIAL_ONLY_PLUGINS: &[&str] = &[];

/// Whether plugin `id` must stay on the audio thread. Allocates (reads
/// the environment); engine side, once per instance.
pub fn serial_only_plugin(id: &str) -> bool {
    SERIAL_ONLY_PLUGINS.contains(&id)
        || std::env::var("RESONANCE_SERIAL_ONLY_PLUGINS")
            .is_ok_and(|ids| ids.split(',').any(|listed| listed.trim() == id))
}

/// Live [`PluginSlot`]s with `serial_only` set, so the render pass can
/// skip looking for them when there are none (the usual case).
static SERIAL_ONLY_SLOTS: AtomicUsize = AtomicUsize::new(0);

/// Whether any live plugin slot is serial-only. One relaxed load.
#[inline]
pub(crate) fn any_serial_only_plugin() -> bool {
    SERIAL_ONLY_SLOTS.load(Ordering::Relaxed) != 0
}

impl PluginSlot {
    /// Wrap a freshly created instance. Reads the plugin's bypass
    /// parameter id and descriptor id once — engine-thread queries, never
    /// on the audio thread.
    pub fn new(instance: ClapInstance) -> Self {
        let bypass_param = instance.bypass_param_id();
        let serial_only = instance
            .descriptor_id()
            .is_some_and(|id| serial_only_plugin(&id));
        Self::with_serial_only(instance, bypass_param, serial_only)
    }

    /// [`Self::new`] with the serial-only flag given (tests).
    #[doc(hidden)]
    pub fn new_serial_only(instance: ClapInstance) -> Self {
        let bypass_param = instance.bypass_param_id();
        Self::with_serial_only(instance, bypass_param, true)
    }

    fn with_serial_only(
        instance: ClapInstance,
        bypass_param: Option<u32>,
        serial_only: bool,
    ) -> Self {
        if serial_only {
            SERIAL_ONLY_SLOTS.fetch_add(1, Ordering::Relaxed);
        }
        Self {
            instance: Mutex::new(SyncClapInstance(instance)),
            bypass: BypassFade::new(),
            bypass_param,
            own_bypass_sent: AtomicI8::new(-1),
            serial_only,
        }
    }

    /// True when the mixer skips this slot entirely this pass: bypassed,
    /// settled, and with no bypass parameter of the plugin's own to drive
    /// instead. Such a slot is not processed, so it contributes no
    /// latency — see `latency::slot_latency`.
    #[inline]
    pub fn host_bypassed(&self) -> bool {
        self.bypass_param.is_none() && self.bypass.bypassed()
    }

    /// This block's [`FadeStage`] for the slot.
    ///
    /// Folds in the own-parameter rule: a plugin that bypasses itself
    /// stays in the chain even when settled-bypassed (its output *is* the
    /// dry signal by then), so the stage never resolves to
    /// [`FadeStage::Dry`] for it. The host crossfade still runs over the
    /// transition, which is what makes a plugin that switches its own
    /// bypass abruptly click-free anyway.
    #[inline]
    pub fn stage(&self, sample_rate: u32, frames: usize, live: bool) -> FadeStage {
        let stage = self.bypass.stage(sample_rate, frames, live);
        if self.bypass_param.is_some() && stage == FadeStage::Dry {
            FadeStage::Wet
        } else {
            stage
        }
    }

    /// Push the plugin's own bypass parameter when it declares one and
    /// the target changed. Call right after locking the instance and
    /// before processing it.
    pub fn sync_own_bypass(&self, inst: &mut ClapInstance) {
        let Some(param_id) = self.bypass_param else {
            return;
        };
        let want: i8 = i8::from(self.bypass.bypassed());
        if self.own_bypass_sent.swap(want, Ordering::Relaxed) == want {
            return;
        }
        inst.set_param(param_id, f64::from(want));
    }
}

impl Drop for PluginSlot {
    fn drop(&mut self) {
        if self.serial_only {
            SERIAL_ONLY_SLOTS.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl std::fmt::Debug for PluginSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never locks the instance: `Debug` of a render graph must be
        // safe to take while the audio thread processes it.
        f.debug_struct("PluginSlot")
            .field("bypassed", &self.bypass.bypassed())
            .field("bypass_param", &self.bypass_param)
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for PluginSlot {
    type Target = Mutex<SyncClapInstance>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.instance
    }
}

/// The engine's live plugin instances, keyed by instance id. One entry
/// per slot across every track, sub-track, bus and master chain.
///
/// Published as a field of the render graph (code review ARCH-02 B-4):
/// the map is immutable once published and each slot is an `Arc`, so a
/// copy-on-write edit of the map shares every untouched slot — and its
/// instance mutex — with the graph the audio thread may still be
/// reading. A removed slot's last owner is the engine thread's retire
/// queue, never a reader (see `engine::render_graph`).
pub type PluginMap = IndexMap<PluginInstanceId, std::sync::Arc<PluginSlot>>;
