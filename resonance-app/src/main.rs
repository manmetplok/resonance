//! Binary entry point. Parses CLI args, wires the iced runtime, and
//! launches the application. All UI / state code lives in the library
//! crate (`resonance_app::*`) so that integration tests under
//! `resonance-app/tests/` can exercise the real view / update loop via
//! `iced_test` — the binary itself is a thin shim.

use iced::Size;
use resonance_app::{parse_startup_tab, theme, Resonance, STARTUP_TAB};

fn main() -> iced::Result {
    if let Some(tab) = parse_startup_tab() {
        let _ = STARTUP_TAB.set(tab);
    }

    let mut app = iced::application(Resonance::new, Resonance::update, Resonance::view)
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
