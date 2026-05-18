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
//! - [`gui`] — embedded GUI lifecycle
//! - [`process`] — audio-processor activate/process/deactivate

use std::sync::atomic::{AtomicBool, AtomicU64};

use clack_extensions::audio_ports::PluginAudioPorts;
use clack_extensions::gui::PluginGui;
use clack_extensions::latency::{PluginLatency, PluginLatencyImpl};
use clack_extensions::note_ports::PluginNotePorts;
use clack_extensions::params::PluginParams;
use clack_extensions::state::PluginState;
use clack_plugin::prelude::*;

use crate::plugin::ResonancePlugin;

mod gui;
mod params;
mod ports;
mod process;
pub mod shared;
mod state;

// Re-export the public types so downstream code keeps using
// `resonance_plugin::clap_bridge::ClapShared` etc.
pub use shared::{ClapAudioProcessor, ClapMainThread, ClapShared};

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

        if let Some(shared) = shared {
            if shared.midi_input {
                builder.register::<PluginNotePorts>();
            }
        } else {
            // First call (no shared yet) — register conservatively
            builder.register::<PluginNotePorts>();
        }

        builder.register::<PluginLatency>();
        // GUI extension is registered unconditionally; plugins without an
        // editor factory return false from is_api_supported, which is the
        // CLAP-correct way to say "no editor".
        builder.register::<PluginGui>();
    }
}

impl<P: ResonancePlugin> DefaultPluginFactory for ClapBridge<P> {
    fn get_descriptor() -> PluginDescriptor {
        let mut desc = PluginDescriptor::new(P::CLAP_ID, P::NAME)
            .with_vendor(P::VENDOR)
            .with_version(P::VERSION)
            .with_description(P::DESCRIPTION);

        let features: Vec<&'static std::ffi::CStr> = P::FEATURES
            .iter()
            .filter_map(|f| match *f {
                "audio-effect" => Some(c"audio-effect"),
                "instrument" => Some(c"instrument"),
                "stereo" => Some(c"stereo"),
                "mono" => Some(c"mono"),
                "reverb" => Some(c"reverb"),
                "sampler" => Some(c"sampler"),
                "drum" | "drum-machine" => Some(c"drum-machine"),
                "synthesizer" | "synth" => Some(c"synthesizer"),
                "cabinet-simulator" => Some(c"cabinet-simulator"),
                _ => None,
            })
            .collect();
        if !features.is_empty() {
            desc = desc.with_features(features);
        }

        desc
    }

    fn new_shared<'a>(host: HostSharedHandle<'a>) -> Result<ClapShared<'a>, PluginError> {
        let temp = P::new();
        let count = temp.param_count();
        let output_ports = temp.output_layout();
        if output_ports.is_empty() {
            return Err(PluginError::Message(
                "Plugin must declare at least one output port",
            ));
        }
        for port in &output_ports {
            if port.channel_count != 1 && port.channel_count != 2 {
                return Err(PluginError::Message(
                    "Only mono and stereo output ports are supported",
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

            // Check for hash collisions
            if let Some(&existing_slot) = clap_id_to_slot.get(&clap_id) {
                panic!(
                    "CLAP param ID collision: params '{}' (slot {}) and '{}' (slot {}) both hash to {}",
                    param_metas[existing_slot].str_id, existing_slot, p.id(), i, clap_id
                );
            }

            param_metas.push(ParamMeta {
                clap_id,
                str_id: p.id().to_string(),
                name: p.name().to_string(),
                min: p.min_plain(),
                max: p.max_plain(),
                default: p.default_plain(),
                is_stepped: p.is_stepped(),
                is_hidden: p.is_hidden(),
            });
            param_values.push(AtomicU64::new(p.default_plain().to_bits()));
            clap_id_to_slot.insert(clap_id, i);
        }

        Ok(ClapShared {
            host,
            param_metas,
            param_values,
            clap_id_to_slot,
            input_channels: P::INPUT_CHANNELS,
            output_ports,
            midi_input: P::MIDI_INPUT,
            params_dirty: AtomicBool::new(false),
        })
    }

    fn new_main_thread<'a>(
        host: HostMainThreadHandle<'a>,
        shared: &'a ClapShared<'a>,
    ) -> Result<ClapMainThread<'a, P>, PluginError> {
        let plugin = P::new();
        for i in 0..plugin.param_count() {
            if i < shared.param_values.len() {
                shared.set_value(i, plugin.param(i).get_plain());
            }
        }

        // Harvest the editor factory and any extra-state saver before the
        // plugin may be moved to the audio processor. Both are None for
        // plugins that don't opt in.
        let editor_factory = plugin.editor_factory();
        let extra_state_saver = plugin.extra_state_saver();

        Ok(ClapMainThread {
            host,
            shared,
            plugin: Some(plugin),
            last_latency: 0,
            editor_factory,
            editor: None,
            extra_state_saver,
        })
    }
}

// ---------------------------------------------------------------------------
// Latency extension
// ---------------------------------------------------------------------------

impl<'a, P: ResonancePlugin> PluginLatencyImpl for ClapMainThread<'a, P> {
    fn get(&mut self) -> u32 {
        if let Some(plugin) = &self.plugin {
            let lat = plugin.latency_samples();
            self.last_latency = lat;
            lat
        } else {
            self.last_latency
        }
    }
}
