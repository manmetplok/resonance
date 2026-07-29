//! `project.*` control handlers (ba doc #265, todo #1151): new / open /
//! save / save-as with EXPLICIT paths, as jobs.
//!
//! All four route through the existing project-I/O internals — the
//! path-carrying messages (`OpenPathSelected` / `SavePathSelected` /
//! `SaveProject`) and the `instantiate.rs` fresh-project/template path.
//! The rfd dialog variants (`OpenProject` / `SaveProjectAs`) are never
//! reachable from control.
//!
//! Each method validates its params up front (absolute path, existing
//! parent, `.rproj` extension applied exactly like the GUI save path),
//! then registers a job on the [`JobBoard`](crate::control_jobs::JobBoard)
//! and dispatches the real operation. Completion is keyed off the
//! existing completion points via [`JobToken`]s: `ProjectSaved` /
//! `ProjectLoaded` (wired in todo #1149) and, for `project.new`, the
//! engine-confirmed clear+replay (`engine_events::project_io::all_cleared`).
//!
//! Guards (doc #265):
//! - `new`/`open` while the current project has unsaved changes, and
//!   saving over an existing path that isn't the project's own, require
//!   `"confirm": true` → `needs_confirmation` otherwise.
//! - A save/load already in flight → `busy` (a second collector would
//!   clobber the first).

use crate::control_jobs::JobToken;
use crate::control_socket::ConnId;
use crate::message::{Message, ProjectIoMessage};
use crate::update::project_io::{
    load_user_template_task, scan_user_templates, BuiltinTemplateId, TemplateEntry,
};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::project::{
    self as proto, NewParams, OpenParams, SaveAsParams, SaveParams,
};
use resonance_control::{Request, Response, RpcError};
use std::path::{Path, PathBuf};

/// Handle a `project.*` request, or `None` when `method` belongs to
/// another namespace. Mutating: the returned [`Task`] must reach the
/// runtime.
pub(super) fn try_handle(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::NEW => new(app, conn, request),
        proto::OPEN => open(app, conn, request),
        proto::SAVE => save(app, conn, request),
        proto::SAVE_AS => save_as(app, conn, request),
        _ => return None,
    };
    Some(handled)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `project.new {template?}` — instantiate a fresh untitled project from
/// a built-in starter (by slug) or a user template (by name). Returns a
/// job that completes once the engine confirms the clear and the fresh
/// project has replayed.
fn new(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: NewParams = match request.params() {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    if let Some(error) = busy_guard(app) {
        return (super::failure(request, error), Task::none());
    }
    if let Some(error) = dirty_guard(app, params.confirm, "project.new") {
        return (super::failure(request, error), Task::none());
    }

    match resolve_template(params.template.as_deref()) {
        Ok(ResolvedTemplate::Builtin(id)) => {
            let started = app.start_control_job(
                proto::NEW,
                &format!("New project from built-in template '{}'", id.slug()),
                JobToken::ProjectNew,
                Some(conn),
            );
            crate::update::project_io::instantiate_builtin(app, id);
            (super::success(request, &started), Task::none())
        }
        Ok(ResolvedTemplate::User(path)) => {
            let started = app.start_control_job(
                proto::NEW,
                &format!("New project from user template at {}", path.display()),
                JobToken::ProjectNew,
                Some(conn),
            );
            // Async template load; lands in `TemplateLoaded`, whose Ok arm
            // instantiates (→ `all_cleared` completes the job) and whose
            // Err arm fails the token.
            (
                super::success(request, &started),
                load_user_template_task(path),
            )
        }
        Err(error) => (super::failure(request, error), Task::none()),
    }
}

/// `project.open {path}` — open a project from an explicit absolute
/// path, as a job resolved by `ProjectLoaded`.
fn open(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: OpenParams = match request.params() {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    if let Some(error) = busy_guard(app) {
        return (super::failure(request, error), Task::none());
    }
    if let Some(error) = dirty_guard(app, params.confirm, "project.open") {
        return (super::failure(request, error), Task::none());
    }

    let path = Path::new(&params.path);
    if !path.is_absolute() {
        return (
            super::failure(
                request,
                RpcError::invalid_params(format!("path must be absolute, got {:?}", params.path)),
            ),
            Task::none(),
        );
    }
    if !path.exists() {
        return (
            super::failure(
                request,
                RpcError::not_found(format!("no project at {:?}", params.path)),
            ),
            Task::none(),
        );
    }

    let started = app.start_control_job(
        proto::OPEN,
        &format!("Open project {}", path.display()),
        JobToken::ProjectLoad,
        Some(conn),
    );
    let task = super::run_via_update(
        app,
        Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(params.path))),
    );
    (super::success(request, &started), task)
}

/// `project.save {path?}` — save in place, or establish `path` on a
/// never-saved project.
fn save(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: SaveParams = match request.params() {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    save_impl(app, conn, request, params.path, params.confirm)
}

/// `project.save_as {path}` — save to an explicit new path.
fn save_as(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: SaveAsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    save_impl(app, conn, request, Some(params.path), params.confirm)
}

/// Shared save kickoff: validate, register the job, dispatch through the
/// path-carrying save messages, and fail the job immediately if the save
/// never started (e.g. the project directory could not be created).
fn save_impl(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
    path: Option<String>,
    confirm: bool,
) -> (Response, Task<Message>) {
    if let Some(error) = busy_guard(app) {
        return (super::failure(request, error), Task::none());
    }

    let message = match path {
        Some(raw) => {
            // Mirror the GUI save path (`SavePathSelected`): the `.rproj`
            // project-directory extension is appended when missing, so the
            // overwrite check below sees the real target.
            let normalized = if raw.ends_with(".rproj") {
                raw
            } else {
                format!("{raw}.rproj")
            };
            let target = Path::new(&normalized);
            if !target.is_absolute() {
                return (
                    super::failure(
                        request,
                        RpcError::invalid_params(format!(
                            "path must be absolute, got {normalized:?}"
                        )),
                    ),
                    Task::none(),
                );
            }
            if !target.parent().is_some_and(Path::is_dir) {
                return (
                    super::failure(
                        request,
                        RpcError::invalid_params(format!(
                            "parent directory of {normalized:?} does not exist"
                        )),
                    ),
                    Task::none(),
                );
            }
            // Overwriting an existing project at a *different* path is
            // destructive; saving over the project's own path is a normal
            // save and needs no confirmation.
            let is_current = app.io.project_path.as_deref() == Some(target);
            if target.exists() && !is_current && !confirm {
                return (
                    super::failure(
                        request,
                        RpcError::needs_confirmation(format!(
                            "{normalized:?} already exists and would be overwritten; \
                             pass \"confirm\": true to replace it"
                        )),
                    ),
                    Task::none(),
                );
            }
            ProjectIoMessage::SavePathSelected(Some(normalized))
        }
        None => {
            if app.io.project_path.is_none() {
                return (
                    super::failure(
                        request,
                        RpcError::invalid_params("the project has never been saved; pass \"path\""),
                    ),
                    Task::none(),
                );
            }
            ProjectIoMessage::SaveProject
        }
    };

    let started = app.start_control_job(
        &request.method,
        "Save project",
        JobToken::ProjectSave,
        Some(conn),
    );
    let task = super::run_via_update(app, Message::ProjectIo(message));

    // `begin_save` bails without a completion message when it cannot
    // create the project directory — fail the job now instead of leaving
    // the client to wait out the timeout.
    if !app.io.saving && app.io.save_state.is_none() {
        let reason = app
            .error_message
            .clone()
            .unwrap_or_else(|| "save did not start".to_owned());
        app.control.jobs.fail(u64::from(started.job_id), reason);
    }
    (super::success(request, &started), task)
}

// ---------------------------------------------------------------------------
// Guards + helpers
// ---------------------------------------------------------------------------

/// `busy` while a project load or save is already in flight — a second
/// save collector would clobber the first, and a load swaps the whole
/// project out from under any concurrent operation.
fn busy_guard(app: &Resonance) -> Option<RpcError> {
    if app.io.loading || app.io.pending_load.is_some() {
        return Some(RpcError::busy("a project load is in progress"));
    }
    if app.io.saving || app.io.save_state.is_some() {
        return Some(RpcError::busy("a project save is in progress"));
    }
    None
}

/// `needs_confirmation` when the open project has unsaved changes that
/// `action` would discard.
fn dirty_guard(app: &Resonance, confirm: bool, action: &str) -> Option<RpcError> {
    if app.dirty && !confirm {
        return Some(RpcError::needs_confirmation(format!(
            "the current project has unsaved changes that {action} would discard; \
             pass \"confirm\": true to proceed"
        )));
    }
    None
}

enum ResolvedTemplate {
    Builtin(BuiltinTemplateId),
    User(PathBuf),
}

/// Resolve the `template` param: `None` → the empty built-in; otherwise
/// a built-in slug, then a user template by exact name.
fn resolve_template(spec: Option<&str>) -> Result<ResolvedTemplate, RpcError> {
    let Some(spec) = spec else {
        return Ok(ResolvedTemplate::Builtin(BuiltinTemplateId::Empty));
    };
    if let Some(id) = BuiltinTemplateId::ALL.iter().find(|id| id.slug() == spec) {
        return Ok(ResolvedTemplate::Builtin(*id));
    }
    for entry in scan_user_templates() {
        if let TemplateEntry::Valid(template) = entry {
            if template.name == spec {
                return Ok(ResolvedTemplate::User(template.path));
            }
        }
    }
    let slugs: Vec<&str> = BuiltinTemplateId::ALL.iter().map(|id| id.slug()).collect();
    Err(RpcError::invalid_params(format!(
        "unknown template {spec:?}; built-in slugs: {}, or a user template name",
        slugs.join(", ")
    )))
}
