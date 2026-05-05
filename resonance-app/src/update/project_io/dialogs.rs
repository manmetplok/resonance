//! File-dialog tasks (rfd glue) and the async project-load task.
//! Pure interaction with the OS file picker; no engine commands, no
//! state mutation.

use iced::Task;

use crate::message::*;
use crate::project;

/// Resolve the default projects directory, `$XDG_DOCUMENTS_DIR/resonance`
/// on Linux (or the platform equivalent from the `dirs` crate),
/// creating it on first use. Falls back to the user's home directory,
/// and ultimately to the current working directory, if the XDG lookup
/// fails.
fn default_projects_dir() -> std::path::PathBuf {
    let base = dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = base.join("resonance");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("Create default projects dir {}: {e}", dir.display());
    }
    dir
}

pub fn save_project_as_dialog() -> Task<Message> {
    let default_dir = default_projects_dir();
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .set_title("Save Project")
                .set_directory(&default_dir)
                .set_file_name("MyProject.rproj")
                .save_file()
                .await
                .map(|f| f.path().to_string_lossy().to_string())
        },
        |r| Message::ProjectIo(ProjectIoMessage::SavePathSelected(r)),
    )
}

pub fn open_project_dialog() -> Task<Message> {
    let default_dir = default_projects_dir();
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .set_title("Open Project")
                .set_directory(&default_dir)
                .add_filter("Resonance Project", &["rproj"])
                .pick_folder()
                .await
                .map(|f| f.path().to_string_lossy().to_string())
        },
        |r| Message::ProjectIo(ProjectIoMessage::OpenPathSelected(r)),
    )
}

pub fn bounce_dialog() -> Task<Message> {
    let default_dir = default_projects_dir();
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .add_filter("WAV Audio", &["wav"])
                .set_title("Bounce to WAV")
                .set_directory(&default_dir)
                .set_file_name("bounce.wav")
                .save_file()
                .await
                .map(|f| f.path().to_string_lossy().to_string())
        },
        |r| Message::ProjectIo(ProjectIoMessage::BouncePathSelected(r)),
    )
}

pub fn chord_sheet_dialog(data: Vec<u8>) -> Task<Message> {
    let default_dir = default_projects_dir();
    Task::perform(
        async move {
            let path = rfd::AsyncFileDialog::new()
                .set_title("Export Chord Sheet")
                .set_directory(&default_dir)
                .set_file_name("chords.pdf")
                .add_filter("PDF", &["pdf"])
                .save_file()
                .await
                .map(|f| f.path().to_string_lossy().to_string());
            (path, data)
        },
        |(path, data)| Message::ProjectIo(ProjectIoMessage::ChordSheetPathSelected(path, data)),
    )
}

/// Kick off an async load of the project on `path`.
pub fn load_project_task(path: std::path::PathBuf) -> Task<Message> {
    Task::perform(
        async move { project::load_project(&path).map(Box::new) },
        |r| Message::ProjectIo(ProjectIoMessage::ProjectLoaded(r)),
    )
}
