//! State extension for the CLAP bridge: preset/project save and load.

use std::io::{Read, Write};
use std::sync::atomic::Ordering;

use clack_extensions::state::PluginStateImpl;
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};

use super::shared::ClapMainThread;
use crate::param::Param;
use crate::plugin::ResonancePlugin;

impl<'a, P: ResonancePlugin> PluginStateImpl for ClapMainThread<'a, P> {
    fn save(&mut self, output: &mut OutputStream) -> Result<(), PluginError> {
        let data = if let Some(plugin) = &self.plugin {
            // Main-thread path: the plugin's own `save_state` composes
            // params with any extra-state saver via the trait default.
            plugin.save_state()
        } else {
            // Audio-processor path: the owned plugin is currently inside
            // `ClapAudioProcessor`, so we can't call `save_state` directly.
            // Serialize params from the shared atomics and merge any
            // extra-state saver's output using the same `"extra" ->
            // top-level` shape the plugin would produce.
            let temp_params: Vec<crate::param::TempParamOwned> = self
                .shared
                .param_metas
                .iter()
                .enumerate()
                .map(|(i, meta)| crate::param::TempParamOwned {
                    id: meta.str_id.clone(),
                    value: self.shared.get_value(i),
                })
                .collect();
            let refs: Vec<&dyn Param> = temp_params.iter().map(|p| p as &dyn Param).collect();
            let mut json = crate::state::params_to_json(&refs);
            if let Some(saver) = &self.extra_state_saver {
                if let Some(obj) = json.as_object_mut() {
                    for (k, v) in saver.save() {
                        obj.insert(k, v);
                    }
                }
            }
            serde_json::to_vec(&json).unwrap_or_default()
        };
        output
            .write_all(&data)
            .map_err(|_| PluginError::Message("Failed to write state"))?;
        Ok(())
    }

    fn load(&mut self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut data = Vec::new();
        input
            .read_to_end(&mut data)
            .map_err(|_| PluginError::Message("Failed to read state"))?;

        if let Some(plugin) = &mut self.plugin {
            if !plugin.load_state(&data) {
                return Err(PluginError::Message("Failed to load state"));
            }
            // Sync loaded values back to shared atomics
            for i in 0..plugin.param_count() {
                if i < self.shared.param_values.len() {
                    self.shared.set_value(i, plugin.param(i).get_plain());
                }
            }
        } else {
            // Audio-processor path: parse once, load params into shared
            // atomics, and hand the parsed value to the extra-state saver
            // so file paths etc. land in their shared storage.
            //
            // Threading: `state::load` is [main-thread] and CLAP allows it
            // to run while `process` ([audio-thread]) is in flight. The
            // synchronization is per-param atomics plus the `params_dirty`
            // flag: values are stored first (Relaxed), then the flag with
            // Release; the audio thread swaps the flag with Acquire at the
            // top of each block, so once it observes the flag, every loaded
            // value is visible and overwrites the plugin's params. Two
            // in-flight races remain and are benign or handled:
            //
            // - A `ParamValue` event in the same block stores into the same
            //   atomics. Host automation racing a host-initiated load has no
            //   defined winner; either serialization is acceptable.
            // - The audio thread's editor push-back could overwrite a slot
            //   loaded after this block's dirty-check. That path uses
            //   `compare_exchange_value` (see `process.rs`), so the
            //   concurrent main-thread write wins and the still-set dirty
            //   flag re-syncs the plugin next block.
            let state: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|_| PluginError::Message("Failed to load state"))?;
            if !crate::state::load_params_from_shared_json(
                &self.shared.param_metas,
                &self.shared.param_values,
                &state,
            ) {
                return Err(PluginError::Message("Failed to load state"));
            }
            if let Some(saver) = &self.extra_state_saver {
                saver.load(&state);
            }
            self.shared.params_dirty.store(true, Ordering::Release);
        }

        Ok(())
    }
}
