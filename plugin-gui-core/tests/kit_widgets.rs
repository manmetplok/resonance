//! The shared chip / segmented / slider kit (ba todo #1334).
//!
//! These three widgets used to be private copies inside the drums,
//! wavetable and granular-delay editors, and the sliders had already
//! diverged. The tests below pin the reconciled behaviour — the fill
//! rule, the state precedence and the pill geometry — so a future edit
//! to the shared widget cannot quietly change what a migrating editor
//! gets.

use plugin_gui_core::egui;
use plugin_gui_core::widgets::{
    chip::{chip_styled, Chip, ChipStyle},
    segmented::{segmented, SegmentedStyle},
    slider::{
        fill_span, signed_to_unit, slider_bipolar, slider_unipolar, unit_to_signed, SliderPalette,
        SliderStyle,
    },
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

/// A click on the track changes nothing: the slider moves by dragging
/// (relative to where it was), never by jumping to the pointer — the
/// forks positioned the value on a click, so a click meant to grab the
/// thumb moved the parameter (ux-guidelines.md: drag-to-adjust).
#[test]
fn clicking_a_slider_does_not_move_it() {
    let width = 100.0;
    let unit = click_at(egui::pos2(75.0, 9.0), |ui| slider_unipolar(ui, width, 0.0));
    assert_eq!(unit, None, "a click positioned the value");
    let signed = click_at(egui::pos2(75.0, 9.0), |ui| slider_bipolar(ui, width, 0.0));
    assert_eq!(signed, None, "a click positioned the bipolar value");
}

// ---------------------------------------------------------------------------
// What the eq migration added to the kit (ba todo #1335)
// ---------------------------------------------------------------------------

/// Drive `frames` through one context, so state that only exists between
/// frames — keyboard focus, in particular — survives from one to the next.
///
/// `click_at` above builds a fresh context per call, which is right for a
/// click but cannot express "focus it, then press a key".
fn drive<R>(
    frames: &[Vec<egui::Event>],
    mut contents: impl FnMut(&mut egui::Ui) -> R,
) -> Vec<R> {
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
    let mut out = Vec::new();
    for events in frames {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events: events.clone(),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| out.push(contents(ui)));
        });
    }
    out
}

fn press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    }
}

/// Frames that Tab focus onto the first widget, then press `key`.
///
/// Tab, not a click: neither this slider nor `egui::Slider` requests
/// focus when clicked — both only ever read `has_focus()` — so the
/// keyboard is reachable by tabbing to the control and no other way.
/// That is a real limitation of the affordance rather than a quirk of
/// the harness, which is why the test states it this way round.
fn tab_then(key: egui::Key) -> Vec<Vec<egui::Event>> {
    vec![vec![], vec![press(egui::Key::Tab)], vec![press(key)]]
}

/// The eq arrived from a raw `egui::Slider`, which nudges on the arrow
/// keys while focused. The shared slider had no keyboard handling at all,
/// so migrating the eq onto it would have silently dropped that — hence
/// this, and hence drums and wavetable gaining it too.
#[test]
fn arrow_keys_nudge_a_focused_slider() {
    let up = drive(&tab_then(egui::Key::ArrowRight), |ui| {
        slider_unipolar(ui, 100.0, 0.5)
    });
    let nudged = up
        .last()
        .copied()
        .flatten()
        .expect("an arrow press on a focused slider reports a new value");
    assert!(
        nudged > 0.5,
        "ArrowRight must move the value up from 0.5, got {nudged}"
    );

    // ...and left goes the other way, by the same step.
    let back = drive(&tab_then(egui::Key::ArrowLeft), |ui| {
        slider_unipolar(ui, 100.0, 0.5)
    })
    .last()
    .copied()
    .flatten()
    .expect("ArrowLeft reports a new value too");
    assert!(back < 0.5, "ArrowLeft must move the value down, got {back}");
    assert!(
        ((nudged - 0.5) - (0.5 - back)).abs() < 1e-6,
        "the two directions must step by the same amount ({nudged} vs {back})"
    );
}

/// Focus is what arms the arrow keys. Without it the same press must do
/// nothing, or a slider would steal the arrows from whatever the user is
/// actually driving.
#[test]
fn arrow_keys_are_ignored_by_an_unfocused_slider() {
    let frames = vec![vec![press(egui::Key::ArrowRight)]];
    let values = drive(&frames, |ui| slider_unipolar(ui, 100.0, 0.5));
    assert_eq!(
        values.last().copied().flatten(),
        None,
        "an unfocused slider must not consume the arrow keys"
    );
}

/// The slider's colours became a parameter so the EQ — then the only
/// editor on the retired blue `classic` palette — could adopt the shared
/// widget at all (ba todo #1335). Ba todo #1338 finished that migration,
/// so `SliderPalette::CLASSIC` is gone and the default is the one
/// canonical palette. A second palette constant reappearing here is the
/// fleet splitting again; `resonance-plugin/tests/fleet_palette.rs`
/// guards the editors, this guards the widget's own default.
#[test]
fn the_shared_slider_defaults_to_the_canonical_palette() {
    assert_eq!(SliderStyle::default(), SliderStyle::LAVENDER);
    assert_eq!(SliderStyle::LAVENDER.palette, SliderPalette::LAVENDER);

    // Every colour is a canonical token, none a freehand value. The
    // accent in particular: a slider painted with the old `#5ac8fa`
    // would read as a different product beside its own knobs.
    assert_eq!(
        SliderPalette::LAVENDER.accent,
        plugin_gui_core::theme::lavender::ACCENT
    );
    assert_eq!(
        SliderPalette::LAVENDER.warm,
        plugin_gui_core::theme::lavender::WARM
    );

    // Geometry is not part of the palette, and the EQ's band columns are
    // laid out around this 18 px row.
    assert_eq!(SliderStyle::LAVENDER.height, 18.0);
}

