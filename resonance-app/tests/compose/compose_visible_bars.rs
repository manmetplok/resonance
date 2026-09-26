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

// -- O(signature changes) width and tick math (FU-V2c remainder) ----------

use resonance_app::view::compose::{
    bar_to_section_tick, bars_span, sample_to_section_tick, section_bars_in_range_every,
    section_total_ticks,
};

/// The every-bar sums the width and tick helpers used to do.
fn naive_span(map: &TempoMap, from: u32, to: u32) -> (u64, u64) {
    (from..to).fold((0, 0), |(beats, ticks), bar| {
        (
            beats + map.numerator_at_bar(bar) as u64,
            ticks + map.bar_len_ticks_at(bar),
        )
    })
}

#[test]
fn the_span_sums_match_the_every_bar_walk() {
    let map = mixed_meter_map();
    for (from, to) in [(0, 0), (0, 1), (0, 30), (3, 12), (5, 9), (7, 40), (21, 25), (9, 3)] {
        assert_eq!(bars_span(&map, from, to), naive_span(&map, from, to), "{from}..{to}");
    }
    for (start, len) in [(0, 30), (3, 12), (7, 40)] {
        assert_eq!(
            section_total_ticks(&map, start, len),
            naive_span(&map, start, start + len).1
        );
        assert_eq!(
            resonance_app::view::compose::section_total_beats(&map, start, len) as u64,
            naive_span(&map, start, start + len).0
        );
    }
    // Before the section the tick is negative.
    assert_eq!(bar_to_section_tick(&map, 7, 7), 0.0);
    assert_eq!(bar_to_section_tick(&map, 7, 12), naive_span(&map, 7, 12).1 as f64);
    assert_eq!(bar_to_section_tick(&map, 7, 2), -(naive_span(&map, 2, 7).1 as f64));
}

#[test]
fn sample_to_section_tick_matches_the_every_bar_walk() {
    let map = mixed_meter_map();
    for bar in [0u32, 4, 5, 6, 9, 15, 20, 33] {
        let sample = map.bar_to_sample(bar) + 1_000;
        let (b, frac) = map.sample_to_bar(sample, 48_000);
        for start in [0u32, 3, 9, 25] {
            let naive = if b >= start {
                naive_span(&map, start, b).1 as f64
            } else {
                -(naive_span(&map, b, start).1 as f64)
            } + frac * map.bar_len_ticks_at(b) as f64;
            let got = sample_to_section_tick(&map, 48_000, start, sample);
            assert!((got - naive).abs() < 1e-6, "bar {bar}, start {start}: {got} vs {naive}");
        }
    }
}

#[test]
fn a_max_length_section_spans_in_one_step_per_meter_run() {
    let map = mixed_meter_map();
    // 5 bars 4/4, 4 bars 7/8, 11 bars 3/4, then 6/8 to the end.
    let rest = u64::from(MAX_BARS - 20);
    let beats = 5 * 4 + 4 * 7 + 11 * 3 + rest * 6;
    let ticks = 5 * 1920 + 4 * 1680 + 11 * 1440 + rest * 1440;
    assert_eq!(bars_span(&map, 0, MAX_BARS), (beats, ticks));
}

#[test]
fn the_strided_walk_keeps_every_nth_bar_of_the_range() {
    let map = mixed_meter_map();
    for stride in [1u32, 2, 3, 7] {
        for (lo, hi) in [(0.0, 1e12), (10.0, 30.0), (13.5, 60.0)] {
            let want: Vec<SectionBar> = naive(&map, 2, 40, BarUnit::Beats, lo, hi)
                .into_iter()
                .filter(|b| b.offset % stride == 0)
                .collect();
            assert_eq!(
                section_bars_in_range_every(&map, 2, 40, BarUnit::Beats, lo, hi, stride),
                want,
                "stride {stride}, [{lo}, {hi}]"
            );
        }
    }
    // A whole max-length section at one line per 4 px of a 1 000 px editor.
    let stride = (4.0 * MAX_BARS as f32 / 1_000.0).ceil() as u32;
    let bars = section_bars_in_range_every(&map, 0, MAX_BARS, BarUnit::Ticks, 0.0, 1e15, stride);
    assert!(bars.len() <= 251, "{} bar lines", bars.len());
}
