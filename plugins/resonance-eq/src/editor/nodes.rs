//! Draggable band-node overlay for the EQ response curve.
//!
//! Each enabled band gets a node drawn at the curve's (freq, gain)
//! position. Mouse interactions:
//!
//! - Left-drag: change freq and gain together (or freq only for cut bands).
//! - Scroll over the hovered node: adjust Q.
//! - Right-click: open a context menu to change type / slope / disable.
//! - Double-click: toggle enabled.

use plugin_gui_core::egui;
use plugin_gui_core::widgets::GestureEdit;
use resonance_plugin::editor_widgets::{apply_gesture, commit_plain};

use crate::band::{BandKind, BandMs, BandSlope};
use crate::editor::response::{color_for_kind, db_to_y, freq_to_x, x_to_freq, y_to_db};
use crate::editor::{theme, EqEditorApp};
use crate::params::{BandSnapshot, NUM_BANDS};

const NODE_RADIUS: f32 = 8.0;
const NODE_HIT_RADIUS: f32 = 12.0;

/// Transient drag state kept on the app between frames.
#[derive(Clone, Copy)]
pub struct DragState {
    pub band_index: usize,
}

pub fn draw_and_interact(
    ui: &mut egui::Ui,
    plot: egui::Rect,
    app: &mut EqEditorApp,
    snapshots: &[BandSnapshot; NUM_BANDS],
) {
    // Allocate an interactive response over the plot area.
    let response = ui.interact(
        plot,
        egui::Id::new("eq_response_area"),
        egui::Sense::click_and_drag(),
    );

    let pointer = ui.ctx().pointer_latest_pos();

    // First draw each enabled band node.
    let mut hover_node: Option<usize> = None;
    for (i, snapshot) in snapshots.iter().enumerate() {
        if !snapshot.enabled {
            continue;
        }
        let pos = node_position(plot, snapshot);
        let is_hovered = pointer
            .map(|p| (p - pos).length() <= NODE_HIT_RADIUS && plot.contains(p))
            .unwrap_or(false);
        if is_hovered {
            hover_node = Some(i);
        }
        let selected = app.selected_band == Some(i);
        draw_node(ui, pos, snapshot, selected || is_hovered, i);
    }

    // Handle drag lifecycle. A node drag is one gesture on Freq (and
    // Gain): written every frame, announced to the host once, when it
    // ends, for each param it moved (PUX-01).
    let ctx = ui.ctx().clone();
    let began = GestureEdit {
        began: true,
        ..GestureEdit::default()
    };
    if response.drag_started() {
        if let Some(i) = hover_node {
            app.drag_state = Some(DragState { band_index: i });
            app.selected_band = Some(i);
            let band = &app.params.bands[i];
            apply_gesture(&ctx, &band.freq, began, |_| {});
            apply_gesture(&ctx, &band.gain, began, |_| {});
        } else if let Some(p) = pointer {
            // Click on empty area: deselect.
            if plot.contains(p) {
                app.selected_band = None;
            }
        }
    }

    if response.dragged() {
        if let Some(drag) = app.drag_state {
            if let Some(p) = pointer {
                let band = &app.params.bands[drag.band_index];
                let kind = BandKind::from_index(band.kind.value());
                let new_freq = x_to_freq(p.x, plot.left(), plot.width());
                let moved = GestureEdit {
                    value: Some(new_freq.clamp(20.0, 20_000.0)),
                    ..GestureEdit::default()
                };
                apply_gesture(&ctx, &band.freq, moved, |v| band.freq.set_value(v));
                if kind.uses_gain() {
                    let new_gain = y_to_db(p.y, plot.top(), plot.height());
                    let moved = GestureEdit {
                        value: Some(new_gain.clamp(-24.0, 24.0)),
                        ..GestureEdit::default()
                    };
                    apply_gesture(&ctx, &band.gain, moved, |v| band.gain.set_value(v));
                }
            }
        }
    }

    if response.drag_stopped() {
        if let Some(drag) = app.drag_state.take() {
            let band = &app.params.bands[drag.band_index];
            let ended = GestureEdit {
                ended: true,
                ..GestureEdit::default()
            };
            apply_gesture(&ctx, &band.freq, ended, |_| {});
            apply_gesture(&ctx, &band.gain, ended, |_| {});
        }
    }

    // Plain click (no drag) — select the hovered node, or clear selection.
    if response.clicked() {
        if let Some(i) = hover_node {
            app.selected_band = Some(i);
        } else {
            app.selected_band = None;
        }
    }

    // Double click toggles the hovered band on/off. (egui reports this via
    // `double_clicked()` which is separate from `clicked()`.)
    if response.double_clicked() {
        if let Some(i) = hover_node {
            let b = &app.params.bands[i];
            let off = if b.enabled.value() { 0.0 } else { 1.0 };
            commit_plain(&ctx, &b.enabled, off);
        }
    }

    // Scroll adjusts Q of the hovered band: one gesture per scroll,
    // announced when the wheel stops (or the pointer leaves the node).
    let mut scrolled = false;
    if let Some(i) = hover_node {
        let scroll = ui.ctx().input(|inp| inp.smooth_scroll_delta.y);
        let uses_q = BandKind::from_index(app.params.bands[i].kind.value()).uses_q();
        if scroll.abs() > 0.0 && uses_q && app.q_scroll.is_none_or(|j| j == i) {
            let b = &app.params.bands[i];
            let q = b.q.value();
            let factor = (scroll * 0.005).exp(); // smooth exponential zoom
            let new_q = (q * factor).clamp(0.1, 10.0);
            let moved = GestureEdit {
                value: Some(new_q),
                began: app.q_scroll.is_none(),
                ended: false,
            };
            apply_gesture(&ctx, &b.q, moved, |v| b.q.set_value(v));
            app.q_scroll = Some(i);
            scrolled = true;
        }
    }
    if !scrolled {
        if let Some(j) = app.q_scroll.take() {
            let ended = GestureEdit {
                ended: true,
                ..GestureEdit::default()
            };
            apply_gesture(&ctx, &app.params.bands[j].q, ended, |_| {});
        }
    }

    // Right-click menu: change kind / slope / remove.
    if let Some(i) = hover_node {
        response.context_menu(|ui| context_menu(ui, app, i));
    } else {
        // Empty-area context menu: no per-band content, close by default.
        response.context_menu(|_ui| {});
    }
}

fn node_position(plot: egui::Rect, snapshot: &BandSnapshot) -> egui::Pos2 {
    let x = plot.left() + freq_to_x(snapshot.freq, plot.width());
    // Cut bands don't have gain — render them on the 0 dB line.
    let gain = if snapshot.kind.uses_gain() {
        snapshot.gain_db
    } else {
        0.0
    };
    let y = plot.top() + db_to_y(gain, plot.height());
    egui::pos2(x, y)
}

fn draw_node(
    ui: &egui::Ui,
    pos: egui::Pos2,
    snapshot: &BandSnapshot,
    highlighted: bool,
    index: usize,
) {
    let painter = ui.painter();
    let color = color_for_kind(snapshot.kind);
    let fill = if highlighted {
        color
    } else {
        color.linear_multiply(0.7)
    };
    let stroke_width = if highlighted { 2.0 } else { 1.0 };
    painter.circle(
        pos,
        NODE_RADIUS,
        fill,
        egui::Stroke::new(stroke_width, theme::TEXT),
    );
    painter.text(
        pos,
        egui::Align2::CENTER_CENTER,
        format!("{}", index + 1),
        egui::FontId::proportional(10.0),
        theme::BG,
    );
}

fn context_menu(ui: &mut egui::Ui, app: &mut EqEditorApp, band_index: usize) {
    let band = &app.params.bands[band_index];

    ui.label(
        egui::RichText::new(format!("Band {}", band_index + 1))
            .strong()
            .color(theme::TEXT_DIM),
    );
    ui.separator();

    let ctx = ui.ctx().clone();
    let mut kind = BandKind::from_index(band.kind.value());
    ui.label("Type");
    for opt in BandKind::ALL {
        if ui.selectable_label(kind == opt, opt.short_name()).clicked() {
            kind = opt;
            commit_plain(&ctx, &band.kind, f64::from(kind.to_index()));
            ui.close();
        }
    }

    if kind.is_cut() {
        ui.separator();
        ui.label("Slope");
        let slope = BandSlope::from_index(band.slope.value());
        for opt in [BandSlope::Db12, BandSlope::Db24, BandSlope::Db48] {
            if ui.selectable_label(slope == opt, opt.label()).clicked() {
                commit_plain(&ctx, &band.slope, f64::from(opt.to_index()));
                ui.close();
            }
        }
    }

    ui.separator();
    ui.label("Channels");
    let ms = BandMs::from_index(band.ms.value());
    for opt in BandMs::ALL {
        if ui.selectable_label(ms == opt, opt.label()).clicked() {
            commit_plain(&ctx, &band.ms, f64::from(opt.to_index()));
            ui.close();
        }
    }

    ui.separator();
    let enabled = band.enabled.value();
    if ui
        .button(if enabled { "Disable" } else { "Enable" })
        .clicked()
    {
        commit_plain(&ctx, &band.enabled, if enabled { 0.0 } else { 1.0 });
        ui.close();
    }
}
