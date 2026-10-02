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
//! entry points map `-1..1` on and off it.
//!
//! **Input is the fleet's knob gesture, turned on its side**
//! (ux-guidelines.md: "drag-to-adjust, shift-for-fine", "double-click
//! reset"):
//!
//! - a drag anywhere on the row moves the value *relative* to where it
//!   was — one track width of travel is the full range, a fifth of that
//!   with Shift held ([`SLIDER_FINE`]); a click alone changes nothing.
//!   The forks positioned the value absolutely on a click, so a click
//!   meant to grab the thumb jumped the parameter, and there was no fine
//!   mode at all;
//! - a double-click resets to [`HSlider::default_unit`], when the caller
//!   gave one;
//! - arrow keys nudge it while it has keyboard focus, the way
//!   `egui::Slider` does, and a run of presses is **one** gesture
//!   ([`KEY_GESTURE_IDLE_SECS`]).
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

/// How much finer a Shift-drag moves a slider than a plain one: a fifth,
/// the same ratio as the knobs' [`super::KNOB_DRAG_SPEED_FINE`] to
/// [`super::KNOB_DRAG_SPEED`].
pub const SLIDER_FINE: f32 = 0.2;

/// Arrow-key presses on a focused slider closer together than this are
/// one gesture: it begins with the first press and ends this long after
/// the last one (with no arrow held), or when the slider loses focus.
/// One undoable edit per run of presses, not one per press.
pub const KEY_GESTURE_IDLE_SECS: f64 = 0.5;

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
    /// The value a double-click resets to, `0..1`; `None` (the default)
    /// leaves a double-click doing nothing.
    pub default_unit: Option<f32>,
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
            default_unit: None,
        }
    }

    /// Reset to `default_unit` (`0..1`) on a double-click.
    pub fn default_unit(mut self, default_unit: f32) -> Self {
        self.default_unit = Some(default_unit.clamp(0.0, 1.0));
        self
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
/// dragged or nudged.
pub fn slider_unipolar(ui: &mut egui::Ui, width: f32, value_unit: f32) -> Option<f32> {
    slider(ui, &HSlider::new(width, value_unit))
}

/// Bipolar slider: `value_signed` is `-1..1`. Returns the new signed
/// value while dragged or nudged.
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
/// value while a drag, a double-click reset or the keyboard moves it.
pub fn slider(ui: &mut egui::Ui, s: &HSlider) -> Option<f32> {
    slider_edit(ui, s).value
}

/// [`slider`], reporting the gesture too ([`super::GestureEdit`]):
///
/// - a drag begins on the frame it starts and ends on release;
/// - a double-click reset is one finished edit, begun and ended at once;
/// - a run of arrow-key presses begins with the first and ends
///   [`KEY_GESTURE_IDLE_SECS`] after the last, or on focus loss — the
///   frame that ends it carries no value.
///
/// A click, or Space/Enter on a focused slider, changes nothing.
pub fn slider_edit(ui: &mut egui::Ui, s: &HSlider) -> super::GestureEdit {
    let style = s.style;
    let size = egui::vec2(s.width, style.height);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

    if ui.is_rect_visible(rect) {
        draw(ui, rect, s, response.hovered());
    }

    let mut edit = super::GestureEdit::default();
    let width = rect.width().max(1.0);

    // Relative drag: the value the drag started from plus the pointer's
    // travel since, kept in memory (not re-read from `s.value_unit`
    // each frame), so a caller that quantizes — an integer param —
    // still moves once the travel adds up to a step, and switching Shift
    // mid-drag never jumps.
    let drag_id = response.id.with("hslider_drag");
    if response.drag_started() {
        ui.data_mut(|d| d.insert_temp(drag_id, (s.value_unit.clamp(0.0, 1.0), 0.0f32)));
        edit.began = true;
    }
    if response.dragged() {
        let dx = response.drag_delta().x;
        let fine = ui.input(|i| i.modifiers.shift);
        let (start, mut travel) = ui
            .data(|d| d.get_temp::<(f32, f32)>(drag_id))
            .unwrap_or((s.value_unit.clamp(0.0, 1.0), 0.0));
        travel += dx * if fine { SLIDER_FINE } else { 1.0 };
        ui.data_mut(|d| d.insert_temp(drag_id, (start, travel)));
        if dx != 0.0 {
            edit.value = Some((start + travel / width).clamp(0.0, 1.0));
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<(f32, f32)>(drag_id));
        edit.ended = true;
    }

    if response.double_clicked() {
        if let Some(default) = s.default_unit {
            edit.value = Some(default);
            edit.began = true;
            edit.ended = true;
        }
    }

    key_gesture(ui, &response, s, width, &mut edit);

    // Same reasoning as the chip's: `egui::Slider` reported itself to
    // AccessKit and the editors that migrated onto this one would
    // otherwise have lost that. The forks never had it.
    response.widget_info(|| {
        egui::WidgetInfo::slider(
            ui.is_enabled(),
            f64::from(edit.value.unwrap_or(s.value_unit)),
            "",
        )
    });

    edit
}

/// Arrow-key nudges of a focused slider, folded into `edit`: a run of
/// presses is one gesture (see [`KEY_GESTURE_IDLE_SECS`]).
fn key_gesture(
    ui: &egui::Ui,
    response: &egui::Response,
    s: &HSlider,
    width: f32,
    edit: &mut super::GestureEdit,
) {
    // When the run's last press landed (egui time), while one is open.
    let run_id = response.id.with("hslider_keys");
    let now = ui.input(|i| i.time);
    let open = ui.data(|d| d.get_temp::<f64>(run_id));
    let steps = arrow_key_steps(ui, response);
    if steps != 0.0 {
        // One pixel of travel per press, which is `egui::Slider`'s own
        // `ui_point_per_step`. What we cannot reproduce is its
        // "smart aim" — the pass that rounds a drag towards a round
        // number — because that needs the plain range, and this widget
        // only ever sees unit travel.
        let from = edit.value.unwrap_or(s.value_unit);
        edit.value = Some((from + steps / width).clamp(0.0, 1.0));
        edit.began |= open.is_none();
        ui.data_mut(|d| d.insert_temp(run_id, now));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(KEY_GESTURE_IDLE_SECS));
        return;
    }
    let Some(last) = open else {
        return;
    };
    let held = ui.input(|i| i.key_down(egui::Key::ArrowLeft) || i.key_down(egui::Key::ArrowRight));
    let idle = now - last;
    if !response.has_focus() || (!held && idle >= KEY_GESTURE_IDLE_SECS) {
        ui.data_mut(|d| d.remove::<f64>(run_id));
        edit.ended = true;
    } else {
        // Come back when the run would end, so it ends without input.
        let left = (KEY_GESTURE_IDLE_SECS - idle).max(0.01);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(left));
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
