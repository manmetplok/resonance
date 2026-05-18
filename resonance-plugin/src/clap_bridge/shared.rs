//! Shared state structs for the CLAP bridge.
//!
//! - `ClapShared`: Send + Sync, holds host handle, param metadata, atomic values.
//! - `ClapMainThread`: holds the plugin (when not active) plus editor/state helpers.
//! - `ClapAudioProcessor`: holds the plugin (when active) plus scratch buffers.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use clack_plugin::prelude::*;

use crate::gui::{EditorFactory, PluginEditor};
use crate::plugin::{NoteEvent, OutputPortSpec, ResonancePlugin};

// ---------------------------------------------------------------------------
// Param metadata stored in SharedState
// ---------------------------------------------------------------------------

pub(crate) struct ParamMeta {
    pub clap_id: u32,
    pub str_id: String,
    pub name: String,
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
    #[allow(dead_code)]
    pub(super) host: HostSharedHandle<'a>,
    pub(crate) param_metas: Vec<ParamMeta>,
    /// Atomic param values (f64 bit-punned to u64), indexed by param slot.
    pub(crate) param_values: Vec<AtomicU64>,
    /// Map from CLAP param ID to slot index.
    pub(crate) clap_id_to_slot: std::collections::HashMap<u32, usize>,
    pub(crate) input_channels: Option<u32>,
    /// Cached output-port layout, captured once from `ResonancePlugin::output_layout()`
    /// at plugin construction. The CLAP audio-ports extension, the host, and the
    /// audio processor all consult this instead of re-calling the plugin hook.
    pub(crate) output_ports: Vec<OutputPortSpec>,
    pub(crate) midi_input: bool,
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
    #[allow(dead_code)]
    pub(super) host: HostMainThreadHandle<'a>,
    pub(crate) shared: &'a ClapShared<'a>,
    pub(crate) plugin: Option<P>,
    pub(crate) last_latency: u32,
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

impl<'a, P: ResonancePlugin> PluginMainThread<'a, ClapShared<'a>> for ClapMainThread<'a, P> {}

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
    /// Pre-allocated output scratch, one `(left, right)` pair per declared
    /// output port. Populated by the plugin on each `process()` call and
    /// then copied back into the CLAP audio buffers.
    pub(crate) output_scratch: Vec<(Vec<f32>, Vec<f32>)>,
    /// Pre-allocated buffer for note events (avoids audio-thread allocation).
    pub(crate) note_events: Vec<NoteEvent>,
}
