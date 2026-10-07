//! The voicebank library overlay's state (vocal-mcp-control-and-quality.md
//! §5, D15): the downloads ([`VoicebankLibrary`]) and, while the overlay
//! is open, its tabs, browsers and the rows they show.
//!
//! The rows are rebuilt in `update::voicebank_library` whenever the
//! registry, the library or the marks change — never in `view` — so a
//! frame only reads them (ux-guidelines.md, view performance).

use std::borrow::Cow;
use std::path::PathBuf;
use std::rc::Rc;

use resonance_audio::types::TrackId;
use resonance_common::library_marks::Marks;
use resonance_plugin::library_view::{BrowserModel, LibraryRows, SortValue, SOURCE_FACET};

use crate::compose::vocal_svs::registry::{LibraryConfig, VoicebankLibrary};

/// The library and, while it shows, the overlay.
#[derive(Debug)]
pub struct VoicebankLibraryState {
    pub lib: VoicebankLibrary,
    pub overlay: Option<LibraryOverlay>,
    /// The library revision the registry's copy of the index ids was last
    /// synced at.
    pub synced_revision: Option<u64>,
}

impl VoicebankLibraryState {
    pub fn new(config: LibraryConfig) -> Self {
        Self {
            lib: VoicebankLibrary::new(config),
            overlay: None,
            synced_revision: None,
        }
    }
}

/// The overlay's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryTab {
    #[default]
    Installed,
    Plok,
}

/// The open overlay: which lane it was opened from, the tab, one browser
/// per tab, and the confirmation it is asking for.
#[derive(Debug)]
pub struct LibraryOverlay {
    /// The lane whose voicebank picker opened it (Use sets its bank).
    pub definition_id: u64,
    pub track_id: TrackId,
    pub tab: LibraryTab,
    pub installed: BrowserModel,
    pub plok: BrowserModel,
    pub installed_rows: Rc<VoicebankRows>,
    pub plok_rows: Rc<VoicebankRows>,
    /// The bank whose terms are shown, waiting for an explicit accept
    /// before its download starts.
    pub accept: Option<String>,
    /// The bank whose Delete is waiting for a confirmation.
    pub confirm_delete: Option<String>,
    /// What the rows were built from: (library revision, marks
    /// generation, registry revision).
    pub built_from: Option<(u64, u64, u64)>,
    /// The library's progress revision the rows' statuses show.
    pub built_progress: u64,
    /// Bumped on every rows rebuild: the browsers' cache key.
    pub rows_generation: u64,
    /// Bumped when a running install's row status is patched in place
    /// (the list's redraw key; the browsers do not re-filter).
    pub status_generation: u64,
    /// The tab labels with their counts, built with the rows.
    pub tab_labels: (String, String),
}

impl LibraryOverlay {
    pub fn new(definition_id: u64, track_id: TrackId) -> Self {
        Self {
            definition_id,
            track_id,
            tab: LibraryTab::Installed,
            installed: BrowserModel::new(),
            plok: BrowserModel::new(),
            installed_rows: Rc::default(),
            plok_rows: Rc::default(),
            accept: None,
            confirm_delete: None,
            built_from: None,
            built_progress: 0,
            rows_generation: 0,
            status_generation: 0,
            tab_labels: ("Installed".to_owned(), "plok.org".to_owned()),
        }
    }

    /// The current tab's browser and rows.
    pub fn current(&self) -> (&BrowserModel, &VoicebankRows) {
        match self.tab {
            LibraryTab::Installed => (&self.installed, &self.installed_rows),
            LibraryTab::Plok => (&self.plok, &self.plok_rows),
        }
    }

    pub fn current_browser_mut(&mut self) -> &mut BrowserModel {
        match self.tab {
            LibraryTab::Installed => &mut self.installed,
            LibraryTab::Plok => &mut self.plok,
        }
    }
}

/// Where a row's bank stands.
#[derive(Debug, Clone, PartialEq)]
pub enum RowStatus {
    /// Installed, and (for an index entry) as the index lists it.
    Installed,
    NotInstalled,
    /// Installed, but the index lists another version or file.
    UpdateAvailable,
    /// Downloading or unpacking.
    Installing { label: String, fraction: Option<f32> },
}

/// One bank as the overlay lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct VoicebankRow {
    /// The marks key, `voicebank:<id>`.
    pub key: String,
    pub id: String,
    pub name: String,
    /// The secondary line, built once.
    pub subtitle: String,
    pub version: Option<String>,
    pub languages: Vec<String>,
    pub size: Option<String>,
    pub credit: Option<String>,
    pub terms_url: Option<String>,
    /// `free_with_credit`, `paid` or `permission`.
    pub commercial: Option<String>,
    pub vocoder: Option<String>,
    pub vocoder_installed: bool,
    /// The detail pane's vocoder line, built with the row.
    pub vocoder_line: String,
    /// `user` (the user folder), `dev` (a development root) or `plok` (an
    /// index entry not installed).
    pub source: &'static str,
    pub status: RowStatus,
    /// The user-folder install, which Delete removes.
    pub user_dir: Option<PathBuf>,
    /// Can sing English (only English banks can be used on a lane).
    pub english: bool,
    pub marks: Option<Marks>,
}

impl VoicebankRow {
    /// The commercial-use summary in words.
    pub fn commercial_text(&self) -> Option<&'static str> {
        Some(match self.commercial.as_deref()? {
            "free_with_credit" => "Commercial use of renders is free with the credit line",
            "paid" => "Commercial use needs a paid licence",
            "permission" => "Commercial use needs the creator's written permission",
            _ => return None,
        })
    }
}

/// A tab's rows, in display order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoicebankRows {
    pub rows: Vec<VoicebankRow>,
}

impl VoicebankRows {
    pub fn row(&self, key: &str) -> Option<&VoicebankRow> {
        self.rows.iter().find(|r| r.key == key)
    }

    pub fn by_id(&self, id: &str) -> Option<&VoicebankRow> {
        self.rows.iter().find(|r| r.id == id)
    }
}

/// The facet a `lang:<code>` search token scopes.
pub const LANGUAGE_FACET: &str = "lang";

impl LibraryRows for VoicebankRows {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, row: usize) -> &str {
        &self.rows[row].key
    }

    fn title(&self, row: usize) -> &str {
        &self.rows[row].name
    }

    fn subtitle(&self, row: usize) -> String {
        self.rows[row].subtitle.clone()
    }

    fn column(&self, row: usize, col: usize) -> Option<Cow<'_, str>> {
        let r = &self.rows[row];
        match col {
            0 => r.version.as_deref().map(Cow::Borrowed),
            1 => r.size.as_deref().map(Cow::Borrowed),
            _ => None,
        }
    }

    fn marks(&self, row: usize) -> Option<&Marks> {
        self.rows[row].marks.as_ref()
    }

    fn search_text(&self, row: usize) -> Vec<&str> {
        let r = &self.rows[row];
        let mut out = vec![r.id.as_str()];
        out.extend(r.credit.as_deref());
        out.extend(r.vocoder.as_deref());
        out
    }

    fn facet_names(&self) -> Vec<&str> {
        vec![LANGUAGE_FACET]
    }

    fn facet_values(&self, row: usize, facet: &str) -> Vec<&str> {
        let r = &self.rows[row];
        match facet {
            SOURCE_FACET => vec![r.source],
            LANGUAGE_FACET => r.languages.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        }
    }

    fn sort_value(&self, row: usize, field: &str) -> SortValue {
        match field {
            "id" => SortValue::Text(self.rows[row].id.clone()),
            _ => SortValue::None,
        }
    }
}
