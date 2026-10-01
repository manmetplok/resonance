//! The host's preset surfaces (plugin-preset-library.md §6.6, §6.7; slice
//! P6): the generic window's preset bar (◀ name ▶ ☆ Presets…), the browser overlay
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

    /// Whether the focused slot is there to take a preset: the preset
    /// commands' availability.
    pub(crate) fn selected_plugin_available(&self) -> bool {
        crate::update::plugin_window::preset_target(self)
            .and_then(|id| self.plugin_slot(id))
            .is_some_and(|slot| slot.availability.reason().is_none())
    }
}

/// While a preset drag is armed: the window-level events that end it — a
/// press anywhere (whatever widget took it: the release that should have
/// ended the drag was lost outside the window) and the window losing
/// focus. A header's own release still drops first: it is its own event.
pub fn drag_end_event(event: &iced::Event) -> Option<Message> {
    let ends = matches!(
        event,
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
            | iced::Event::Window(iced::window::Event::Unfocused)
    );
    ends.then(|| Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::DragEnd)))
}

/// The preset browser's search field.
pub(crate) fn search_input_id() -> iced::widget::Id {
    iced::widget::Id::new("preset-browser-search")
}

/// Keys while the preset browser is open (checked before the text field's
/// capture, like the palette): Esc reverts and closes, ↑ / ↓ audition,
/// ↵ keeps.
pub(crate) fn key(r: &mut Resonance, chord: crate::commands::KeyChord) -> Option<Task<Message>> {
    use crate::commands::{KeyChord, Mods, NamedKey};
    let named = |n| KeyChord::named(n, Mods::NONE);
    let m = if chord == named(NamedKey::Escape) {
        PresetUiMessage::CloseBrowser { keep: false }
    } else if chord == named(NamedKey::ArrowUp) {
        PresetUiMessage::BrowserMove(-1)
    } else if chord == named(NamedKey::ArrowDown) {
        PresetUiMessage::BrowserMove(1)
    } else if chord == named(NamedKey::Enter) {
        PresetUiMessage::CloseBrowser { keep: true }
    } else {
        return None;
    };
    Some(handle(r, m))
}

/// A plugin instance is gone: drop everything the preset surfaces held
/// for it — its identity, a parked preset, and a browser open over it
/// (closed without a revert: there is nothing left to send it to).
pub(crate) fn forget_instance(r: &mut Resonance, instance_id: PluginInstanceId) {
    r.presets.plugin_preset_identity.remove(&instance_id);
    r.presets.pending_plugin_presets.remove(&instance_id);
    r.presets.revert_on_capture.retain(|_, id| *id != instance_id);
    if r
        .presets
        .host_browser
        .as_ref()
        .is_some_and(|b| b.instance_id == instance_id)
    {
        r.presets.host_browser = None;
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
                auditioned: None,
            });
            // Type to search straight away.
            return iced::widget::operation::focus(search_input_id());
        }
        PresetUiMessage::CloseBrowser { keep } => return close_browser(r, keep),
        PresetUiMessage::BrowserSearch(text) => with_browser_list(r, |list| list.query = text),
        PresetUiMessage::BrowserFavoritesOnly(on) => {
            with_browser_list(r, |list| list.favorites_only = on)
        }
        PresetUiMessage::BrowserAudition(index) => audition(r, index),
        PresetUiMessage::BrowserMove(delta) => {
            let next = r.presets.host_browser.as_ref().and_then(|b| {
                let n = b.list.rows.len() as i64;
                (n > 0).then(|| match b.list.selected {
                    Some(i) => (i as i64 + delta as i64).clamp(0, n - 1) as usize,
                    None if delta < 0 => (n - 1) as usize,
                    None => 0,
                })
            });
            if let Some(index) = next {
                audition(r, index);
            }
        }
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
        PresetUiMessage::MediaPress(index) => {
            if let Some(row) = r.presets.media_presets.rows.get(index).cloned() {
                r.presets.media_presets.selected = Some(index);
                r.presets.dragging = Some(crate::state::presets::PresetDrag {
                    row,
                    origin: None,
                    moved: false,
                });
            }
        }
        PresetUiMessage::DragMoved(at) => {
            if let Some(drag) = r.presets.dragging.as_mut() {
                match drag.origin {
                    None => drag.origin = Some(at),
                    Some(o) => {
                        let (dx, dy) = (at.x - o.x, at.y - o.y);
                        let far = crate::state::presets::PresetDrag::THRESHOLD;
                        if (dx * dx + dy * dy).sqrt() > far {
                            drag.moved = true;
                        }
                    }
                }
            }
        }
        PresetUiMessage::DropOnTrack(track_id) => return drop_on_track(r, track_id),
        PresetUiMessage::DragEnd => r.presets.dragging = None,
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
    let fresh = rows(r, list);
    if fresh != list.rows {
        list.rows = fresh;
        list.generation = list.generation.wrapping_add(1);
    }
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

/// The library changed under the host (a `presets.*` edit, another
/// process): every list, the add pickers' favourites and the loaded
/// identities' names follow. Cheap to call; a list whose rows did not
/// change keeps its generation (and its cached widgets).
pub(crate) fn library_changed(r: &mut Resonance) {
    marks_changed(r);
    // A renamed preset's loaded identity shows its new name; a deleted
    // user preset is no longer loaded as such.
    let lib = crate::plugin_preset_library::library(r);
    let ids: Vec<PluginInstanceId> = r.presets.plugin_preset_identity.keys().copied().collect();
    for instance_id in ids {
        let Some(clap_id) = r.plugin_slot(instance_id).map(|s| s.clap_plugin_id.clone()) else {
            continue;
        };
        let Some(identity) = r.presets.plugin_preset_identity.get(&instance_id).cloned() else {
            continue;
        };
        if identity.reported {
            continue;
        }
        let preset = resonance_plugin::presets::PresetRef {
            source: match identity.source {
                PluginPresetSource::Factory => resonance_plugin::presets::PresetSource::Factory,
                PluginPresetSource::User => resonance_plugin::presets::PresetSource::User,
            },
            id: identity.id.clone(),
            name: identity.name.clone(),
        };
        match lib.peek_record(&clap_id, &preset) {
            Some(record) => {
                if let Some(i) = r.presets.plugin_preset_identity.get_mut(&instance_id) {
                    i.name = record.meta.name.clone();
                }
            }
            None if identity.source == PluginPresetSource::User => {
                r.presets.plugin_preset_identity.remove(&instance_id);
            }
            None => {}
        }
    }
}

/// While the preset browser or the Presets tab shows: re-query now and
/// then, so another process's edits (or a plugin's own browser) show.
pub(crate) fn poll_visible_lists(r: &mut Resonance) {
    let tab_visible = r.media.browser.visible
        && r.media.browser.tab == crate::state::BrowserTab::Presets
        && matches!(r.ui.view_mode, crate::state::ViewMode::Arrange);
    if r.presets.host_browser.is_none() && !tab_visible {
        return;
    }
    let due = r
        .presets
        .lists_polled
        .is_none_or(|t| t.elapsed() >= LIST_POLL_INTERVAL);
    if due {
        r.presets.lists_polled = Some(std::time::Instant::now());
        library_changed(r);
    }
}

/// How often a visible preset list re-queries the library.
const LIST_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

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
    let source = pp::wire_source(target.source);
    match pp::host_load_message(r, instance_id, &clap_id, &target.id, source) {
        Ok(Message::Plugin(load)) => {
            record_use(r, &clap_id, &target.id);
            r.update(Message::Plugin(PluginMessage::PresetStep {
                instance_id,
                load: Box::new(load),
            }))
        }
        Ok(_) => Task::none(),
        Err(e) => {
            r.banners.error_message = Some(format!("Could not load the preset: {}", e.message));
            Task::none()
        }
    }
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
        // The whole sound comes back on a revert: the engine saves the
        // plugin's full state under its lock just before this first
        // audition loads (a model, an IR, user tables may exist nowhere
        // else).
        let state = crate::undo::snapshot::LateBlob::default();
        // A revert still waiting on its capture for this plugin: that
        // capture is this origin too (the sound before any audition) —
        // take it over instead of letting it land on top of this audition,
        // and capture nothing new (it would see the first audition).
        let waiting = r
            .presets
            .revert_on_capture
            .iter()
            .find(|(_, id)| **id == instance_id)
            .map(|(t, _)| *t);
        let token = match waiting {
            Some(token) => {
                r.presets.revert_on_capture.remove(&token);
                r.presets.pending_captures.entry(token).or_default().push(state.clone());
                token
            }
            None => {
                r.presets.capture_seq += 1;
                let token = r.presets.capture_seq;
                r.presets.pending_captures.entry(token).or_default().push(state.clone());
                r.presets.forced_capture = Some(token);
                token
            }
        };
        if let Some(b) = r.presets.host_browser.as_mut() {
            b.origin = Some(AuditionOrigin {
                values,
                identity,
                state,
                token,
            });
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
        b.auditioned = Some(row);
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
    // What was auditioned, whatever the list shows now (a search may have
    // filtered it out since).
    let picked = browser.auditioned;
    match (keep, picked) {
        (true, Some(row)) => {
            // Put the origin back into the mirror only (the engine already
            // plays the pick), so the recorded load's "before" is the sound
            // the browser opened on — one undo entry, back to it.
            restore_mirror(r, instance_id, &origin);
            // The kept load's undo entry returns to the origin's full
            // state, not to what a capture now would see (the audition).
            r.presets.capture_from = Some((origin.state.clone(), origin.token));
            let task = recorded_load(r, instance_id, &row.plugin_id, &row.id, row.source);
            r.presets.capture_from = None;
            task
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

/// Undo an audition: the plugin's full state as it was before the first
/// audition (the engine's capture), then every origin value. A capture
/// still in flight completes the revert when it lands.
fn revert(r: &mut Resonance, instance_id: PluginInstanceId, origin: &AuditionOrigin) {
    let state = origin.state.lock().ok().and_then(|b| b.clone());
    match state {
        Some(blob) => {
            let _ = r.engine.send(AudioCommand::LoadPluginState {
                instance_id,
                data: blob.to_vec(),
            });
            r.plugin_mirror.state_cache.insert(instance_id, blob);
        }
        None => {
            r.presets.revert_on_capture.insert(origin.token, instance_id);
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
    let Some(instance_id) = crate::update::plugin_window::preset_target(r) else {
        r.banners.error_message = Some(format!(
            "Open a {} plugin's window in the mixer to load {:?} onto it",
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

fn drop_on_track(r: &mut Resonance, track_id: resonance_audio::types::TrackId) -> Task<Message> {
    // Only a real drag drops: a press released without moving is a click.
    let Some(row) = r.presets.dragging.take().filter(|d| d.moved).map(|d| d.row) else {
        r.presets.dragging = None;
        return Task::none();
    };
    let Some(plugin) = r
        .plugin_catalog
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == row.plugin_id)
        .cloned()
    else {
        return Task::none();
    };
    let Some(track) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
        return Task::none();
    };
    // An instrument goes onto an instrument track that has none; an effect
    // onto a track that already has its sound source (on an empty
    // instrument track it would take the instrument slot); nothing onto a
    // multi-output sub-track, whose chain belongs to its parent. Anything
    // else is said, not guessed.
    let is_instrument_track =
        matches!(track.track_type, resonance_audio::types::TrackType::Instrument);
    let refusal = if track.sub_track.is_some() {
        Some(format!("{} is a sub-track: drop it on its parent", track.name))
    } else if plugin.is_instrument && !(is_instrument_track && track.plugins.is_empty()) {
        Some(format!(
            "{} is an instrument: drop it on an instrument track with no instrument yet",
            plugin.name
        ))
    } else if !plugin.is_instrument && is_instrument_track && track.plugins.is_empty() {
        Some(format!(
            "{} is an effect: add an instrument to {} first",
            plugin.name, track.name
        ))
    } else {
        None
    };
    if let Some(message) = refusal {
        r.banners.error_message = Some(message);
        return Task::none();
    }
    add_with_preset(
        r,
        PresetAddOwner::Track(track_id),
        PresetAddPick {
            plugin,
            preset_id: row.id,
            preset_name: row.name,
            source: row.source,
        },
    )
}

fn add_with_preset(r: &mut Resonance, owner: PresetAddOwner, pick: PresetAddPick) -> Task<Message> {
    let instance_id = r.allocate_plugin_id();
    let clap_id = pick.plugin.clap_plugin_id.clone();
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
    let task = r.update(message);
    // Only an add that happened (a frozen track's gate refuses it) parks
    // the preset for the echo and counts as a use.
    if r.plugin_slot(instance_id).is_some() {
        record_use(r, &clap_id, &pick.preset_id);
        r.presets
            .pending_plugin_presets
            .insert(instance_id, (clap_id, pick.preset_id, pick.source));
    }
    task
}
