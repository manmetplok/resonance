//! State extension for the CLAP bridge: preset/project save and load.

use std::io::{Read, Write};
use std::sync::atomic::Ordering;

use clack_extensions::state::PluginStateImpl;
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};

use super::shared::ClapMainThread;
use crate::param::Param;
use crate::plugin::ResonancePlugin;

/// Temporary param used for serialization when the plugin instance is active
/// (owned by `ClapAudioProcessor`) and not accessible from the main thread.
struct TempParamOwned {
    id: String,
    value: f64,
}

impl Param for TempParamOwned {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.id
    }
    fn get_plain(&self) -> f64 {
        self.value
    }
    fn set_plain(&self, _v: f64) {}
    fn default_plain(&self) -> f64 {
        self.value
    }
    fn min_plain(&self) -> f64 {
        0.0
    }
    fn max_plain(&self) -> f64 {
        1.0
    }
    fn display(&self, value: f64) -> String {
        format!("{:.4}", value)
    }
    fn parse(&self, _text: &str) -> Option<f64> {
        None
    }
}

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
            let temp_params: Vec<TempParamOwned> = self
                .shared
                .param_metas
                .iter()
                .enumerate()
                .map(|(i, meta)| TempParamOwned {
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
            // flag: every value is stored first (Relaxed), then the extra
            // state, then the flag with Release. The audio thread swaps the
            // flag with Acquire at the top of each block, so once it
            // observes the flag, the whole load — values and extra state —
            // is visible, and the values overwrite the plugin's params.
            //
            // The flag alone was never enough, though. It is stored
            // *after* the values, so between a value's store and the flag
            // a loaded value sat in the atomics with nothing announcing
            // it, and the audio thread's editor push-back — which had only
            // that flag and a compare-exchange to go on — could overwrite
            // it with the plugin's stale one. Permanently: the flag that
            // followed then copied the overwritten value into the plugin.
            // ba todo #1363 narrowed that window; it did not close it.
            //
            // What closes it is `params_gen`, bumped either side of this
            // whole block (ba todo #1374). Odd means "a load is publishing
            // right now", and the push-back reads it around its own store
            // and stands down when it is odd or when it moved. So there is
            // no longer any moment at which a value stored here is
            // exposed: before the flag it is covered by the generation,
            // after the flag by the flag itself.
            //
            // Two in-flight races remain and are benign or handled:
            //
            // - A `ParamValue` event in the same block stores into the same
            //   atomics. Host automation racing a host-initiated load has no
            //   defined winner; either serialization is acceptable.
            // - The generation check and the push-back's compare-exchange
            //   are two instructions, not one, so a load that stores a
            //   slot the value it already held — indistinguishable from
            //   not storing it at all — can still lose that slot to a
            //   pending editor edit. See the note at the push-back in
            //   `process.rs` for why that case changes nothing the host
            //   reads back or re-saves.
            //
            // Ordering of the two halves is therefore no longer a trade.
            // It used to be one: the saver ran *before* the param stores
            // purely to keep its unbounded work (reading an IR off disk,
            // rebuilding a wavetable) out of the window in which a stored
            // value could still be clobbered (ba todo #1363), at the cost
            // of announcing the extra state well ahead of the params it
            // belongs with. With the window closed, the order is chosen
            // for the reason it should be — matching the inactive path,
            // whose `ResonancePlugin::load_state` default loads params and
            // then calls the saver — and the two halves now reach the
            // audio thread as one transition: the params are invisible
            // until the flag, which is stored once the saver has returned,
            // and the generation keeps the audio thread from applying
            // either half early.
            let mut state: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|_| PluginError::Message("Failed to load state"))?;
            // Migrate first, exactly as the inactive path's
            // `ResonancePlugin::load_state` default does, so both the
            // params and the extra-state saver below see the *migrated*
            // blob. Calling `state::migrate` here rather than re-deriving
            // renames from the param metadata is deliberate — it is the
            // one place that knows a rename is version-gated and that
            // chained renames apply oldest-first, and two copies of those
            // rules is exactly how this path came to ignore renames in the
            // first place (ba todo #1360).
            crate::state::migrate(&mut state, self.shared.param_renames);

            // Everything from here to `end_param_publish` is one
            // transition as far as the audio thread is concerned: it will
            // neither apply a half-published load nor write into one.
            //
            // The window is a guard so it closes even if something in it
            // unwinds: a generation left odd would disable the re-sync and
            // the editor push-back for this instance forever (PLG-06).
            let publish = self.shared.param_publish_guard();

            let params_ok = crate::state::load_params_from_shared_json(
                &self.shared.param_metas,
                &self.shared.param_values,
                &state,
            );

            // Then the extra state, in the same order as the inactive
            // path's `ResonancePlugin::load_state` default. `load()` is
            // arbitrary plugin work — reading an IR off disk, rebuilding a
            // wavetable — so it can span many audio blocks; ba todo #1363
            // hoisted it above the param stores to keep that unbounded
            // stretch out of the window where a stored-but-unannounced
            // value could be clobbered. The generation this block is
            // wrapped in closes that window however long it stays open, so
            // the hoist has nothing left to buy and the natural order is
            // back (ba todo #1374).
            //
            // It runs whether or not the params parsed, which is what the
            // inactive path has always done (`load_state` calls the saver
            // regardless of the param result). Otherwise the same file
            // would load differently depending on whether the transport
            // happened to be running — the divergence this whole path
            // exists to avoid — so the failure below is reported *after*
            // the saver has had the blob.
            //
            // The saver is arbitrary plugin code, so a panic in it is
            // caught here rather than unwound through: the params half has
            // already been stored and is still announced below — what the
            // inactive path leaves behind too, since its `load_state` has
            // set the params before it calls the saver — and the load is
            // then reported as failed (PLG-06).
            let saver_ok = match &self.extra_state_saver {
                Some(saver) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    saver.load(&state)
                }))
                .is_ok(),
                None => true,
            };

            if params_ok {
                self.shared.params_dirty.store(true, Ordering::Release);
            }
            drop(publish);
            if !saver_ok {
                return Err(PluginError::Message("Extra state failed to load"));
            }
            if !params_ok {
                // Nothing was stored, so there was nothing to announce.
                return Err(PluginError::Message("Failed to load state"));
            }
        }

        Ok(())
    }
}
