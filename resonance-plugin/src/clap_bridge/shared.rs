//! Shared state structs for the CLAP bridge.
//!
//! - `ClapShared`: Send + Sync, holds host handle, param metadata, atomic values.
//! - `ClapMainThread`: holds the plugin (when not active) plus editor/state helpers.
//! - `ClapAudioProcessor`: holds the plugin (when active) plus scratch buffers.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use clack_extensions::latency::HostLatency;
use clack_plugin::prelude::*;

use crate::gui::{EditorFactory, PluginEditor};
use crate::plugin::{OutputPortSpec, PluginEvent, ResonancePlugin};

// ---------------------------------------------------------------------------
// Param metadata stored in SharedState
// ---------------------------------------------------------------------------

pub(crate) struct ParamMeta {
    pub clap_id: u32,
    pub str_id: String,
    pub name: String,
    /// The param's group, as CLAP's `/`-separated module path. Captured
    /// once here because `get_info` runs on every host reload and must
    /// hand out a borrow, not build a string (ba todo #1289).
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub is_stepped: bool,
    pub is_hidden: bool,
}

// ---------------------------------------------------------------------------
// SharedState (Send + Sync, shared between threads)
// ---------------------------------------------------------------------------

pub struct ClapShared<'a> {
    /// The thread-safe host handle. Wrapped into the plugin-facing
    /// [`HostHandle`](crate::host::HostHandle) in `new_main_thread`, which is
    /// what lets a plugin report a runtime latency change (ba todo #1296).
    pub(super) host: HostSharedHandle<'a>,
    pub(crate) param_metas: Vec<ParamMeta>,
    /// Indices into `param_metas` of non-hidden params, computed once
    /// at construction. The CLAP host iterates `get_info` for each
    /// visible param every reload; without this cache, every call
    /// would `Vec::collect` a filtered list of references.
    pub(crate) visible_indices: Vec<usize>,
    /// Atomic param values (f64 bit-punned to u64), indexed by param slot.
    pub(crate) param_values: Vec<AtomicU64>,
    /// Map from CLAP param ID to slot index.
    pub(crate) clap_id_to_slot: std::collections::HashMap<u32, usize>,
    pub(crate) input_channels: Option<u32>,
    /// Channel count of the optional external sidechain (key) input port, or
    /// `None` when the plugin declares no sidechain. Captured once from
    /// `ResonancePlugin::SIDECHAIN_INPUT` at construction; consulted by the
    /// audio-ports extension and the audio processor.
    pub(crate) sidechain_channels: Option<u32>,
    /// Cached output-port layout, captured once from `ResonancePlugin::output_layout()`
    /// at plugin construction. The CLAP audio-ports extension, the host, and the
    /// audio processor all consult this instead of re-calling the plugin hook.
    pub(crate) output_ports: Vec<OutputPortSpec>,
    pub(crate) midi_input: bool,
    /// The plugin's parameter-id rename table, harvested once at
    /// construction from `ResonancePlugin::param_renames`.
    ///
    /// It lives here because state can be loaded while the plugin object
    /// is inside `ClapAudioProcessor`, and that path (`clap_bridge::state`)
    /// has no plugin to ask. Without the table, a blob written before a
    /// rename restored that parameter at its default whenever the host
    /// happened to load it while the plugin was active — the *same* file
    /// loading differently depending on transport state (ba todo #1360).
    ///
    /// The whole slice is carried rather than flattening legacy ids into
    /// each `ParamMeta`, because a rename is more than an alias: it is
    /// gated on the blob's `since_version` and renames chain oldest-first.
    /// Keeping the table intact lets both load paths run the one
    /// [`crate::state::migrate`] instead of two rules that must agree.
    pub(crate) param_renames: &'static [crate::state::ParamRename],
    /// Flag: shared param values have been updated (e.g. state load while active).
    /// The audio processor should re-sync plugin params from shared atomics.
    pub(crate) params_dirty: AtomicBool,
}

impl ClapShared<'_> {
    pub fn find_slot(&self, clap_id: u32) -> Option<usize> {
        self.clap_id_to_slot.get(&clap_id).copied()
    }

    pub fn get_value(&self, slot: usize) -> f64 {
        f64::from_bits(self.param_values[slot].load(Ordering::Relaxed))
    }

    pub fn set_value(&self, slot: usize, value: f64) {
        self.param_values[slot].store(value.to_bits(), Ordering::Relaxed);
    }

    /// Store `new` into the slot only if it still holds `current`.
    ///
    /// Used by the audio thread's editor push-back so it cannot clobber a
    /// value the main thread wrote concurrently (state load): if the CAS
    /// loses, the main-thread write stays in place and `params_dirty`
    /// (set by the writer) makes the next block re-sync the plugin from
    /// shared. Lock-free, no allocation — a single `compare_exchange` on
    /// the slot's `AtomicU64`.
    pub fn compare_exchange_value(&self, slot: usize, current: f64, new: f64) -> bool {
        self.param_values[slot]
            .compare_exchange(
                current.to_bits(),
                new.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
    }
}

// SAFETY: HostSharedHandle wraps CLAP host function pointers which the CLAP spec
// mandates to be thread-safe (the host must support concurrent calls from any thread).
// All other fields are atomics, HashMap (read-only after construction), or Send+Sync types.
unsafe impl Send for ClapShared<'_> {}
unsafe impl Sync for ClapShared<'_> {}

impl<'a> PluginShared<'a> for ClapShared<'a> {}

// ---------------------------------------------------------------------------
// MainThreadState
// ---------------------------------------------------------------------------

pub struct ClapMainThread<'a, P: ResonancePlugin> {
    /// Main-thread host handle. Used to call `clap_host_latency.changed()`,
    /// which the CLAP spec restricts to this thread.
    pub(super) host: HostMainThreadHandle<'a>,
    pub(crate) shared: &'a ClapShared<'a>,
    pub(crate) plugin: Option<P>,
    /// The plugin-facing host handle, also held by the plugin itself (it is
    /// handed over in `ResonancePlugin::set_host`). Carries the latency the
    /// plugin last reported: captured from the plugin after `initialize()`
    /// inside `activate`, refreshed by any direct main-thread query while
    /// inactive, and *pushed* by the plugin when its latency changes at
    /// runtime. The CLAP latency extension serves this value while the
    /// plugin object lives in the audio processor — in particular for the
    /// host's post-activation `latency.get()` query.
    pub(crate) host_handle: std::sync::Arc<crate::host::HostHandle>,
    /// Editor factory harvested from the plugin at construction time. `None`
    /// if the plugin has no GUI. Kept alive across activate/deactivate so
    /// the host can open the editor while audio is running.
    pub(crate) editor_factory: Option<std::sync::Arc<dyn EditorFactory>>,
    /// The currently-open editor, if any. Created by `gui_create`, dropped
    /// by `gui_destroy`.
    pub(crate) editor: Option<Box<dyn PluginEditor>>,
    /// Extra-state saver harvested from the plugin at construction time.
    /// `None` if the plugin has no extra state. Kept alive across
    /// activate/deactivate so the host can save/load project state while
    /// the plugin is in the audio processor.
    pub(crate) extra_state_saver: Option<std::sync::Arc<dyn crate::plugin::ExtraStateSaver>>,
}

impl<'a, P: ResonancePlugin> PluginMainThread<'a, ClapShared<'a>> for ClapMainThread<'a, P> {
    /// Runs on the main thread in response to `clap_host.request_callback()`.
    ///
    /// The only thing the bridge defers here is the latency notification:
    /// `clap_host_latency.changed()` is `[main-thread]`, but a plugin reports
    /// its new latency from wherever it noticed — typically the audio thread
    /// inside `process()`, or an editor thread. `set_latency_samples` asks
    /// for this callback; here we tell the host to re-query, which is what
    /// makes it recompute plugin delay compensation (ba todo #1296).
    fn on_main_thread(&mut self) {
        if !self.host_handle.take_latency_dirty() {
            return;
        }
        if let Some(latency) = self.host.shared().get_extension::<HostLatency>() {
            latency.changed(&mut self.host);
        }
    }
}

/// Retire the plugin-facing host handle when the instance is destroyed.
///
/// The handle stores the `clap_host` pointer with an erased lifetime so the
/// plugin (which is `'static`) can hold it. This drop runs while that pointer
/// is still valid and flips the handle inert, so a clone that outlived the
/// instance — leaked into an editor thread, say — degrades to no-ops instead
/// of calling through a dangling pointer.
impl<P: ResonancePlugin> Drop for ClapMainThread<'_, P> {
    fn drop(&mut self) {
        self.host_handle.retire();
    }
}

// ---------------------------------------------------------------------------
// AudioProcessor
// ---------------------------------------------------------------------------

pub struct ClapAudioProcessor<'a, P: ResonancePlugin> {
    pub(crate) plugin: P,
    pub(crate) shared: &'a ClapShared<'a>,
    /// Pre-allocated scratch buffers for the effect/instrument input
    /// (read from host into these before the plugin call).
    pub(crate) input_left: Vec<f32>,
    pub(crate) input_right: Vec<f32>,
    /// Pre-allocated scratch for the external sidechain (key) signal, read
    /// from the host's secondary input port before the plugin call. Always
    /// stereo-shaped (a mono key port is mirrored into both); empty when the
    /// plugin declares no sidechain. Sized at activation, never reallocated
    /// on the audio thread.
    pub(crate) key_left: Vec<f32>,
    pub(crate) key_right: Vec<f32>,
    /// Pre-allocated output scratch, one `(left, right)` pair per declared
    /// output port. Populated by the plugin on each `process()` call and
    /// then copied back into the CLAP audio buffers.
    pub(crate) output_scratch: Vec<(Vec<f32>, Vec<f32>)>,
    /// Pre-allocated buffer for input events — notes *and* MIDI controllers,
    /// interleaved in host order (avoids audio-thread allocation).
    pub(crate) input_events: Vec<PluginEvent>,
}
