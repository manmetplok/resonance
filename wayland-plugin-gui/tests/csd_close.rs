//! Close-on-CSD-click hit-testing for the client-side-decoration fallback
//! frame.
//!
//! When the compositor forces client-side decorations, `wayland-plugin-gui`
//! draws its own titlebar with a close button. A click on that button must map
//! to the same `close_requested` path the server-side `xdg_toplevel.close`
//! event uses. The live paint code and this test share one source of truth for
//! the button geometry: [`FrameLayout::is_close_click`] (exposed via the
//! crate's hidden `csd_geometry` re-export), so a button the user can see is a
//! button this test can hit.

use wayland_plugin_gui::csd_geometry::{
    FrameLayout, BORDER_WIDTH, CLOSE_BUTTON_SIZE, TITLEBAR_HEIGHT,
};

/// A click in the close-button square (top-right of the titlebar) is a close.
#[test]
fn click_on_close_button_closes() {
    let (w, h) = (800.0_f32, 600.0_f32);
    let layout = FrameLayout::new(w, h);

    // Center of the close button square.
    let cb = layout.close_button_rect();
    let cx = (cb.min_x + cb.max_x) / 2.0;
    let cy = (cb.min_y + cb.max_y) / 2.0;
    assert!(
        layout.is_close_click(cx, cy),
        "center of the close button must register as a close ({cx},{cy})"
    );

    // The button sits flush in the top-right corner, inside the border.
    assert!(layout.is_close_click(w - BORDER_WIDTH - 1.0, BORDER_WIDTH + 1.0));
}

/// A click anywhere else in the titlebar (the drag area) must NOT close.
#[test]
fn click_on_titlebar_does_not_close() {
    let layout = FrameLayout::new(800.0, 600.0);

    // Left edge of the titlebar, where the title text sits.
    assert!(!layout.is_close_click(20.0, BORDER_WIDTH + TITLEBAR_HEIGHT / 2.0));
    // Just left of the close button.
    let cb = layout.close_button_rect();
    assert!(!layout.is_close_click(cb.min_x - 5.0, cb.min_y + 5.0));
}

/// A click in the app content area (below the titlebar) must NOT close — it
/// belongs to the app UI.
#[test]
fn click_in_content_does_not_close() {
    let layout = FrameLayout::new(800.0, 600.0);
    let content = layout.content_rect();
    let cx = (content.min_x + content.max_x) / 2.0;
    let cy = (content.min_y + content.max_y) / 2.0;
    assert!(!layout.is_close_click(cx, cy));

    // The content rect starts below the titlebar and inside the border.
    assert_eq!(content.min_y, BORDER_WIDTH + TITLEBAR_HEIGHT);
    assert_eq!(content.min_x, BORDER_WIDTH);
}

/// A click outside the window bounds never closes.
#[test]
fn click_outside_window_does_not_close() {
    let layout = FrameLayout::new(800.0, 600.0);
    assert!(!layout.is_close_click(-10.0, -10.0));
    assert!(!layout.is_close_click(10_000.0, 10_000.0));
    // Exactly on the far edges is treated as outside (half-open rects).
    assert!(!layout.is_close_click(800.0, 5.0));
}

/// The close button stays within the window and the titlebar for small and
/// large windows alike — it must never spill outside the frame the user sees.
#[test]
fn close_button_stays_within_frame() {
    for &(w, h) in &[(400.0_f32, 300.0_f32), (1920.0, 1080.0), (120.0, 90.0)] {
        let layout = FrameLayout::new(w, h);
        let cb = layout.close_button_rect();
        assert!(cb.min_x >= BORDER_WIDTH, "button left inside border (w={w})");
        assert!(cb.max_x <= w - BORDER_WIDTH + 0.001, "button right inside border (w={w})");
        assert!(cb.min_y >= BORDER_WIDTH, "button top inside border (w={w})");
        assert!(
            cb.max_y <= BORDER_WIDTH + TITLEBAR_HEIGHT + 0.001,
            "button bottom inside titlebar (w={w})"
        );
    }
}

/// The close button is the documented square size on a roomy window.
#[test]
fn close_button_has_expected_size() {
    let layout = FrameLayout::new(800.0, 600.0);
    let cb = layout.close_button_rect();
    assert!((cb.max_x - cb.min_x - CLOSE_BUTTON_SIZE).abs() < 0.001);
    assert!((cb.max_y - cb.min_y - CLOSE_BUTTON_SIZE).abs() < 0.001);
}
