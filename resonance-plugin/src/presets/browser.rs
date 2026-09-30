//! The preset browser's state and behaviour, GUI-agnostic
//! (plugin-preset-library.md §6.3–§6.5, slice P4).
//!
//! A thin layer over the shared [`BrowserModel`] (search, facets,
//! favourites-first sort, stepping, confirm-in-place delete) and
//! [`AuditionBracket`] (provisional loads that commit or revert), with the
//! preset-specific actions on top: the metadata form (Save as… / Edit
//! info… / marks-only on a factory preset), rename, duplicate, delete to
//! the trash, import and export. The egui skin in [`crate::preset_ui`] only
//! turns clicks into these calls; the iced host skin can drive the same
//! state.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::library_view::{
    Audition, AuditionBracket, AuditionEvent, BrowserModel, Sort, SortKey,
};
use crate::param::Param;

use super::bank::{PresetBank, SaveOptions};
use super::editor::PresetEvent;
use super::format::PresetMeta;
use super::library::PresetRecord;
use super::rows::PresetRows;
use super::session::{PresetSession, SoundSnapshot};
use super::{PresetRef, PresetSource};

/// What the metadata form is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormMode {
    /// A new user preset from the current sound.
    SaveAs,
    /// A user preset's own metadata (by id).
    EditInfo(String),
    /// A factory preset: only the user's marks (star, personal tags).
    MarksOnly(String),
}

/// The metadata form (§6.5).
#[derive(Debug, Clone, PartialEq)]
pub struct MetaForm {
    pub mode: FormMode,
    pub name: String,
    pub meta: PresetMeta,
    /// Personal tags (the only editable tags in `MarksOnly`).
    pub personal_tags: Vec<String>,
    pub favorite: bool,
    /// A user preset already has this name (Save as…): the form shows a
    /// warning with an Overwrite button.
    pub name_clash: bool,
    /// Draft text of each tag-row field, by facet.
    pub drafts: [String; 4],
    pub error: Option<String>,
}

/// The browser.
pub struct PresetBrowser {
    /// The personal-tag completion for the current draft, keyed by the
    /// draft and the tags it excludes (recomputed only when they change).
    pub tag_suggest: Option<(String, Vec<String>)>,
    pub open: bool,
    pub model: BrowserModel,
    rows: PresetRows,
    rows_from: Option<(usize, u64)>,
    audition: AuditionBracket<SoundSnapshot>,
    pub form: Option<MetaForm>,
    /// A row being renamed: its key and the name being typed.
    pub rename: Option<(String, String)>,
    /// Draft of the personal-tag field in the detail pane.
    pub tag_draft: String,
}

impl Default for PresetBrowser {
    fn default() -> Self {
        let mut model = BrowserModel::new();
        // Bank order by default (today's list order), favourites first.
        model.set_sort(Sort::by(SortKey::Natural));
        Self {
            tag_suggest: None,
            open: false,
            model,
            rows: PresetRows::default(),
            rows_from: None,
            audition: AuditionBracket::new(),
            form: None,
            rename: None,
            tag_draft: String::new(),
        }
    }
}

/// The plugin as the audition bracket sees it.
struct Target<'a> {
    bank: &'a PresetBank,
    session: &'a PresetSession,
    params: &'a [&'a dyn Param],
    rows: &'a PresetRows,
}

impl Audition for Target<'_> {
    type Snapshot = SoundSnapshot;
    fn capture(&mut self) -> SoundSnapshot {
        self.session.capture(self.params)
    }
    fn apply(&mut self, key: &str) -> Result<(), String> {
        let row = self
            .rows
            .find(key)
            .ok_or_else(|| "that preset is gone".to_string())?;
        let preset = self.rows.rows[row].record.preset.clone();
        if self.session.load_preset(self.bank, &preset, self.params) {
            Ok(())
        } else {
            Err(format!("Preset '{}' could not be loaded", preset.name))
        }
    }
    fn restore(&mut self, snapshot: SoundSnapshot) {
        self.session.restore(snapshot, self.params);
    }
}

impl PresetBrowser {
    /// The rows, rebuilt when the index or the marks moved. Cheap to call
    /// every frame: the index is read through `records_cached` and the
    /// marks store re-read at most every `max_age`.
    pub fn refresh(&mut self, bank: &PresetBank, max_age: Duration) -> &PresetRows {
        let records = bank.records_cached(max_age);
        let lib = bank.library();
        lib.refresh_marks(max_age);
        let key = (Arc::as_ptr(&records) as usize, lib.marks().generation());
        if self.rows_from != Some(key) {
            let marks = lib.marks();
            self.rows =
                PresetRows::build([(bank.plugin_id(), records.as_slice())], marks.as_ref());
            self.rows_from = Some(key);
            self.model.invalidate();
        }
        self.model.refresh(&self.rows, (key.0 as u64, key.1));
        &self.rows
    }

    /// Rebuild on the next [`refresh`](Self::refresh) (after a write).
    pub fn invalidate(&mut self) {
        self.rows_from = None;
    }

    pub fn rows(&self) -> &PresetRows {
        &self.rows
    }

    /// The record of `key`, if listed.
    pub fn record(&self, key: &str) -> Option<&PresetRecord> {
        self.rows.find(key).map(|r| &self.rows.rows[r].record)
    }

    fn key_of(bank: &PresetBank, preset: &PresetRef) -> String {
        super::marks::mark_key(bank.plugin_id(), &preset.id)
    }

    /// Open the browser with the loaded preset selected.
    pub fn open(&mut self, bank: &PresetBank, session: &PresetSession) {
        self.open = true;
        if let Some(c) = session.current().filter(|c| c.is_resolved()) {
            self.model.select(Self::key_of(bank, &c));
        }
    }

    /// Whether an audition is in progress (a revert snapshot is held).
    pub fn auditioning(&self) -> bool {
        self.audition.is_open()
    }

    /// Load `key` provisionally (↑/↓, a single click).
    pub fn audition(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        params: &[&dyn Param],
        key: &str,
    ) -> PresetEvent {
        let mut target = Target {
            bank,
            session,
            params,
            rows: &self.rows,
        };
        match self.audition.audition(&mut target, key) {
            AuditionEvent::Auditioned(_) => match session.current() {
                Some(p) => PresetEvent::Auditioned(p),
                None => PresetEvent::None,
            },
            AuditionEvent::Failed(e) => {
                self.model.set_error(e);
                PresetEvent::None
            }
            _ => PresetEvent::None,
        }
    }

    /// Keep what is loaded (Enter, a double-click, a click outside), and
    /// record the pick in the recents.
    pub fn commit(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        params: &[&dyn Param],
        key: Option<&str>,
    ) -> PresetEvent {
        // A double-click on a row that was not auditioned yet loads it.
        if let Some(key) = key {
            if self.audition.provisional() != Some(key) {
                self.audition(bank, session, params, key);
            }
        }
        match self.audition.commit() {
            AuditionEvent::Committed(_) => match session.current() {
                Some(p) => {
                    if let Err(e) = bank.library().record_use(bank.plugin_id(), &p.id) {
                        tracing::debug!("preset recents: {e}");
                    }
                    self.invalidate();
                    PresetEvent::Loaded(p)
                }
                None => PresetEvent::None,
            },
            _ => PresetEvent::None,
        }
    }

    /// Put the pre-audition sound back (Esc, ×).
    pub fn revert(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        params: &[&dyn Param],
    ) -> PresetEvent {
        let mut target = Target {
            bank,
            session,
            params,
            rows: &self.rows,
        };
        match self.audition.revert(&mut target) {
            AuditionEvent::Reverted => PresetEvent::Reverted,
            _ => PresetEvent::None,
        }
    }

    /// Close the browser: an open audition reverts (× and Esc) or commits
    /// (a click outside), as the caller says.
    pub fn close(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        params: &[&dyn Param],
        keep: bool,
    ) -> PresetEvent {
        let event = if keep {
            self.commit(bank, session, params, None)
        } else {
            self.revert(bank, session, params)
        };
        self.open = false;
        self.form = None;
        self.rename = None;
        self.model.cancel_delete();
        event
    }

    /// ☆/★ on `key`.
    pub fn toggle_favorite(&mut self, bank: &PresetBank, key: &str) {
        let Some(record) = self.record(key).cloned() else {
            return;
        };
        let lib = bank.library();
        let fav = lib.preset_marks(bank.plugin_id(), &record.preset.id).favorite;
        if let Err(e) = lib.set_favorite(bank.plugin_id(), &record.preset.id, !fav) {
            self.model.set_error(e);
        }
        self.invalidate();
    }

    /// Add or remove one personal tag on `key` (factory presets too).
    pub fn edit_personal_tag(&mut self, bank: &PresetBank, key: &str, tag: &str, add: bool) {
        let Some(record) = self.record(key).cloned() else {
            return;
        };
        let lib = bank.library();
        let mut tags = lib.preset_marks(bank.plugin_id(), &record.preset.id).tags;
        if add {
            tags.push(tag.to_string());
        } else {
            let tag = resonance_common::library_marks::normalize_tag(tag).unwrap_or_default();
            tags.retain(|t| *t != tag);
        }
        if let Err(e) = lib.set_personal_tags(bank.plugin_id(), &record.preset.id, &tags) {
            self.model.set_error(e);
        }
        self.invalidate();
    }

    // -- The metadata form ------------------------------------------------

    /// Save as…: the form pre-filled from the loaded preset (its facets;
    /// the name as the bar always proposed it; §6.4).
    pub fn begin_save_as(&mut self, bank: &PresetBank, session: &PresetSession) {
        let loaded = session.current().and_then(|c| bank.record(&c));
        let mut meta = loaded.as_ref().map(|r| r.meta.clone()).unwrap_or_default();
        meta.created = None;
        meta.modified = None;
        meta.derived_from = None;
        let name = match session.current() {
            Some(p) if p.source == PresetSource::User => format!("{} copy", p.name),
            Some(p) => format!("{} (edit)", p.name),
            None => "My Preset".to_string(),
        };
        let name = bank.library().unique_name(bank.plugin_id(), &name);
        meta.name = name.clone();
        self.form = Some(MetaForm {
            mode: FormMode::SaveAs,
            name,
            meta,
            personal_tags: Vec::new(),
            favorite: false,
            name_clash: false,
            drafts: Default::default(),
            error: None,
        });
    }

    /// Edit info… on `key`: the whole form for a user preset, marks only
    /// for a factory one.
    pub fn begin_edit(&mut self, bank: &PresetBank, key: &str) {
        let Some(record) = self.record(key).cloned() else {
            return;
        };
        let marks = bank.library().preset_marks(bank.plugin_id(), &record.preset.id);
        let mode = match record.preset.source {
            PresetSource::User => FormMode::EditInfo(record.preset.id.clone()),
            PresetSource::Factory => FormMode::MarksOnly(record.preset.id.clone()),
        };
        self.form = Some(MetaForm {
            mode,
            name: record.meta.name.clone(),
            meta: record.meta.clone(),
            personal_tags: marks.tags,
            favorite: marks.favorite,
            name_clash: false,
            drafts: Default::default(),
            error: None,
        });
    }

    /// Commit the form. `overwrite` confirms a Save as… over a user
    /// preset of the same name.
    pub fn submit_form(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        params: &[&dyn Param],
        overwrite: bool,
    ) -> PresetEvent {
        let Some(form) = self.form.clone() else {
            return PresetEvent::None;
        };
        let lib = bank.library();
        let result = match &form.mode {
            FormMode::SaveAs => {
                let clash = bank
                    .list_user()
                    .iter()
                    .any(|p| p.name.trim().eq_ignore_ascii_case(form.name.trim()));
                if clash && !overwrite {
                    if let Some(f) = &mut self.form {
                        f.name_clash = true;
                    }
                    return PresetEvent::None;
                }
                let loaded = session.current().filter(|c| c.is_resolved());
                bank.save_with(
                    &form.name,
                    params,
                    SaveOptions {
                        meta: Some(form.meta.clone()),
                        derived_from: loaded.map(|c| c.id),
                        extra: session_extra(session),
                        ..SaveOptions::default()
                    },
                )
                .inspect(|saved| session.set_current_with_baseline(Some(saved.clone()), params))
                .map(PresetEvent::Saved)
            }
            FormMode::EditInfo(id) => {
                let preset = PresetRef::user(id.clone(), form.name.clone());
                let renamed = match bank.record(&preset) {
                    Some(r) if r.meta.name != form.name.trim() => {
                        bank.rename(&preset, &form.name).map(|_| ())
                    }
                    _ => Ok(()),
                };
                renamed
                    .and_then(|()| {
                        lib.update_meta(bank.plugin_id(), &preset, |m| {
                            let name = m.name.clone();
                            *m = PresetMeta {
                                name,
                                created: m.created.clone(),
                                derived_from: m.derived_from.clone(),
                                ..form.meta.clone()
                            };
                        })
                    })
                    .map(|r| PresetEvent::MetaChanged(r.preset))
            }
            FormMode::MarksOnly(id) => lib
                .set_personal_tags(bank.plugin_id(), id, &form.personal_tags)
                .and_then(|_| lib.set_favorite(bank.plugin_id(), id, form.favorite))
                .map(|_| PresetEvent::None),
        };
        match result {
            Ok(event) => {
                self.form = None;
                self.invalidate();
                event
            }
            Err(e) => {
                if let Some(f) = &mut self.form {
                    f.error = Some(e);
                }
                PresetEvent::None
            }
        }
    }

    // -- Rename / duplicate / delete / import / export --------------------

    pub fn begin_rename(&mut self, key: &str) {
        if let Some(r) = self.record(key) {
            if r.preset.source == PresetSource::User {
                self.rename = Some((key.to_string(), r.meta.name.clone()));
            }
        }
    }

    pub fn submit_rename(&mut self, bank: &PresetBank, session: &PresetSession) -> PresetEvent {
        let Some((key, name)) = self.rename.clone() else {
            return PresetEvent::None;
        };
        let Some(record) = self.record(&key).cloned() else {
            self.rename = None;
            return PresetEvent::None;
        };
        match session.rename(bank, &record.preset, &name) {
            Ok(p) => {
                self.rename = None;
                self.invalidate();
                PresetEvent::Renamed(p)
            }
            Err(e) => {
                self.model.set_error(e);
                PresetEvent::None
            }
        }
    }

    pub fn duplicate(&mut self, bank: &PresetBank, key: &str) -> PresetEvent {
        let Some(record) = self.record(key).cloned() else {
            return PresetEvent::None;
        };
        match bank.duplicate(&record.preset) {
            Ok(copy) => {
                self.invalidate();
                self.model.select(Self::key_of(bank, &copy));
                self.model.set_info(format!("Duplicated as '{}'", copy.name));
                PresetEvent::Saved(copy)
            }
            Err(e) => {
                self.model.set_error(e);
                PresetEvent::None
            }
        }
    }

    /// The second click of the confirm-in-place delete (the key comes
    /// from `BrowserModel::confirm_delete`). User presets go to the
    /// trash; the loaded one keeps sounding with its identity cleared.
    pub fn delete(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        key: &str,
    ) -> PresetEvent {
        let Some(record) = self.record(key).cloned() else {
            return PresetEvent::None;
        };
        match session.delete(bank, &record.preset) {
            Ok(()) => {
                self.invalidate();
                self.model
                    .set_info(format!("'{}' moved to the trash", record.meta.name));
                PresetEvent::Deleted(record.preset)
            }
            Err(e) => {
                self.model.set_error(e);
                PresetEvent::None
            }
        }
    }

    pub fn import(
        &mut self,
        bank: &PresetBank,
        params: &[&dyn Param],
        path: &Path,
    ) -> PresetEvent {
        let ids: Vec<&str> = params.iter().map(|p| p.id()).collect();
        match bank.import(path, &ids) {
            Ok((p, reminted)) => {
                self.invalidate();
                self.model.select(Self::key_of(bank, &p));
                self.model.set_info(if reminted {
                    format!("Imported '{}' (its id was taken, so it got a new one)", p.name)
                } else {
                    format!("Imported '{}'", p.name)
                });
                PresetEvent::Imported(p)
            }
            Err(e) => {
                self.model.set_error(e);
                PresetEvent::None
            }
        }
    }

    pub fn export(&mut self, bank: &PresetBank, key: &str, path: &Path) {
        let Some(record) = self.record(key).cloned() else {
            return;
        };
        match bank.export(&record.preset, path) {
            Ok(()) => self.model.set_info(format!("Exported to {}", path.display())),
            Err(e) => self.model.set_error(e),
        }
    }
}

/// The sound-bearing extra state a new preset stores.
fn session_extra(session: &PresetSession) -> serde_json::Map<String, serde_json::Value> {
    use crate::plugin::ExtraStateSaver;
    session.save_for_preset()
}
