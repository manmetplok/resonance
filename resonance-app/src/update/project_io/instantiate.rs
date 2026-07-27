//! Instantiate a template as a fresh, untitled project (impl-plan doc #197,
//! todo #665).
//!
//! Both a built-in starter and a user template become a [`LoadedProject`]
//! that is replayed through the normal open path — *except* the project path
//! is left `None`. That has two consequences the feature relies on:
//!
//! * The next Save sees no path and becomes Save-As (see
//!   [`super::handle`], `SaveProject`), so the template source is **never**
//!   overwritten.
//! * The template is not added to the recent-projects list — a template is a
//!   source, not a project the user opened.
//!
//! Built-ins are built in memory (no disk read); user templates load from
//! disk via [`crate::project::load_project`] over the template folder, which
//! has the same on-disk shape as a saved project. The actual rebuild runs in
//! [`crate::engine_events`]' `all_cleared` once the engine confirms the clear
//! with `AudioEvent::AllCleared`, exactly as a normal project open does.

use std::collections::HashMap;

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::project::LoadedProject;
use crate::Resonance;

use super::templates::BuiltinTemplateId;

/// Instantiate a built-in starter as a fresh untitled project. The starter is
/// built entirely in memory (no disk read), so callers pass the stable
/// [`BuiltinTemplateId`] rather than the (localizable) display name.
pub fn instantiate_builtin(r: &mut Resonance, id: BuiltinTemplateId) {
    let built = id.build();
    let loaded = Box::new(LoadedProject {
        file: built.file,
        // Built-ins ship no files: an empty directory matches a brand-new
        // project and keeps the engine from being pointed at any source
        // folder. Save-As sets a real directory on the first save.
        project_dir: std::path::PathBuf::new(),
        midi_notes: built.midi_notes,
        plugin_states: HashMap::new(),
    });
    begin_instantiate(r, loaded);
}

/// Async load task for a *user* template, analogous to
/// [`super::dialogs::load_project_task`]. Reads the template folder from disk
/// (same on-disk shape as a project) and routes the result back through
/// [`ProjectIoMessage::TemplateLoaded`], whose handler replays it as a fresh
/// untitled project.
pub fn load_user_template_task(path: std::path::PathBuf) -> Task<Message> {
    Task::perform(
        async move { crate::project::load_project(&path).map(Box::new) },
        |r| Message::ProjectIo(ProjectIoMessage::TemplateLoaded(r)),
    )
}

/// Replay `loaded` as a fresh untitled project. Mirrors the
/// [`ProjectIoMessage::ProjectLoaded`] open path, but forces the project path
/// to `None` (so the next Save becomes Save-As and the source is never
/// overwritten) and never touches the recent-projects list. The rebuild
/// itself happens in `engine_events::project_io::all_cleared` once the engine
/// confirms the clear with `AudioEvent::AllCleared`.
pub fn begin_instantiate(r: &mut Resonance, loaded: Box<LoadedProject>) {
    let _ = r.engine.send(AudioCommand::Stop);
    r.transport.playing = false;
    r.transport.recording = false;
    // Drop any previously-open project's path *before* the clear/replay so
    // `all_cleared` can't restore it: a template always lands untitled.
    r.io.project_path = None;
    r.io.loading = true;
    r.io.pending_load = Some(loaded);
    r.undo.clear();
    r.plugin_state_cache.clear();
    r.dirty = false;
    let _ = r.engine.send(AudioCommand::ClearAll);
    r.io.has_active_project = true;
}
