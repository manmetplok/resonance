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
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NewParams {
    /// Optional template: a built-in template slug (e.g. `"empty"`,
    /// `"beatmaking"`) or a user template name; omitted means an empty
    /// project. `template_id` is accepted as an alias.
    #[serde(
        default,
        alias = "template_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub template: Option<String>,
    /// Required (`true`) when the current project has unsaved changes.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `project.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct OpenParams {
    /// Absolute path to the project (`.rproj` directory).
    pub path: String,
    /// Required (`true`) when the current project has unsaved changes.
    #[serde(default)]
    pub confirm: bool,
    /// Open the project's autosave instead of its last saved version when
    /// an unclean exit left a newer one behind (the job result's
    /// `autosave_available` says so). The recovered project opens with
    /// unsaved changes; `project.save` writes it. Default `false`: the last
    /// saved version opens.
    #[serde(default)]
    pub recover_autosave: bool,
}

/// Params for `project.save`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
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
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
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
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ProjectResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub revision: u64,
    /// `project.open` only: the project's autosave was opened
    /// (`recover_autosave`), so the project has unsaved changes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recovered_autosave: bool,
    /// `project.open` only: an unclean exit left an autosave newer than
    /// the saved project. Absent when there is none.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub autosave_available: bool,
}
