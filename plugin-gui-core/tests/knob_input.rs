//! The platform-wide knob drag feel (ba todo #1266).
//!
//! Both knob families — the classic range-mapped `knob` and the themed
//! `knob_themed` one the lavender editors use — resolve a vertical drag
//! through `knob_drag_unit`. These tests pin the numbers, because the
//! whole point of the shared helper is that a plugin cannot quietly
//! grow its own drag feel again: the granular delay used to answer the
//! same gesture at 0.008 per pixel (0.002 with Shift) while everything
//! else ran at 0.005.

use plugin_gui_core::widgets::{
    knob_drag_unit, KnobStyle, KNOB_DRAG_SPEED, KNOB_DRAG_SPEED_FINE,
};

/// Egui reports downward pointer motion as a positive `drag_delta().y`,
/// and dragging down must lower the value.
#[test]
fn dragging_down_lowers_the_value() {
    assert!(knob_drag_unit(0.5, 10.0, false) < 0.5);
    assert!(knob_drag_unit(0.5, -10.0, false) > 0.5);
}

#[test]
fn full_scale_sweep_is_two_hundred_pixels() {
    assert_eq!(KNOB_DRAG_SPEED, 0.005);
    // 200 px of upward drag covers exactly 0..1.
    let unit = knob_drag_unit(0.0, -200.0, false);
    assert!((unit - 1.0).abs() < 1e-6, "expected full sweep, got {unit}");
    // Half that, half the range.
    let half = knob_drag_unit(0.0, -100.0, false);
    assert!((half - 0.5).abs() < 1e-6, "expected half sweep, got {half}");
}

#[test]
fn shift_is_the_finer_of_the_two_speeds() {
    assert_eq!(KNOB_DRAG_SPEED_FINE, 0.001);
    // The Shift speed must be the slower of the two — and by enough to
    // feel like a different gesture. Merely slower is not enough: this
    // was briefly 0.004 against a 0.005 normal speed, a 20% difference
    // no one can perceive while dragging. Require at least 3x finer so
    // a future tweak cannot quietly reduce Shift to a no-op again.
    const { assert!(KNOB_DRAG_SPEED_FINE < KNOB_DRAG_SPEED) };
    const { assert!(KNOB_DRAG_SPEED >= KNOB_DRAG_SPEED_FINE * 3.0) };
    let coarse = knob_drag_unit(0.5, -20.0, false);
    let fine = knob_drag_unit(0.5, -20.0, true);
    assert!(
        fine < coarse,
        "Shift moved the value {fine} at least as far as the plain drag {coarse}"
    );
}

#[test]
fn drag_clamps_to_unit_range() {
    assert_eq!(knob_drag_unit(0.9, -1000.0, false), 1.0);
    assert_eq!(knob_drag_unit(0.1, 1000.0, false), 0.0);
    assert_eq!(knob_drag_unit(0.5, 0.0, false), 0.5);
}

/// The default cell is the 52 px lavender knob the drums editor lays
/// out with; a style change here reflows every editor using it.
#[test]
fn lavender_style_cell_is_unchanged() {
    let cell = KnobStyle::LAVENDER.cell();
    assert_eq!((cell.x, cell.y), (60.0, 84.0));
    assert_eq!(KnobStyle::default(), KnobStyle::LAVENDER);
}

/// A style is nothing but geometry: the cell always spans the dial plus
/// its paddings, whatever diameter a plugin picks.
#[test]
fn cell_tracks_the_dial_diameter() {
    let style = KnobStyle {
        diameter: 38.0,
        pad_x: 12.0,
        text_h: 30.0,
        ..KnobStyle::LAVENDER
    };
    let cell = style.cell();
    assert_eq!((cell.x, cell.y), (50.0, 68.0));
}

// ---------------------------------------------------------------------------
// Gesture reporting (`knob_themed_edit` / `slider_edit`)
// ---------------------------------------------------------------------------

mod gesture {
    use plugin_gui_core::egui;
    use plugin_gui_core::widgets::{
        knob_themed_edit, slider_edit, GestureEdit, HSlider, ThemedKnob,
    };

    /// One headless frame with `events`, drawing `widget` at the top-left
    /// of a 400×300 window. Returns what it reported.
    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        widget: &mut dyn FnMut(&mut egui::Ui) -> GestureEdit,
    ) -> GestureEdit {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = GestureEdit::default();
        let _ = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| out = widget(ui));
        });
        out
    }

    fn button(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    /// Press at `from`, move in steps to `to`, release: every frame's
    /// report, in order.
    fn drag(
        widget: &mut dyn FnMut(&mut egui::Ui) -> GestureEdit,
        from: egui::Pos2,
        to: egui::Pos2,
    ) -> Vec<GestureEdit> {
        let ctx = egui::Context::default();
        let mut log = vec![frame(&ctx, Vec::new(), widget)];
        log.push(frame(&ctx, button(from, true), widget));
        for i in 1..=5 {
            let t = i as f32 / 5.0;
            let p = from + (to - from) * t;
            log.push(frame(&ctx, vec![egui::Event::PointerMoved(p)], widget));
        }
        log.push(frame(&ctx, button(to, false), widget));
        log.push(frame(&ctx, Vec::new(), widget));
        log
    }

    /// One drag of a knob is one gesture: it begins once, moves the value,
    /// and ends exactly once — on release, never mid-drag.
    #[test]
    fn a_knob_drag_begins_once_and_ends_once() {
        let mut unit = 0.5f32;
        let mut knob = |ui: &mut egui::Ui| {
            let edit = knob_themed_edit(ui, &ThemedKnob::new("Level", unit, "0 dB", 0.5));
            if let Some(v) = edit.value {
                unit = v;
            }
            edit
        };
        let log = drag(&mut knob, egui::pos2(30.0, 26.0), egui::pos2(30.0, -14.0));
        assert_eq!(log.iter().filter(|e| e.began).count(), 1, "{log:?}");
        assert_eq!(log.iter().filter(|e| e.ended).count(), 1, "{log:?}");
        let end = log.iter().position(|e| e.ended).unwrap();
        assert!(
            log[..end].iter().any(|e| e.value.is_some()),
            "the drag never moved the value: {log:?}"
        );
        assert!(
            log[end + 1..].iter().all(|e| e.value.is_none() && !e.began),
            "the knob kept reporting after release: {log:?}"
        );
        assert!(unit > 0.5, "dragging up must raise the value, got {unit}");
    }

    /// A slider drag: the same one-begin, one-end shape.
    #[test]
    fn a_slider_drag_begins_once_and_ends_once() {
        let mut unit = 0.2f32;
        let mut slider = |ui: &mut egui::Ui| {
            let edit = slider_edit(ui, &HSlider::new(200.0, unit));
            if let Some(v) = edit.value {
                unit = v;
            }
            edit
        };
        let log = drag(&mut slider, egui::pos2(40.0, 9.0), egui::pos2(160.0, 9.0));
        assert_eq!(log.iter().filter(|e| e.began).count(), 1, "{log:?}");
        assert_eq!(log.iter().filter(|e| e.ended).count(), 1, "{log:?}");
        assert!((unit - 0.8).abs() < 0.02, "the slider ended at {unit}");
    }

    /// A click on a slider's track is not an edit: nothing moves, and no
    /// gesture opens or closes.
    #[test]
    fn a_slider_click_is_no_edit() {
        let ctx = egui::Context::default();
        let mut slider = |ui: &mut egui::Ui| slider_edit(ui, &HSlider::new(200.0, 0.5));
        frame(&ctx, Vec::new(), &mut slider);
        let press = frame(&ctx, button(egui::pos2(50.0, 9.0), true), &mut slider);
        let release = frame(&ctx, button(egui::pos2(50.0, 9.0), false), &mut slider);
        assert_eq!(press, GestureEdit::default());
        assert_eq!(release, GestureEdit::default());
    }

    /// A slider drag is relative: it starts from the value, not from where
    /// the pointer went down, and Shift makes it five times finer.
    #[test]
    fn a_slider_drag_is_relative_and_shift_is_fine() {
        for (shift, want) in [(false, 0.5 + 60.0 / 200.0), (true, 0.5 + 60.0 / 200.0 * 0.2)] {
            let ctx = egui::Context::default();
            let mut unit = 0.5f32;
            let mods = if shift {
                egui::Modifiers::SHIFT
            } else {
                egui::Modifiers::NONE
            };
            let mut slider = |ui: &mut egui::Ui| {
                let edit = slider_edit(ui, &HSlider::new(200.0, unit));
                if let Some(v) = edit.value {
                    unit = v;
                }
                edit
            };
            let mut run = |events: Vec<egui::Event>| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    modifiers: mods,
                    events,
                    ..Default::default()
                };
                let _ = ctx.run_ui(input, |ui| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show_inside(ui, |ui| {
                            slider(ui);
                        });
                });
            };
            run(Vec::new());
            // Down near the left end — far from the thumb at 0.5.
            run(button(egui::pos2(20.0, 9.0), true));
            for i in 1..=6 {
                run(vec![egui::Event::PointerMoved(egui::pos2(20.0 + 10.0 * i as f32, 9.0))]);
            }
            run(button(egui::pos2(80.0, 9.0), false));
            assert!((unit - want).abs() < 0.02, "shift {shift}: ended at {unit}, want {want}");
        }
    }

    fn double_click(
        ctx: &egui::Context,
        at: egui::Pos2,
        widget: &mut dyn FnMut(&mut egui::Ui) -> GestureEdit,
    ) -> Vec<GestureEdit> {
        let mut log = Vec::new();
        for pressed in [true, false, true, false] {
            log.push(frame(ctx, button(at, pressed), widget));
        }
        log
    }

    /// A double-click resets a slider to its default: one gesture, begun
    /// and ended on the same frame. Without a default it does nothing.
    #[test]
    fn a_slider_double_click_resets_to_its_default() {
        let ctx = egui::Context::default();
        let mut unit = 0.9f32;
        let mut slider = |ui: &mut egui::Ui| {
            let edit = slider_edit(ui, &HSlider::new(200.0, unit).default_unit(0.25));
            if let Some(v) = edit.value {
                unit = v;
            }
            edit
        };
        frame(&ctx, Vec::new(), &mut slider);
        let log = double_click(&ctx, egui::pos2(100.0, 9.0), &mut slider);
        assert_eq!(unit, 0.25, "{log:?}");
        let resets: Vec<_> = log.iter().filter(|e| e.value.is_some()).collect();
        assert_eq!(resets.len(), 1, "{log:?}");
        assert!(resets[0].began && resets[0].ended, "{log:?}");

        let ctx = egui::Context::default();
        let mut plain = |ui: &mut egui::Ui| slider_edit(ui, &HSlider::new(200.0, 0.9));
        frame(&ctx, Vec::new(), &mut plain);
        let log = double_click(&ctx, egui::pos2(100.0, 9.0), &mut plain);
        assert!(log.iter().all(|e| *e == GestureEdit::default()), "{log:?}");
    }

    /// A knob's double-click reset is one gesture too: began and ended
    /// together, so a caller pairing them never sees a lone end.
    #[test]
    fn a_knob_double_click_is_one_gesture() {
        let ctx = egui::Context::default();
        let mut unit = 0.9f32;
        let mut knob = |ui: &mut egui::Ui| {
            let edit = knob_themed_edit(ui, &ThemedKnob::new("Level", unit, "0 dB", 0.5));
            if let Some(v) = edit.value {
                unit = v;
            }
            edit
        };
        frame(&ctx, Vec::new(), &mut knob);
        let log = double_click(&ctx, egui::pos2(30.0, 26.0), &mut knob);
        assert_eq!(unit, 0.5);
        assert_eq!(log.iter().filter(|e| e.began).count(), 1, "{log:?}");
        assert_eq!(log.iter().filter(|e| e.ended).count(), 1, "{log:?}");
        let reset = log.iter().find(|e| e.value.is_some()).unwrap();
        assert!(reset.began && reset.ended, "{log:?}");
    }

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// One frame at egui time `t`.
    fn frame_at(
        ctx: &egui::Context,
        t: f64,
        events: Vec<egui::Event>,
        widget: &mut dyn FnMut(&mut egui::Ui) -> GestureEdit,
    ) -> GestureEdit {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            time: Some(t),
            events,
            ..Default::default()
        };
        let mut out = GestureEdit::default();
        let _ = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| out = widget(ui));
        });
        out
    }

    /// A run of arrow-key presses on a focused slider is ONE gesture: it
    /// begins with the first press, every press moves the value, and it
    /// ends once the keys have been idle for `KEY_GESTURE_IDLE_SECS` —
    /// one undoable edit, not one per press.
    #[test]
    fn a_run_of_arrow_keys_is_one_gesture() {
        let ctx = egui::Context::default();
        let mut unit = 0.5f32;
        let mut slider = |ui: &mut egui::Ui| {
            let edit = slider_edit(ui, &HSlider::new(200.0, unit));
            if let Some(v) = edit.value {
                unit = v;
            }
            edit
        };
        let mut log = Vec::new();
        let mut t = 0.0;
        log.push(frame_at(&ctx, t, Vec::new(), &mut slider));
        t += 0.016;
        log.push(frame_at(&ctx, t, vec![key(egui::Key::Tab, true)], &mut slider));
        for _ in 0..4 {
            t += 0.1;
            log.push(frame_at(
                &ctx,
                t,
                vec![key(egui::Key::ArrowRight, true), key(egui::Key::ArrowRight, false)],
                &mut slider,
            ));
        }
        // Idle: frames keep coming (the widget asked for them).
        for _ in 0..10 {
            t += 0.1;
            log.push(frame_at(&ctx, t, Vec::new(), &mut slider));
        }
        assert!((unit - (0.5 + 4.0 / 200.0)).abs() < 1e-5, "{unit}");
        assert_eq!(log.iter().filter(|e| e.began).count(), 1, "{log:?}");
        assert_eq!(log.iter().filter(|e| e.ended).count(), 1, "{log:?}");
        let end = log.iter().position(|e| e.ended).unwrap();
        let last_step = log.iter().rposition(|e| e.value.is_some()).unwrap();
        assert!(end > last_step, "the run ended before its last press: {log:?}");
        assert!(log[end].value.is_none(), "{log:?}");
    }

    /// Moving focus away ends a key run at once.
    #[test]
    fn losing_focus_ends_a_key_run() {
        let ctx = egui::Context::default();
        let mut slider = |ui: &mut egui::Ui| {
            let a = slider_edit(ui, &HSlider::new(200.0, 0.5));
            // A second focusable widget, for Tab to move to.
            let _ = ui.button("next");
            a
        };
        frame_at(&ctx, 0.0, Vec::new(), &mut slider);
        frame_at(&ctx, 0.02, vec![key(egui::Key::Tab, true)], &mut slider);
        let step = frame_at(&ctx, 0.04, vec![key(egui::Key::ArrowRight, true)], &mut slider);
        assert!(step.began && step.value.is_some(), "{step:?}");
        let away = frame_at(
            &ctx,
            0.06,
            vec![key(egui::Key::ArrowRight, false), key(egui::Key::Tab, true)],
            &mut slider,
        );
        let after = frame_at(&ctx, 0.08, Vec::new(), &mut slider);
        assert!(away.ended || after.ended, "focus moved and the run never ended: {away:?} {after:?}");
    }

    /// The plain functions still report the value alone.
    #[test]
    fn the_plain_functions_are_the_value_half() {
        let ctx = egui::Context::default();
        let mut unit = None;
        let mut both = |ui: &mut egui::Ui| {
            unit = plugin_gui_core::widgets::slider(ui, &HSlider::new(200.0, 0.5));
            GestureEdit::default()
        };
        frame(&ctx, Vec::new(), &mut both);
        frame(&ctx, button(egui::pos2(100.0, 9.0), true), &mut both);
        frame(&ctx, vec![egui::Event::PointerMoved(egui::pos2(150.0, 9.0))], &mut both);
        assert!(unit.is_some_and(|v| (v - 0.75).abs() < 0.01), "{unit:?}");
    }
}
