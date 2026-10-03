//! Non-blocking native file/folder picker for plugin editors (code
//! review PUX-07).
//!
//! `rfd::FileDialog` runs its dialog *synchronously* on the calling
//! thread. Called straight from `EditorApp::ui()` on Linux, that blocks
//! the Wayland editor thread for as long as the dialog is up: no
//! repaint, no Wayland dispatch, and a host `destroy()` that lands
//! while it's open pays the full `DESTROY_JOIN_TIMEOUT`
//! (`wayland-plugin-gui/src/editor.rs`) and then leaks the thread.
//!
//! [`FilePicker`] runs the dialog on a thread of its own and is polled
//! from `ui()` instead — the pattern the drums editor already used
//! (`resonance-drums/src/editor/jobs.rs::Picker`), generalised so every
//! plugin editor can share it instead of re-implementing it.
//!
//! On Cocoa the dialog already runs inside a guarded modal run loop
//! (the AppKit main thread, checked by the `modal_reentrancy` test), so
//! Cocoa keeps calling `rfd` directly on the main thread and has no use
//! for this type — callers gate the two paths with
//! `#[cfg(target_os = "macos")]` / `#[cfg(not(target_os = "macos"))]`.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;
use plugin_gui_core::egui;

/// What the dialog should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialogKind {
    OpenFile,
    OpenFiles,
    OpenFolder,
    SaveFile,
}

/// A configured dialog, built with the methods below and handed to
/// [`FilePicker::start`]. Everything here is owned data (no `egui` or
/// `rfd` types), so building one never touches the dialog itself —
/// that only happens on the picker's background thread.
pub struct FileDialogRequest {
    kind: DialogKind,
    title: Option<String>,
    filter: Option<(String, Vec<String>)>,
    file_name: Option<String>,
}

impl FileDialogRequest {
    fn new(kind: DialogKind) -> Self {
        Self { kind, title: None, filter: None, file_name: None }
    }

    /// Pick a single existing file.
    pub fn open_file() -> Self {
        Self::new(DialogKind::OpenFile)
    }

    /// Pick one or more existing files.
    pub fn open_files() -> Self {
        Self::new(DialogKind::OpenFiles)
    }

    /// Pick an existing folder.
    pub fn open_folder() -> Self {
        Self::new(DialogKind::OpenFolder)
    }

    /// Choose a destination path to write to (may not exist yet).
    pub fn save_file() -> Self {
        Self::new(DialogKind::SaveFile)
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Restrict the dialog to files with one of `extensions` (no
    /// leading dot), shown under `label`.
    pub fn filter(mut self, label: impl Into<String>, extensions: &[&str]) -> Self {
        self.filter = Some((
            label.into(),
            extensions.iter().map(|s| s.to_string()).collect(),
        ));
        self
    }

    /// Suggested file name for a [`Self::save_file`] dialog.
    pub fn file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    fn build(&self) -> rfd::FileDialog {
        let mut dlg = rfd::FileDialog::new();
        if let Some(title) = &self.title {
            dlg = dlg.set_title(title);
        }
        if let Some((label, extensions)) = &self.filter {
            let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
            dlg = dlg.add_filter(label, &extensions);
        }
        if let Some(name) = &self.file_name {
            dlg = dlg.set_file_name(name);
        }
        dlg
    }

    fn run(self) -> PickerAnswer {
        let dlg = self.build();
        match self.kind {
            DialogKind::OpenFile => PickerAnswer::One(dlg.pick_file()),
            DialogKind::OpenFolder => PickerAnswer::One(dlg.pick_folder()),
            DialogKind::SaveFile => PickerAnswer::One(dlg.save_file()),
            DialogKind::OpenFiles => {
                PickerAnswer::Many(dlg.pick_files().unwrap_or_default())
            }
        }
    }
}

/// What a finished dialog answered.
#[derive(Debug, Clone)]
pub enum PickerAnswer {
    /// [`FileDialogRequest::open_file`] / `open_folder` / `save_file`:
    /// `None` when the dialog was dismissed.
    One(Option<PathBuf>),
    /// [`FileDialogRequest::open_files`]: empty when none were picked.
    Many(Vec<PathBuf>),
}

impl PickerAnswer {
    /// The single path, or `None` (including for [`Self::Many`] — use
    /// that variant directly if more than one path matters).
    pub fn into_one(self) -> Option<PathBuf> {
        match self {
            PickerAnswer::One(p) => p,
            PickerAnswer::Many(mut v) => v.pop(),
        }
    }

    /// The picked paths as a `Vec` — one entry, many, or none.
    pub fn into_many(self) -> Vec<PathBuf> {
        match self {
            PickerAnswer::One(p) => p.into_iter().collect(),
            PickerAnswer::Many(v) => v,
        }
    }
}

/// A single outstanding dialog, polled from `ui()` until it resolves.
/// Dropping a `FilePicker` while a dialog is up (the editor closing)
/// detaches the thread — the user's dismissal is simply never read,
/// matching the drums picker this generalises.
#[derive(Default)]
pub struct FilePicker {
    handle: Option<JoinHandle<PickerAnswer>>,
}

impl FilePicker {
    /// Whether a dialog is currently up.
    pub fn busy(&self) -> bool {
        self.handle.is_some()
    }

    /// Open `request`'s dialog on its own thread, unless one is
    /// already up for this picker (in which case `request` is
    /// dropped and this returns `false`).
    pub fn start(&mut self, request: FileDialogRequest) -> bool {
        self.start_with(move || request.run())
    }

    /// [`Self::start`]'s underlying primitive: run `f` on its own
    /// thread and poll its result. Split out (and left `pub`, for
    /// `tests/file_picker.rs`) so tests can exercise the busy/poll
    /// state machine with a fast, deterministic closure instead of an
    /// actual native dialog, which blocks on a display server this
    /// process may not have. Not plugin API.
    #[doc(hidden)]
    pub fn start_with(&mut self, f: impl FnOnce() -> PickerAnswer + Send + 'static) -> bool {
        if self.busy() {
            return false;
        }
        match std::thread::Builder::new()
            .name("resonance-file-dialog".into())
            .spawn(f)
        {
            Ok(h) => {
                self.handle = Some(h);
                true
            }
            Err(e) => {
                tracing::warn!("could not open the file dialog: {e}");
                false
            }
        }
    }

    /// `Some(answer)` once the dialog has closed. Call this every
    /// frame after [`Self::start`] until it returns `Some`.
    pub fn poll(&mut self) -> Option<PickerAnswer> {
        if !self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            return None;
        }
        let h = self.handle.take()?;
        h.join().ok()
    }
}

/// A [`FilePicker`] (plus an optional `with` payload) kept alive
/// across frames in an `egui::Context`'s temp storage, so a dialog
/// started on one frame can be polled on later ones without adding a
/// field to the caller's `App` struct — the same mechanism
/// `editor_widgets` uses for the `EditAnnouncer` and widget drag
/// state. For a caller that would rather own the `FilePicker` itself
/// (one picker, one clear owner — e.g. the IR editor's `Load IR…`),
/// a plain field works just as well; this is for call sites with
/// several interchangeable pickers (one per wavetable oscillator) or
/// that only have free functions to work with (the preset browser's
/// Import/Export).
///
/// `with` rides alongside the dialog so a caller can remember *which*
/// thing it was started for — the preset browser needs to remember
/// which preset "Export…" was clicked for, since the user is free to
/// click a different row while the (non-modal) dialog is still up.
#[derive(Clone)]
pub struct CtxPicker<T = ()>(Arc<Mutex<(FilePicker, Option<T>)>>);

impl<T> Default for CtxPicker<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new((FilePicker::default(), None))))
    }
}

impl<T: Clone + Send + Sync + 'static> CtxPicker<T> {
    /// The picker keyed by `id` in `ctx`'s temp storage, creating it
    /// on first use.
    pub fn get(ctx: &egui::Context, id: egui::Id) -> Self {
        if let Some(existing) = ctx.data(|d| d.get_temp::<Self>(id)) {
            return existing;
        }
        let picker = Self::default();
        ctx.data_mut(|d| d.insert_temp(id, picker.clone()));
        picker
    }

    /// Start `request`'s dialog, unless one is already up for this
    /// picker. `with` is handed back unchanged by [`Self::poll`] once
    /// the dialog resolves.
    pub fn start(&self, request: FileDialogRequest, with: T) -> bool {
        let mut guard = self.0.lock();
        let started = guard.0.start(request);
        if started {
            guard.1 = Some(with);
        }
        started
    }

    /// The answer and its `with` value, once the dialog has closed.
    /// Call every frame (not just after a click) — that is what keeps
    /// this entry alive in `ctx.data`'s temp storage.
    pub fn poll(&self) -> Option<(PickerAnswer, T)> {
        let mut guard = self.0.lock();
        let answer = guard.0.poll()?;
        let with = guard.1.take()?;
        Some((answer, with))
    }
}
