//! Control-socket path resolution, shared by the app (server side) and
//! `resonance-mcp` (client side) so the two can never disagree on where
//! the socket lives. Used to be two hand-kept copies of the same logic;
//! this crate is already the single source of truth both sides compile
//! against (doc #265), so the path rule lives here too.

use std::path::PathBuf;

/// Env var overriding the control-socket path.
pub const SOCKET_PATH_ENV: &str = "RESONANCE_CONTROL_SOCKET";

/// Resolve the control-socket path (doc #265):
/// `$RESONANCE_CONTROL_SOCKET` verbatim when set, else
/// `$XDG_RUNTIME_DIR/resonance/control.sock`, else — on macOS, which
/// sets no `XDG_RUNTIME_DIR` — `$TMPDIR/resonance/control.sock`, else
/// `/tmp/resonance-<uid>/control.sock`.
pub fn socket_path() -> PathBuf {
    resolve_socket_path(
        std::env::var(SOCKET_PATH_ENV).ok(),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var_os("TMPDIR").map(PathBuf::from),
        process_uid(),
    )
}

/// The resolution rule itself, pure over its inputs so tests can drive
/// it without touching the process environment.
///
/// `tmpdir` is consulted only on macOS, where `$TMPDIR` points at a
/// per-user directory by construction (`/var/folders/.../T/`) — the
/// closest equivalent of `XDG_RUNTIME_DIR`. Empty values are treated as
/// unset, matching how the env-reading wrapper always behaved.
pub fn resolve_socket_path(
    override_path: Option<String>,
    runtime_dir: Option<PathBuf>,
    tmpdir: Option<PathBuf>,
    uid: u32,
) -> PathBuf {
    if let Some(path) = override_path {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Some(runtime) = runtime_dir {
        if !runtime.as_os_str().is_empty() {
            return runtime.join("resonance").join("control.sock");
        }
    }
    if cfg!(target_os = "macos") {
        if let Some(tmp) = tmpdir {
            if !tmp.as_os_str().is_empty() {
                return tmp.join("resonance").join("control.sock");
            }
        }
    }
    PathBuf::from(format!("/tmp/resonance-{uid}")).join("control.sock")
}

/// The process's real uid, for the `/tmp/resonance-<uid>` fallback.
/// This used to be read from `/proc/self` to avoid the libc dependency,
/// which silently yields uid 0 anywhere `/proc` doesn't exist (macOS) —
/// every user would then share, and fight over, `/tmp/resonance-0`.
#[cfg(unix)]
pub fn process_uid() -> u32 {
    // SAFETY: getuid takes no arguments and cannot fail.
    unsafe { libc::getuid() }
}

/// Non-unix stub so the crate's types stay portable; the socket itself
/// is unix-only anyway.
#[cfg(not(unix))]
pub fn process_uid() -> u32 {
    0
}
