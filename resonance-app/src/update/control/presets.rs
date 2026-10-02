//! `presets.*` — the per-user plugin preset library across every plugin
//! (plugin-preset-library.md §12.2). Library state, not the project: no
//! project needed, no undo entry, no `revision` bump. Answered above the
//! mutation gate, like `amp_models.*`.

use resonance_control::methods::plugin_preset::{PluginPresetEntry, PresetFilter};
use resonance_control::methods::presets;
use resonance_control::{Request, Response, RpcError};
use resonance_plugin::presets::{PresetRef, PresetSource};

use super::plugin_presets::{apply_meta_input, bank_for, entry_from_hit, query_for};
use super::reply::{failure, success};
use crate::Resonance;

/// Handle a `presets.*` request, or `None` for another namespace.
pub(super) fn try_handle(app: &mut Resonance, request: &Request) -> Option<Response> {
    Some(match request.method.as_str() {
        presets::SET_MARKS => set_marks(app, request),
        presets::UPDATE_META => update_meta(app, request),
        presets::VOCABULARY => vocabulary(app, request),
        presets::SEARCH => search(app, request),
        presets::RENAME => rename(app, request),
        presets::DELETE => delete(app, request),
        _ => return None,
    })
}

/// The wire entry of one preset, marks included.
pub(crate) fn entry_for(
    app: &Resonance,
    plugin_id: &str,
    preset_id: &str,
) -> Option<PluginPresetEntry> {
    let lib = crate::plugin_preset_library::library(app);
    let _ = bank_for(app, plugin_id);
    lib.query(&query_for(&PresetFilter::default(), vec![plugin_id.to_string()]))
        .hits
        .iter()
        .find(|h| h.record.preset.id == preset_id)
        .map(entry_from_hit)
}

/// Refuse a `plugin_id` the app does not know (code review STATE2-01).
/// The id is a free-form wire string the library would otherwise index
/// as a directory; only a scanned plugin, a plugin with a registered
/// factory bank, or one whose preset directory already exists is
/// accepted, and never one that is not a plain directory name.
pub(crate) fn known_plugin(app: &Resonance, plugin_id: &str) -> Result<(), RpcError> {
    let not_found = || {
        RpcError::not_found(format!(
            "no plugin with id {plugin_id:?}; plugin ids are CLAP ids such as \
             \"com.resonance.wavetable\" (plugins_list reports them)"
        ))
    };
    if !resonance_plugin::presets::is_valid_plugin_id(plugin_id) {
        return Err(not_found());
    }
    let scanned = app
        .plugin_catalog
        .available_plugins
        .iter()
        .any(|p| p.clap_plugin_id == plugin_id);
    let lib = crate::plugin_preset_library::library(app);
    let has_dir = || lib.plugin_dir(plugin_id).is_some_and(|d| d.is_dir());
    if scanned || lib.factory_len(plugin_id) > 0 || has_dir() {
        Ok(())
    } else {
        Err(not_found())
    }
}

/// The preset `preset_id` names, as a reference the library resolves.
fn resolve(app: &Resonance, plugin_id: &str, preset_id: &str) -> Result<PresetRef, RpcError> {
    known_plugin(app, plugin_id)?;
    bank_for(app, plugin_id)
        .list()
        .into_iter()
        .find(|p| p.id == preset_id.trim())
        .ok_or_else(|| {
            RpcError::not_found(format!(
                "plugin {plugin_id:?} has no preset with id {preset_id:?}; \
                 *_plugin_presets reports the ids"
            ))
        })
}

fn reply_entry(app: &Resonance, request: &Request, plugin_id: &str, preset_id: &str) -> Response {
    match entry_for(app, plugin_id, preset_id) {
        Some(entry) => success(
            request,
            &presets::EntryResult {
                plugin_id: plugin_id.to_string(),
                entry,
            },
        ),
        None => failure(request, RpcError::not_found("the preset is gone")),
    }
}

/// `presets.set_marks`.
fn set_marks(app: &mut Resonance, request: &Request) -> Response {
    let params: presets::SetMarksParams = match request.params() {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    if params.favorite.is_none() && params.tags.is_none() {
        return failure(
            request,
            RpcError::invalid_params("give `favorite`, `tags` or both"),
        );
    }
    let preset = match resolve(app, &params.plugin_id, &params.preset_id) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    let lib = crate::plugin_preset_library::library(app);
    if let Some(favorite) = params.favorite {
        if let Err(e) = lib.set_favorite(&params.plugin_id, &preset.id, favorite) {
            return failure(request, RpcError::internal(e));
        }
    }
    if let Some(tags) = &params.tags {
        if let Err(e) = lib.set_personal_tags(&params.plugin_id, &preset.id, tags) {
            return failure(request, RpcError::internal(e));
        }
    }
    crate::update::plugin_preset_ui::library_changed(app);
    reply_entry(app, request, &params.plugin_id, &preset.id)
}

/// `presets.update_meta`.
fn update_meta(app: &mut Resonance, request: &Request) -> Response {
    let params: presets::UpdateMetaParams = match request.params() {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    let preset = match resolve(app, &params.plugin_id, &params.preset_id) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    if preset.source == PresetSource::Factory {
        return failure(
            request,
            RpcError::invalid_params(format!(
                "{:?} is a factory preset: it is read-only. Star it or give it personal tags \
                 with presets_set_marks, or save a copy with *_save_plugin_preset",
                preset.name
            )),
        );
    }
    let lib = crate::plugin_preset_library::library(app);
    let result = lib.update_meta(&params.plugin_id, &preset, |meta| {
        if let Some(set) = &params.set {
            apply_meta_input(meta, set);
        }
        for t in &params.add_tags {
            if !meta.tags.contains(t) {
                meta.tags.push(t.clone());
            }
        }
        let remove: Vec<String> = params
            .remove_tags
            .iter()
            .filter_map(|t| resonance_common::library_marks::normalize_tag(t))
            .collect();
        meta.tags.retain(|t| {
            let slug = resonance_common::library_marks::normalize_tag(t).unwrap_or_default();
            !remove.contains(&slug)
        });
    });
    if let Err(e) = result {
        return failure(request, RpcError::internal(e));
    }
    crate::update::plugin_preset_ui::library_changed(app);
    reply_entry(app, request, &params.plugin_id, &preset.id)
}

/// `presets.search`.
fn search(app: &mut Resonance, request: &Request) -> Response {
    let params: presets::SearchParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    let lib = crate::plugin_preset_library::library(app);
    let plugins: Vec<String> = match &params.plugin_id {
        Some(id) => {
            if let Err(e) = known_plugin(app, id) {
                return failure(request, e);
            }
            vec![id.clone()]
        }
        None => app
            .plugin_catalog
            .available_plugins
            .iter()
            .map(|p| p.clap_plugin_id.clone())
            .collect(),
    };
    for id in &plugins {
        let _ = bank_for(app, id);
    }
    let result = lib.query(&query_for(&params.filter, plugins));
    let offset = params.filter.offset.unwrap_or(0) as usize;
    let limit = params
        .filter
        .limit
        .unwrap_or(super::plugin_presets::DEFAULT_LIMIT) as usize;
    let hits = result
        .hits
        .iter()
        .skip(offset)
        .take(limit)
        .map(|h| presets::SearchHit {
            plugin_id: h.plugin_id.clone(),
            entry: entry_from_hit(h),
        })
        .collect();
    success(
        request,
        &presets::SearchResult {
            total: result.hits.len() as u32,
            hits,
            facets: super::plugin_presets::wire_facets(&result.facets),
            library_generation: lib.marks().generation(),
        },
    )
}

/// `presets.rename`.
fn rename(app: &mut Resonance, request: &Request) -> Response {
    let params: presets::RenameParams = match request.params() {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    let preset = match resolve(app, &params.plugin_id, &params.preset_id) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    if preset.source == PresetSource::Factory {
        return failure(
            request,
            RpcError::invalid_params("factory presets cannot be renamed; save a copy instead"),
        );
    }
    use resonance_plugin::presets::RenameError;
    let bank = bank_for(app, &params.plugin_id);
    match bank.library().rename_typed(&params.plugin_id, &preset, &params.name) {
        Ok(record) => {
            let response = reply_entry(app, request, &params.plugin_id, &record.preset.id);
            crate::update::plugin_preset_ui::library_changed(app);
            response
        }
        Err(RenameError::Invalid(e)) => failure(request, RpcError::invalid_params(e)),
        Err(RenameError::NotFound(e)) => failure(request, RpcError::not_found(e)),
        Err(RenameError::Io(e)) => failure(request, RpcError::internal(e)),
    }
}

/// `presets.delete`.
fn delete(app: &mut Resonance, request: &Request) -> Response {
    let params: presets::DeleteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    let preset = match resolve(app, &params.plugin_id, &params.preset_id) {
        Ok(p) => p,
        Err(e) => return failure(request, e),
    };
    if preset.source == PresetSource::Factory {
        return failure(
            request,
            RpcError::invalid_params("factory presets cannot be deleted"),
        );
    }
    if !params.confirm {
        return failure(
            request,
            RpcError::needs_confirmation(format!(
                "this moves the user preset {:?} of {:?} to the trash (recoverable for 30 \
                 days); pass confirm: true to delete it",
                preset.name, params.plugin_id
            )),
        );
    }
    match bank_for(app, &params.plugin_id).trash(&preset) {
        Ok(path) => {
            crate::update::plugin_preset_ui::library_changed(app);
            success(
                request,
                &presets::DeleteResult {
                    trashed_path: path.to_string_lossy().into_owned(),
                },
            )
        }
        Err(e) => failure(request, RpcError::internal(e)),
    }
}

/// `presets.vocabulary`.
fn vocabulary(app: &mut Resonance, request: &Request) -> Response {
    use resonance_common::library_marks::vocab;
    let lib = crate::plugin_preset_library::library(app);
    for p in &app.plugin_catalog.available_plugins {
        let _ = bank_for(app, &p.clap_plugin_id);
    }
    let result = lib.query(&query_for(&PresetFilter::default(), Vec::new()));
    let seeded = |s: &[&str]| s.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    let with_used = |mut base: Vec<String>, used: &[(String, usize)]| {
        for (v, _) in used {
            if !base.contains(v) {
                base.push(v.clone());
            }
        }
        base
    };
    let mut tags: Vec<String> = result.facets.tags.iter().map(|(v, _)| v.clone()).collect();
    for t in lib.marks().complete_tag("", &[], 200) {
        if !tags.contains(&t) {
            tags.push(t);
        }
    }
    success(
        request,
        &presets::Vocabulary {
            categories_instrument: seeded(vocab::CATEGORIES_INSTRUMENT),
            categories_effect: seeded(vocab::CATEGORIES_EFFECT),
            instrument: with_used(seeded(vocab::INSTRUMENT), &result.facets.instrument),
            genres: with_used(seeded(vocab::GENRES), &result.facets.genres),
            character: with_used(seeded(vocab::CHARACTER), &result.facets.character),
            tags,
        },
    )
}
