//! Compose canvases draw only the bars on screen (FU-V2c).
//!
//! Sections may be up to `MAX_BARS` (100 000) bars long, and the lane
//! canvases used to walk — and paint — every one of them on each repaint;
//! the track grid even re-summed the section's length for every tick→x
//! mapping, which made it quadratic. The workspace `Scrollable` now reports
//! its viewport and the canvases walk only the bars inside it.

use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::view::compose::{
    section_bars_in_range, visible_x_window, BarUnit, SectionBar, WORKSPACE_PAD_X,
};
use resonance_app::Resonance;
use resonance_audio::types::{SignaturePoint, TempoMap};
use resonance_control::MAX_BARS;

fn mixed_meter_map() -> TempoMap {
    let mut map = TempoMap::default();
    map.signature_points = vec![
        SignaturePoint { bar: 0, numerator: 4, denominator: 4 },
        SignaturePoint { bar: 5, numerator: 7, denominator: 8 },
        SignaturePoint { bar: 9, numerator: 3, denominator: 4 },
        SignaturePoint { bar: 20, numerator: 6, denominator: 8 },
    ];
    map.rebuild_bar_table(48_000);
    map
}

/// The every-bar walk the canvases used to do, filtered to the range.
fn naive(map: &TempoMap, start: u32, len: u32, unit: BarUnit, lo: f64, hi: f64) -> Vec<SectionBar> {
    let (mut beat, mut tick) = (0u64, 0u64);
    let mut out = Vec::new();
    for offset in 0..len {
        let bar = start + offset;
        let beats = map.numerator_at_bar(bar) as u32;
        let ticks = map.bar_len_ticks_at(bar);
        let (pos, size) = match unit {
            BarUnit::Beats => (beat as f64, beats as f64),
            BarUnit::Ticks => (tick as f64, ticks as f64),
        };
        if pos <= hi && pos + size > lo {
            out.push(SectionBar { offset, beat, tick, beats, ticks });
        }
        beat += beats as u64;
        tick += ticks;
    }
    out
}

#[test]
fn the_visible_bars_match_the_every_bar_walk_across_meter_changes() {
    let map = mixed_meter_map();
    for (start, len) in [(0, 30), (3, 12), (7, 40), (21, 5)] {
        for unit in [BarUnit::Beats, BarUnit::Ticks] {
            let scale = match unit {
                BarUnit::Beats => 1.0,
                BarUnit::Ticks => 480.0,
            };
            for (lo, hi) in [(0.0, 1e12), (0.0, 0.0), (10.0, 30.0), (13.5, 13.6), (-5.0, 2.0), (200.0, 300.0)] {
                let (lo, hi) = (lo * scale, hi * scale);
                assert_eq!(
                    section_bars_in_range(&map, start, len, unit, lo, hi),
                    naive(&map, start, len, unit, lo, hi),
                    "start {start}, len {len}, {unit:?}, [{lo}, {hi}]"
                );
            }
        }
    }
}

#[test]
fn a_max_length_section_walks_a_screenful_of_bars() {
    let map = mixed_meter_map();
    // One screen at the far end of the section, in beats (56 px each).
    let total_beats = resonance_app::view::compose::section_total_beats(&map, 0, MAX_BARS) as f64;
    let lo = total_beats - 40.0;
    let bars = section_bars_in_range(&map, 0, MAX_BARS, BarUnit::Beats, lo, total_beats);
    assert!(
        !bars.is_empty() && bars.len() <= 16,
        "{} bars for a 40-beat window",
        bars.len()
    );
    assert_eq!(bars.last().unwrap().offset, MAX_BARS - 1, "reaches the section end");

    // Scrolled into the middle: bounded the same way.
    // 48 000 ticks of 6/8 (1 440-tick bars) is 34 bars.
    let bars = section_bars_in_range(&map, 0, MAX_BARS, BarUnit::Ticks, 5e7, 5e7 + 48_000.0);
    assert!(!bars.is_empty() && bars.len() <= 35, "{} bars", bars.len());
}

#[test]
fn the_window_covers_the_viewport_and_is_bounded() {
    // Before the scrollable reports: the start of the section.
    let (lo, hi) = visible_x_window(None);
    assert_eq!(lo, 0.0);
    assert!(hi >= 4096.0);

    let (offset, width) = (250_000.0f32, 1_500.0f32);
    let (lo, hi) = visible_x_window(Some((offset, width)));
    assert!(lo <= offset - WORKSPACE_PAD_X, "left edge on screen: {lo}");
    assert!(hi >= offset - WORKSPACE_PAD_X + width, "right edge on screen: {hi}");
    assert!(hi - lo <= width + 4.0 * 1024.0, "bounded: {lo}..{hi}");

    // A small scroll stays in the same block: the canvas caches survive.
    assert_eq!(visible_x_window(Some((offset + 3.0, width))), (lo, hi));
}

#[test]
fn the_workspace_scroll_is_recorded() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    assert_eq!(app.compose_state().workspace_view, None);
    let _ = app.update(Message::Compose(ComposeMessage::WorkspaceScrolled {
        offset_x: 1_234.0,
        width: 900.0,
    }));
    assert_eq!(app.compose_state().workspace_view, Some((1_234.0, 900.0)));
}
