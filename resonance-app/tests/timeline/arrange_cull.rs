//! Horizontal draw-culling of the arrange canvas's cached pass
//! (view-performance batch) — the pure seams in
//! `resonance_app::view::timeline::cull`.
//!
//! The invariant that matters: the quantized window derived from a
//! frame's visible viewport must always cover that viewport (plus the
//! predicate's slack), because the fingerprint — which embeds the
//! window — is re-checked inside the same frame's draw. Quantization is
//! allowed to *over*-draw as much as it likes; it must never under-draw.

use iced::Rectangle;
use resonance_app::view::timeline::cull;

fn viewport(x: f32, width: f32) -> Rectangle {
    Rectangle {
        x,
        y: 0.0,
        width,
        height: 800.0,
    }
}

/// The covering guarantee, swept across scroll positions and viewport
/// widths (including the degenerate near-zero width an early layout
/// pass can report).
#[test]
fn the_quantized_window_always_covers_the_visible_range()  {
    for width in [1.0, 320.0, 1280.0, 2560.0, 5000.0] {
        for i in 0..400 {
            let x = i as f32 * 137.3;
            let (w0, w1) = cull::quantized_window(viewport(x, width));
            assert!(
                w0 <= x && w1 >= x + width,
                "window [{w0}, {w1}] must cover visible [{x}, {}]",
                x + width
            );
        }
    }
}

/// Hysteresis: a scroll smaller than the quantization step must be able
/// to keep the window — and therefore the cache — unchanged, while a
/// long scroll must eventually move it (so revealed content is drawn).
#[test]
fn small_scrolls_share_a_window_and_long_scrolls_move_it() {
    let width = 1280.0;
    let base = cull::quantized_window(viewport(10_000.0, width));
    assert_eq!(
        base,
        cull::quantized_window(viewport(10_100.0, width)),
        "a 100 px scroll inside one quantization cell keeps the window"
    );
    assert_ne!(
        base,
        cull::quantized_window(viewport(10_000.0 + width * 2.0, width)),
        "a two-viewport scroll must land in a new window"
    );
}

/// The window never extends left of the canvas origin.
#[test]
fn the_window_is_clamped_at_zero() {
    let (w0, _) = cull::quantized_window(viewport(0.0, 1280.0));
    assert_eq!(w0, 0.0);
}

/// The skip predicate: a clip wholly outside the window is culled, one
/// straddling either window edge (or wholly inside) is drawn, and an
/// unknown window (`None` — tests, first frame) draws everything.
#[test]
fn the_skip_predicate_culls_outside_and_keeps_edge_straddlers() {
    let window = Some((2000.0, 4000.0));
    let pad = cull::SPAN_PAD;
    // Far outside on both sides — beyond the slack — is culled.
    assert!(!cull::span_may_be_visible((0.0, 2000.0 - pad - 1.0), window));
    assert!(!cull::span_may_be_visible(
        (4000.0 + pad + 1.0, 9000.0),
        window
    ));
    // Straddling either edge is drawn.
    assert!(cull::span_may_be_visible((1000.0, 2500.0), window));
    assert!(cull::span_may_be_visible((3900.0, 5000.0), window));
    // Wholly inside is drawn; spanning the whole window is drawn.
    assert!(cull::span_may_be_visible((2500.0, 2600.0), window));
    assert!(cull::span_may_be_visible((0.0, 9000.0), window));
    // Within the slack of an edge is still drawn (group indent, beads).
    assert!(cull::span_may_be_visible(
        (2000.0 - pad + 1.0 - 10.0, 2000.0 - pad + 1.0),
        window
    ));
    // No window — draw everything.
    assert!(cull::span_may_be_visible((-1e9, -1e9 + 1.0), None));
}

/// The span helper maps samples to pixels exactly like `sample_to_x`:
/// seconds × zoom − scroll.
#[test]
fn sample_span_matches_the_canvas_mapping() {
    let (x0, x1) = cull::sample_span_x(48_000, 96_000, 48_000, 100.0, 50.0);
    assert!((x0 - 50.0).abs() < 1e-3, "start: 1 s × 100 px/s − 50 = 50");
    assert!((x1 - 150.0).abs() < 1e-3, "end: 2 s × 100 px/s − 50 = 150");
}
