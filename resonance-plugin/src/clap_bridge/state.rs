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
/// Carries the metadata the preset-modified comparison reads, too.
pub(super) struct TempParamOwned {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) value: f64,
    pub(super) min: f64,
    pub(super) max: f64,
    pub(super) clap_id: u32,
    pub(super) preset_excluded: bool,
    pub(super) state_excluded: bool,
}

impl TempParamOwned {
    /// Every param, with the value `value(slot)` gives it.
    pub(super) fn all_from(
        shared: &super::shared::ClapShared<'_>,
        value: impl Fn(usize) -> f64,
    ) -> Vec<Self> {
        shared
            .param_metas
            .iter()
            .enumerate()
            .map(|(i, meta)| TempParamOwned {
                id: meta.str_id.clone(),
                name: meta.name.clone(),
                value: value(i),
                min: meta.min,
                max: meta.max,
                // The plugin-side id (`Param::clap_id`'s default), like
                // the plugin's own params report — not the rename-pinned
                // wire id: the preset comparison matches these against
                // ids translated the same way.
                clap_id: crate::stable_hash(&meta.str_id),
                preset_excluded: meta.preset_excluded,
                state_excluded: meta.state_excluded,
            })
            .collect()
    }
}

impl Param for TempParamOwned {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn get_plain(&self) -> f64 {
        self.value
    }
    fn set_plain(&self, _v: f64) {}
    fn default_plain(&self) -> f64 {
        self.value
    }
    fn min_plain(&self) -> f64 {
        self.min
    }
    fn max_plain(&self) -> f64 {
        self.max
    }
    fn clap_id(&self) -> u32 {
        self.clap_id
    }
    fn preset_excluded(&self) -> bool {
        self.preset_excluded
    }
    fn state_excluded(&self) -> bool {
        self.state_excluded
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
        let data = self.save_bytes();
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
        self.load_bytes(&data)
    }
}

impl<'a, P: ResonancePlugin> ClapMainThread<'a, P> {
    /// A param's current value as the host should see it — what
    /// `params.get_value` answers and what `state.save` and the
    /// preset-modified comparison serialise (HOST-01). `[main-thread]`.
    ///
    /// * Inactive: the plugin object is here, and it is the newer side
    ///   (an editor writes only the plugin's own params).
    /// * Active, a load published and not yet applied (`params_dirty`, or
    ///   a publication window open): the mirror. It holds the load; the
    ///   plugin still holds what the load replaces.
    /// * Active otherwise: the plugin's **live** value when it hands the
    ///   bridge one ([`ParamTextSource::live_value`]), else the mirror. The
    ///   mirror catches up with editor writes only when the audio thread
    ///   pushes them back — every `process()` and every `params.flush` —
    ///   so with the transport stopped and no flush, a plugin that gives
    ///   no live value reads its last pushed-back value.
    ///
    /// A read-only or state-excluded param is not part of any load, so it
    /// reads live even while one is pending.
    ///
    /// [`ParamTextSource::live_value`]: crate::plugin::ParamTextSource::live_value
    pub(super) fn current_value(&self, slot: usize) -> f64 {
        if let Some(plugin) = &self.plugin {
            if slot < plugin.param_count() {
                return plugin.param(slot).get_plain();
            }
        }
        let meta = &self.shared.param_metas[slot];
        let loaded_not_applied = self.shared.params_dirty.load(Ordering::Acquire)
            || self.shared.param_publish_gen() & 1 == 1;
        if loaded_not_applied && !meta.is_read_only && !meta.state_excluded {
            return self.shared.get_value(slot);
        }
        self.param_text_source
            .as_ref()
            .and_then(|s| s.live_value(slot))
            .filter(|v| v.is_finite())
            .unwrap_or_else(|| self.shared.get_value(slot))
    }

    /// Every param at [`Self::current_value`], for a serialisation made
    /// while the plugin object is in the audio processor.
    pub(super) fn current_params(&self) -> Vec<TempParamOwned> {
        TempParamOwned::all_from(self.shared, |slot| self.current_value(slot))
    }

    /// The full state document, whether the plugin object is here or in
    /// the audio processor. Shared by `clap.state` and the preset form.
    pub(super) fn save_bytes(&self) -> Vec<u8> {
        if let Some(plugin) = &self.plugin {
            // Main-thread path: the plugin's own `save_state` composes
            // params with any extra-state saver via the trait default.
            plugin.save_state()
        } else {
            // Audio-processor path: the owned plugin is currently inside
            // `ClapAudioProcessor`, so we can't call `save_state` directly.
            // Serialize params at their current values (live where the
            // plugin hands them over, else the shared mirror — see
            // `current_value`) and merge any extra-state saver's output
            // using the same `"extra" -> top-level` shape the plugin would
            // produce.
            let temp_params = self.current_params();
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
        }
    }

    /// Load a full state document, whether the plugin object is here or in
    /// the audio processor. Shared by `clap.state` and the preset form.
    pub(super) fn load_bytes(&mut self, data: &[u8]) -> Result<(), PluginError> {
        if let Some(plugin) = &mut self.plugin {
            if !plugin.load_state(data) {
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
            let mut state: serde_json::Value = serde_json::from_slice(data)
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
            // The plugin's own upgrade too (`STATE_UPGRADE`): a v1 state
            // loaded while active must arrive as converted as one loaded
            // inactive.
            crate::state::migrate_and_upgrade(
                &mut state,
                self.shared.param_renames,
                self.shared.state_upgrade,
            );

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
