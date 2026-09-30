//! Small widgets for library browsers (plugin presets, NAM models):
//! the ☆/★ favourite toggle and the removable tag pill
//! (plugin-preset-library.md §6.3, nam-model-library.md §6.2).
//!
//! The star is painted, not a font glyph, so it looks the same whatever
//! fonts an editor loaded and it can never render as tofu.

use egui::{Color32, Pos2, Response, Sense, Stroke, Vec2};

use super::chip::ChipStyle;
use crate::theme::lavender as theme;

/// Default edge of the square a [`star_toggle`] occupies, px.
pub const STAR_SIZE: f32 = 16.0;

/// The ten vertices of a five-pointed star centred on `center` with outer
/// radius `r`, first point straight up.
pub fn star_points(center: Pos2, r: f32) -> Vec<Pos2> {
    let inner = r * 0.45;
    (0..10)
        .map(|i| {
            let radius = if i % 2 == 0 { r } else { inner };
            let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            center + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect()
}

/// Paint a star: filled [`theme::WARM`] when `on`, an outline in `idle`
/// otherwise.
pub fn paint_star(painter: &egui::Painter, center: Pos2, r: f32, on: bool, idle: Color32) {
    let points = star_points(center, r);
    if on {
        // A concave polygon does not fill correctly as one convex path, so
        // fill the pentagon core and the five tips separately.
        let core: Vec<Pos2> = points.iter().skip(1).step_by(2).copied().collect();
        painter.add(egui::Shape::convex_polygon(core, theme::WARM, Stroke::NONE));
        for i in (0..10).step_by(2) {
            let tip = vec![points[(i + 9) % 10], points[i], points[(i + 1) % 10]];
            painter.add(egui::Shape::convex_polygon(tip, theme::WARM, Stroke::NONE));
        }
    } else {
        let mut closed = points;
        closed.push(closed[0]);
        painter.add(egui::Shape::line(closed, Stroke::new(1.2, idle)));
    }
}

/// A ☆/★ favourite toggle of `size`×`size` px. Returns the click response;
/// the caller flips its own state on `clicked()`. Hover brightens an
/// unset star so it reads as clickable.
pub fn star_toggle_sized(ui: &mut egui::Ui, on: bool, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, "Favourite")
    });
    if ui.is_rect_visible(rect) {
        let idle = if response.hovered() {
            theme::WARM
        } else {
            theme::TEXT_3
        };
        paint_star(ui.painter(), rect.center(), size * 0.45, on, idle);
    }
    response.on_hover_text(if on {
        "Remove from favourites"
    } else {
        "Add to favourites"
    })
}

/// [`star_toggle_sized`] at [`STAR_SIZE`].
pub fn star_toggle(ui: &mut egui::Ui, on: bool) -> Response {
    star_toggle_sized(ui, on, STAR_SIZE)
}

/// What happened to a [`tag_pill`] this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TagPillResponse {
    /// The label part was clicked (e.g. filter by this tag).
    pub clicked: bool,
    /// The × was clicked.
    pub removed: bool,
}

/// A tag chip in the compact chip style, with a × when `removable`.
/// `active` tints it like an active chip (e.g. the tag is a live filter).
pub fn tag_pill(ui: &mut egui::Ui, label: &str, active: bool, removable: bool) -> TagPillResponse {
    let style = ChipStyle::COMPACT;
    let font = egui::FontId::proportional(style.font_size + 1.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let x_w = if removable { style.height } else { 0.0 };
    let size = Vec2::new(
        galley.size().x + style.pad_x * 2.0 + x_w,
        style.height.max(galley.size().y + 2.0),
    );
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let label_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x - x_w, rect.max.y));
    let id = ui.id().with(("tag_pill", label));
    let label_resp = ui.interact(label_rect, id, Sense::click());
    label_resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label)
    });
    let x_resp = removable.then(|| {
        let x_rect = egui::Rect::from_min_max(egui::pos2(rect.max.x - x_w, rect.min.y), rect.max);
        let r = ui.interact(x_rect, id.with("x"), Sense::click());
        r.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Remove tag {label}"))
        });
        (x_rect, r)
    });
    if ui.is_rect_visible(rect) {
        let hovered = label_resp.hovered() || x_resp.as_ref().is_some_and(|(_, r)| r.hovered());
        let colors = style.colors(active, true, hovered);
        let painter = ui.painter();
        painter.rect_filled(rect, style.radius, colors.fill);
        if let Some(stroke) = colors.stroke {
            painter.rect_stroke(
                rect,
                style.radius,
                Stroke::new(1.0, stroke),
                egui::StrokeKind::Inside,
            );
        }
        let text_pos = egui::pos2(
            label_rect.min.x + style.pad_x,
            label_rect.center().y - galley.size().y * 0.5,
        );
        painter.galley(text_pos, galley, colors.text);
        if let Some((x_rect, r)) = &x_resp {
            let c = x_rect.center();
            let d = style.height * 0.18;
            let color = if r.hovered() { theme::BAD } else { colors.text };
            let stroke = Stroke::new(1.2, color);
            painter.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
            painter.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
        }
    }
    TagPillResponse {
        clicked: label_resp.clicked(),
        removed: x_resp.is_some_and(|(_, r)| r.clicked()),
    }
}
