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
                        // Never hang the caller on a session bus that does
                        // not answer.
                        "--reply-timeout=2000".into(),
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
        // `xdg-open` in generic mode can stay alive as long as the file
        // manager it started. Detach it through a shell that exits at once,
        // so the caller only waits on the short-lived `sh` and no thread or
        // child of ours outlives a plugin library that is unloaded.
        out.push(RevealCommand {
            program: "sh",
            args: vec![
                "-c".into(),
                "xdg-open \"$1\" >/dev/null 2>&1 &".into(),
                "_".into(),
                folder.as_os_str().to_owned(),
            ],
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

/// Reveal `path` in the file manager. Tries each of [`reveal_commands`]
/// in turn and stops at the first that exits successfully. Every
/// invocation is waited for and is short-lived (the D-Bus call has a 2 s
/// reply timeout; the final folder open is detached through `sh`), so no
/// reaper thread is left behind — it would otherwise outlive a plugin
/// `.so` that is unloaded. Call it from a UI thread, never an audio thread.
pub fn reveal(path: &Path) -> std::io::Result<()> {
    let mut last_err = None;
    for cmd in reveal_commands(path) {
        let status = Command::new(cmd.program)
            .args(&cmd.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => last_err = Some(std::io::Error::other(format!("{} exited {s}", cmd.program))),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("no file manager launcher")))
}
