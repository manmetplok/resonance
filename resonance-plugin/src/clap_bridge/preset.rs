//! The preset form of a plugin's state (plugin-preset-library.md §6.7,
//! §9.2, slice P2) and loading a preset by location (§7, slice P5).
//!
//! - `clap.state-context` `save(FOR_PRESET)` writes what a preset holds:
//!   every param not marked `preset_excluded`, plus the sound-bearing extra
//!   state (`ExtraStateSaver::save_for_preset`) — no session or UI keys.
//!   `load(FOR_PRESET)` lays such a document over the current state
//!   ([`crate::presets::overlay_preset`]): session keys stay, a preset key
//!   the preset lacks is cleared, excluded params keep their value. Any
//!   other context is the plain `clap.state` save/load. This is how the
//!   host saves and recalls a first-party preset so that it sounds exactly
//!   as the plugin's own browser would make it.
//! - `clap.preset-load` `from_location` loads a preset the plugin owns:
//!   `PLUGIN` + load key = a factory preset id; `FILE` = a preset file.
//!   It is the same overlay, with the preset's identity, and then tells
//!   the host `loaded()` so its bar shows what is loaded.
//!
//! Both are `[main-thread]` in CLAP and run where `clap.state` runs: the
//! plugin object here (inactive) or the audio processor's shared atomics
//! (active), through the same `load_bytes`.

use std::ffi::CStr;
use std::io::{Read, Write};

use clack_extensions::preset_discovery::preset_data::Location;
use clack_extensions::preset_discovery::{HostPresetLoad, PluginPresetLoadImpl};
use clack_extensions::state_context::{PluginStateContextImpl, StateContextType};
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};

use super::shared::ClapMainThread;
use crate::plugin::ResonancePlugin;
use crate::presets::{self, PresetFile, PresetSource, PRESET_STATE_KEY};

impl<'a, P: ResonancePlugin> ClapMainThread<'a, P> {
    fn preset_keys(&self) -> &'static [&'static str] {
        self.extra_state_saver
            .as_ref()
            .map(|s| s.preset_keys())
            .unwrap_or(&[])
    }

    fn is_preset_excluded(&self, id: &str) -> bool {
        self.shared
            .param_metas
            .iter()
            .any(|m| m.preset_excluded && m.str_id == id)
    }

    /// The preset form of the current state.
    pub(super) fn preset_bytes(&self) -> Vec<u8> {
        let full: serde_json::Value =
            serde_json::from_slice(&self.save_bytes()).unwrap_or_default();
        let mut doc = serde_json::Map::new();
        if let Some(v) = full.get(crate::state::VERSION_KEY) {
            doc.insert(crate::state::VERSION_KEY.into(), v.clone());
        }
        if let Some(params) = full.get("params").and_then(|p| p.as_object()) {
            let kept: serde_json::Map<String, serde_json::Value> = params
                .iter()
                .filter(|(id, _)| !self.is_preset_excluded(id))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            doc.insert("params".into(), serde_json::Value::Object(kept));
        }
        if let Some(saver) = &self.extra_state_saver {
            for (k, v) in saver.save_for_preset() {
                doc.insert(k, v);
            }
        }
        serde_json::to_vec(&serde_json::Value::Object(doc)).unwrap_or_default()
    }

    /// Lay a preset document (bare state or a whole preset file) over the
    /// current state and load the result.
    pub(super) fn load_preset_bytes(&mut self, data: &[u8]) -> Result<(), PluginError> {
        let mut doc: serde_json::Value = serde_json::from_slice(data)
            .map_err(|_| PluginError::Message("Preset is not JSON"))?;
        let identity = doc.get(PRESET_STATE_KEY).cloned();
        crate::state::migrate(&mut doc, self.shared.param_renames);
        if let (Some(identity), Some(obj)) = (identity, doc.as_object_mut()) {
            obj.entry(PRESET_STATE_KEY.to_string()).or_insert(identity);
        }
        if doc.get("params").and_then(|p| p.as_object()).is_none() {
            return Err(PluginError::Message("Preset carries no params"));
        }
        let mut current: serde_json::Value =
            serde_json::from_slice(&self.save_bytes()).unwrap_or_default();
        let keys = self.preset_keys();
        let excluded: Vec<String> = self
            .shared
            .param_metas
            .iter()
            .filter(|m| m.preset_excluded)
            .map(|m| m.str_id.clone())
            .collect();
        presets::overlay_preset(&mut current, &doc, keys, &|id| {
            excluded.iter().any(|e| e == id)
        });
        let bytes = serde_json::to_vec(&current).unwrap_or_default();
        self.load_bytes(&bytes)
    }

    /// The preset document (with its identity) at `location`.
    fn preset_at(
        &self,
        location: Location<'_>,
        load_key: Option<&CStr>,
    ) -> Result<serde_json::Value, String> {
        let identity = |id: &str, name: &str, source: PresetSource| {
            serde_json::json!({
                "id": id, "name": name, "source": source.as_str(), "modified": false,
            })
        };
        match location {
            Location::Plugin => {
                let key = load_key
                    .and_then(|k| k.to_str().ok())
                    .ok_or("a factory preset needs a load key")?;
                let entry = P::FACTORY_PRESETS
                    .iter()
                    .find(|p| p.id == key)
                    .ok_or_else(|| format!("no factory preset {key:?}"))?;
                let mut doc = entry.state_doc();
                if let Some(obj) = doc.as_object_mut() {
                    obj.insert(
                        PRESET_STATE_KEY.into(),
                        identity(entry.id, entry.name, PresetSource::Factory),
                    );
                }
                Ok(doc)
            }
            Location::File { path } => {
                let path = std::path::PathBuf::from(
                    path.to_str().map_err(|_| "preset path is not UTF-8")?,
                );
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("read {}: {e}", path.display()))?;
                let value: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("not JSON: {e}"))?;
                if presets::format::is_envelope(&value) {
                    let file = PresetFile::from_value(value)?;
                    if !file.plugin.id.is_empty() && file.plugin.id != P::CLAP_ID {
                        return Err(format!("a preset for {}", file.plugin.id));
                    }
                    let mut doc = file.state.doc.clone().ok_or("no state document")?;
                    if let Some(obj) = doc.as_object_mut() {
                        obj.insert(
                            PRESET_STATE_KEY.into(),
                            identity(&file.id, &file.meta.name, PresetSource::User),
                        );
                    }
                    Ok(doc)
                } else {
                    Ok(value)
                }
            }
        }
    }
}

impl<'a, P: ResonancePlugin> PluginStateContextImpl for ClapMainThread<'a, P> {
    fn save(
        &mut self,
        output: &mut OutputStream,
        context_type: StateContextType,
    ) -> Result<(), PluginError> {
        let data = match context_type {
            StateContextType::ForPreset => self.preset_bytes(),
            _ => self.save_bytes(),
        };
        output
            .write_all(&data)
            .map_err(|_| PluginError::Message("Failed to write state"))
    }

    fn load(
        &mut self,
        input: &mut InputStream,
        context_type: StateContextType,
    ) -> Result<(), PluginError> {
        let mut data = Vec::new();
        input
            .read_to_end(&mut data)
            .map_err(|_| PluginError::Message("Failed to read state"))?;
        match context_type {
            StateContextType::ForPreset => self.load_preset_bytes(&data),
            _ => self.load_bytes(&data),
        }
    }
}

impl<'a, P: ResonancePlugin> PluginPresetLoadImpl for ClapMainThread<'a, P> {
    fn load_from_location(
        &mut self,
        location: Location,
        load_key: Option<&CStr>,
    ) -> Result<(), PluginError> {
        let result = self
            .preset_at(location, load_key)
            .and_then(|doc| {
                let bytes = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
                self.load_preset_bytes(&bytes)
                    .map_err(|_| "the preset did not load".to_string())
            });
        let host_ext = self.host.shared().get_extension::<HostPresetLoad>();
        match result {
            Ok(()) => {
                // The identity report (and, for a factory preset, CLAP's
                // `loaded()`) goes out now, from this main-thread call; a
                // file location is announced here as CLAP asks.
                self.report_preset_identity();
                if let (Location::File { .. }, Some(ext)) = (location, host_ext) {
                    ext.loaded(&mut self.host, location, load_key);
                }
                Ok(())
            }
            Err(message) => {
                tracing::warn!("{}: preset load failed: {message}", P::CLAP_ID);
                if let Some(ext) = host_ext {
                    let text = std::ffi::CString::new(message).ok();
                    ext.on_error(&mut self.host, location, load_key, 0, text.as_deref());
                }
                Err(PluginError::Message("Preset load failed"))
            }
        }
    }
}
