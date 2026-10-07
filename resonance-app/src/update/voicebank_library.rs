//! The voicebank library overlay and the install bookkeeping on the update
//! loop (vocal-mcp-control-and-quality.md §5, D15–D17).
//!
//! - [`handle`] runs the overlay: tabs `Installed | plok.org`, search and
//!   favourites over the shared `BrowserModel` and marks (`voicebank:<id>`),
//!   Download behind an explicit accept of the bank's terms and credit,
//!   Cancel, Update, Delete behind a confirmation, and Use (sets the lane's
//!   bank through the lane inspector's own message, so it is one undoable
//!   edit like picking it from the list).
//! - [`poll`] runs from the tick: it collects finished installs, rescans
//!   the registry, and only then resolves a `vocal.voicebank_install`
//!   control job — so a client that waited sees the bank listed — and
//!   rebuilds the overlay's rows when the library or the marks moved.
//!
//! None of this is project state: every message is `UndoAction::Skip`.

use std::rc::Rc;

use iced::Task;
use resonance_audio::types::TrackId;
use resonance_common::library_marks::{kind, mark_key};

use crate::compose::messages::LaneInspectorMsg;
use crate::compose::vocal_svs::registry::library::{delete_user_bank, install_is_current};
use crate::compose::vocal_svs::registry::{InstallErrorKind, InstallJob, PlokIndex, PlokVoicebank};
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::state::voicebank_library::{
    LibraryOverlay, LibraryTab, RowStatus, VoicebankRow, VoicebankRows,
};
use crate::Resonance;

/// The overlay's messages. Transient UI and machine state only.
#[derive(Debug, Clone, PartialEq)]
pub enum VoicebankLibraryMessage {
    /// Open the overlay from a vocal lane's voicebank picker.
    Open { definition_id: u64, track_id: TrackId },
    Close,
    SetTab(LibraryTab),
    Search(String),
    FavoritesOnly(bool),
    /// Select a row (its marks key).
    Select(String),
    /// Toggle a row's favourite (its marks key).
    ToggleFavorite(String),
    /// Rescan the folders and fetch the index again.
    Refresh,
    /// Show bank `id`'s terms and credit; the download starts on
    /// [`Self::AcceptTerms`]. Also Update and Re-download.
    Download(String),
    AcceptTerms,
    DeclineTerms,
    /// Stop bank `id`'s install; nothing is left behind.
    Cancel(String),
    /// Ask to delete the user install of bank `id`.
    Delete(String),
    ConfirmDelete,
    CancelDelete,
    /// Sing the lane the overlay was opened from with bank `id`.
    Use(String),
}

impl VoicebankLibraryMessage {
    /// Exhaustive on purpose (ARCH-06 A6-4): none of these touches the
    /// project. `Use` re-enters `update()` as the lane inspector's own
    /// message, which records its undo entry there.
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            Self::Open { .. }
            | Self::Close
            | Self::SetTab(_)
            | Self::Search(_)
            | Self::FavoritesOnly(_)
            | Self::Select(_)
            | Self::ToggleFavorite(_)
            | Self::Refresh
            | Self::Download(_)
            | Self::AcceptTerms
            | Self::DeclineTerms
            | Self::Cancel(_)
            | Self::Delete(_)
            | Self::ConfirmDelete
            | Self::CancelDelete
            | Self::Use(_) => UndoAction::Skip,
        }
    }
}

pub fn handle(app: &mut Resonance, message: VoicebankLibraryMessage) -> Task<Message> {
    use VoicebankLibraryMessage as M;
    let banks_dir = app.voicebanks.roots().banks.clone();
    match message {
        M::Open {
            definition_id,
            track_id,
        } => {
            app.rescan_voicebanks();
            let lib = &app.voicebank_library.lib;
            let _ = lib.index(banks_dir.as_deref());
            lib.refresh_index(banks_dir, false);
            let mut overlay = LibraryOverlay::new(definition_id, track_id);
            if let Some(bank) = lane_bank(app, definition_id, track_id) {
                overlay.installed.select(mark_key(kind::VOICEBANK, &bank));
            }
            app.voicebank_library.overlay = Some(overlay);
        }
        M::Close => {
            // Installs keep running; the next Open shows their progress.
            app.voicebank_library.overlay = None;
            return Task::none();
        }
        M::SetTab(tab) => {
            if let Some(o) = app.voicebank_library.overlay.as_mut() {
                o.tab = tab;
                o.accept = None;
                o.confirm_delete = None;
            }
            if tab == LibraryTab::Plok {
                app.voicebank_library.lib.refresh_index(banks_dir, false);
            }
        }
        M::Search(query) => {
            if let Some(o) = app.voicebank_library.overlay.as_mut() {
                o.current_browser_mut().set_query(query);
            }
        }
        M::FavoritesOnly(on) => {
            if let Some(o) = app.voicebank_library.overlay.as_mut() {
                o.current_browser_mut().set_favorites_only(on);
            }
        }
        M::Select(key) => {
            if let Some(o) = app.voicebank_library.overlay.as_mut() {
                o.current_browser_mut().select(key);
            }
        }
        M::ToggleFavorite(key) => {
            let marks = crate::plugin_preset_library::marks(app);
            if let Err(e) = marks.toggle_favorite(&key) {
                notice_error(app, format!("favourite: {e}"));
            }
        }
        M::Refresh => {
            app.rescan_voicebanks();
            app.voicebank_library.lib.refresh_index(banks_dir, true);
        }
        M::Download(id) => {
            if let Some(o) = app.voicebank_library.overlay.as_mut() {
                o.confirm_delete = None;
                o.accept = Some(id);
            }
        }
        M::DeclineTerms => {
            if let Some(o) = app.voicebank_library.overlay.as_mut() {
                o.accept = None;
            }
        }
        M::AcceptTerms => {
            let Some(id) = app
                .voicebank_library
                .overlay
                .as_mut()
                .and_then(|o| o.accept.take())
            else {
                return Task::none();
            };
            let roots = app.voicebanks.roots().clone();
            if let Err(e) = app.voicebank_library.lib.start_install(&id, roots, None) {
                notice_error(app, e);
            }
        }
        M::Cancel(id) => app.voicebank_library.lib.cancel(&id),
        M::Delete(id) => {
            if let Some(o) = app.voicebank_library.overlay.as_mut() {
                o.accept = None;
                o.confirm_delete = Some(id);
            }
        }
        M::CancelDelete => {
            if let Some(o) = app.voicebank_library.overlay.as_mut() {
                o.confirm_delete = None;
            }
        }
        M::ConfirmDelete => {
            let Some(id) = app
                .voicebank_library
                .overlay
                .as_mut()
                .and_then(|o| o.confirm_delete.take())
            else {
                return Task::none();
            };
            delete_bank(app, &id);
        }
        M::Use(id) => {
            let Some((definition_id, track_id)) = app
                .voicebank_library
                .overlay
                .as_ref()
                .map(|o| (o.definition_id, o.track_id))
            else {
                return Task::none();
            };
            app.voicebank_library.overlay = None;
            return app.update(Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetVocalVoicebank(resonance_music_theory::VoicebankId::new(
                    &id,
                )),
            }));
        }
    }
    refresh_rows(app, true);
    Task::none()
}

/// The bank the lane `(definition_id, track_id)` sings with.
fn lane_bank(app: &Resonance, definition_id: u64, track_id: TrackId) -> Option<String> {
    app.compose
        .find_definition(definition_id)
        .and_then(|d| d.lane_generators.get(&track_id))
        .and_then(|c| match &c.kind {
            crate::compose::LaneGeneratorKind::Vocal(p) => Some(p.voicebank.as_str().to_owned()),
            _ => None,
        })
}

fn notice_error(app: &mut Resonance, text: String) {
    if let Some(o) = app.voicebank_library.overlay.as_mut() {
        o.current_browser_mut().set_error(text);
    } else {
        app.banners.error_message = Some(text);
    }
}

fn notice_info(app: &mut Resonance, text: String) {
    if let Some(o) = app.voicebank_library.overlay.as_mut() {
        o.current_browser_mut().set_info(text);
    }
}

fn delete_bank(app: &mut Resonance, id: &str) {
    let Some(banks) = app.voicebanks.roots().banks.clone() else {
        return;
    };
    let Some(dir) = app
        .voicebanks
        .get(id)
        .filter(|b| b.user)
        .map(|b| b.scanned_dir.clone())
    else {
        notice_error(app, format!("{id} is not installed in your voicebank folder"));
        return;
    };
    match delete_user_bank(&banks, &dir) {
        Ok(()) => {
            app.rescan_voicebanks();
            notice_info(app, format!("Deleted {id}"));
        }
        Err(e) => notice_error(app, e),
    }
}

/// From the tick: collect finished installs (rescan, then resolve their
/// control jobs and tell the overlay), keep the registry's view of the
/// index current, and rebuild the overlay's rows when anything moved.
pub(crate) fn poll(app: &mut Resonance) {
    let lib = app.voicebank_library.lib.clone();
    let finished = if lib.has_finished() {
        lib.take_finished()
    } else {
        Vec::new()
    };
    if !finished.is_empty() {
        app.rescan_voicebanks();
        for job in finished {
            settle(app, job);
        }
    }
    refresh_rows(app, false);
}

/// Resolve one finished install: its control job, and the overlay's line.
fn settle(app: &mut Resonance, job: InstallJob) {
    let Some(result) = job.result else {
        return;
    };
    match &result {
        Ok(outcome) => {
            let text = if outcome.already_installed {
                format!("{} is already installed", outcome.name)
            } else if !outcome.vocoder_installed {
                format!(
                    "Installed {} — its vocoder {} is not available, so it cannot render yet",
                    outcome.name,
                    outcome.vocoder.as_deref().unwrap_or("?")
                )
            } else {
                format!("Installed {}", outcome.name)
            };
            notice_info(app, text);
        }
        Err(e) if e.kind == InstallErrorKind::Cancelled => {
            notice_info(app, format!("Cancelled {}", job.name));
        }
        Err(e) => notice_error(app, e.message.clone()),
    }
    if let Some(job_id) = job.control_job {
        crate::update::control::voicebank_install_settled(app, job_id, result);
    }
}

impl Resonance {
    /// The library changed outside a message (a control call started an
    /// install): rebuild the overlay's rows now.
    pub(crate) fn voicebank_library_changed(&mut self) {
        refresh_rows(self, true);
    }
}

/// Rebuild the overlay's rows when the library, the marks or (with
/// `force`) anything else changed, and keep the registry's copy of the
/// index's ids current for the `vocal.set_params` typo guard.
fn refresh_rows(app: &mut Resonance, force: bool) {
    let lib = app.voicebank_library.lib.clone();
    let revision = lib.revision();
    if app.voicebank_library.synced_revision != Some(revision) {
        if let Some(index) = lib.state().index.clone() {
            app.voicebanks.set_index(&index);
        }
        app.voicebank_library.synced_revision = Some(revision);
    }
    if app.voicebank_library.overlay.is_none() {
        return;
    }
    let marks = crate::plugin_preset_library::marks(app);
    marks.refresh();
    let key = (revision, marks.generation(), 0);
    let built = app
        .voicebank_library
        .overlay
        .as_ref()
        .and_then(|o| o.built_from);
    if !force && built == Some(key) {
        return;
    }
    let index = lib.index(app.voicebanks.roots().banks.as_deref());
    let (installed, plok) = build_rows(app, index.as_ref(), &marks);
    let Some(o) = app.voicebank_library.overlay.as_mut() else {
        return;
    };
    o.rows_generation += 1;
    let generation = o.rows_generation;
    o.installed_rows = Rc::new(installed);
    o.plok_rows = Rc::new(plok);
    o.installed.refresh(&*o.installed_rows, generation);
    o.plok.refresh(&*o.plok_rows, generation);
    o.built_from = Some(key);
}

fn build_rows(
    app: &Resonance,
    index: Option<&PlokIndex>,
    marks: &resonance_common::library_marks::SharedMarks,
) -> (VoicebankRows, VoicebankRows) {
    let lib = app.voicebank_library.lib.state();
    let installing = |id: &str| {
        lib.job(id).filter(|j| j.result.is_none()).map(|j| RowStatus::Installing {
            label: j.phase.label(),
            fraction: j.phase.fraction(),
        })
    };
    let marks_of = |key: &str| Some(marks.marks(key)).filter(|m| !m.is_default());
    let entry_of = |id: &str| index.and_then(|i| i.voicebank(id));

    let mut installed = Vec::new();
    for bank in app.voicebanks.installed() {
        let profile = app
            .voicebanks
            .profile(&resonance_music_theory::VoicebankId::new(&bank.id));
        let builtin = profile.builtin();
        let plok = bank.plok.as_ref();
        let entry = entry_of(&bank.id);
        let status = installing(&bank.id).unwrap_or_else(|| match entry {
            Some(e) if bank.user && plok.is_some() && !install_is_current(&bank.scanned_dir, e) => {
                RowStatus::UpdateAvailable
            }
            _ => RowStatus::Installed,
        });
        let version = bank
            .version
            .clone()
            .or_else(|| plok.and_then(|p| p.version.clone()));
        let languages: Vec<String> = bank.languages.keys().cloned().collect();
        let source = if bank.user { "user" } else { "dev" };
        let mut sub = Vec::new();
        sub.extend(version.clone());
        if !languages.is_empty() {
            sub.push(languages.join(", "));
        }
        sub.push(if bank.user { "your folder" } else { "development models" }.to_owned());
        if bank.vocoder_config.is_none() {
            sub.push("vocoder missing".to_owned());
        }
        let key = mark_key(kind::VOICEBANK, &bank.id);
        installed.push(VoicebankRow {
            marks: marks_of(&key),
            key,
            id: bank.id.clone(),
            name: bank.name.clone(),
            subtitle: sub.join(" \u{00b7} "),
            version,
            languages,
            size: None,
            credit: plok
                .and_then(|p| p.credit.clone())
                .or_else(|| builtin.and_then(|b| b.credit).map(str::to_owned)),
            terms_url: plok
                .and_then(|p| p.terms_url.clone())
                .or_else(|| builtin.and_then(|b| b.terms_url).map(str::to_owned)),
            commercial: plok
                .and_then(|p| p.commercial.clone())
                .or_else(|| builtin.and_then(|b| b.commercial).map(str::to_owned)),
            vocoder: bank.vocoder_name.clone(),
            vocoder_installed: bank.vocoder_config.is_some(),
            source,
            status,
            user_dir: bank.user.then(|| bank.scanned_dir.clone()),
            english: profile.english(),
        });
    }

    let mut plok = Vec::new();
    for entry in index.map(|i| i.voicebanks.as_slice()).unwrap_or_default() {
        let bank = app.voicebanks.get(&entry.id);
        let status = installing(&entry.id).unwrap_or_else(|| match bank {
            None => RowStatus::NotInstalled,
            Some(b) if b.user && b.plok.is_some() && !install_is_current(&b.scanned_dir, entry) => {
                RowStatus::UpdateAvailable
            }
            Some(_) => RowStatus::Installed,
        });
        plok.push(plok_row(entry, status, bank.is_some_and(|b| b.user), &marks_of));
    }
    (
        VoicebankRows { rows: installed },
        VoicebankRows { rows: plok },
    )
}

fn plok_row(
    entry: &PlokVoicebank,
    status: RowStatus,
    user_install: bool,
    marks_of: &dyn Fn(&str) -> Option<resonance_common::library_marks::Marks>,
) -> VoicebankRow {
    let size = entry.size_text();
    let mut sub = Vec::new();
    sub.extend(entry.version.clone());
    if !entry.languages.is_empty() {
        sub.push(entry.languages.join(", "));
    }
    sub.extend(size.clone());
    let key = mark_key(kind::VOICEBANK, &entry.id);
    VoicebankRow {
        marks: marks_of(&key),
        key,
        id: entry.id.clone(),
        name: entry.name.clone(),
        subtitle: sub.join(" \u{00b7} "),
        version: entry.version.clone(),
        languages: entry.languages.clone(),
        size,
        credit: entry.credit.clone(),
        terms_url: entry.terms_url.clone(),
        commercial: entry.commercial.clone(),
        vocoder: entry.vocoder.clone(),
        vocoder_installed: true,
        source: if user_install { "user" } else { "plok" },
        status,
        user_dir: None,
        english: entry.languages.is_empty() || entry.languages.iter().any(|l| l == "en"),
    }
}
