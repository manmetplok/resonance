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
//! thumb" mode, and never had one in either fork. Arrow keys nudge it
//! while it has keyboard focus, the way `egui::Slider` does.
//!
//! The EQ moved onto this slider from a raw `egui::Slider` (ba todo
//! #1335) while it was still on the retired blue `classic` palette, so
//! the surface colours were split out into a [`SliderPalette`] and a
//! second `CLASSIC` constant carried the blue. Ba todo #1338 moved the
//! EQ onto the canonical palette and that constant is gone —
//! [`SliderPalette`] stays a struct because the geometry and the colours
//! are still chosen independently, but there is one palette to choose.
//!
//! Both of the geometry constants the two editors' forks had are the
//! same 18 px row, which is also what `egui::Slider` allocated in the EQ
//! (`spacing.interact_size.y`), so nothing moved when it swapped.

use crate::theme::lavender as theme;

/// Surface colours of a slider.
///
/// Split out of the geometry because the two are chosen independently:
/// every editor in the fleet wants the same 18 px row, and the EQ wants
/// it in a different palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderPalette {
    /// Unfilled track.
    pub track: egui::Color32,
    /// Track outline.
    pub track_stroke: egui::Color32,
    /// Fill and thumb ring of an [`SliderTone::Accent`] slider.
    pub accent: egui::Color32,
    /// Fill of a [`SliderTone::Warm`] slider, and of the negative side
    /// of any bipolar one.
    pub warm: egui::Color32,
    /// Thumb interior at rest.
    pub thumb: egui::Color32,
    /// Thumb interior while hovered.
    pub thumb_hover: egui::Color32,
    /// The centre tick of a bipolar slider.
    pub tick: egui::Color32,
}

impl SliderPalette {
    /// The canonical lavender palette — what every editor paints with.
    pub const LAVENDER: Self = Self {
        track: theme::BG_1,
        track_stroke: theme::LINE_2,
        accent: theme::ACCENT,
        warm: theme::WARM,
        thumb: theme::BG_3,
        thumb_hover: theme::ACCENT_SOFT,
        tick: theme::TEXT_4,
    };
}

/// Which of a palette's two accents a slider paints its fill and thumb
/// ring with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderTone {
    /// The brand accent — the default.
    Accent,
    /// The warm token, for parameters that are already polarity- or
    /// temperature-coloured (the drums mic-balance slider).
    Warm,
}

impl SliderTone {
    /// The colour this tone paints with, in a given palette.
    pub fn color(self, palette: &SliderPalette) -> egui::Color32 {
        match self {
            Self::Accent => palette.accent,
            Self::Warm => palette.warm,
        }
    }
}

/// Geometry and palette of a horizontal slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderStyle {
    /// Total height of the allocated row, px.
    pub height: f32,
    /// Track thickness, px.
    pub track_height: f32,
    /// Thumb radius, px.
    pub thumb_radius: f32,
    /// Surface colours.
    pub palette: SliderPalette,
}

impl SliderStyle {
    /// The 18 px row both editors' forks used, and the one the EQ's
    /// band columns are laid out around.
    pub const LAVENDER: Self = Self {
        height: 18.0,
        track_height: 3.0,
        thumb_radius: 5.5,
        palette: SliderPalette::LAVENDER,
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
/// value while the pointer or the keyboard is positioning it.
pub fn slider(ui: &mut egui::Ui, s: &HSlider) -> Option<f32> {
    slider_edit(ui, s).value
}

/// [`slider`], reporting the gesture too ([`super::GestureEdit`]): a drag
/// begins and ends; a click on the track and an arrow-key step are each
/// one finished edit.
pub fn slider_edit(ui: &mut egui::Ui, s: &HSlider) -> super::GestureEdit {
    let style = s.style;
    let size = egui::vec2(s.width, style.height);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

    if ui.is_rect_visible(rect) {
        draw(ui, rect, s, response.hovered());
    }

    let mut new_unit = None;
    if response.dragged() || response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            new_unit = Some(((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
        }
    }
    let steps = arrow_key_steps(ui, &response);
    if steps != 0.0 {
        // One pixel of travel per press, which is `egui::Slider`'s own
        // `ui_point_per_step`. What we cannot reproduce is its
        // "smart aim" — the pass that rounds a drag towards a round
        // number — because that needs the plain range, and this widget
        // only ever sees unit travel.
        let from = new_unit.unwrap_or(s.value_unit);
        new_unit = Some((from + steps / rect.width().max(1.0)).clamp(0.0, 1.0));
    }

    // Same reasoning as the chip's: `egui::Slider` reported itself to
    // AccessKit and the editors that migrated onto this one would
    // otherwise have lost that. The forks never had it.
    response.widget_info(|| {
        egui::WidgetInfo::slider(
            ui.is_enabled(),
            f64::from(new_unit.unwrap_or(s.value_unit)),
            "",
        )
    });

    super::GestureEdit {
        value: new_unit,
        began: response.drag_started(),
        ended: response.drag_stopped() || response.clicked() || steps != 0.0,
    }
}

/// Net arrow-key presses this frame — right minus left — while the
/// slider holds keyboard focus, or `0.0` when it does not.
///
/// The horizontal arrows are locked to the widget while it is focused
/// so they adjust the value instead of moving focus to the next
/// control, exactly as `egui::Slider` does for a horizontal slider.
fn arrow_key_steps(ui: &egui::Ui, response: &egui::Response) -> f32 {
    if !response.has_focus() {
        return 0.0;
    }
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            response.id,
            egui::EventFilter {
                horizontal_arrows: true,
                ..Default::default()
            },
        );
    });
    let (left, right) = ui.input(|i| {
        (
            i.num_presses(egui::Key::ArrowLeft),
            i.num_presses(egui::Key::ArrowRight),
        )
    });
    right as f32 - left as f32
}

fn draw(ui: &egui::Ui, rect: egui::Rect, s: &HSlider, hovered: bool) {
    let style = s.style;
    let palette = style.palette;
    let painter = ui.painter_at(rect);

    let track_y = rect.center().y;
    let track_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), track_y - style.track_height * 0.5),
        egui::vec2(rect.width(), style.track_height),
    );
    painter.rect_filled(track_rect, 1.5, palette.track);
    painter.rect_stroke(
        track_rect,
        1.5,
        egui::Stroke::new(1.0, palette.track_stroke),
        egui::StrokeKind::Inside,
    );

    let v = s.value_unit.clamp(0.0, 1.0);
    let thumb_x = rect.left() + v * rect.width();
    let primary = s.tone.color(&palette);

    let (from, to, negative) = fill_span(v, s.bipolar);
    let fill_color = if negative { palette.warm } else { primary };
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
            egui::Stroke::new(1.0, palette.tick),
        );
    }

    // Thumb.
    let thumb_color = if hovered {
        palette.thumb_hover
    } else {
        palette.thumb
    };
    let center = egui::pos2(thumb_x, track_y);
    painter.circle_filled(center, style.thumb_radius, thumb_color);
    painter.circle_stroke(
        center,
        style.thumb_radius,
        egui::Stroke::new(1.5, primary),
    );
}
