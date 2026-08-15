//! The shared chip / segmented / slider kit (ba todo #1334).
//!
//! These three widgets used to be private copies inside the drums,
//! wavetable and granular-delay editors, and the sliders had already
//! diverged. The tests below pin the reconciled behaviour — the fill
//! rule, the state precedence and the pill geometry — so a future edit
//! to the shared widget cannot quietly change what a migrating editor
//! gets.

use wayland_plugin_gui::egui;
use wayland_plugin_gui::widgets::{
    chip::{chip_styled, Chip, ChipStyle},
    segmented::{segmented, SegmentedStyle},
    slider::{fill_span, signed_to_unit, slider_bipolar, slider_unipolar, unit_to_signed},
};

// ---------------------------------------------------------------------------
// Slider fill rule — the one thing the two forks disagreed on
// ---------------------------------------------------------------------------

#[test]
fn unipolar_fills_from_the_left_edge() {
    assert_eq!(fill_span(0.0, false), (0.0, 0.0, false));
    assert_eq!(fill_span(0.4, false), (0.0, 0.4, false));
    assert_eq!(fill_span(1.0, false), (0.0, 1.0, false));
}

/// Bipolar fills between the centre and the value, and only the
/// negative side is flagged warm. Wavetable's fork hard-coded the
/// positive side to the accent; drums' used the slider's own tone. For
/// an accent slider the two agree — which is why the drums rule is the
/// superset and the one promoted.
#[test]
fn bipolar_fills_outward_from_the_centre() {
    assert_eq!(fill_span(0.5, true), (0.5, 0.5, false));
    assert_eq!(fill_span(0.75, true), (0.5, 0.75, false));
    let (from, to, negative) = fill_span(0.25, true);
    assert_eq!((from, to), (0.25, 0.5));
    assert!(negative, "below centre must paint warm, whatever the tone");
}

#[test]
fn fill_span_clamps_out_of_range_values() {
    assert_eq!(fill_span(-3.0, false), (0.0, 0.0, false));
    assert_eq!(fill_span(9.0, false), (0.0, 1.0, false));
    assert_eq!(fill_span(9.0, true), (0.5, 1.0, false));
}

#[test]
fn bipolar_mapping_round_trips() {
    for signed in [-1.0f32, -0.5, 0.0, 0.25, 1.0] {
        let back = unit_to_signed(signed_to_unit(signed));
        assert!((back - signed).abs() < 1e-6, "{signed} -> {back}");
    }
    assert_eq!(signed_to_unit(0.0), 0.5);
    assert_eq!(unit_to_signed(0.5), 0.0);
}

// ---------------------------------------------------------------------------
// Chip geometry + state precedence
// ---------------------------------------------------------------------------

/// The pill is at least as tall as its style says and always pads the
/// label on both sides. `LAVENDER` pads by egui's own default button
/// padding, because it replaces an `egui::Button`-based fork and must
/// lay out at the same width.
#[test]
fn chip_size_pads_the_label_on_both_sides() {
    let label = egui::vec2(40.0, 12.0);
    assert_eq!(ChipStyle::LAVENDER.pad_x, 4.0);
    assert_eq!(ChipStyle::LAVENDER.size_for(label), egui::vec2(48.0, 22.0));
    assert_eq!(ChipStyle::COMPACT.size_for(label), egui::vec2(54.0, 16.0));
    // A label taller than the pill grows it rather than overflowing.
    let tall = egui::vec2(10.0, 30.0);
    assert_eq!(ChipStyle::LAVENDER.size_for(tall).y, 32.0);
}

#[test]
fn chip_state_precedence_is_disabled_then_active_then_hover() {
    let s = ChipStyle::LAVENDER;
    assert_eq!(s.colors(true, false, true), s.palette.disabled);
    assert_eq!(s.colors(true, true, true), s.palette.active);
    assert_eq!(s.colors(false, true, true), s.palette.hover);
    assert_eq!(s.colors(false, true, false), s.palette.idle);
}

/// The lavender chip keeps the fork's exact active tint (the lavender
/// accent at 0x18 alpha) so a migrated editor does not shift colour.
#[test]
fn lavender_active_tint_matches_the_promoted_forks() {
    assert_eq!(
        ChipStyle::LAVENDER.palette.active.fill,
        egui::Color32::from_rgba_unmultiplied(0x8b, 0x6d, 0xff, 0x18)
    );
    assert_eq!(ChipStyle::default(), ChipStyle::LAVENDER);
}

/// Segments are border-less; free-standing chips are not. That is the
/// visual difference between the two controls, so it lives in the style
/// rather than in a second widget.
#[test]
fn segment_style_is_borderless_and_framed_by_default() {
    assert!(ChipStyle::SEGMENT.palette.idle.stroke.is_none());
    assert!(ChipStyle::SEGMENT.palette.active.stroke.is_none());
    assert!(ChipStyle::LAVENDER.palette.idle.stroke.is_some());

    assert_eq!(SegmentedStyle::default(), SegmentedStyle::LAVENDER);
    const { assert!(SegmentedStyle::LAVENDER.framed) };
    const { assert!(!SegmentedStyle::LAVENDER.vertical) };
    // The compact strip is the granular-delay idiom: no frame, and it
    // can stack vertically.
    const { assert!(!SegmentedStyle::COMPACT.framed) };
    assert!(SegmentedStyle::COMPACT.vertical(true).vertical);
}

// ---------------------------------------------------------------------------
// Live behaviour, driven through a headless egui context
// ---------------------------------------------------------------------------

/// Run `contents` for three frames — a warm-up frame that registers the
/// widget rects, a press frame and a release frame — with the pointer at
/// `pos`, and return what the last frame produced.
///
/// Three frames because egui resolves a press against the *previous*
/// frame's widget rects, and only reports a click once the button is
/// released.
fn click_at<R>(pos: egui::Pos2, mut contents: impl FnMut(&mut egui::Ui) -> R) -> R {
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
    let mut result = None;
    for press in [None, Some(true), Some(false)] {
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = press {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    result = Some(contents(ui));
                });
        });
    }
    result.expect("the panel body always runs")
}

#[test]
fn clicking_a_chip_reports_it_and_a_disabled_chip_does_not() {
    // The panel has no frame, so the chip starts at the origin; (6, 8)
    // is inside the smallest pill either style produces.
    let hit = egui::pos2(6.0, 8.0);
    assert!(click_at(hit, |ui| chip_styled(
        ui,
        &Chip::new("SNARE", false)
    )));
    assert!(!click_at(hit, |ui| chip_styled(
        ui,
        &Chip::new("SNARE", false).enabled(false)
    )));
}

#[test]
fn clicking_a_segment_reports_its_index() {
    // Segment 0 sits just inside the frame margin.
    let first = click_at(egui::pos2(8.0, 12.0), |ui| {
        segmented(ui, &["ONE", "TWO", "THREE"], 2)
    });
    assert_eq!(first, Some(0), "a click near the left edge hits segment 0");
    // Far outside every segment: nothing is reported.
    let none = click_at(egui::pos2(560.0, 360.0), |ui| {
        segmented(ui, &["ONE", "TWO", "THREE"], 0)
    });
    assert_eq!(none, None);
}

#[test]
fn clicking_a_slider_positions_the_value() {
    let width = 100.0;
    let unit = click_at(egui::pos2(75.0, 9.0), |ui| {
        slider_unipolar(ui, width, 0.0)
    })
    .expect("a click on the track sets the value");
    assert!((unit - 0.75).abs() < 1e-3, "expected ~0.75, got {unit}");

    // The bipolar entry point reports the same click in -1..1 space.
    let signed = click_at(egui::pos2(75.0, 9.0), |ui| {
        slider_bipolar(ui, width, 0.0)
    })
    .expect("a click on the track sets the value");
    assert!((signed - 0.5).abs() < 1e-3, "expected ~0.5, got {signed}");

    // No pointer anywhere near it: no value change.
    let idle = click_at(egui::pos2(560.0, 360.0), |ui| {
        slider_unipolar(ui, width, 0.25)
    });
    assert_eq!(idle, None);
}
