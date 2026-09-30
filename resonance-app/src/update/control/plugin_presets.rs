//! Plugin presets over the control API, shared by all three chain
//! surfaces (ba todo #1333).
//!
//! `track.*`, `bus.*` and `master.*` each declare their own three
//! methods, because addressing a plugin differs per surface. Everything
//! after "which plugin instance is this?" is the same, and lives here.
//!
//! # One library
//!
//! Both sets come from the same `PresetLibrary` the plugin's own window
//! uses ([`crate::plugin_preset_library`]): **factory** presets from the
//! scan (read from each first-party binary's `resonance_factory_presets`
//! symbol, ids and metadata included), **user** presets from the preset
//! directory. A preset saved, starred or renamed on either side is on the
//! other.
//!
//! # The whole sound (slice P2)
//!
//! A recall sets the params through the app's own path (below) and then
//! hands the plugin the preset's state document with its identity
//! (`AudioCommand::LoadPluginPresetState`): a first-party plugin lays it
//! over its current state through `clap.state-context` (`FOR_PRESET`), so
//! a model, an IR or user wavetables come along and the plugin's bar names
//! the preset. A save captures the plugin's preset form
//! (`SavePluginPresetState`), not the whole project state.
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
    FacetCount, PluginPresetEntry, PluginPresetSource, PluginPresetsView, PresetFacets,
    PresetFilter, PresetMetaInput, PresetSort,
};
use resonance_control::methods::track::PluginParamView;
use resonance_control::RpcError;
use resonance_plugin::presets::{
    Hit, PresetBank, PresetMeta, PresetRecord, PresetRef, PresetSource, Query, Sort,
    PRESET_STATE_KEY,
};

/// The bank for one plugin over the app's library.
pub(crate) fn bank_for(app: &Resonance, clap_id: &str) -> PresetBank {
    crate::plugin_preset_library::bank(app, clap_id)
}

pub(crate) fn wire_source(source: PresetSource) -> PluginPresetSource {
    match source {
        PresetSource::Factory => PluginPresetSource::Factory,
        PresetSource::User => PluginPresetSource::User,
    }
}

fn library_source(source: PluginPresetSource) -> PresetSource {
    match source {
        PluginPresetSource::Factory => PresetSource::Factory,
        PluginPresetSource::User => PresetSource::User,
    }
}

/// The wire entry for one search hit: content metadata plus marks.
pub(crate) fn entry_from_hit(hit: &Hit) -> PluginPresetEntry {
    let r = &hit.record;
    let m = &r.meta;
    PluginPresetEntry {
        name: m.name.clone(),
        source: wire_source(r.preset.source),
        id: r.preset.id.clone(),
        category: m.category.clone(),
        instrument: m.instrument.clone(),
        genres: m.genres.clone(),
        character: m.character.clone(),
        tags: hit.tags.clone(),
        personal_tags: hit.personal_tags.clone(),
        favorite: hit.favorite,
        author: m.author.clone(),
        description: m.description.clone(),
        plugin_version: r.plugin_version.clone(),
        modified_at: m.modified.clone(),
        derived_from: m.derived_from.clone(),
        last_used: hit.last_used.clone(),
    }
}

/// A wire filter as a library query over `plugins` (empty = every plugin).
pub(crate) fn query_for(filter: &PresetFilter, plugins: Vec<String>) -> Query {
    Query {
        text: filter.query.clone().unwrap_or_default(),
        plugins,
        sources: filter.source.map(library_source).into_iter().collect(),
        favorites_only: filter.favorites_only,
        category: filter.category.clone().into_iter().collect(),
        instrument: filter.instrument.clone(),
        genres: filter.genres.clone(),
        character: filter.character.clone(),
        tags: filter.tags.clone(),
        sort: match filter.sort.unwrap_or_default() {
            PresetSort::Bank => Sort::Bank,
            PresetSort::Name => Sort::Name,
            PresetSort::Category => Sort::Category,
            PresetSort::Recent => Sort::RecentlyUsed,
            PresetSort::Modified => Sort::RecentlyModified,
        },
        favorites_first: false,
    }
}

/// Facet counts on the wire.
pub(crate) fn wire_facets(f: &resonance_plugin::presets::Facets) -> PresetFacets {
    let conv = |v: &[(String, usize)]| -> Vec<FacetCount> {
        v.iter()
            .map(|(value, count)| FacetCount {
                value: value.clone(),
                count: *count as u32,
            })
            .collect()
    };
    PresetFacets {
        source: conv(&f.source),
        category: conv(&f.category),
        instrument: conv(&f.instrument),
        genres: conv(&f.genres),
        character: conv(&f.character),
        tags: conv(&f.tags),
    }
}

/// Default page size of a preset list.
pub(crate) const DEFAULT_LIMIT: u32 = 100;

/// Every preset available for one plugin instance, filtered and sorted
/// (factory first in bank order by default), with facet counts.
pub(crate) fn view(
    app: &Resonance,
    clap_id: &str,
    instance_id: PluginInstanceId,
    filter: &PresetFilter,
) -> PluginPresetsView {
    let lib = crate::plugin_preset_library::library(app);
    // Make sure the bank (and so the factory registration) exists.
    let _ = bank_for(app, clap_id);
    let result = lib.query(&query_for(filter, vec![clap_id.to_string()]));
    let total = result.hits.len() as u32;
    let offset = filter.offset.unwrap_or(0) as usize;
    let limit = filter.limit.unwrap_or(DEFAULT_LIMIT) as usize;
    let presets = result
        .hits
        .iter()
        .skip(offset)
        .take(limit)
        .map(entry_from_hit)
        .collect();
    // The loaded preset, as the plugin last reported it (or as the host
    // last loaded it, for a plugin that does not report): the full entry
    // when the library still has it, a bare one when it does not.
    let identity = app.presets.plugin_preset_identity.get(&instance_id);
    let current = identity.map(|i| {
        let full = lib.query(&query_for(&PresetFilter::default(), vec![clap_id.to_string()]));
        full.hits
            .iter()
            .find(|h| h.record.preset.id == i.id && wire_source(h.record.preset.source) == i.source)
            .map(entry_from_hit)
            .unwrap_or_else(|| PluginPresetEntry {
                name: i.name.clone(),
                source: i.source,
                id: i.id.clone(),
                ..PluginPresetEntry::default()
            })
    });
    PluginPresetsView {
        plugin_id: clap_id.to_string(),
        presets,
        current,
        modified: identity.is_some_and(|i| i.modified),
        modified_known: identity.is_some_and(|i| i.reported),
        total,
        facets: wire_facets(&result.facets),
    }
}

/// The preset a name (and optional source) addresses: user presets first
/// unless the caller asks for the factory original, names compared
/// case-insensitively (they are unique per plugin among user presets).
/// A `preset_id` wins over the name.
pub(crate) fn find(
    app: &Resonance,
    clap_id: &str,
    preset: &str,
    preset_id: Option<&str>,
    source: Option<PluginPresetSource>,
) -> Result<(PresetBank, PresetRef), RpcError> {
    let wanted = preset.trim();
    let bank = bank_for(app, clap_id);
    let all = bank.list();
    if let Some(id) = preset_id.filter(|id| !id.trim().is_empty()) {
        let hit = all
            .iter()
            .find(|p| p.id == id.trim() && source.is_none_or(|s| library_source(s) == p.source))
            .cloned();
        return match hit {
            Some(p) => Ok((bank, p)),
            None => Err(RpcError::not_found(format!(
                "plugin {clap_id:?} has no preset with id {id:?}"
            ))),
        };
    }
    let pick = |s: PresetSource| {
        all.iter()
            .find(|p| p.source == s && p.name.eq_ignore_ascii_case(wanted))
            .cloned()
    };
    let found = match source {
        Some(PluginPresetSource::User) => pick(PresetSource::User),
        Some(PluginPresetSource::Factory) => pick(PresetSource::Factory),
        None => pick(PresetSource::User).or_else(|| pick(PresetSource::Factory)),
    };
    match found {
        Some(p) => Ok((bank, p)),
        None => {
            let known: Vec<String> = all.into_iter().map(|p| p.name).collect();
            Err(RpcError::not_found(format!(
                "plugin {clap_id:?} has no preset {wanted:?} (has: [{}])",
                known.join(", ")
            )))
        }
    }
}

/// The preset a `preset` argument on `*.add_effect` / `add_instrument`
/// names: an id first, then a name (user before factory).
pub(crate) fn resolve_add_preset(
    app: &Resonance,
    clap_id: &str,
    preset: &str,
) -> Result<PresetRef, RpcError> {
    find(app, clap_id, preset, Some(preset), None)
        .or_else(|_| find(app, clap_id, preset, None, None))
        .map(|(_, found)| found)
}

/// Park `found` for the plugin being added as `instance_id`; the
/// `PluginAdded` echo loads it ([`apply_pending_preset`]).
pub(crate) fn park_add_preset(
    app: &mut Resonance,
    instance_id: PluginInstanceId,
    clap_id: &str,
    found: &PresetRef,
) {
    if let Err(e) = bank_for(app, clap_id).library().record_use(clap_id, &found.id) {
        tracing::debug!("presets: recents not recorded: {e}");
    }
    app.presets.pending_plugin_presets.insert(
        instance_id,
        (clap_id.to_string(), found.id.clone(), wire_source(found.source)),
    );
}

/// Load the preset parked for a just-added plugin, now that its params
/// are known. Unrecorded: the add it belongs to is the undo step.
pub(crate) fn apply_pending_preset(app: &mut Resonance, instance_id: PluginInstanceId) {
    let Some((clap_id, preset_id, source)) = app.presets.pending_plugin_presets.remove(&instance_id)
    else {
        return;
    };
    let Some(params) = app.with_plugin_mut(instance_id, |slot| {
        slot.params
            .iter()
            .map(super::view_model::param_view)
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    let message = find(app, &clap_id, "", Some(&preset_id), Some(source))
        .and_then(|(bank, found)| load_message_for(&bank, &found, instance_id, &params));
    match message {
        Ok(Message::Plugin(m)) => crate::update::plugin::apply_preset_load(app, m),
        Ok(_) => {}
        Err(e) => {
            app.banners.error_message = Some(format!("Could not load preset: {}", e.message));
        }
    }
}

/// The recall for one preset onto a plugin slot, by id: the host preset
/// surfaces' load (slice P6). The slot's params are read from the mirror.
pub(crate) fn host_load_message(
    app: &mut Resonance,
    instance_id: PluginInstanceId,
    clap_id: &str,
    preset_id: &str,
    source: PluginPresetSource,
) -> Result<Message, RpcError> {
    let params = app
        .with_plugin_mut(instance_id, |slot| {
            slot.params
                .iter()
                .map(super::view_model::param_view)
                .collect::<Vec<_>>()
        })
        .ok_or_else(|| RpcError::not_found(format!("no plugin instance {instance_id}")))?;
    let (bank, found) = find(app, clap_id, "", Some(preset_id), Some(source))?;
    load_message_for(&bank, &found, instance_id, &params)
}

/// A plugin's presets in bank order (factory as declared, then the user's
/// by name): what the bar's ◀ / ▶ walk.
pub(crate) fn bank_order(app: &Resonance, clap_id: &str) -> Vec<PresetRef> {
    bank_for(app, clap_id).list()
}

/// What a `*.load_plugin_preset` asks for.
pub(crate) struct LoadArgs<'a> {
    pub preset: &'a str,
    pub preset_id: Option<&'a str>,
    pub source: Option<PluginPresetSource>,
    /// Load the sound-bearing extra state too (default true).
    pub extra: bool,
}

/// Resolve and build a recall for a control-API load, and record the pick
/// in the user's recents (a control-API load is a user pick, §4.5).
pub(crate) fn load_request(
    app: &Resonance,
    clap_id: &str,
    instance_id: PluginInstanceId,
    params: &[PluginParamView],
    args: &LoadArgs<'_>,
) -> Result<Message, RpcError> {
    let (bank, found) = find(app, clap_id, args.preset, args.preset_id, args.source)?;
    let mut message = load_message_for(&bank, &found, instance_id, params)?;
    if !args.extra {
        // An opaque (third-party) preset has no params to recall on their
        // own: `extra: false` cannot split it, so it loads whole.
        if let Message::Plugin(PluginMessage::LoadPluginPreset {
            preset_state,
            values,
            ..
        }) = &mut message
        {
            if !values.is_empty() {
                *preset_state = None;
            }
        }
    }
    if let Err(e) = bank.library().record_use(clap_id, &found.id) {
        tracing::debug!("presets: recents not recorded: {e}");
    }
    Ok(message)
}

/// A recall for a preset already resolved.
///
/// `params` is the plugin's parameter list as the app mirrors it, which
/// is what maps the preset's string ids onto CLAP ids and lets an unknown
/// id be reported rather than silently dropped.
pub(crate) fn load_message_for(
    bank: &PresetBank,
    found: &PresetRef,
    instance_id: PluginInstanceId,
    params: &[PluginParamView],
) -> Result<Message, RpcError> {
    let preset = found.name.as_str();
    // A third-party preset is opaque: the whole thing goes to the plugin
    // and the mirror follows the engine's refresh (§8 tier T0).
    if let Some(blob) = bank.blob_for(found) {
        return Ok(Message::Plugin(PluginMessage::LoadPluginPreset {
            instance_id,
            values: Vec::new(),
            preset_name: found.name.clone(),
            preset_state: Some(blob),
            preset_id: found.id.clone(),
            preset_source: wire_source(found.source),
        }));
    }
    let json = bank
        .json_for(found)
        .ok_or_else(|| RpcError::not_found(format!("preset {preset:?} is gone")))?;
    let mut document: serde_json::Value = serde_json::from_str(&json)
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

    // The whole sound, with the identity the plugin's own bar shows.
    if let Some(obj) = document.as_object_mut() {
        obj.insert(
            PRESET_STATE_KEY.to_string(),
            serde_json::json!({
                "id": found.id,
                "name": found.name,
                "source": found.source.as_str(),
                "modified": false,
            }),
        );
    }
    let preset_state = serde_json::to_vec(&document).ok();

    // A preset from an older build may name parameters this plugin no
    // longer has. That is exactly the case `ParamRename` migration
    // handles inside the plugin, and dropping the strays is what the
    // plugin's own loader does too, so recall the rest rather than
    // refusing the lot.
    Ok(Message::Plugin(PluginMessage::LoadPluginPreset {
        instance_id,
        values,
        preset_name: found.name.clone(),
        preset_state,
        preset_id: found.id.clone(),
        preset_source: wire_source(found.source),
    }))
}

/// What a `*.save_plugin_preset` asks for.
pub(crate) struct SaveArgs {
    pub name: String,
    pub overwrite: bool,
    pub meta: Option<PresetMetaInput>,
    pub favorite: Option<bool>,
    pub overwrite_id: Option<String>,
}

/// Check a save is allowed before anything is written, and return the id
/// the preset will have: the existing preset's (overwrite by name or by
/// `overwrite_id`) or a UUID minted now.
///
/// Overwriting an existing user preset by name is destructive, so it
/// takes an explicit `overwrite`, per the control API's convention.
/// `overwrite_id` names its target outright. Factory presets are never
/// touched: saving under a factory name creates a user preset that
/// shadows it, which is what the plugin's own window does.
pub(crate) fn check_save(
    app: &Resonance,
    clap_id: &str,
    args: &SaveArgs,
) -> Result<String, RpcError> {
    let wanted = args.name.trim();
    if wanted.is_empty() {
        return Err(RpcError::invalid_params("a preset needs a name"));
    }
    let users = bank_for(app, clap_id).list_user();
    if let Some(target) = &args.overwrite_id {
        let Some(found) = users.iter().find(|p| p.id == *target) else {
            return Err(RpcError::not_found(format!(
                "{clap_id:?} has no user preset with id {target:?}"
            )));
        };
        if users
            .iter()
            .any(|p| p.id != found.id && p.name.eq_ignore_ascii_case(wanted))
        {
            return Err(RpcError::invalid_params(format!(
                "another user preset is already named {wanted:?}"
            )));
        }
        return Ok(found.id.clone());
    }
    match users.iter().find(|p| p.name.eq_ignore_ascii_case(wanted)) {
        Some(_) if !args.overwrite => Err(RpcError::needs_confirmation(format!(
            "a user preset named {wanted:?} already exists for {clap_id:?}; \
             pass overwrite: true to replace it"
        ))),
        Some(existing) => Ok(existing.id.clone()),
        None => Ok(resonance_plugin::presets::format::new_uuid()),
    }
}

/// Arm a capture: the plugin hands back its preset form, and
/// [`write_saved_state`] writes it with this request's id and metadata.
/// Returns the preset's id.
pub(crate) fn arm_save(
    app: &mut Resonance,
    clap_id: String,
    instance_id: PluginInstanceId,
    args: SaveArgs,
) -> Result<String, RpcError> {
    let id = check_save(app, &clap_id, &args)?;
    app.presets.pending_plugin_preset_save = Some(crate::PendingPluginPresetSave {
        instance_id,
        clap_id,
        name: args.name.trim().to_string(),
        id: id.clone(),
        meta: args.meta,
        favorite: args.favorite,
        target: args.overwrite_id,
    });
    let _ = app
        .engine
        .send(resonance_audio::types::AudioCommand::SavePluginPresetState { instance_id });
    Ok(id)
}

/// The reply to a save: the ack plus the preset's id.
pub(crate) fn saved_reply(
    app: &Resonance,
    request: &resonance_control::Request,
    id: String,
) -> resonance_control::Response {
    super::success(
        request,
        &resonance_control::methods::plugin_preset::SavePluginPresetResult {
            revision: app.session.revision,
            id,
        },
    )
}

/// A wire metadata input laid over library metadata.
pub(crate) fn apply_meta_input(meta: &mut PresetMeta, input: &PresetMetaInput) {
    if let Some(v) = &input.author {
        meta.author = Some(v.clone());
    }
    if let Some(v) = &input.description {
        meta.description = Some(v.clone());
    }
    if let Some(v) = &input.category {
        meta.category = Some(v.clone());
    }
    if let Some(v) = &input.instrument {
        meta.instrument = v.clone();
    }
    if let Some(v) = &input.genres {
        meta.genres = v.clone();
    }
    if let Some(v) = &input.character {
        meta.character = v.clone();
    }
    if let Some(v) = &input.tags {
        meta.tags = v.clone();
    }
}

/// Write the plugin's preset state as a user preset, with what the save
/// request asked for (id, metadata, star, target).
///
/// Called when the engine's `PluginPresetStateSaved` echo lands: the
/// plugin is the only thing that knows its current sound (edits made in
/// its own window never reach the app's mirror).
/// `blob` as a first-party state document, when it is one.
fn resonance_document(blob: &[u8]) -> Option<serde_json::Value> {
    let document: serde_json::Value = serde_json::from_slice(blob).ok()?;
    document
        .get("params")
        .is_some_and(|p| p.is_object())
        .then_some(document)
}

pub(crate) fn write_saved_state(
    app: &Resonance,
    pending: &crate::PendingPluginPresetSave,
    blob: &[u8],
) -> Result<PresetRecord, String> {
    let bank = bank_for(app, &pending.clap_id);
    let meta = pending.meta.as_ref().map(|input| {
        let mut meta = PresetMeta::default();
        apply_meta_input(&mut meta, input);
        meta
    });
    let options = resonance_plugin::presets::SaveOptions {
        meta,
        id: Some(pending.id.clone()),
        target: pending.target.clone(),
        ..Default::default()
    };
    // A first-party plugin's state is a JSON document with a `params`
    // object; anything else is a third-party plugin's opaque state, kept
    // as a `clap-state` blob (§8 tier T0).
    let saved = match resonance_document(blob) {
        Some(document) => bank.write_user_preset_with(pending.name.trim(), &document, options)?,
        None => bank.write_user_blob_with(pending.name.trim(), blob, options)?,
    };
    if let Some(favorite) = pending.favorite {
        bank.library()
            .set_favorite(&pending.clap_id, &saved.id, favorite)?;
    }
    bank.record(&saved).ok_or_else(|| "the saved preset is not listed".to_string())
}
