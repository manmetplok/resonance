//! Show a file or folder in the platform file manager — the one launcher
//! the app and the plugins share (nam-model-library.md §6.2 "Reveal").
//!
//! A file is *selected* in its folder where the platform supports that
//! (`open -R` on macOS, `explorer /select,` on Windows, the freedesktop
//! `org.freedesktop.FileManager1.ShowItems` D-Bus call on Linux); otherwise
//! its containing folder opens. A folder opens as itself. Failures are
//! returned, never fatal: the user can still navigate there by hand.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One program invocation the launcher may try, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealCommand {
    pub program: &'static str,
    pub args: Vec<OsString>,
}

/// The invocations [`reveal`] tries for `path`, first to last, for this
/// platform. Pure (no filesystem access beyond `is_dir`), so the choice is
/// testable.
pub fn reveal_commands(path: &Path) -> Vec<RevealCommand> {
    let is_dir = path.is_dir();
    let folder: PathBuf = if is_dir {
        path.to_path_buf()
    } else {
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    {
        let mut args: Vec<OsString> = Vec::new();
        if !is_dir {
            args.push("-R".into());
        }
        args.push(path.as_os_str().to_owned());
        out.push(RevealCommand {
            program: "open",
            args,
        });
    }
    #[cfg(target_os = "windows")]
    {
        let arg: OsString = if is_dir {
            path.as_os_str().to_owned()
        } else {
            let mut a = OsString::from("/select,");
            a.push(path.as_os_str());
            a
        };
        out.push(RevealCommand {
            program: "explorer",
            args: vec![arg],
        });
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if !is_dir {
            if let Some(uri) = file_uri(path) {
                out.push(RevealCommand {
                    program: "dbus-send",
                    args: vec![
                        "--session".into(),
                        "--print-reply".into(),
                        "--dest=org.freedesktop.FileManager1".into(),
                        "--type=method_call".into(),
                        "/org/freedesktop/FileManager1".into(),
                        "org.freedesktop.FileManager1.ShowItems".into(),
                        format!("array:string:{uri}").into(),
                        "string:".into(),
                    ],
                });
            }
        }
        out.push(RevealCommand {
            program: "xdg-open",
            args: vec![folder.as_os_str().to_owned()],
        });
    }
    let _ = &folder;
    out
}

/// `file://` URI for an absolute path, percent-encoding everything outside
/// the unreserved set and `/`. `None` for a relative or non-UTF-8 path.
pub fn file_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let text = path.to_str()?;
    let mut out = String::from("file://");
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Some(out)
}

/// Reveal `path` in the file manager. Tries each of [`reveal_commands`] in
/// turn and stops at the first that starts and exits successfully; the
/// last one (the plain folder open) is spawned without waiting. Blocking
/// for at most the D-Bus round trip, so call it from a UI thread, never an
/// audio thread.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    let commands = reveal_commands(path);
    let last = commands.len().saturating_sub(1);
    let mut last_err = None;
    for (i, cmd) in commands.into_iter().enumerate() {
        let mut c = Command::new(cmd.program);
        c.args(&cmd.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if i < last {
            match c.status() {
                Ok(s) if s.success() => return Ok(()),
                Ok(s) => last_err = Some(std::io::Error::other(format!("{} exited {s}", cmd.program))),
                Err(e) => last_err = Some(e),
            }
        } else {
            match c.spawn() {
                Ok(mut child) => {
                    // Reap it off-thread so no zombie is left behind.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return Ok(());
                }
                Err(e) => last_err = Some(e),
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no file manager launcher")))
}
