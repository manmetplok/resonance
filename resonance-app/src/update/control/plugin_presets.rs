//! Plugin presets over the control API, shared by all three chain
//! surfaces (ba todo #1333).
//!
//! `track.*`, `bus.*` and `master.*` each declare their own three
//! methods, because addressing a plugin differs per surface. Everything
//! after "which plugin instance is this?" is the same, and lives here.
//!
//! # Where the two sets come from
//!
//! **Factory** presets are compiled into the plugin binary. The scan
//! reads them from the first-party `resonance_factory_presets` symbol and
//! parks them on [`ScannedPlugin::factory_presets`], so listing them costs
//! nothing and works before a plugin is even instantiated.
//!
//! **User** presets are files under the documented preset directory, read
//! through the very same [`PresetBank`] the plugin's own window uses — so
//! a preset saved in the GUI is listed here, and one saved here appears in
//! the GUI, without either side knowing about the other.
//!
//! # Why a recall is applied parameter by parameter
//!
//! The engine has `LoadPluginState`, and handing the blob to the plugin
//! would be less code. It would also desynchronise the app: its parameter
//! mirror is filled once at instantiation and updated only for changes
//! that went through `SetPluginParam`, so a plugin-side load moves the
//! sound while `track.plugin_params` keeps reporting the old values (the
//! gap ba todo #1294 closes). Applying the preset through the app's own
//! path keeps the mirror, the engine and the plugin window agreeing, and
//! [`PluginMessage::LoadPluginPreset`] carries the whole set so it is
//! still one undo entry.
//!
//! A preset's JSON is keyed by each parameter's **string** id (`"mix"`),
//! while the app's mirror knows the CLAP numeric id. They are the same
//! thing hashed: `clap_id == stable_hash(string_id)`, which is how the
//! plugin bridge derives it, so [`resonance_plugin::stable_hash`] is the
//! translation.

use crate::message::{Message, PluginMessage};
use crate::Resonance;
use resonance_audio::types::PluginInstanceId;
use resonance_control::methods::plugin_preset::{
    PluginPresetEntry, PluginPresetSource, PluginPresetsView,
};
use resonance_control::methods::track::PluginParamView;
use resonance_control::RpcError;
use resonance_plugin::presets::{PresetBank, PresetRef};

/// The bank for one plugin: its baked-in factory presets, plus this
/// user's own directory for it.
///
/// Built per call rather than cached — it is a plugin id and a path, and
/// the user's directory can change under us at any time (they may have
/// saved from the plugin's own window a second ago).
fn bank_for(app: &Resonance, clap_id: &str) -> PresetBank {
    // The bank carries the *user* half only. `PresetBank` takes
    // `&'static [FactoryPreset]` because inside a plugin the factory bank
    // is a compile-time constant; the host's copy arrived over a channel
    // at runtime, so it is kept beside the bank in
    // [`factory_presets`] rather than forced into it.
    let bank = PresetBank::new(clap_id, &[]);
    match &app.plugin_preset_root {
        Some(root) => bank.with_root(root.clone()),
        None => bank,
    }
}

/// Factory presets for `clap_id`, as `(name, state json)`.
fn factory_presets(app: &Resonance, clap_id: &str) -> Vec<(String, String)> {
    app.available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == clap_id)
        .map(|p| p.factory_presets.clone())
        .unwrap_or_default()
}

/// Every preset available for one plugin instance, factory first.
pub(crate) fn view(app: &Resonance, clap_id: &str) -> PluginPresetsView {
    let mut presets: Vec<PluginPresetEntry> = factory_presets(app, clap_id)
        .into_iter()
        .map(|(name, _)| PluginPresetEntry {
            name,
            source: PluginPresetSource::Factory,
        })
        .collect();
    presets.extend(
        bank_for(app, clap_id)
            .list_user()
            .into_iter()
            .map(|p| PluginPresetEntry {
                name: p.name,
                source: PluginPresetSource::User,
            }),
    );

    PluginPresetsView {
        plugin_id: clap_id.to_string(),
        presets,
        // The loaded-preset identity lives inside the plugin's own state
        // and does not reach the app (ba todo #1294 is the same gap).
        // Reporting `None` is the honest answer; inventing one from the
        // last preset this API loaded would go wrong the moment the user
        // touched a knob in the plugin's window.
        current: None,
        modified: false,
    }
}

/// The state JSON behind one preset, or the error explaining why there is
/// none.
///
/// `source` picks a set; omitted prefers the user's own, so a preset
/// deliberately saved over a factory name wins unless the caller asks for
/// the factory original by name.
fn json_for(
    app: &Resonance,
    clap_id: &str,
    preset: &str,
    source: Option<PluginPresetSource>,
) -> Result<String, RpcError> {
    let wanted = preset.trim();
    let factory = factory_presets(app, clap_id);
    let bank = bank_for(app, clap_id);

    let user = |bank: &PresetBank| {
        bank.list_user()
            .into_iter()
            .find(|p| p.name.eq_ignore_ascii_case(wanted))
            .and_then(|p| bank.json_for(&PresetRef::user(p.name)))
    };
    let fact = || {
        factory
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, json)| json.clone())
    };

    let found = match source {
        Some(PluginPresetSource::User) => user(&bank),
        Some(PluginPresetSource::Factory) => fact(),
        None => user(&bank).or_else(fact),
    };

    found.ok_or_else(|| {
        let mut known: Vec<String> = factory.iter().map(|(n, _)| n.clone()).collect();
        known.extend(bank.list_user().into_iter().map(|p| p.name));
        RpcError::not_found(format!(
            "plugin {clap_id:?} has no preset {wanted:?} (has: [{}])",
            known.join(", ")
        ))
    })
}

/// Build the one-edit recall message for a preset, or explain why not.
///
/// `params` is the plugin's parameter list as the app mirrors it, which
/// is what maps the preset's string ids onto CLAP ids and lets an unknown
/// id be reported rather than silently dropped.
pub(crate) fn load_message(
    app: &Resonance,
    clap_id: &str,
    instance_id: PluginInstanceId,
    params: &[PluginParamView],
    preset: &str,
    source: Option<PluginPresetSource>,
) -> Result<Message, RpcError> {
    let json = json_for(app, clap_id, preset, source)?;
    let document: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| RpcError::internal(format!("preset {preset:?} is not valid JSON: {e}")))?;
    let Some(map) = document.get("params").and_then(|v| v.as_object()) else {
        return Err(RpcError::internal(format!(
            "preset {preset:?} carries no params object"
        )));
    };

    // Index the mirror by CLAP id once, so a 90-parameter preset is not
    // a quadratic walk.
    let known: std::collections::HashSet<u32> = params.iter().map(|p| p.id).collect();
    let mut values = Vec::with_capacity(map.len());
    let mut unknown = Vec::new();
    for (string_id, value) in map {
        let Some(value) = value.as_f64() else { continue };
        let clap_id_hash = resonance_plugin::stable_hash(string_id);
        if known.contains(&clap_id_hash) {
            values.push((clap_id_hash, value));
        } else {
            unknown.push(string_id.clone());
        }
    }

    if values.is_empty() {
        return Err(RpcError::internal(format!(
            "preset {preset:?} names none of this plugin's parameters — it was \
             probably written for a different plugin (its keys: [{}])",
            unknown.join(", ")
        )));
    }

    // A preset from an older build may name parameters this plugin no
    // longer has. That is exactly the case `ParamRename` migration
    // handles inside the plugin, and dropping the strays is what the
    // plugin's own loader does too, so recall the rest rather than
    // refusing the lot.
    Ok(Message::Plugin(PluginMessage::LoadPluginPreset {
        instance_id,
        values,
        preset_name: preset.trim().to_string(),
    }))
}

/// Check a save is allowed before anything is written.
///
/// Overwriting an existing user preset is destructive, so it takes an
/// explicit `overwrite`, per the control API's convention. Factory
/// presets are never touched: saving under a factory name creates a user
/// preset that shadows it, which is what the plugin's own window does.
pub(crate) fn check_save(
    app: &Resonance,
    clap_id: &str,
    name: &str,
    overwrite: bool,
) -> Result<(), RpcError> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return Err(RpcError::invalid_params("a preset needs a name"));
    }
    let exists = bank_for(app, clap_id)
        .list_user()
        .into_iter()
        .any(|p| p.name.eq_ignore_ascii_case(wanted));
    if exists && !overwrite {
        return Err(RpcError::needs_confirmation(format!(
            "a user preset named {wanted:?} already exists for {clap_id:?}; \
             pass overwrite: true to replace it"
        )));
    }
    Ok(())
}

/// Write the plugin's saved state blob as a user preset.
///
/// Called when the engine's `PluginStateSaved` echo lands, because the
/// plugin is the only thing that knows its current state: the app's
/// parameter mirror does not see edits made in the plugin's own window
/// (ba todo #1294), and saving from it would quietly capture the wrong
/// sound.
pub(crate) fn write_saved_state(
    app: &Resonance,
    clap_id: &str,
    name: &str,
    blob: &[u8],
) -> Result<(), String> {
    let text = std::str::from_utf8(blob).map_err(|e| format!("plugin state is not utf-8: {e}"))?;
    let document: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("plugin state is not JSON: {e}"))?;

    bank_for(app, clap_id)
        .write_user_preset(name.trim(), &document)
        .map(|_| ())
}
