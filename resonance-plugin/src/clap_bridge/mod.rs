//! CLAP bridge: maps ResonancePlugin to clack-plugin's trait hierarchy.
//!
//! Architecture:
//! - `ClapShared`: holds param metadata + atomic values, host handle (Send+Sync)
//! - `ClapMainThread`: holds Option<P> (plugin when not active), extension impls
//! - `ClapAudioProcessor`: holds P (plugin when active), processes audio
//!
//! The module is split into focused submodules grouped by CLAP extension:
//! - [`shared`] — `ClapInstance` shared state structs + small helpers
//! - [`ports`] — audio/note port discovery and descriptors
//! - [`params`] — main-thread + audio-processor parameter handling
//! - [`state`] — preset/project save/load
//! - [`preset`] — the preset form of the state (state-context) and preset-load
//! - [`gui`] — embedded GUI lifecycle
//! - [`process`] — audio-processor activate/process/deactivate

use std::sync::atomic::{AtomicBool, AtomicU64};

use clack_extensions::audio_ports::PluginAudioPorts;
use clack_extensions::gui::PluginGui;
use clack_extensions::latency::{PluginLatency, PluginLatencyImpl};
use clack_extensions::note_ports::PluginNotePorts;
use clack_extensions::params::PluginParams;
use clack_extensions::preset_discovery::PluginPresetLoad;
use clack_extensions::render::{PluginRender, PluginRenderImpl, RenderMode};
use clack_extensions::state::PluginState;
use clack_extensions::state_context::PluginStateContext;
use clack_plugin::prelude::*;

use crate::plugin::ResonancePlugin;

mod gui;
mod midi;
mod param_flags;
mod param_output;
mod params;
mod ports;
mod process;
pub mod shared;
mod preset;
mod preset_session;
mod state;

// Re-export the public types so downstream code keeps using
// `resonance_plugin::clap_bridge::ClapShared` etc.
pub use shared::{ClapAudioProcessor, ClapMainThread, ClapShared};

// Sidechain (key) input-port policy — also consumed by the host mixer that
// delivers the external key to the target plugin's sidechain port.
pub use ports::{input_port_count, sidechain_port_index, SIDECHAIN_PORT_ID};

// Raw MIDI decoding, exposed for the bridge's own tests.
pub use midi::decode_midi;

// Param metadata is `pub(crate)` and accessed through `clap_bridge::shared`.
pub(crate) use shared::ParamMeta;

// ---------------------------------------------------------------------------
// Plugin marker trait + DefaultPluginFactory
// ---------------------------------------------------------------------------

pub struct ClapBridge<P: ResonancePlugin>(std::marker::PhantomData<P>);

impl<P: ResonancePlugin> Plugin for ClapBridge<P> {
    type AudioProcessor<'a> = ClapAudioProcessor<'a, P>;
    type Shared<'a> = ClapShared<'a>;
    type MainThread<'a> = ClapMainThread<'a, P>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, shared: Option<&ClapShared<'_>>) {
        builder.register::<PluginAudioPorts>();
        builder.register::<PluginParams>();
        builder.register::<PluginState>();
        // The preset form of the state (save/load FOR_PRESET) and loading
        // a preset by location: plugin-preset-library.md §6.7, §7.
        builder.register::<PluginStateContext>();
        builder.register::<PluginPresetLoad>();
        builder.register::<preset_session::PluginPresetSessionExt>();
        // Which params the state leaves out (com.resonance.param-flags):
        // CLAP has no flag for it, and a host persisting them beside the
        // state would override what the state recalls.
        builder.register::<param_flags::PluginParamFlagsExt>();

        if let Some(shared) = shared {
            if shared.midi_input {
                builder.register::<PluginNotePorts>();
            }
        } else {
            // First call (no shared yet) — register conservatively
            builder.register::<PluginNotePorts>();
        }

        builder.register::<PluginLatency>();
        // Realtime vs offline rendering (`ResonancePlugin::set_render_mode`).
        builder.register::<PluginRender>();
        // GUI extension is registered unconditionally; plugins without an
        // editor factory return false from is_api_supported, which is the
        // CLAP-correct way to say "no editor".
        builder.register::<PluginGui>();
    }
}

impl<P: ResonancePlugin> ClapBridge<P> {
    /// A plugin has to tell the host what kind of thing it is.
    ///
    /// Evaluated for every exported plugin by `get_descriptor` below, so
    /// a `FEATURES` list with no main category is a build failure rather
    /// than a plugin that turns up nowhere useful in a browser
    /// (ba todo #1298).
    const HAS_CATEGORY: () = assert!(
        crate::features::has_category(P::FEATURES),
        "FEATURES must declare at least one CLAP main category \
         (features::AUDIO_EFFECT, INSTRUMENT, NOTE_EFFECT, NOTE_DETECTOR or ANALYZER)"
    );
}

impl<P: ResonancePlugin> DefaultPluginFactory for ClapBridge<P> {
    fn get_descriptor() -> PluginDescriptor {
        // Forces the const assertion above for this concrete plugin.
        let () = Self::HAS_CATEGORY;

        let mut desc = PluginDescriptor::new(P::CLAP_ID, P::NAME)
            .with_vendor(P::VENDOR)
            .with_version(P::VERSION)
            .with_description(P::DESCRIPTION);

        // Verbatim: the features a plugin declares are already CLAP's own
        // `&CStr` constants. The bridge used to map them through a
        // hand-written whitelist and `filter_map` away everything else,
        // which is how six standard categories and one typo went missing
        // (finding X6).
        if !P::FEATURES.is_empty() {
            desc = desc.with_features(P::FEATURES.iter().copied());
        }

        desc
    }

    fn new_shared<'a>(host: HostSharedHandle<'a>) -> Result<ClapShared<'a>, PluginError> {
        // A bundle has its own tracing dispatcher; give it a subscriber
        // before anything below can log (ARCH-05).
        crate::logging::ensure_subscriber();
        let temp = P::new();
        let count = temp.param_count();
        let output_ports = temp.output_layout();
        if output_ports.is_empty() {
            return Err(PluginError::Message(
                "Plugin must declare at least one output port",
            ));
        }
        if output_ports.len() > shared::MAX_OUTPUT_PORTS {
            return Err(PluginError::Message(
                "At most 8 output ports are supported",
            ));
        }
        for port in &output_ports {
            if port.channel_count != 1 && port.channel_count != 2 {
                return Err(PluginError::Message(
                    "Only mono and stereo output ports are supported",
                ));
            }
        }

        // Validate the optional sidechain (key) input port. Like output
        // ports, only mono/stereo are supported.
        if let Some(ch) = P::SIDECHAIN_INPUT {
            if ch != 1 && ch != 2 {
                return Err(PluginError::Message(
                    "Only mono and stereo sidechain input ports are supported",
                ));
            }
        }

        let mut param_metas: Vec<ParamMeta> = Vec::with_capacity(count);
        let mut param_values: Vec<AtomicU64> = Vec::with_capacity(count);
        let mut clap_id_to_slot: std::collections::HashMap<u32, usize> =
            std::collections::HashMap::with_capacity(count);

        for i in 0..count {
            let p = temp.param(i);
            let clap_id = p.clap_id();

            // Check for hash collisions. Panicking across the C FFI
            // boundary into a CLAP host is undefined behaviour
            // (`extern "C"`, not `"C-unwind"`); print a diagnostic
            // and return a PluginError instead so the host reports
            // a clean load failure.
            if let Some(&existing_slot) = clap_id_to_slot.get(&clap_id) {
                tracing::error!(
                    "resonance-plugin: CLAP param ID collision — params '{}' (slot {}) and '{}' (slot {}) both hash to {}",
                    param_metas[existing_slot].str_id, existing_slot, p.id(), i, clap_id
                );
                return Err(PluginError::Message("CLAP param ID hash collision (see stderr)"));
            }

            param_metas.push(ParamMeta {
                clap_id,
                str_id: p.id().to_string(),
                name: p.name().to_string(),
                module: p.module().to_string(),
                min: p.min_plain(),
                max: p.max_plain(),
                default: p.default_plain(),
                is_stepped: p.is_stepped(),
                is_hidden: p.is_hidden(),
                preset_excluded: p.preset_excluded() || p.state_excluded(),
                is_automatable: p.is_automatable() && !p.is_read_only(),
                is_read_only: p.is_read_only(),
                state_excluded: p.state_excluded(),
            });
            param_values.push(AtomicU64::new(p.default_plain().to_bits()));
            clap_id_to_slot.insert(clap_id, i);
        }

        // Pre-compute the indices of non-hidden params so `get_info`
        // doesn't have to filter+collect on every host query.
        let visible_indices: Vec<usize> = param_metas
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.is_hidden)
            .map(|(i, _)| i)
            .collect();

        Ok(ClapShared {
            host,
            param_metas,
            visible_indices,
            param_values,
            clap_id_to_slot,
            input_channels: P::INPUT_CHANNELS,
            sidechain_channels: P::SIDECHAIN_INPUT,
            output_ports,
            midi_input: P::MIDI_INPUT,
            // Harvested from the same throwaway instance the param metadata
            // came from: the state extension needs it while the real plugin
            // lives in the audio processor (ba todo #1360).
            param_renames: temp.param_renames(),
            params_dirty: AtomicBool::new(false),
            params_gen: AtomicU64::new(0),
            preset_compare_due: AtomicBool::new(false),
            param_preset_ignored: (0..count).map(|_| AtomicBool::new(false)).collect(),
            render_offline: AtomicBool::new(false),
            render_mode_dirty: AtomicBool::new(false),
        })
    }

    fn new_main_thread<'a>(
        host: HostMainThreadHandle<'a>,
        shared: &'a ClapShared<'a>,
    ) -> Result<ClapMainThread<'a, P>, PluginError> {
        let mut plugin = P::new();
        for i in 0..plugin.param_count() {
            if i < shared.param_values.len() {
                shared.set_value(i, plugin.param(i).get_plain());
            }
        }

        // Hand the plugin its handle to the host, before it can be activated
        // and before anything else may query it. Plugins that never talk back
        // to the host use the default `set_host`, which drops it.
        let clap_ids: Vec<u32> = shared.param_metas.iter().map(|m| m.clap_id).collect();
        let host_handle =
            crate::host::HostHandle::new(shared.host, plugin.latency_samples(), &clap_ids);
        plugin.set_host(host_handle.clone());

        // Harvest the editor factory and any extra-state saver before the
        // plugin may be moved to the audio processor. Both are None for
        // plugins that don't opt in.
        let editor_factory = plugin.editor_factory();
        let extra_state_saver = plugin.extra_state_saver();
        let param_text_source = plugin.param_text_source();
        // The loaded-preset identity reaches the host from `on_main_thread`
        // (com.resonance.preset-session): the session flags a change from
        // whatever thread it happens on.
        if let Some(saver) = &extra_state_saver {
            let handle = host_handle.clone();
            saver.set_change_notifier(std::sync::Arc::new(move || handle.report_preset_change()));
        }

        Ok(ClapMainThread {
            host,
            shared,
            plugin: Some(plugin),
            host_handle,
            editor_factory,
            editor: None,
            editor_serial: 0,
            extra_state_saver,
            param_text_source,
            last_preset_report: None,
            last_preset_compare: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Latency extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginLatencyImpl for ClapMainThread<'a, P> {
    fn get(&mut self) -> u32 {
        if let Some(plugin) = &self.plugin {
            // Inactive: the plugin lives on the main thread — ask it
            // directly. Note that before the first activation some plugins
            // report 0 here because their DSP chain is only built in
            // `initialize()`; the CLAP spec only defines this query while
            // the plugin is active.
            let lat = plugin.latency_samples();
            self.host_handle.store_latency(lat);
            lat
        } else {
            // Active: the plugin object moved into the audio processor.
            // Serve the cached value — captured post-`initialize()` during
            // `activate`, or pushed by the plugin itself through
            // `HostHandle::set_latency_samples` when its latency changed at
            // runtime. This is the path both the host's activation-time
            // query and its re-query after `clap_host_latency.changed()`
            // take.
            self.host_handle.latency_samples()
        }
    }
}

// ---------------------------------------------------------------------------
// Render extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginRenderImpl for ClapMainThread<'a, P> {
    /// No bridged plugin is a proxy to hardware: a host may always render
    /// it offline.
    fn has_hard_realtime_requirement(&self) -> bool {
        false
    }

    /// `[main-thread]`. Inactive: the plugin is here, tell it now. Active:
    /// latch it for the audio processor, which tells the plugin at the top
    /// of its next block (`ResonancePlugin::set_render_mode`).
    fn set(&mut self, mode: RenderMode) -> Result<(), PluginError> {
        let offline = matches!(mode, RenderMode::Offline);
        self.shared
            .render_offline
            .store(offline, std::sync::atomic::Ordering::Release);
        match &mut self.plugin {
            Some(plugin) => plugin.set_render_mode(offline),
            None => self
                .shared
                .render_mode_dirty
                .store(true, std::sync::atomic::Ordering::Release),
        }
        Ok(())
    }
}
