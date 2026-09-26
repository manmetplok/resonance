//! The CLAP bundle's log subscriber (code review ARCH-05 A5-1).
//!
//! A plugin is a cdylib with its own statically linked copy of
//! `tracing-core`, so the host's global subscriber never sees an event
//! raised inside it — this bundle's `resonance-plugin`,
//! `resonance-common` and editor-runtime (`wayland-plugin-gui` /
//! `cocoa-plugin-gui`) logs would be dropped. The bridge installs a
//! stderr subscriber the first time the host creates an instance, with
//! the same `RUST_LOG` filter and default as the app.
//!
//! When the bridge is linked into a process that already has a global
//! subscriber (the app, a test binary that installed one) the install is
//! a no-op: `set_global_default` refuses to replace it.

use std::io::IsTerminal;
use std::sync::Once;

use tracing_subscriber::EnvFilter;

/// `RUST_LOG` unset: warnings from everything, `info` from the workspace
/// crates (a target prefix: `resonance` covers every `resonance_*`) —
/// the lowest level any former `eprintln!` maps to, so the default output
/// is what it was. `resonance_svs` stays at `warn`: its per-segment `info`
/// progress had no subscriber before and still prints nothing by default.
/// The app binary (`resonance-app/src/main.rs`) installs this same filter
/// via the crate-root re-export — the one definition (code review FU-H6c).
pub const DEFAULT_LOG_FILTER: &str = "warn,resonance=info,resonance_svs=warn,\
     wayland_plugin_gui=info,cocoa_plugin_gui=info";

/// Install this bundle's stderr subscriber once. Called from the bridge
/// before anything that can log.
pub(crate) fn ensure_subscriber() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
        // `set_global_default`, not `try_init`: no `log` → `tracing`
        // bridge, so crates on the `log` facade stay as silent as before.
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(std::io::stderr().is_terminal())
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}
