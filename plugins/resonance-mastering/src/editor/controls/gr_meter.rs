//! Slim horizontal gain-reduction meter, drawn once per multiband band.
//!
//! Setting a per-band threshold with no feedback is guesswork: the user
//! cannot tell whether the band is compressing at all, let alone by how
//! much. Each band compressor already measures its own gain reduction
//! (and decays it over ~250 ms, so the number is readable rather than
//! flickering); this draws it.
//!
//! It reads right-to-left — the bar grows leftwards from the 0 dB end,
//! the way a gain-reduction meter conventionally does, so "more bar"
//! means "more compression" at a glance without reading the number.

use wayland_plugin_gui::egui;

use super::theme;

/// Full-scale deflection. 12 dB covers the useful mastering range with
/// room to spare; beyond it the bar simply pins.
pub const FULL_SCALE_DB: f32 = 12.0;

/// Below this the band is treated as not compressing, so the meter shows
/// an empty track instead of a sliver that reads as "something is
/// happening". Also the point at which the numeric readout appears.
pub const ACTIVE_THRESHOLD_DB: f32 = 0.1;

/// Height of the meter strip.
pub const HEIGHT: f32 = 12.0;

/// How much of the track is filled, 0..=1, for a gain reduction in dB.
/// Anything at or below [`ACTIVE_THRESHOLD_DB`] reads as empty and
/// anything past [`FULL_SCALE_DB`] pins full.
pub fn fill_fraction(gr_db: f32) -> f32 {
    if !gr_db.is_finite() || gr_db <= ACTIVE_THRESHOLD_DB {
        return 0.0;
    }
    (gr_db / FULL_SCALE_DB).min(1.0)
}

/// Whether the band is compressing enough to be worth calling out.
pub fn is_active(gr_db: f32) -> bool {
    fill_fraction(gr_db) > 0.0
}

/// The label to the right of the bar. Idle bands print a dash rather
/// than "0.0 dB", so a glance down the four bands separates "not
/// compressing" from "compressing a little".
pub fn readout(gr_db: f32) -> String {
    if is_active(gr_db) {
        format!("-{gr_db:.1} dB")
    } else {
        "—".to_string()
    }
}

/// Draw the meter into `rect`. `gr_db` is positive for attenuation.
pub fn draw(painter: &egui::Painter, rect: egui::Rect, gr_db: f32) {
    // Leave room on the right for the numeric readout.
    let label_w = 52.0;
    let track = egui::Rect::from_min_max(
        rect.min,
        egui::pos2((rect.right() - label_w).max(rect.left()), rect.bottom()),
    );

    painter.rect_filled(track, 2.0, theme::BG);
    painter.rect_stroke(
        track,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let fill = fill_fraction(gr_db);
    if fill > 0.0 {
        // Grows leftwards from the right-hand (0 dB) end.
        let w = track.width() * fill;
        let bar = egui::Rect::from_min_max(
            egui::pos2(track.right() - w, track.top() + 1.0),
            egui::pos2(track.right(), track.bottom() - 1.0),
        );
        // Warn once the band is doing more than gentle levelling.
        let colour = if gr_db > 6.0 {
            theme::WARN
        } else {
            theme::ACCENT
        };
        painter.rect_filled(bar, 1.0, colour);
    }

    painter.text(
        egui::pos2(rect.right(), rect.center().y),
        egui::Align2::RIGHT_CENTER,
        readout(gr_db),
        egui::FontId::proportional(10.0),
        if is_active(gr_db) {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        },
    );
}
