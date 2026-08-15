//! Horizontal slider with a filled track and a circular thumb.
//!
//! The canonical version of the slider the drums and wavetable editors
//! each carried privately (ba doc #275, LOW finding; promoted by ba todo
//! #1334). Unlike the chip and the segmented control, these two had
//! already diverged, so the shared version is the union of both:
//!
//! - drums' copy has unipolar, bipolar and *warm* bipolar entry points
//!   ([`SliderTone::Warm`], the mic-balance slider); wavetable's has
//!   bipolar only. All of it is here.
//! - **bipolar fill colour** — the divergence. Both fill the negative
//!   side warm; on the positive side wavetable always fills accent,
//!   while drums fills with the slider's own tone (accent normally, warm
//!   for a warm slider, so a warm slider is warm on both sides). Drums'
//!   rule is the superset — it reproduces wavetable's exactly for an
//!   accent slider — so that is the rule here, and the design intent it
//!   encodes is written down in [`fill_span`] rather than left to be
//!   re-derived a third time.
//!
//! Values are in unit (`0..1`) space at the drawing layer; the bipolar
//! entry points map `-1..1` on and off it. A click or a drag anywhere on
//! the track positions the value — the widget has no separate "grab the
//! thumb" mode, and never had one in either fork.

use crate::theme::lavender as theme;

/// Which accent a slider paints its fill and thumb ring with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderTone {
    /// The lavender brand accent — the default.
    Accent,
    /// The warm token, for parameters that are already polarity- or
    /// temperature-coloured (the drums mic-balance slider).
    Warm,
}

impl SliderTone {
    /// The colour this tone paints with.
    pub fn color(self) -> egui::Color32 {
        match self {
            Self::Accent => theme::ACCENT,
            Self::Warm => theme::WARM,
        }
    }
}

/// Geometry of a horizontal slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderStyle {
    /// Total height of the allocated row, px.
    pub height: f32,
    /// Track thickness, px.
    pub track_height: f32,
    /// Thumb radius, px.
    pub thumb_radius: f32,
}

impl SliderStyle {
    /// The 18 px row both editors' forks used.
    pub const LAVENDER: Self = Self {
        height: 18.0,
        track_height: 3.0,
        thumb_radius: 5.5,
    };
}

impl Default for SliderStyle {
    fn default() -> Self {
        Self::LAVENDER
    }
}

/// One horizontal slider, in unit (`0..1`) value space.
#[derive(Debug, Clone, Copy)]
pub struct HSlider {
    /// Track width, px.
    pub width: f32,
    /// Current value, `0..1`.
    pub value_unit: f32,
    /// Fill outward from the centre instead of from the left edge, and
    /// draw a centre tick.
    pub bipolar: bool,
    /// Accent used for the fill and the thumb ring.
    pub tone: SliderTone,
    /// Row geometry.
    pub style: SliderStyle,
}

impl HSlider {
    /// A default-styled accent slider.
    pub fn new(width: f32, value_unit: f32) -> Self {
        Self {
            width,
            value_unit,
            bipolar: false,
            tone: SliderTone::Accent,
            style: SliderStyle::LAVENDER,
        }
    }

    /// Fill outward from the centre.
    pub fn bipolar(mut self, bipolar: bool) -> Self {
        self.bipolar = bipolar;
        self
    }

    /// Paint with a non-default accent.
    pub fn tone(mut self, tone: SliderTone) -> Self {
        self.tone = tone;
        self
    }

    /// Use non-default geometry.
    pub fn style(mut self, style: SliderStyle) -> Self {
        self.style = style;
        self
    }
}

/// Unipolar slider: `value_unit` is `0..1`. Returns the new value while
/// dragged or clicked.
pub fn slider_unipolar(ui: &mut egui::Ui, width: f32, value_unit: f32) -> Option<f32> {
    slider(ui, &HSlider::new(width, value_unit))
}

/// Bipolar slider: `value_signed` is `-1..1`. Returns the new signed
/// value while dragged or clicked.
pub fn slider_bipolar(ui: &mut egui::Ui, width: f32, value_signed: f32) -> Option<f32> {
    bipolar_with_tone(ui, width, value_signed, SliderTone::Accent)
}

/// Bipolar slider in the warm palette — the drums mic-balance slider.
pub fn slider_bipolar_warm(ui: &mut egui::Ui, width: f32, value_signed: f32) -> Option<f32> {
    bipolar_with_tone(ui, width, value_signed, SliderTone::Warm)
}

fn bipolar_with_tone(
    ui: &mut egui::Ui,
    width: f32,
    value_signed: f32,
    tone: SliderTone,
) -> Option<f32> {
    let unit = (value_signed + 1.0) * 0.5;
    let s = HSlider::new(width, unit).bipolar(true).tone(tone);
    slider(ui, &s).map(unit_to_signed)
}

/// Map a unit (`0..1`) value onto the bipolar (`-1..1`) range.
pub fn unit_to_signed(unit: f32) -> f32 {
    unit * 2.0 - 1.0
}

/// Map a bipolar (`-1..1`) value onto unit (`0..1`) space.
pub fn signed_to_unit(signed: f32) -> f32 {
    (signed + 1.0) * 0.5
}

/// The horizontal span (as fractions of the track, `0..1`) the fill
/// covers, and whether it sits on the negative side of a bipolar
/// slider.
///
/// Split out from the drawing so the fill rule — the one thing the two
/// forks disagreed on — is stated once and testable without a window:
///
/// - unipolar fills from the left edge to the value;
/// - bipolar fills between the centre and the value. Below centre the
///   fill is *always* warm so the polarity reads at a glance even on an
///   accent slider; at or above centre it takes the slider's own tone
///   (which is why a warm slider is warm on both sides — it is already
///   polarity-coloured).
pub fn fill_span(value_unit: f32, bipolar: bool) -> (f32, f32, bool) {
    let v = value_unit.clamp(0.0, 1.0);
    if !bipolar {
        return (0.0, v, false);
    }
    if v >= 0.5 {
        (0.5, v, false)
    } else {
        (v, 0.5, true)
    }
}

/// Draw a configured slider and handle its input. Returns the new unit
/// value while the pointer is positioning it.
pub fn slider(ui: &mut egui::Ui, s: &HSlider) -> Option<f32> {
    let style = s.style;
    let size = egui::vec2(s.width, style.height);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

    if ui.is_rect_visible(rect) {
        draw(ui, rect, s, response.hovered());
    }

    if response.dragged() || response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            return Some(((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
        }
    }
    None
}

fn draw(ui: &egui::Ui, rect: egui::Rect, s: &HSlider, hovered: bool) {
    let style = s.style;
    let painter = ui.painter_at(rect);

    let track_y = rect.center().y;
    let track_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), track_y - style.track_height * 0.5),
        egui::vec2(rect.width(), style.track_height),
    );
    painter.rect_filled(track_rect, 1.5, theme::BG_1);
    painter.rect_stroke(
        track_rect,
        1.5,
        egui::Stroke::new(1.0, theme::LINE_2),
        egui::StrokeKind::Inside,
    );

    let v = s.value_unit.clamp(0.0, 1.0);
    let thumb_x = rect.left() + v * rect.width();
    let primary = s.tone.color();

    let (from, to, negative) = fill_span(v, s.bipolar);
    let fill_color = if negative { theme::WARM } else { primary };
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.left() + from * rect.width(), track_rect.top()),
            egui::pos2(rect.left() + to * rect.width(), track_rect.bottom()),
        ),
        1.5,
        fill_color,
    );

    if s.bipolar {
        let center_x = rect.center().x;
        painter.line_segment(
            [
                egui::pos2(center_x, track_y - 4.0),
                egui::pos2(center_x, track_y + 4.0),
            ],
            egui::Stroke::new(1.0, theme::TEXT_4),
        );
    }

    // Thumb.
    let thumb_color = if hovered {
        theme::ACCENT_SOFT
    } else {
        theme::BG_3
    };
    let center = egui::pos2(thumb_x, track_y);
    painter.circle_filled(center, style.thumb_radius, thumb_color);
    painter.circle_stroke(
        center,
        style.thumb_radius,
        egui::Stroke::new(1.5, primary),
    );
}
