//! `project.*` — new/open/save/save-as with explicit paths (never file
//! dialogs). All four are asynchronous in the app and return
//! [`crate::job::JobStarted`]; poll `job.status` / `job.wait` for
//! completion.
//!
//! Destructive cases require `"confirm": true`: `new`/`open` while the
//! current project has unsaved changes, and saving over an existing
//! file at a *different* path. Without it the server answers with a
//! `needs_confirmation` error summarizing what would be lost.

use serde::{Deserialize, Serialize};

/// `project.new` — start a fresh project ([`NewParams`]) -> job.
pub const NEW: &str = "project.new";
/// `project.open` — open a project from disk ([`OpenParams`]) -> job.
pub const OPEN: &str = "project.open";
/// `project.save` — save in place, or to `path` on first save
/// ([`SaveParams`]) -> job.
pub const SAVE: &str = "project.save";
/// `project.save_as` — save to a new path ([`SaveAsParams`]) -> job.
pub const SAVE_AS: &str = "project.save_as";

/// All `project.*` method names.
pub const METHODS: &[&str] = &[NEW, OPEN, SAVE, SAVE_AS];

/// Params for `project.new`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewParams {
    /// Optional template name; omitted means an empty project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Required (`true`) when the current project has unsaved changes.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `project.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenParams {
    /// Absolute path to the project (`.rproj` directory).
    pub path: String,
    /// Required (`true`) when the current project has unsaved changes.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `project.save`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveParams {
    /// Target path; required only when the project has never been saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Required (`true`) to overwrite an existing file at a new path.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `project.save_as`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveAsParams {
    /// Absolute target path.
    pub path: String,
    /// Required (`true`) to overwrite an existing file at `path`.
    #[serde(default)]
    pub confirm: bool,
}

/// Job payload once a `project.*` job completes: where the project
/// lives now (absent for an unsaved `project.new`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub revision: u64,
}
