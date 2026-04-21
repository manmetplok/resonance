//! Built-in tuner widget. Draws:
//!
//! - A large note name and cents label in the centre.
//! - A horizontal strobe bar from -50 to +50 cents with a moving
//!   indicator. Three colour zones: green when within ±3 ¢, yellow
//!   when within ±15 ¢, dim grey otherwise.
//! - Everything fades toward grey when the tracker's confidence is
//!   low or the signal is silent, so ambient noise doesn't pin the
//!   needle at nonsense.

use wayland_plugin_gui::egui;

use crate::viz::AmpViz;

use super::theme;

/// Twelve chromatic note names starting from C.
const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Semitone offset that counts as "in tune" for the green indicator.
const IN_TUNE_CENTS: f32 = 3.0;
/// Semitone offset that counts as "close" (yellow).
const NEAR_CENTS: f32 = 15.0;
/// Half-width of the strobe bar in cents.
const STROBE_CENTS: f32 = 50.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, viz: &AmpViz) {
    // Background panel.
    painter.rect_filled(rect.shrink(4.0), 4.0, theme::PANEL);
    painter.rect_stroke(
        rect.shrink(4.0),
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    // Header label sits in the top-left corner.
    painter.text(
        egui::pos2(rect.left() + 12.0, rect.top() + 6.0),
        egui::Align2::LEFT_TOP,
        "TUNER",
        egui::FontId::proportional(10.0),
        theme::TEXT_DIM,
    );

    let (hz, conf) = viz.read_tuner();
    let active = hz > 0.0 && conf > 0.4;

    // Layout inside the panel.
    let inner = rect.shrink2(egui::vec2(16.0, 22.0));
    if inner.width() < 40.0 || inner.height() < 20.0 {
        return;
    }

    // Strobe bar takes the bottom half; note + cents label the top half.
    let top_rect = egui::Rect::from_min_max(
        inner.min,
        egui::pos2(inner.right(), inner.top() + inner.height() * 0.55),
    );
    let bar_rect =
        egui::Rect::from_min_max(egui::pos2(inner.left(), top_rect.bottom() + 2.0), inner.max);

    // Note name and cents label.
    if active {
        let (name, octave, cents) = hz_to_note_cents(hz);
        let note_text = format!("{name}{octave}");
        let cents_text = format_cents(cents);
        let needle_color = color_for_cents(cents);

        painter.text(
            egui::pos2(top_rect.center().x - 40.0, top_rect.center().y),
            egui::Align2::CENTER_CENTER,
            note_text,
            egui::FontId::proportional(28.0),
            theme::TEXT,
        );
        painter.text(
            egui::pos2(top_rect.center().x + 40.0, top_rect.center().y),
            egui::Align2::CENTER_CENTER,
            cents_text,
            egui::FontId::proportional(18.0),
            needle_color,
        );
        painter.text(
            egui::pos2(inner.right() - 4.0, top_rect.top() + 2.0),
            egui::Align2::RIGHT_TOP,
            format!("{hz:>6.1} Hz"),
            egui::FontId::monospace(10.0),
            theme::TEXT_DIM,
        );
    } else {
        painter.text(
            top_rect.center(),
            egui::Align2::CENTER_CENTER,
            "— — —",
            egui::FontId::proportional(22.0),
            theme::TEXT_DIM,
        );
    }

    draw_strobe(painter, bar_rect, active, hz);
}

fn draw_strobe(painter: &egui::Painter, rect: egui::Rect, active: bool, hz: f32) {
    // Trough.
    painter.rect_filled(rect, 3.0, theme::BG);
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    // Tick marks at 0, ±10, ±25, ±50 cents.
    for &c in &[-50.0, -25.0, -10.0, 0.0, 10.0, 25.0, 50.0] {
        let x = cents_to_x(c, rect);
        let tall = c == 0.0;
        let (top_dy, bot_dy) = if tall { (2.0, 2.0) } else { (6.0, 6.0) };
        painter.line_segment(
            [
                egui::pos2(x, rect.top() + top_dy),
                egui::pos2(x, rect.bottom() - bot_dy),
            ],
            egui::Stroke::new(
                if tall { 1.5 } else { 0.5 },
                if tall { theme::TEXT_DIM } else { theme::BORDER },
            ),
        );
    }

    if !active {
        return;
    }

    let (_, _, cents) = hz_to_note_cents(hz);
    let clamped = cents.clamp(-STROBE_CENTS, STROBE_CENTS);
    let x = cents_to_x(clamped, rect);
    let color = color_for_cents(cents);

    // Needle: a thick vertical bar with a soft glow fill on the "off"
    // side so the eye can see which way to tune.
    let needle_rect = egui::Rect::from_min_max(
        egui::pos2(x - 2.5, rect.top() + 1.0),
        egui::pos2(x + 2.5, rect.bottom() - 1.0),
    );
    painter.rect_filled(needle_rect, 1.0, color);

    // Label the detected cents value just below the needle.
    painter.text(
        egui::pos2(x, rect.bottom() + 1.0),
        egui::Align2::CENTER_BOTTOM,
        format_cents(cents),
        egui::FontId::monospace(9.0),
        color,
    );
}

fn cents_to_x(cents: f32, rect: egui::Rect) -> f32 {
    let t = (cents / (STROBE_CENTS * 2.0)) + 0.5;
    rect.left() + t.clamp(0.0, 1.0) * rect.width()
}

fn color_for_cents(cents: f32) -> egui::Color32 {
    let abs = cents.abs();
    if abs <= IN_TUNE_CENTS {
        theme::TUNE_OK
    } else if abs <= NEAR_CENTS {
        theme::TUNE_NEAR
    } else {
        theme::TUNE_OFF
    }
}

fn format_cents(cents: f32) -> String {
    if cents.abs() < 0.5 {
        "±0 ¢".to_string()
    } else if cents >= 0.0 {
        format!("+{:.0} ¢", cents)
    } else {
        format!("{:.0} ¢", cents)
    }
}

/// Convert a frequency in Hz to (note name, octave, cents offset).
/// A4 (440 Hz) is note "A" octave 4 with 0 cents.
pub fn hz_to_note_cents(hz: f32) -> (&'static str, i32, f32) {
    // Semitones from A4.
    let semis = 12.0 * (hz / 440.0).log2();
    let rounded = semis.round() as i32;
    let cents = (semis - rounded as f32) * 100.0;

    // A4 is midi note 69; `rounded` is semis from A4, so midi = 69 + rounded.
    let midi = 69 + rounded;
    // Note index: 0=C, 1=C#, …, 11=B. Midi 60 = C4.
    let note_idx = ((midi % 12) + 12) % 12;
    let octave = midi / 12 - 1;
    (NOTE_NAMES[note_idx as usize], octave, cents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_is_a4_zero_cents() {
        let (n, o, c) = hz_to_note_cents(440.0);
        assert_eq!(n, "A");
        assert_eq!(o, 4);
        assert!(c.abs() < 0.01, "cents should be ~0, got {c}");
    }

    #[test]
    fn low_e_is_e2() {
        let (n, o, c) = hz_to_note_cents(82.407);
        assert_eq!(n, "E");
        assert_eq!(o, 2);
        assert!(c.abs() < 1.0);
    }

    #[test]
    fn high_e_is_e4() {
        let (n, o, c) = hz_to_note_cents(329.628);
        assert_eq!(n, "E");
        assert_eq!(o, 4);
        assert!(c.abs() < 1.0);
    }

    #[test]
    fn plus_thirty_cents_sharp() {
        // 440 * 2^(0.3/12) ≈ 447.70 — unambiguously +30 cents above A4.
        let hz = 440.0 * 2f32.powf(0.3 / 12.0);
        let (n, o, c) = hz_to_note_cents(hz);
        assert_eq!(n, "A");
        assert_eq!(o, 4);
        assert!((c - 30.0).abs() < 0.5, "expected +30 cents, got {c}");
    }

    #[test]
    fn minus_thirty_cents_flat() {
        // 440 * 2^(-0.3/12) ≈ 432.47 — unambiguously -30 cents below A4.
        let hz = 440.0 * 2f32.powf(-0.3 / 12.0);
        let (n, o, c) = hz_to_note_cents(hz);
        assert_eq!(n, "A");
        assert_eq!(o, 4);
        assert!((c + 30.0).abs() < 0.5, "expected -30 cents, got {c}");
    }
}
