//! Rendering pipeline for the piano-roll canvas. Separated from input
//! handling (see `input.rs`) because visual layout and edit UX change for
//! different reasons.

use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use crate::view::piano_roll::{self, NoteStyle};
use crate::theme;

use resonance_audio::quantize::BarRuler;
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};

use super::{PianoRollCanvas, PianoRollState};
use super::input::quantize_grid_steps;

/// Entry point called by `canvas::Program::draw`. Manages the draw cache,
/// draws the static note layer, then overlays the live rubber-band marquee
/// (if active) as a separate uncached frame.
pub(super) fn draw_canvas(
    canvas: &PianoRollCanvas<'_>,
    state: &PianoRollState,
    renderer: &Renderer,
    bounds: Rectangle,
    _theme: &Theme,
) -> Vec<canvas::Geometry> {
    let fp = canvas.fingerprint(state);
    if state.cache_fingerprint.get() != fp {
        state.cache.clear();
        state.cache_fingerprint.set(fp);
    }
    let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
        draw_into(canvas, frame, bounds);
    });
    // The rubber-band marquee is drawn live, outside the cached note
    // layer, so dragging it doesn't invalidate ~100 cached note rects
    // every frame; it repaints with the periodic UI tick.
    if let Some(ed) = state.empty_drag {
        if ed.is_marquee() {
            let rect = ed.rect();
            let mut overlay = canvas::Frame::new(renderer, bounds.size());
            overlay.fill_rectangle(
                Point::new(rect.x, rect.y),
                Size::new(rect.width, rect.height),
                Color {
                    a: 0.15,
                    ..theme::ACCENT
                },
            );
            overlay.stroke(
                &canvas::Path::rectangle(
                    Point::new(rect.x, rect.y),
                    Size::new(rect.width, rect.height),
                ),
                canvas::Stroke::default()
                    .with_color(theme::ACCENT)
                    .with_width(1.0),
            );
            return vec![geometry, overlay.into_geometry()];
        }
    }
    vec![geometry]
}

/// Full scene draw into `frame` — called once per cache miss.
fn draw_into(canvas: &PianoRollCanvas<'_>, frame: &mut canvas::Frame, bounds: Rectangle) {
    let layout = canvas.layout(bounds);
    let viewport = canvas.viewport();
    let grid_x = layout.grid_x();
    let grid_w = bounds.width - grid_x;
    let grid_h = layout.grid_h;

    // --- Background ---
    frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

    // --- Note row backgrounds ---
    draw_note_rows(canvas, frame, &viewport, grid_x, grid_w, grid_h);

    // --- Grid lines ---
    draw_grid_lines(canvas, frame, &viewport, grid_x, grid_w, grid_h);

    // --- Active quantize grid (triplet / dotted / swing) ---
    draw_quantize_grid(canvas, frame, &viewport, grid_x, grid_w, grid_h);

    // --- Notes ---
    draw_notes(canvas, frame, &layout, &viewport);

    // --- Ghost-target preview for the current selection ---
    draw_ghost_notes(canvas, frame, &layout, &viewport);

    // --- Piano keyboard ---
    piano_roll::draw_keyboard(frame, &layout, &viewport);

    // --- Velocity lane ---
    draw_velocity_lane(canvas, frame, &viewport, grid_x, grid_w, grid_h, bounds.height);

    // --- Separator lines ---
    // Vertical separator between keyboard and grid
    frame.fill_rectangle(
        Point::new(grid_x, 0.0),
        Size::new(1.0, grid_h),
        theme::SEPARATOR,
    );
    // Horizontal separator between grid and velocity lane
    frame.fill_rectangle(
        Point::new(0.0, grid_h),
        Size::new(bounds.width, 1.0),
        theme::SEPARATOR,
    );
}

/// Draw alternating row backgrounds for each semitone.
fn draw_note_rows(
    _canvas: &PianoRollCanvas<'_>,
    frame: &mut canvas::Frame,
    viewport: &crate::view::piano_roll::PianoRollViewport,
    grid_x: f32,
    grid_w: f32,
    grid_h: f32,
) {
    use crate::view::piano_roll::{is_black_key, NOTE_COUNT};
    // Backdrop is BG_2; only black-key rows darken to BG_1. White
    // keys reuse the backdrop so the row striping reads softly.
    frame.fill_rectangle(
        Point::new(grid_x, 0.0),
        Size::new(grid_w, grid_h),
        theme::BG_2,
    );
    for midi_note in 0..NOTE_COUNT {
        let y = viewport.note_to_y_local(midi_note);
        let h = viewport.zoom_y;

        if y + h < 0.0 || y > grid_h {
            continue;
        }

        if is_black_key(midi_note) {
            frame.fill_rectangle(Point::new(grid_x, y), Size::new(grid_w, h), theme::BG_1);
        }

        if midi_note % 12 == 0 {
            frame.fill_rectangle(
                Point::new(grid_x, y + h - 1.0),
                Size::new(grid_w, 1.0),
                theme::LINE_2,
            );
        }
    }
}

/// Draw vertical grid lines at beat and bar boundaries.
fn draw_grid_lines(
    canvas: &PianoRollCanvas<'_>,
    frame: &mut canvas::Frame,
    viewport: &crate::view::piano_roll::PianoRollViewport,
    grid_x: f32,
    grid_w: f32,
    grid_h: f32,
) {
    let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
    let ticks_per_bar = TICKS_PER_QUARTER_NOTE * canvas.time_sig_num as u64;
    let pixels_per_beat = ticks_per_beat as f32 * viewport.zoom_x;

    // Determine visible tick range
    let start_tick = (viewport.scroll_x / viewport.zoom_x).max(0.0) as u64;
    let end_tick = ((viewport.scroll_x + grid_w) / viewport.zoom_x) as u64 + ticks_per_beat;

    // Draw beat lines
    if pixels_per_beat >= 8.0 {
        let first_beat = start_tick / ticks_per_beat;
        let last_beat = end_tick / ticks_per_beat + 1;

        for beat_idx in first_beat..=last_beat {
            let tick = beat_idx * ticks_per_beat;
            let x = grid_x + viewport.tick_to_x_local(tick);

            if x < grid_x || x > grid_x + grid_w {
                continue;
            }

            let is_bar = tick.is_multiple_of(ticks_per_bar);
            let color = if is_bar {
                theme::BAR_LINE
            } else {
                theme::BEAT_LINE
            };

            frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, grid_h), color);
        }
    }

    // Draw subdivision lines (16th notes) if zoomed in enough
    let snap_px = canvas.snap_ticks as f32 * viewport.zoom_x;
    if snap_px >= 8.0 && canvas.snap_ticks < ticks_per_beat {
        let first = start_tick / canvas.snap_ticks;
        let last = end_tick / canvas.snap_ticks + 1;
        for idx in first..=last {
            let tick = idx * canvas.snap_ticks;
            if tick.is_multiple_of(ticks_per_beat) {
                continue; // already drawn as beat/bar line
            }
            let x = grid_x + viewport.tick_to_x_local(tick);
            if x < grid_x || x > grid_x + grid_w {
                continue;
            }
            frame.fill_rectangle(
                Point::new(x, 0.0),
                Size::new(1.0, grid_h),
                Color {
                    a: 0.5,
                    ..theme::LINE_2
                },
            );
        }
    }
}

/// Draw the active quantize grid: vertical lines at every step of the
/// selected division, swung on odd steps, anchored to bars via the
/// project tempo map so triplet / dotted / swing feels read correctly.
///
/// These sit on top of the editor's plain beat / bar / snap lines in a
/// faint accent tint so the user can see exactly where Apply will pull
/// notes — and they shift live as the grid / swing change because the
/// whole geometry is keyed on `quantize_hash` in the draw cache.
fn draw_quantize_grid(
    canvas: &PianoRollCanvas<'_>,
    frame: &mut canvas::Frame,
    viewport: &crate::view::piano_roll::PianoRollViewport,
    grid_x: f32,
    grid_w: f32,
    grid_h: f32,
) {
    let g = canvas.quantize.division.ticks();
    let step_px = g as f32 * viewport.zoom_x;
    // Too dense to read (or zero-width) — skip rather than smear the grid.
    if step_px < 5.0 {
        return;
    }

    let swung = canvas.quantize.swing > f32::EPSILON;

    // Visible absolute-tick span (clip-relative ticks == absolute here:
    // the grid and the ghost both anchor at clip tick 0, matching how
    // the editor already draws its bar / beat lines).
    let start_tick = (viewport.scroll_x / viewport.zoom_x).max(0.0) as u64;
    let end_tick = ((viewport.scroll_x + grid_w) / viewport.zoom_x) as u64 + g;

    let ruler = BarRuler::new(canvas.tempo_map);
    let (mut bar_start, mut bar_len) = ruler.bar_at(start_tick);

    // Walk bar by bar so a mid-project signature change re-anchors the
    // grid; cap the walk so a degenerate tempo map can't spin forever.
    let mut guard = 0;
    while bar_start < end_tick && guard < 4096 {
        guard += 1;
        for (k, local) in quantize_grid_steps(g, bar_len, canvas.quantize.swing)
            .into_iter()
            .enumerate()
        {
            // Downbeats are already a bold bar line; don't double-draw.
            if local == 0 {
                continue;
            }
            let x = grid_x + viewport.tick_to_x_local(bar_start + local);
            if x < grid_x || x > grid_x + grid_w {
                continue;
            }
            // Swung off-beats (odd steps) read a touch brighter so the
            // swing offset is visible against the straight steps.
            let alpha = if swung && k % 2 == 1 { 0.42 } else { 0.26 };
            frame.fill_rectangle(
                Point::new(x, 0.0),
                Size::new(1.0, grid_h),
                Color {
                    a: alpha,
                    ..theme::ACCENT
                },
            );
        }
        bar_start += bar_len;
        bar_len = ruler.bar_at(bar_start).1;
    }
}

/// Notes Apply would land on, for the current selection and quantize
/// settings. Returns `None` when nothing is selected (the grid alone
/// previews the target then) — the ghost is scoped to the selection so
/// it stays readable, matching the panel's "selected notes" wording.
fn ghost_notes(canvas: &PianoRollCanvas<'_>) -> Option<Vec<MidiNote>> {
    if canvas.selected_notes.is_empty() || canvas.quantize.strength <= f32::EPSILON {
        return None;
    }
    let selection: Vec<usize> = canvas.selected_notes.iter().copied().collect();
    Some(super::input::ghost_targets(
        &canvas.clip.notes,
        &selection,
        &canvas.quantize,
        canvas.tempo_map,
    ))
}

/// Draw the non-destructive ghost preview: a dashed, translucent warm
/// rectangle at each selected note's quantized target, with a faint
/// connector from its current position so the move reads at a glance.
/// Targets that don't actually move (already on grid) are skipped.
fn draw_ghost_notes(
    canvas: &PianoRollCanvas<'_>,
    frame: &mut canvas::Frame,
    layout: &crate::view::piano_roll::PianoRollLayout,
    viewport: &crate::view::piano_roll::PianoRollViewport,
) {
    let Some(ghosts) = ghost_notes(canvas) else {
        return;
    };
    const DASH: canvas::LineDash<'static> = canvas::LineDash {
        segments: &[3.0, 2.0],
        offset: 0,
    };
    for &i in canvas.selected_notes {
        let Some(orig) = canvas.clip.notes.get(i) else {
            continue;
        };
        let Some(ghost) = ghosts.get(i) else {
            continue;
        };
        // No movement → nothing to preview.
        if ghost.start_tick == orig.start_tick && ghost.duration_ticks == orig.duration_ticks {
            continue;
        }
        let rect = canvas.note_rect(layout, viewport, ghost);
        // Cull ghosts fully off the grid area.
        if rect.x + rect.width < layout.grid_x()
            || rect.x > layout.grid_x() + 2000.0
            || rect.y + rect.height < 0.0
            || rect.y > layout.grid_h
        {
            continue;
        }

        let orig_rect = canvas.note_rect(layout, viewport, orig);
        // Connector from the current note to its target (mid-height).
        let mid_y = orig_rect.y + orig_rect.height * 0.5;
        frame.stroke(
            &canvas::Path::line(
                Point::new(orig_rect.x, mid_y),
                Point::new(rect.x, rect.y + rect.height * 0.5),
            ),
            canvas::Stroke {
                line_dash: DASH,
                ..canvas::Stroke::default()
                    .with_color(Color {
                        a: 0.5,
                        ..theme::WARM
                    })
                    .with_width(1.0)
            },
        );

        let path = canvas::Path::rounded_rectangle(
            Point::new(rect.x, rect.y),
            Size::new(rect.width.max(1.0), rect.height),
            2.0.into(),
        );
        frame.fill(
            &path,
            Color {
                a: 0.16,
                ..theme::WARM
            },
        );
        frame.stroke(
            &path,
            canvas::Stroke {
                line_dash: DASH,
                ..canvas::Stroke::default()
                    .with_color(theme::WARM)
                    .with_width(1.2)
            },
        );
    }
}

/// Draw MIDI note rectangles on the grid.
fn draw_notes(
    canvas: &PianoRollCanvas<'_>,
    frame: &mut canvas::Frame,
    layout: &crate::view::piano_roll::PianoRollLayout,
    viewport: &crate::view::piano_roll::PianoRollViewport,
) {
    let grid_x = layout.grid_x();
    for (i, n) in canvas.clip.notes.iter().enumerate() {
        let rect = canvas.note_rect(layout, viewport, n);

        if rect.x + rect.width < grid_x
            || rect.x > grid_x + 2000.0
            || rect.y + rect.height < 0.0
            || rect.y > layout.grid_h
        {
            continue;
        }

        let style = if canvas.selected_notes.contains(&i) {
            NoteStyle::selected()
        } else {
            NoteStyle::plain()
        };
        piano_roll::draw_note(frame, rect, n.velocity, style);
    }
}

/// Draw the velocity lane at the bottom.
fn draw_velocity_lane(
    canvas: &PianoRollCanvas<'_>,
    frame: &mut canvas::Frame,
    viewport: &crate::view::piano_roll::PianoRollViewport,
    grid_x: f32,
    grid_w: f32,
    grid_h: f32,
    total_h: f32,
) {
    let lane_y = grid_h + 1.0;
    let lane_h = total_h - grid_h - 1.0;

    // Lane background
    frame.fill_rectangle(
        Point::new(0.0, lane_y),
        Size::new(grid_x + grid_w, lane_h),
        theme::PANEL_DARK,
    );

    // "Vel" label
    frame.fill_text(canvas::Text {
        content: "Vel".to_string(),
        position: Point::new(4.0, lane_y + 2.0),
        color: theme::TEXT_DIM,
        size: 9.0.into(),
        ..canvas::Text::default()
    });

    // Velocity bars for each note
    for (i, n) in canvas.clip.notes.iter().enumerate() {
        let x = grid_x + viewport.tick_to_x_local(n.start_tick);
        let w = viewport.duration_to_w(n.duration_ticks).clamp(2.0, 6.0);

        if x + w < grid_x || x > grid_x + 2000.0 {
            continue;
        }

        let bar_h = n.velocity.clamp(0.0, 1.0) * (lane_h - 4.0);
        let bar_y = lane_y + lane_h - bar_h - 2.0;

        let is_selected = canvas.selected_notes.contains(&i);
        let color = if is_selected {
            theme::ACCENT
        } else {
            theme::ACCENT_SOFT
        };

        frame.fill_rectangle(Point::new(x, bar_y), Size::new(w, bar_h), color);
    }
}
