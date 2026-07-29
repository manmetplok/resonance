//! Hero buffer view — the static frame of the circular-buffer
//! visualization (ba todo #1139, design doc #264 req-1; prototype
//! states 'idle', 'default', 'synced').
//!
//! Everything is painter-drawn (no GL) from params + `GranularViz`
//! reads at the editor's 16 ms repaint cadence:
//! time-axis mapping with the write head at the right edge, the ±24 st
//! pitch ruler, the coarse-peaks backdrop silhouette, division tick
//! marks while synced, the mint pulsing WRITE head (amber HOLD while
//! frozen, with the backdrop tinted), the accent delay tap with its
//! ms/division flag and grab dots, the empty state and the gesture
//! legend. The grain cloud itself is the next todo; it draws into the
//! seam [`draw`] leaves between the backdrop and the heads. Dragging
//! is the interaction todo — the tap hit-rect is exposed via
//! [`HeroLayout`] for it.

use egui::Ui;
use resonance_plugin::Param;
use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;
use crate::quantize::{mode_from_index, quantize_transpose, root_from_index, PitchQuantize};
use crate::sync::DIVISION_LABELS;
use crate::viz::{GrainSnapshot, GranularViz, GRAIN_SLOTS, PEAK_BINS};

use super::controls::ROOT_LABELS;
use super::theme;

/// Right-hand pitch-ruler gutter width, px.
const RULER_W: f32 = 54.0;
/// Vertical inset of the head/tap lines from the band edges, px.
const LINE_INSET: f32 = 14.0;
/// Half-width of the delay-tap hit zone, px (the interaction todo's
/// grab region; the cursor affordance uses the same rect).
pub const TAP_HIT_HALF_W: f32 = 14.0;

/// Pitch-axis half-range, semitones (±24 st ruler).
pub const PITCH_RANGE_ST: f32 = 24.0;

/// Frame layout + axis mapping of the hero band, rebuilt every frame
/// (pure function of the band rect and the effective delay). Exposed
/// so the grain-cloud and direct-manipulation todos share the exact
/// same mapping and hit zones.
#[derive(Debug, Clone, Copy)]
pub struct HeroLayout {
    /// The whole hero band.
    pub canvas: egui::Rect,
    /// The plot area (canvas minus the pitch-ruler gutter); the write
    /// head sits at its right edge.
    pub plot: egui::Rect,
    /// Visible buffer window behind the write head, seconds.
    pub window_seconds: f32,
    /// Delay-tap x position, px.
    pub tap_x: f32,
    /// Delay-tap grab zone (full plot height, ±[`TAP_HIT_HALF_W`]).
    pub tap_hit: egui::Rect,
}

impl HeroLayout {
    /// Build the layout for a band rect and the current effective
    /// delay. The visible window scales with the delay (2× the tap,
    /// clamped to 1.5 – 4 s) so the tap always sits mid-view.
    pub fn new(canvas: egui::Rect, delay_ms: f32) -> Self {
        let plot = egui::Rect::from_min_max(
            canvas.min,
            egui::pos2(canvas.right() - RULER_W, canvas.bottom()),
        );
        let window_seconds = (delay_ms * 0.001 * 2.0).clamp(1.5, crate::dsp::MAX_DELAY_SECONDS);
        let mut layout = Self {
            canvas,
            plot,
            window_seconds,
            tap_x: 0.0,
            tap_hit: egui::Rect::NOTHING,
        };
        layout.tap_x = layout.x_of_ms(delay_ms);
        layout.tap_hit = egui::Rect::from_min_max(
            egui::pos2(layout.tap_x - TAP_HIT_HALF_W, plot.top()),
            egui::pos2(layout.tap_x + TAP_HIT_HALF_W, plot.bottom()),
        );
        layout
    }

    /// X coordinate of `ms` behind the write head (head at the right
    /// edge, older content to the left).
    pub fn x_of_ms(&self, ms: f32) -> f32 {
        self.plot.left() + self.plot.width() * (1.0 - ms * 0.001 / self.window_seconds)
    }

    /// Milliseconds behind the write head at `x`.
    pub fn ms_of_x(&self, x: f32) -> f32 {
        (1.0 - (x - self.plot.left()) / self.plot.width()) * self.window_seconds * 1000.0
    }

    /// Y coordinate of a pitch offset in semitones (0 st slightly above
    /// centre, +24 up, −24 down).
    pub fn y_of_st(&self, st: f32) -> f32 {
        self.mid_y() - st * (self.canvas.height() * 0.38) / PITCH_RANGE_ST
    }

    /// Pitch offset (semitones) at `y`.
    pub fn st_of_y(&self, y: f32) -> f32 {
        (self.mid_y() - y) * PITCH_RANGE_ST / (self.canvas.height() * 0.38)
    }

    /// The 0 st axis (backdrop midline).
    pub fn mid_y(&self) -> f32 {
        self.canvas.top() + self.canvas.height() * 0.52
    }
}

/// Draw the hero band into the available rect and return the frame's
/// layout (the interaction todo consumes `tap_hit`).
pub fn draw(ui: &mut Ui, params: &GranularDelayParams, viz: &GranularViz) -> HeroLayout {
    let canvas = ui.available_rect_before_wrap();
    let layout = HeroLayout::new(canvas, viz.read_delay_ms());
    let painter = ui.painter_at(canvas);
    let frozen = params.freeze.value();
    let now = ui.input(|i| i.time);

    painter.rect_filled(canvas, theme::RADIUS_PANEL, theme::BG_1);
    if frozen {
        // Amber wash over the held buffer (freeze state, req-5).
        painter.rect_filled(layout.plot, theme::RADIUS_PANEL, theme::WARM.gamma_multiply(0.035));
    }

    // Backdrop silhouette from the coarse buffer peaks (ba todo #1135).
    let mut peaks = [0.0f32; PEAK_BINS];
    let bin_ms = viz.read_peaks(&mut peaks);
    let peak_max = peaks.iter().fold(0.0f32, |m, &p| m.max(p));
    draw_backdrop(&painter, &layout, &peaks, bin_ms, frozen);

    if params.sync.value() {
        draw_division_ticks(&painter, &layout, params, viz.read_bpm());
    }
    draw_pitch_ruler(&painter, &layout);
    draw_scale_lanes(&painter, &layout, params);

    // Live grain cloud (ba todo #1143): decoded into a fixed
    // stack-local buffer — no per-frame heap allocation on this path.
    let mut grains = [GrainSnapshot::default(); GRAIN_SLOTS];
    let grain_count = viz.read_grains(&mut grains);
    draw_grain_cloud(
        &painter,
        &layout,
        &grains[..grain_count],
        params.texture.value(),
    );

    draw_write_head(&painter, &layout, frozen, now);
    draw_delay_tap(&painter, &layout, params, viz.read_delay_ms());

    // Empty state: no sounding grains and a silent buffer — flat line
    // only (already flat: silent peaks), plus the quiet hint. No fake
    // activity.
    let silent = viz.read_active_grains() == 0
        && viz.read_psola_voices() == 0
        && peak_max < 1.0e-4;
    if silent && !frozen {
        painter.text(
            egui::pos2(layout.plot.center().x, layout.mid_y() - 26.0),
            egui::Align2::CENTER_CENTER,
            "silence — the cloud appears when audio reaches the buffer",
            egui::FontId::proportional(11.0),
            theme::TEXT_4,
        );
    }

    // Gesture legend, bottom-left (req-2 affordance).
    painter.text(
        egui::pos2(layout.plot.left() + 12.0, layout.plot.bottom() - 18.0),
        egui::Align2::LEFT_CENTER,
        "drag tap ⇄ time · drag cloud ⇅ pitch · scroll = density",
        egui::FontId::proportional(9.0),
        theme::TEXT_4,
    );

    layout
}

/// Dim waveform/energy silhouette of the buffer contents: one vertical
/// span per ~3 px column, amplitude from the coarse peak bin at that
/// time offset. Silent buffers draw the flat midline.
fn draw_backdrop(
    painter: &egui::Painter,
    layout: &HeroLayout,
    peaks: &[f32; PEAK_BINS],
    bin_ms: f32,
    frozen: bool,
) {
    let color = if frozen {
        theme::WARM.gamma_multiply(0.16)
    } else {
        theme::TEXT_3.gamma_multiply(0.22)
    };
    let stroke = egui::Stroke::new(1.0, color);
    let mid = layout.mid_y();
    let amp_px = layout.canvas.height() * 0.09;

    // Flat midline under everything (also the whole empty state).
    painter.line_segment(
        [
            egui::pos2(layout.plot.left() + 8.0, mid),
            egui::pos2(layout.plot.right() - 2.0, mid),
        ],
        egui::Stroke::new(1.0, color.gamma_multiply(0.6)),
    );
    if bin_ms <= 0.0 {
        return;
    }

    let mut x = layout.plot.left() + 2.0;
    while x < layout.plot.right() - 2.0 {
        let ms = layout.ms_of_x(x);
        // Peaks are ordered oldest → newest with the head bin last.
        let back = (ms / bin_ms) as usize;
        if back < PEAK_BINS {
            let p = peaks[PEAK_BINS - 1 - back].clamp(0.0, 1.0);
            if p > 1.0e-4 {
                let a = (p.sqrt() * amp_px).max(0.5);
                painter.line_segment(
                    [egui::pos2(x, mid - a), egui::pos2(x, mid + a)],
                    stroke,
                );
            }
        }
        x += 3.0;
    }
}

/// Division tick marks across the buffer while tempo-synced (one per
/// division interval behind the write head), req-4.
fn draw_division_ticks(
    painter: &egui::Painter,
    layout: &HeroLayout,
    params: &GranularDelayParams,
    bpm: f32,
) {
    if bpm <= 0.0 {
        return;
    }
    let tempo = resonance_plugin::TempoInfo {
        bpm,
        time_sig_num: 4,
        time_sig_den: 4,
        playing: false,
        song_pos_beats: 0.0,
    };
    let div_seconds = crate::sync::delay_seconds(
        true,
        params.division.value() as usize,
        0.0,
        Some(tempo),
        crate::dsp::MAX_DELAY_SECONDS,
    );
    if div_seconds <= 0.0 {
        return;
    }
    let stroke = egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.10));
    let top = layout.plot.top() + 26.0;
    let bottom = layout.plot.bottom() - 26.0;
    let mut i = 1;
    while i as f32 * div_seconds < layout.window_seconds {
        let x = layout.x_of_ms(i as f32 * div_seconds * 1000.0);
        painter.line_segment([egui::pos2(x, top), egui::pos2(x, bottom)], stroke);
        if i < 9 {
            painter.text(
                egui::pos2(x + 3.0, top + 10.0),
                egui::Align2::LEFT_CENTER,
                format!("{i}×"),
                egui::FontId::monospace(9.0),
                theme::TEXT_4,
            );
        }
        i += 1;
    }
}

/// ±24 st pitch ruler in the right gutter + faint horizontal
/// gridlines at the octaves.
fn draw_pitch_ruler(painter: &egui::Painter, layout: &HeroLayout) {
    painter.line_segment(
        [
            egui::pos2(layout.plot.right() + 0.5, layout.canvas.top()),
            egui::pos2(layout.plot.right() + 0.5, layout.canvas.bottom()),
        ],
        egui::Stroke::new(1.0, theme::LINE_2),
    );
    for st in [-24i32, -12, 0, 12, 24] {
        let y = layout.y_of_st(st as f32);
        let (text_color, line_alpha) = if st == 0 {
            (theme::TEXT_3, 0.25)
        } else {
            (theme::TEXT_4, 0.10)
        };
        painter.text(
            egui::pos2(layout.plot.right() + 8.0, y),
            egui::Align2::LEFT_CENTER,
            format!("{}{st} st", if st > 0 { "+" } else { "" }),
            egui::FontId::monospace(9.0),
            text_color,
        );
        painter.line_segment(
            [
                egui::pos2(layout.plot.left(), y + 0.5),
                egui::pos2(layout.plot.right(), y + 0.5),
            ],
            egui::Stroke::new(1.0, theme::TEXT_3.gamma_multiply(line_alpha)),
        );
    }
}

/// The write head at the right plot edge: mint pulsing `WRITE` while
/// streaming; amber, pulse stopped, `HOLD` while frozen.
fn draw_write_head(painter: &egui::Painter, layout: &HeroLayout, frozen: bool, now: f64) {
    let x = layout.plot.right() - 2.0;
    let color = if frozen {
        theme::WARM.gamma_multiply(0.5)
    } else {
        let pulse = 0.55 + 0.35 * (now * 6.0).sin() as f32;
        theme::GOOD.gamma_multiply(pulse)
    };
    painter.line_segment(
        [
            egui::pos2(x, layout.plot.top() + LINE_INSET),
            egui::pos2(x, layout.plot.bottom() - LINE_INSET),
        ],
        egui::Stroke::new(2.0, color),
    );
    painter.text(
        egui::pos2(x - 6.0, layout.plot.top() + 20.0),
        egui::Align2::RIGHT_CENTER,
        if frozen { "HOLD" } else { "WRITE" },
        egui::FontId::proportional(9.0),
        if frozen { theme::WARM } else { theme::GOOD },
    );
}

/// The delay tap: accent vertical line with the ms/division flag and
/// grab dots (render only — dragging is the interaction todo).
fn draw_delay_tap(
    painter: &egui::Painter,
    layout: &HeroLayout,
    params: &GranularDelayParams,
    delay_ms: f32,
) {
    let x = layout.tap_x;
    painter.line_segment(
        [
            egui::pos2(x, layout.plot.top() + LINE_INSET),
            egui::pos2(x, layout.plot.bottom() - LINE_INSET),
        ],
        egui::Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.75)),
    );

    // Flag: division label while synced, ms otherwise.
    let label = if params.sync.value() {
        DIVISION_LABELS
            .get(params.division.value() as usize)
            .copied()
            .unwrap_or("?")
            .to_string()
    } else {
        format!("{delay_ms:.0} ms")
    };
    let font = egui::FontId::monospace(10.0);
    let galley = painter.layout_no_wrap(label, font, theme::ACCENT_SOFT);
    let w = galley.size().x + 14.0;
    let flag = egui::Rect::from_min_size(
        egui::pos2(x - w * 0.5, layout.plot.top() + 8.0),
        egui::vec2(w, 17.0),
    );
    painter.rect_filled(flag, theme::RADIUS_CHIP, theme::ACCENT.gamma_multiply(0.16));
    painter.rect_stroke(
        flag,
        theme::RADIUS_CHIP,
        egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.34)),
        egui::StrokeKind::Inside,
    );
    let text_pos = egui::pos2(flag.center().x - galley.size().x * 0.5, flag.center().y - galley.size().y * 0.5);
    painter.galley(text_pos, galley, theme::ACCENT_SOFT);

    // Grab dots at mid-height.
    let mid = layout.plot.center().y;
    for dy in [-5.0f32, 0.0, 5.0] {
        painter.rect_filled(
            egui::Rect::from_center_size(egui::pos2(x, mid + dy), egui::vec2(2.0, 2.0)),
            0.0,
            theme::ACCENT_SOFT.gamma_multiply(0.8),
        );
    }
}

/// Which hero gesture is in flight (ba todo #1144). Latched at drag
/// start so the tap grab wins over the cloud drag for the whole
/// gesture even when the pointer leaves the hit zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeroDrag {
    /// Horizontal delay-tap drag (time / division).
    Tap,
    /// Vertical cloud drag (pitch).
    Cloud,
}

/// Canvas interactions (ba todo #1144, design doc #264 req-2): tap
/// drag ⇄ delay time (snapping to the division grid while synced),
/// cloud drag ⇅ Pitch, scroll = Density, double-click on the tap
/// resets the active time param to its default. All writes go through
/// `Param::set_plain` — the same GUI→host path as the strip knobs —
/// so host automation records the gestures. Interaction is scoped to
/// the plot rect, so the header/strip widgets are untouched.
pub fn interact(
    ui: &mut Ui,
    params: &GranularDelayParams,
    viz: &GranularViz,
    layout: &HeroLayout,
    drag: &mut Option<HeroDrag>,
) {
    let response = ui.interact(
        layout.plot,
        ui.id().with("hero_canvas"),
        egui::Sense::click_and_drag(),
    );
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let over_tap = pointer.is_some_and(|p| layout.tap_hit.contains(p));

    // Latch the gesture at drag start: the tap hit-zone wins over the
    // cloud drag when they overlap.
    if response.drag_started() {
        *drag = Some(if over_tap { HeroDrag::Tap } else { HeroDrag::Cloud });
    }
    if response.drag_stopped() {
        *drag = None;
    }

    // Cursor affordances: horizontal-resize near/while dragging the
    // tap, grab over the cloud body, grabbing while dragging it.
    let cursor = match (*drag, over_tap, response.hovered()) {
        (Some(HeroDrag::Tap), _, _) => Some(egui::CursorIcon::ResizeHorizontal),
        (Some(HeroDrag::Cloud), _, _) => Some(egui::CursorIcon::Grabbing),
        (None, true, _) => Some(egui::CursorIcon::ResizeHorizontal),
        (None, false, true) => Some(egui::CursorIcon::Grab),
        _ => None,
    };
    if let Some(cursor) = cursor {
        ui.ctx().set_cursor_icon(cursor);
    }

    // Double-click on the tap: reset the active time param (free-run
    // time in ms, or the division while synced) to its default.
    if response.double_clicked() && over_tap {
        let p: &dyn resonance_plugin::Param = if params.sync.value() {
            &params.division
        } else {
            &params.time_ms
        };
        p.set_plain(p.default_plain());
        return;
    }

    match (*drag, pointer) {
        (Some(HeroDrag::Tap), Some(pos)) => {
            let target_ms = layout.ms_of_x(pos.x);
            let bpm = viz.read_bpm();
            if params.sync.value() && bpm > 0.0 {
                // Snap to the division grid: step the division param
                // to the division nearest the pointer time (the same
                // grid the tick marks draw), and let the flag readout
                // follow live.
                let tempo = resonance_plugin::TempoInfo {
                    bpm,
                    time_sig_num: 4,
                    time_sig_den: 4,
                    playing: false,
                    song_pos_beats: 0.0,
                };
                let mut best = params.division.value() as usize;
                let mut best_err = f32::INFINITY;
                for div in 0..DIVISION_LABELS.len() {
                    let ms = crate::sync::delay_seconds(
                        true,
                        div,
                        0.0,
                        Some(tempo),
                        crate::dsp::MAX_DELAY_SECONDS,
                    ) * 1000.0;
                    let err = (ms - target_ms).abs();
                    if err < best_err {
                        best_err = err;
                        best = div;
                    }
                }
                if best as i32 != params.division.value() {
                    params.division.set_plain(best as f64);
                }
            } else {
                // Free-run: x maps to delay ms within the param range.
                let p = &params.time_ms;
                let ms = f64::from(target_ms).clamp(p.min_plain(), p.max_plain());
                p.set_plain(ms);
            }
        }
        (Some(HeroDrag::Cloud), Some(pos)) => {
            // Vertical drag maps the pointer's ruler position to the
            // bipolar Pitch param (±24 st); with Quantize == SCALE the
            // DSP's spawn quantization makes grains land on the lanes.
            let p = &params.pitch;
            let st = f64::from(layout.st_of_y(pos.y)).clamp(p.min_plain(), p.max_plain());
            p.set_plain(st);
        }
        _ => {}
    }

    // Scroll over the canvas: fine multiplicative Density steps.
    // `density_sync` (PER-BEAT) has no separate synced-density param
    // in params.rs (its DSP is a declared TODO), so the free-run
    // density param is the single scroll target in both modes.
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let p = &params.density_hz;
            let factor = f64::from((-scroll * 0.0015).exp());
            let next = (p.get_plain() * factor).clamp(p.min_plain(), p.max_plain());
            p.set_plain(next);
        }
    }
}

/// Scale lanes (ba todo #1143, req-1; prototype 'scale' state): with
/// Quantize == SCALE, horizontal accent lanes at every allowed scale
/// degree within the ±24 st ruler, computed through the exact
/// quantizer the DSP snaps grains with (`crate::quantize`) — capsules
/// land on the lanes because both sides share the lattice. Small
/// 'D MINOR LANES' caption bottom-left.
fn draw_scale_lanes(painter: &egui::Painter, layout: &HeroLayout, params: &GranularDelayParams) {
    if params.pitch_quantize.value() != 2 {
        return;
    }
    let root = params.root.value();
    let scale = resonance_music_theory::Scale::new(
        root_from_index(root),
        mode_from_index(params.scale.value()),
    );
    let stroke = egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.13));
    let range = PITCH_RANGE_ST as i32;
    for st in -range..=range {
        // A lane exists where the quantizer is a fixed point.
        let snapped = quantize_transpose(st as f32, PitchQuantize::Scale, scale);
        if (snapped - st as f32).abs() < 0.01 {
            let y = layout.y_of_st(st as f32);
            painter.line_segment(
                [
                    egui::pos2(layout.plot.left(), y + 0.5),
                    egui::pos2(layout.plot.right(), y + 0.5),
                ],
                stroke,
            );
        }
    }
    let root_label = ROOT_LABELS
        .get(root.max(0) as usize % 12)
        .copied()
        .unwrap_or("?");
    painter.text(
        egui::pos2(layout.plot.left() + 12.0, layout.plot.bottom() - 34.0),
        egui::Align2::LEFT_CENTER,
        format!(
            "{root_label} {} LANES",
            scale.mode.as_str().to_uppercase()
        ),
        egui::FontId::proportional(9.0),
        theme::ACCENT_SOFT.gamma_multiply(0.55),
    );
}

/// The live grain cloud (ba todo #1143, req-1): soft capsules at
/// x = buffer position / y = pitch, width = grain size, alpha = the
/// snapshot level (the DSP's Texture-shaped envelope value at this
/// instant). Reversed grains carry a warm left-edge taper; feedback
/// ghosts (generation ≥ 1) draw in the plain accent token, further
/// dimmed per generation on top of the loop-gain scaling the
/// publisher applied (and already positioned one delay further back
/// per generation, pitch including recirculation transposition);
/// PSOLA voices draw as tight mint period-slivers. Pure iteration
/// over the caller's stack buffer — no heap allocation.
fn draw_grain_cloud(
    painter: &egui::Painter,
    layout: &HeroLayout,
    grains: &[GrainSnapshot],
    texture: f32,
) {
    let plot = layout.plot;
    for g in grains {
        // Ghosts: flat extra dim on top of the publisher's fb^gen level
        // scaling, deepening slightly per generation.
        let ghost_dim = match g.generation {
            0 => 0.95,
            1 => 0.55,
            2 => 0.45,
            _ => 0.35,
        };
        let alpha = (g.level * ghost_dim).clamp(0.0, 1.0);
        if alpha < 0.01 {
            continue;
        }
        let x = layout.x_of_ms(g.position_ms);
        let w = ((g.size_ms * 0.001 / layout.window_seconds) * plot.width()).max(10.0);
        if x + w * 0.5 < plot.left() || x - w * 0.5 > plot.right() {
            continue; // fully outside the visible window
        }
        let y = layout.y_of_st(g.pitch_semitones.clamp(-26.0, 26.0));
        let color = if g.voiced {
            theme::GOOD
        } else if g.generation > 0 {
            theme::ACCENT
        } else {
            theme::ACCENT_SOFT
        };
        // Voiced period-slivers are tight; cloud capsules thicken with
        // the Texture window shape.
        let h = if g.voiced { 5.0 } else { 8.0 + texture * 5.0 };
        let rect = egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(w, h))
            .intersect(plot);
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            continue;
        }
        painter.rect_filled(rect, h * 0.5, color.gamma_multiply(alpha));

        // Reversed grains: warm marker on the leading (left) edge —
        // the grain plays toward older material. Drawn as two stacked
        // strips (wide + narrow) approximating the prototype's taper
        // without building a per-grain polygon (no heap allocation).
        if g.reversed {
            let lx = x - w * 0.5;
            if lx >= plot.left() - 1.0 && lx <= plot.right() {
                let warm = theme::WARM.gamma_multiply(alpha);
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(lx - 1.0, y - h * 0.5),
                        egui::vec2(3.0, h),
                    )
                    .intersect(plot),
                    1.0,
                    warm,
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(lx + 2.0, y - h * 0.25),
                        egui::vec2(2.5, h * 0.5),
                    )
                    .intersect(plot),
                    1.0,
                    warm,
                );
            }
        }
    }
}
