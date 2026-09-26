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

// --------------------------------------------------------------------
// VIEW-28: per-column culling inside a partly visible clip
// --------------------------------------------------------------------

/// The drawn column range of a clip is bounded by the cull window (plus
/// slack), however long the clip: a 5-minute clip at 100 px/s is 30 000
/// px wide, but only a window's worth of columns is tessellated.
#[test]
fn clip_column_range_is_bounded_by_the_window() {
    let window = cull::quantized_window(viewport(12_000.0, 1280.0));
    let (lo, hi) = cull::clip_px_range(0.0, 30_000.0, Some(window));
    assert!(hi - lo <= (window.1 - window.0) + 2.0 * cull::SPAN_PAD);
    // It still covers the visible part of the clip.
    assert!(lo <= 12_000.0 && hi >= 12_000.0 + 1280.0);

    // A clip starting mid-window: clip-local range starts at 0.
    let (lo, hi) = cull::clip_px_range(12_500.0, 30_000.0, Some(window));
    assert_eq!(lo, 0.0);
    assert!(hi >= 12_000.0 + 1280.0 - 12_500.0);

    // No window (tests, first frame) draws the whole clip.
    assert_eq!(cull::clip_px_range(-50.0, 300.0, None), (0.0, 300.0));
}

/// Deterministic pseudo-random stream (no RNG dependency).
fn lcg(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *seed >> 33
}

/// The coverage sweep answers exactly what the per-column `any()` scan
/// over every note answered, for ticks walked in order.
#[test]
fn coverage_sweep_matches_the_brute_force_scan() {
    let mut seed = 7;
    for _ in 0..50 {
        let notes: Vec<(f32, f32)> = (0..40)
            .map(|_| {
                let s = (lcg(&mut seed) % 4000) as f32 - 200.0;
                (s, s + (lcg(&mut seed) % 600) as f32)
            })
            .collect();
        let mut sweep = cull::CoverageSweep::new(notes.clone());
        let mut tick = -300.0_f32;
        while tick < 4_500.0 {
            let brute = notes.iter().any(|&(s, e)| tick >= s && tick < e);
            assert_eq!(sweep.covers(tick), brute, "tick {tick}");
            tick += 7.3;
        }
    }
}

/// The per-track sweep yields exactly the overlapping same-track pairs
/// the old all-pairs walk found, in the same order.
#[test]
fn overlapping_pairs_match_the_all_pairs_walk() {
    let mut seed = 11;
    for _ in 0..50 {
        let clips: Vec<(u64, u64, u64)> = (0..30)
            .map(|_| {
                let track = lcg(&mut seed) % 4;
                let start = lcg(&mut seed) % 10_000;
                (track, start, start + lcg(&mut seed) % 2_000)
            })
            .collect();
        let mut expected = Vec::new();
        for i in 0..clips.len() {
            for j in (i + 1)..clips.len() {
                let (ta, sa, ea) = clips[i];
                let (tb, sb, eb) = clips[j];
                if ta == tb && ea.min(eb) > sa.max(sb) {
                    expected.push((i, j));
                }
            }
        }
        let got: Vec<(usize, usize)> = cull::overlapping_pairs(&clips)
            .into_iter()
            .filter(|&(i, j)| {
                let (_, sa, ea) = clips[i];
                let (_, sb, eb) = clips[j];
                ea.min(eb) > sa.max(sb)
            })
            .collect();
        assert_eq!(got, expected);
    }
}
