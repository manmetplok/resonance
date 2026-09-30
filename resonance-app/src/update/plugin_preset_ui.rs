//! The host's preset surfaces (plugin-preset-library.md §6.6, §6.7; slice
//! P6): the plugin panel's bar (◀ name ▶ ☆ Presets…), the browser overlay
//! it opens over one plugin instance, the media browser's Presets tab, and
//! "with preset…" in the add pickers.
//!
//! Undo (§6.7): an audition is applied unrecorded, remembering the sound
//! the browser opened on (`AuditionOrigin`). Keeping it puts that origin
//! back into the mirror and re-dispatches the pick as a recorded
//! `LoadPluginPreset`, so the one undo entry returns to the origin. A
//! revert re-sends the origin's values (and, when it was a library preset,
//! that preset's state first). The ◀ / ▶ steps and a media-tab load are
//! plain recorded loads.
//!
//! Every list is recomputed here, on a query change or a star, never in
//! `view` (ui-work.md §11).

use iced::Task;
use resonance_audio::types::{AudioCommand, PluginInstanceId};
use resonance_control::methods::plugin_preset::PluginPresetSource;

use crate::message::{
    BusMessage, MasterMessage, Message, PluginMessage, PresetAddOwner, PresetUiMessage,
};
use crate::state::presets::{
    AuditionOrigin, HostPresetBrowser, HostPresetList, HostPresetRow, PresetAddPick,
    PresetPluginChoice,
};
use crate::update::control::plugin_presets as pp;
use crate::Resonance;

impl Resonance {
    /// The slot behind `instance_id`, on any chain.
    pub(crate) fn plugin_slot(
        &self,
        instance_id: PluginInstanceId,
    ) -> Option<&crate::state::PluginSlotState> {
        self.registry
            .tracks
            .iter()
            .flat_map(|t| t.plugins.iter())
            .chain(self.registry.busses.iter().flat_map(|b| b.plugins.iter()))
            .chain(self.master.plugins.iter())
            .find(|p| p.instance_id == instance_id)
    }

    /// Whether the plugin panel's plugin is there to take a preset: the
    /// preset commands' availability.
    pub(crate) fn selected_plugin_available(&self) -> bool {
        self.ui
            .mixer
            .selected_plugin
            .and_then(|id| self.plugin_slot(id))
            .is_some_and(|slot| slot.availability.reason().is_none())
    }
}

pub fn handle(r: &mut Resonance, m: PresetUiMessage) -> Task<Message> {
    match m {
        PresetUiMessage::Step { instance_id, delta } => return step(r, instance_id, delta),
        PresetUiMessage::ToggleFavorite(instance_id) => {
            let Some((clap_id, identity)) = slot_identity(r, instance_id) else {
                return Task::none();
            };
            if let Some(identity) = identity {
                let lib = crate::plugin_preset_library::library(r);
                let now = lib.preset_marks(&clap_id, &identity.id).favorite;
                if let Err(e) = lib.set_favorite(&clap_id, &identity.id, !now) {
                    r.banners.error_message = Some(format!("Could not star the preset: {e}"));
                }
                marks_changed(r);
            }
        }
        PresetUiMessage::OpenBrowser(instance_id) => {
            let Some((clap_id, identity)) = slot_identity(r, instance_id) else {
                return Task::none();
            };
            let plugin_name = plugin_name(r, &clap_id);
            let mut list = HostPresetList {
                plugin: Some(clap_id),
                ..HostPresetList::default()
            };
            refresh(r, &mut list);
            list.selected = identity.and_then(|i| {
                list.rows
                    .iter()
                    .position(|row| row.id == i.id && row.source == i.source)
            });
            r.presets.host_browser = Some(HostPresetBrowser {
                instance_id,
                plugin_name,
                list,
                origin: None,
            });
        }
        PresetUiMessage::CloseBrowser { keep } => return close_browser(r, keep),
        PresetUiMessage::BrowserSearch(text) => with_browser_list(r, |list| list.query = text),
        PresetUiMessage::BrowserFavoritesOnly(on) => {
            with_browser_list(r, |list| list.favorites_only = on)
        }
        PresetUiMessage::BrowserAudition(index) => audition(r, index),
        PresetUiMessage::BrowserToggleRowFavorite(index) => {
            let row = r
                .presets
                .host_browser
                .as_ref()
                .and_then(|b| b.list.rows.get(index).cloned());
            if let Some(row) = row {
                toggle_row_favorite(r, &row);
            }
        }
        PresetUiMessage::MediaSearch(text) => {
            let mut list = std::mem::take(&mut r.presets.media_presets);
            list.query = text;
            refresh(r, &mut list);
            r.presets.media_presets = list;
        }
        PresetUiMessage::MediaFavoritesOnly(on) => {
            let mut list = std::mem::take(&mut r.presets.media_presets);
            list.favorites_only = on;
            refresh(r, &mut list);
            r.presets.media_presets = list;
        }
        PresetUiMessage::MediaPlugin(choice) => {
            let mut list = std::mem::take(&mut r.presets.media_presets);
            list.plugin = choice.plugin_id;
            refresh(r, &mut list);
            r.presets.media_presets = list;
        }
        PresetUiMessage::MediaSelect(index) => {
            if index < r.presets.media_presets.rows.len() {
                r.presets.media_presets.selected = Some(index);
            }
        }
        PresetUiMessage::MediaLoad(index) => return media_load(r, index),
        PresetUiMessage::MediaToggleRowFavorite(index) => {
            if let Some(row) = r.presets.media_presets.rows.get(index).cloned() {
                toggle_row_favorite(r, &row);
            }
        }
        PresetUiMessage::AddWithPreset { owner, pick } => return add_with_preset(r, owner, pick),
    }
    Task::none()
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

/// Re-run a list's query against the library; keeps the selection on the
/// same preset when it is still listed.
pub(crate) fn refresh(r: &Resonance, list: &mut HostPresetList) {
    let selected = list
        .selected
        .and_then(|i| list.rows.get(i))
        .map(|row| (row.plugin_id.clone(), row.id.clone()));
    list.rows = rows(r, list);
    list.selected = selected.and_then(|(plugin, id)| {
        list.rows
            .iter()
            .position(|row| row.plugin_id == plugin && row.id == id)
    });
}

fn rows(r: &Resonance, list: &HostPresetList) -> Vec<HostPresetRow> {
    let plugins: Vec<String> = match &list.plugin {
        Some(p) => vec![p.clone()],
        None => r
            .plugin_catalog
            .available_plugins
            .iter()
            .map(|p| p.clap_plugin_id.clone())
            .collect(),
    };
    // A bank per plugin registers its factory presets and indexes its
    // user directory.
    for p in &plugins {
        let _ = pp::bank_for(r, p);
    }
    let lib = crate::plugin_preset_library::library(r);
    let query = resonance_plugin::presets::Query {
        text: list.query.clone(),
        plugins,
        favorites_only: list.favorites_only,
        ..resonance_plugin::presets::Query::default()
    };
    lib.query(&query)
        .hits
        .iter()
        .map(|hit| HostPresetRow {
            plugin_name: plugin_name(r, &hit.plugin_id),
            plugin_id: hit.plugin_id.clone(),
            id: hit.record.preset.id.clone(),
            name: hit.record.meta.name.clone(),
            source: pp::wire_source(hit.record.preset.source),
            category: hit.record.meta.category.clone(),
            favorite: hit.favorite,
        })
        .collect()
}

fn with_browser_list(r: &mut Resonance, f: impl FnOnce(&mut HostPresetList)) {
    let Some(mut browser) = r.presets.host_browser.take() else {
        return;
    };
    f(&mut browser.list);
    refresh(r, &mut browser.list);
    r.presets.host_browser = Some(browser);
}

/// A star moved: every open list and the add pickers' favourites follow.
fn marks_changed(r: &mut Resonance) {
    if let Some(mut browser) = r.presets.host_browser.take() {
        refresh(r, &mut browser.list);
        r.presets.host_browser = Some(browser);
    }
    if !r.presets.media_presets.rows.is_empty() {
        let mut list = std::mem::take(&mut r.presets.media_presets);
        refresh(r, &mut list);
        r.presets.media_presets = list;
    }
    rebuild_add_picks(r);
}

fn toggle_row_favorite(r: &mut Resonance, row: &HostPresetRow) {
    let lib = crate::plugin_preset_library::library(r);
    if let Err(e) = lib.set_favorite(&row.plugin_id, &row.id, !row.favorite) {
        r.banners.error_message = Some(format!("Could not star the preset: {e}"));
    }
    marks_changed(r);
}

/// Rebuild the Presets tab's plugin choices and the add pickers'
/// favourites. Called on a plugin scan.
pub(crate) fn rebuild_caches(r: &mut Resonance) {
    let mut choices = vec![PresetPluginChoice {
        plugin_id: None,
        name: "All plugins".to_string(),
    }];
    choices.extend(r.plugin_catalog.available_plugins.iter().map(|p| PresetPluginChoice {
        plugin_id: Some(p.clap_plugin_id.clone()),
        name: p.name.clone(),
    }));
    r.presets.media_plugin_choices = std::rc::Rc::from(choices);
    rebuild_add_picks(r);
    if !r.presets.media_presets.rows.is_empty() || r.presets.media_presets.plugin.is_some() {
        let mut list = std::mem::take(&mut r.presets.media_presets);
        refresh(r, &mut list);
        r.presets.media_presets = list;
    }
}

fn rebuild_add_picks(r: &mut Resonance) {
    let list = HostPresetList {
        favorites_only: true,
        ..HostPresetList::default()
    };
    let favorites = rows(r, &list);
    let pick = |instrument: bool| -> Vec<PresetAddPick> {
        favorites
            .iter()
            .filter_map(|row| {
                let plugin = r
                    .plugin_catalog
                    .available_plugins
                    .iter()
                    .find(|p| p.clap_plugin_id == row.plugin_id && p.is_instrument == instrument)?;
                Some(PresetAddPick {
                    plugin: plugin.clone(),
                    preset_id: row.id.clone(),
                    preset_name: row.name.clone(),
                    source: row.source,
                })
            })
            .collect()
    };
    r.presets.fx_favorite_picks = std::rc::Rc::from(pick(false));
    r.presets.instrument_favorite_picks = std::rc::Rc::from(pick(true));
}

/// The Presets tab was opened: fill it on first show.
pub(crate) fn media_tab_shown(r: &mut Resonance) {
    let mut list = std::mem::take(&mut r.presets.media_presets);
    refresh(r, &mut list);
    r.presets.media_presets = list;
}

// ---------------------------------------------------------------------------
// Slots
// ---------------------------------------------------------------------------

fn plugin_name(r: &Resonance, clap_id: &str) -> String {
    r.plugin_catalog
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == clap_id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| clap_id.to_string())
}

/// The slot's CLAP id and loaded-preset identity.
fn slot_identity(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
) -> Option<(String, Option<crate::state::presets::SlotPresetIdentity>)> {
    let clap_id = r.with_plugin_mut(instance_id, |slot| slot.clap_plugin_id.clone())?;
    Some((clap_id, r.presets.plugin_preset_identity.get(&instance_id).cloned()))
}

fn record_use(r: &Resonance, clap_id: &str, preset_id: &str) {
    let lib = crate::plugin_preset_library::library(r);
    if let Err(e) = lib.record_use(clap_id, preset_id) {
        tracing::debug!("presets: recents not recorded: {e}");
    }
}

/// A recorded load onto `instance_id` (one undo entry).
fn recorded_load(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    clap_id: &str,
    preset_id: &str,
    source: PluginPresetSource,
) -> Task<Message> {
    match pp::host_load_message(r, instance_id, clap_id, preset_id, source) {
        Ok(message) => {
            record_use(r, clap_id, preset_id);
            r.update(message)
        }
        Err(e) => {
            r.banners.error_message = Some(format!("Could not load the preset: {}", e.message));
            Task::none()
        }
    }
}

fn step(r: &mut Resonance, instance_id: PluginInstanceId, delta: i32) -> Task<Message> {
    let Some((clap_id, identity)) = slot_identity(r, instance_id) else {
        return Task::none();
    };
    let order = pp::bank_order(r, &clap_id);
    if order.is_empty() {
        return Task::none();
    }
    let n = order.len() as i64;
    let at = identity.and_then(|i| {
        order
            .iter()
            .position(|p| p.id == i.id && pp::wire_source(p.source) == i.source)
    });
    let next = match at {
        Some(i) => (i as i64 + delta as i64).rem_euclid(n) as usize,
        None if delta < 0 => order.len() - 1,
        None => 0,
    };
    let target = order[next].clone();
    recorded_load(r, instance_id, &clap_id, &target.id, pp::wire_source(target.source))
}

// ---------------------------------------------------------------------------
// Audition
// ---------------------------------------------------------------------------

fn audition(r: &mut Resonance, index: usize) {
    let Some(browser) = r.presets.host_browser.as_ref() else {
        return;
    };
    let instance_id = browser.instance_id;
    let Some(row) = browser.list.rows.get(index).cloned() else {
        return;
    };
    if browser.origin.is_none() {
        let values = r
            .with_plugin_mut(instance_id, |slot| {
                slot.params
                    .iter()
                    .map(|p| (p.id, p.current_value))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let identity = r.presets.plugin_preset_identity.get(&instance_id).cloned();
        if let Some(b) = r.presets.host_browser.as_mut() {
            b.origin = Some(AuditionOrigin { values, identity });
        }
    }
    match pp::host_load_message(r, instance_id, &row.plugin_id, &row.id, row.source) {
        Ok(Message::Plugin(m)) => crate::update::plugin::apply_preset_load(r, m),
        Ok(_) => {}
        Err(e) => {
            r.banners.error_message = Some(format!("Could not load the preset: {}", e.message));
            return;
        }
    }
    if let Some(b) = r.presets.host_browser.as_mut() {
        b.list.selected = Some(index);
    }
}

fn close_browser(r: &mut Resonance, keep: bool) -> Task<Message> {
    let Some(browser) = r.presets.host_browser.take() else {
        return Task::none();
    };
    let Some(origin) = browser.origin else {
        return Task::none();
    };
    let instance_id = browser.instance_id;
    let picked = browser.list.selected.and_then(|i| browser.list.rows.get(i).cloned());
    match (keep, picked) {
        (true, Some(row)) => {
            // Put the origin back into the mirror only (the engine already
            // plays the pick), so the recorded load's "before" is the sound
            // the browser opened on — one undo entry, back to it.
            restore_mirror(r, instance_id, &origin);
            recorded_load(r, instance_id, &row.plugin_id, &row.id, row.source)
        }
        _ => {
            revert(r, instance_id, &origin);
            Task::none()
        }
    }
}

fn restore_mirror(r: &mut Resonance, instance_id: PluginInstanceId, origin: &AuditionOrigin) {
    r.with_plugin_mut(instance_id, |slot| {
        for (id, value) in &origin.values {
            if let Some(p) = slot.params.iter_mut().find(|p| p.id == *id) {
                p.current_value = *value;
            }
        }
    });
    match &origin.identity {
        Some(identity) => {
            r.presets
                .plugin_preset_identity
                .insert(instance_id, identity.clone());
        }
        None => {
            r.presets.plugin_preset_identity.remove(&instance_id);
        }
    }
}

/// Undo an audition: the origin preset's state first (the rest of its
/// sound — a model, an IR), when it was one, then every origin value.
fn revert(r: &mut Resonance, instance_id: PluginInstanceId, origin: &AuditionOrigin) {
    let clap_id = r.with_plugin_mut(instance_id, |slot| slot.clap_plugin_id.clone());
    if let (Some(clap_id), Some(identity)) = (clap_id, &origin.identity) {
        if let Ok(Message::Plugin(PluginMessage::LoadPluginPreset {
            preset_state: Some(data),
            ..
        })) = pp::host_load_message(r, instance_id, &clap_id, &identity.id, identity.source)
        {
            let _ = r
                .engine
                .send(AudioCommand::LoadPluginPresetState { instance_id, data });
        }
    }
    for (param_id, value) in &origin.values {
        let _ = r.engine.send(AudioCommand::SetPluginParam {
            instance_id,
            param_id: *param_id,
            value: *value,
        });
    }
    restore_mirror(r, instance_id, origin);
}

// ---------------------------------------------------------------------------
// Media tab and add pickers
// ---------------------------------------------------------------------------

fn media_load(r: &mut Resonance, index: usize) -> Task<Message> {
    let Some(row) = r.presets.media_presets.rows.get(index).cloned() else {
        return Task::none();
    };
    r.presets.media_presets.selected = Some(index);
    let Some(instance_id) = r.ui.mixer.selected_plugin else {
        r.banners.error_message = Some(format!(
            "Select a {} slot to load {:?} onto",
            row.plugin_name, row.name
        ));
        return Task::none();
    };
    let same = r
        .with_plugin_mut(instance_id, |slot| slot.clap_plugin_id == row.plugin_id)
        .unwrap_or(false);
    if !same {
        r.banners.error_message = Some(format!(
            "{:?} is a {} preset; the selected plugin is something else",
            row.name, row.plugin_name
        ));
        return Task::none();
    }
    recorded_load(r, instance_id, &row.plugin_id, &row.id, row.source)
}

fn add_with_preset(r: &mut Resonance, owner: PresetAddOwner, pick: PresetAddPick) -> Task<Message> {
    let instance_id = r.allocate_plugin_id();
    let clap_id = pick.plugin.clap_plugin_id.clone();
    record_use(r, &clap_id, &pick.preset_id);
    r.presets
        .pending_plugin_presets
        .insert(instance_id, (clap_id, pick.preset_id, pick.source));
    let plugin = pick.plugin;
    let message = match owner {
        PresetAddOwner::Track(track_id) => {
            Message::Plugin(PluginMessage::AddPluginToTrackWithId {
                track_id,
                instance_id,
                plugin,
            })
        }
        PresetAddOwner::Bus(bus_id) => Message::Bus(BusMessage::AddPluginToBusWithId {
            bus_id,
            instance_id,
            plugin,
        }),
        PresetAddOwner::Master => {
            Message::Master(MasterMessage::AddPluginToMasterWithId { instance_id, plugin })
        }
    };
    r.update(message)
}
