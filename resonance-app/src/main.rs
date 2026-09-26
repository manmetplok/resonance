//! Binary entry point. Parses CLI args, wires the iced runtime, and
//! launches the application. All UI / state code lives in the library
//! crate (`resonance_app::*`) so that integration tests under
//! `resonance-app/tests/` can exercise the real view / update loop via
//! `iced_test` — the binary itself is a thin shim.

use iced::Size;
use resonance_app::{parse_startup_tab, theme, Resonance, STARTUP_TAB};

// `RUST_LOG` unset: `warn` everywhere, `info` from the workspace crates
// (ARCH-05 A5-1). One definition, shared with every CLAP bundle's own
// subscriber (a cdylib has its own tracing dispatcher).
use resonance_plugin::DEFAULT_LOG_FILTER;

/// The process's log subscriber: stderr, filtered by `RUST_LOG`. Only
/// the binary installs one — library crates just emit `tracing` events,
/// and tests (no subscriber) drop them. No `log` bridge: crates on the
/// `log` facade (wgpu, iced) stay as silent as before.
fn install_log_subscriber() {
    use std::io::IsTerminal;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn main() -> iced::Result {
    install_log_subscriber();

    if let Some(tab) = parse_startup_tab() {
        let _ = STARTUP_TAB.set(tab);
    }

    // `new_with_control` = `new()` + the unix-socket control endpoint
    // (ba doc #265). Tests construct via `new()` and never bind the
    // per-user socket.
    let mut app = iced::application(Resonance::new_with_control, Resonance::update, Resonance::view)
        .title("Resonance")
        .font(theme::ICON_FONT_BYTES);
    for face in theme::UI_FONT_FACES {
        app = app.font(*face);
    }
    app.default_font(theme::UI_FONT)
        .subscription(Resonance::subscription)
        .theme(theme::resonance_theme())
        .window(iced::window::Settings {
            size: Size::new(1440.0, 900.0),
            min_size: Some(Size::new(1440.0, 900.0)),
            exit_on_close_request: false,
            ..Default::default()
        })
        // MSAA is expensive on Linux/Wayland with wgpu — every redraw
        // pays for a 4× sample buffer. Our canvases use rounded paths
        // sparingly and the lavender accent is forgiving without AA, so
        // disabling it speeds up the steady-state and makes window
        // resize visibly smoother. Tested on radv (Vulkan) where the AA
        // pass was the dominant per-frame cost.
        .antialiasing(false)
        .run()
}
