//! Frequency-response rendering for the EQ editor.
//!
//! Computes the composite magnitude curve at ~256 log-spaced frequencies
//! from 20 Hz to 20 kHz by summing (in dB) each enabled band's biquad
//! magnitude. Also renders a faint per-band contribution curve for the
//! selected/hovered band so the user can see what they're adjusting.
//!
//! The actual node interaction (hit test, drag, scroll, right-click) lives
//! in `nodes.rs` — this file only draws.

use resonance_dsp::Biquad;
use wayland_plugin_gui::egui;

use crate::analyzer::SpectrumSnapshot;
use crate::band::{BandKind, MAX_STAGES_PER_BAND};
use crate::editor::{nodes, theme, AnalyzerMode, EqEditorApp};
use crate::params::{BandSnapshot, NUM_BANDS};

const NUM_POINTS: usize = 256;
const MIN_FREQ: f32 = 20.0;
const MAX_FREQ: f32 = 20_000.0;
/// Nominal sample rate used only for the on-screen response visualization.
/// The actual audio runs at whatever sample rate the host requested; the
/// curve shape is nearly identical at any sensible SR.
const VIS_SR: f32 = 48_000.0;
const DB_MIN: f32 = -24.0;
const DB_MAX: f32 = 24.0;

pub fn draw(ui: &mut egui::Ui, rect: egui::Rect, app: &mut EqEditorApp) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let pad = 12.0f32;
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top() + pad),
        egui::pos2(rect.right() - pad, rect.bottom() - pad),
    );

    // Spectrum first, then the grid on top of it, then the EQ curve on
    // top of the grid. Drawing the grid on top of the spectrum keeps the
    // gridlines crisp instead of bleeding through the translucent fill
    // as horizontal stripes — the fill reads as a single surface.
    if let Some(snap) = take_analyzer_snapshot(app) {
        draw_spectrum(&painter, plot, &snap);
    }

    draw_grid(&painter, plot);

    // Snapshot every band once per frame so the curve, node list, and hit
    // testing all see the same state.
    let snapshots: [BandSnapshot; NUM_BANDS] = std::array::from_fn(|i| app.params.bands[i].snapshot());

    draw_composite_curve(&painter, plot, &snapshots);
    nodes::draw_and_interact(ui, plot, app, &snapshots);
}

fn take_analyzer_snapshot(app: &EqEditorApp) -> Option<SpectrumSnapshot> {
    match app.analyzer_mode {
        AnalyzerMode::Off => None,
        AnalyzerMode::Pre => Some(app.analyzer.pre.lock().clone()),
        AnalyzerMode::Post => Some(app.analyzer.post.lock().clone()),
    }
}

/// Raw signal dB range displayed by the analyzer, *after* the pink-tilt
/// compensation below is applied. 0 dBFS stays pinned at the top of the
/// spectrum region; anything quieter than ANALYZER_BOT_DB sits on the
/// floor. −80 dB gives enough headroom for typical music (individual
/// bins of a mix sit around −40 to −60 dBFS after tilting).
const ANALYZER_TOP_DB: f32 = 0.0;
const ANALYZER_BOT_DB: f32 = -80.0;

/// Pink-tilt slope applied to the displayed spectrum. Each octave above
/// PINK_REF_HZ gains this many dB, which compensates for the natural
/// 1/f fall-off of typical music / pink noise so that pink noise renders
/// as a roughly flat line. Pro-Q 3 uses ~4.5 dB/oct by default.
const PINK_TILT_DB_PER_OCT: f32 = 4.5;
const PINK_REF_HZ: f32 = 1_000.0;

/// The spectrum is constrained to the lower portion of the plot so it
/// visually lives beneath the EQ response curve. The top edge of the
/// spectrum region lines up with this EQ-scale dB value (a little below
/// the 0 dB gridline so the two curves don't collide).
const SPECTRUM_TOP_EQ_DB: f32 = -2.0;

fn draw_spectrum(painter: &egui::Painter, plot: egui::Rect, snap: &SpectrumSnapshot) {
    if snap.magnitudes_db.is_empty() || snap.sample_rate <= 0.0 {
        return;
    }
    let bins = &snap.magnitudes_db;
    let bin_count = bins.len();
    let sr = snap.sample_rate;
    let fft_size = bin_count * 2;

    // Pre-compute the y bounds of the spectrum region once so every
    // vertex can interpolate into the same band.
    let top_y = plot.top() + db_to_y(SPECTRUM_TOP_EQ_DB, plot.height());
    let bot_y = plot.bottom();

    // Build the top edge of the spectrum at NUM_POINTS log-spaced
    // frequencies. For each display point we look at the band of FFT
    // bins that fall inside that point's frequency slice and take the
    // loudest one, which preserves high-frequency peaks that a single
    // point-sample would miss while still smoothing away the per-bin
    // jitter from the windowed FFT. The pink-tilt correction is added
    // afterwards so that typical music reads as a roughly flat shape
    // instead of a massive bass bulge.
    let mut top: Vec<egui::Pos2> = Vec::with_capacity(NUM_POINTS);
    for i in 0..NUM_POINTS {
        let t = i as f32 / (NUM_POINTS - 1) as f32;
        // Frequency band covered by this display point: half a step to
        // the left and half a step to the right, in log space.
        let half_step = 0.5 / (NUM_POINTS - 1) as f32;
        let t_lo = (t - half_step).max(0.0);
        let t_hi = (t + half_step).min(1.0);
        let f_lo = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t_lo);
        let f_hi = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t_hi);
        let freq = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t);
        let raw_db = max_band_db(bins, bin_count, sr, fft_size, f_lo, f_hi);
        let tilt = PINK_TILT_DB_PER_OCT * (freq / PINK_REF_HZ).log2();
        let mag_db = raw_db + tilt;
        let x = plot.left() + freq_to_x(freq, plot.width());
        let y = spectrum_y(mag_db, top_y, bot_y);
        top.push(egui::pos2(x, y));
    }

    // Build the fill as an explicit strip of quads instead of feeding a
    // closed path to egui's tessellator. Between every pair of adjacent
    // top vertices we emit a trapezoid — (x_i, bot), (x_i, y_i),
    // (x_{i+1}, y_{i+1}), (x_{i+1}, bot) — as two triangles in a shared
    // mesh. This avoids the fan-triangulation artifact that `Shape::Path`
    // produced on the non-convex top edge (visible as diagonal lines
    // radiating out of one corner of the plot). The result is a clean
    // scan-line-style fill whatever shape the top edge takes.
    let fill = spectrum_fill();
    let mut mesh = egui::epaint::Mesh::default();
    for pair in top.windows(2) {
        let p0 = pair[0];
        let p1 = pair[1];
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(p0.x, bot_y), fill);
        mesh.colored_vertex(p0, fill);
        mesh.colored_vertex(p1, fill);
        mesh.colored_vertex(egui::pos2(p1.x, bot_y), fill);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(egui::Shape::mesh(mesh));

    // Top edge stroke — slightly brighter accent so the spectrum envelope
    // reads as a crisp line on top of the translucent fill.
    painter.add(egui::Shape::line(top, egui::Stroke::new(1.0, spectrum_stroke())));
}

/// Map an already-tilted dB value into an absolute y coordinate within
/// the spectrum region.
fn spectrum_y(db: f32, top_y: f32, bot_y: f32) -> f32 {
    let t = ((db - ANALYZER_BOT_DB) / (ANALYZER_TOP_DB - ANALYZER_BOT_DB)).clamp(0.0, 1.0);
    bot_y + t * (top_y - bot_y)
}

/// Return the maximum dB value across every FFT bin whose center
/// frequency lies within `[f_lo, f_hi]`. Falls back to linear
/// interpolation when the band is narrower than a single bin (low
/// frequencies on the log axis) so the transitions between bins look
/// smooth instead of stepping.
fn max_band_db(
    bins: &[f32],
    bin_count: usize,
    sr: f32,
    fft_size: usize,
    f_lo: f32,
    f_hi: f32,
) -> f32 {
    let bin_width_hz = sr / fft_size as f32;
    let mut idx_lo = (f_lo / bin_width_hz).floor() as isize;
    let mut idx_hi = (f_hi / bin_width_hz).ceil() as isize;
    idx_lo = idx_lo.clamp(0, bin_count as isize - 1);
    idx_hi = idx_hi.clamp(0, bin_count as isize - 1);

    // Band narrower than one bin: interpolate linearly at the band's
    // center so adjacent display points don't snap to the same bin.
    if idx_hi - idx_lo <= 1 {
        let f_mid = 0.5 * (f_lo + f_hi);
        let bin_f = f_mid / bin_width_hz;
        let lo = bin_f.floor() as usize;
        let hi = (lo + 1).min(bin_count - 1);
        let frac = (bin_f - lo as f32).clamp(0.0, 1.0);
        return bins[lo] * (1.0 - frac) + bins[hi] * frac;
    }

    // Band wider than one bin: take the peak so loud bins dominate the
    // display rather than getting averaged into invisibility.
    let mut peak = f32::NEG_INFINITY;
    for i in idx_lo..=idx_hi {
        let v = bins[i as usize];
        if v > peak {
            peak = v;
        }
    }
    peak
}

fn spectrum_fill() -> egui::Color32 {
    // Slightly higher alpha now that the grid is drawn on top of the
    // spectrum instead of bleeding through it.
    egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0x3c)
}

fn spectrum_stroke() -> egui::Color32 {
    // Near-opaque white gives a crisp envelope line above the translucent
    // cyan fill — matches the Pro-Q 3 look where the spectrum body is
    // colored but its top edge reads as a sharp bright trace.
    egui::Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 0xe0)
}

fn draw_grid(painter: &egui::Painter, plot: egui::Rect) {
    // Vertical gridlines at decades + half-decades.
    for freq in [20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0] {
        let x = plot.left() + freq_to_x(freq, plot.width());
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        painter.text(
            egui::pos2(x, plot.bottom() - 2.0),
            egui::Align2::CENTER_BOTTOM,
            label_freq(freq),
            egui::FontId::proportional(9.0),
            theme::TEXT_DIM,
        );
    }

    // Horizontal gridlines every 6 dB.
    for db in [-24.0, -18.0, -12.0, -6.0, 0.0, 6.0, 12.0, 18.0, 24.0] {
        let y = plot.top() + db_to_y(db, plot.height());
        let stroke = if db == 0.0 {
            egui::Stroke::new(0.8, theme::BORDER)
        } else {
            egui::Stroke::new(0.3, theme::BORDER)
        };
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            stroke,
        );
        if db != 0.0 {
            painter.text(
                egui::pos2(plot.left() + 2.0, y),
                egui::Align2::LEFT_CENTER,
                format!("{:+} dB", db as i32),
                egui::FontId::proportional(9.0),
                theme::TEXT_DIM,
            );
        }
    }
}

fn draw_composite_curve(
    painter: &egui::Painter,
    plot: egui::Rect,
    snapshots: &[BandSnapshot; NUM_BANDS],
) {
    // Build per-band coefficient caches once and reuse for every frequency
    // sample to keep the render cheap.
    let band_coeffs: [(usize, [Biquad; MAX_STAGES_PER_BAND]); NUM_BANDS] = std::array::from_fn(|i| {
        let mut stages = [Biquad::identity(); MAX_STAGES_PER_BAND];
        let n = crate::band::configure_stages(&snapshots[i], VIS_SR, &mut stages);
        (n, stages)
    });

    let mut points: Vec<egui::Pos2> = Vec::with_capacity(NUM_POINTS);
    for i in 0..NUM_POINTS {
        let t = i as f32 / (NUM_POINTS - 1) as f32;
        let freq = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t);
        let mag_db = composite_magnitude_db(freq, &band_coeffs);
        let x = plot.left() + freq_to_x(freq, plot.width());
        let y = plot.top() + db_to_y(mag_db, plot.height());
        points.push(egui::pos2(x, y));
    }

    // Glow underlay + solid line on top.
    painter.add(egui::Shape::line(
        points.clone(),
        egui::Stroke::new(4.0, theme::ACCENT_GLOW),
    ));
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.8, theme::ACCENT),
    ));
}

fn composite_magnitude_db(
    freq: f32,
    band_coeffs: &[(usize, [Biquad; MAX_STAGES_PER_BAND])],
) -> f32 {
    let mut total_lin = 1.0f32;
    for (active, stages) in band_coeffs {
        for stage in stages.iter().take(*active) {
            total_lin *= stage.magnitude(freq, VIS_SR);
        }
    }
    20.0 * total_lin.max(1e-10).log10()
}

pub fn freq_to_x(freq: f32, width: f32) -> f32 {
    let t = (freq.max(MIN_FREQ) / MIN_FREQ).log10() / (MAX_FREQ / MIN_FREQ).log10();
    t.clamp(0.0, 1.0) * width
}

pub fn x_to_freq(x: f32, plot_left: f32, width: f32) -> f32 {
    let t = ((x - plot_left) / width).clamp(0.0, 1.0);
    MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(t)
}

pub fn db_to_y(db: f32, height: f32) -> f32 {
    let t = 1.0 - (db - DB_MIN) / (DB_MAX - DB_MIN);
    t.clamp(0.0, 1.0) * height
}

pub fn y_to_db(y: f32, plot_top: f32, height: f32) -> f32 {
    let t = ((y - plot_top) / height).clamp(0.0, 1.0);
    DB_MAX - t * (DB_MAX - DB_MIN)
}

fn label_freq(freq: f32) -> String {
    if freq >= 1000.0 {
        if freq == freq.round() && (freq / 1000.0).fract() == 0.0 {
            format!("{}k", (freq / 1000.0) as i32)
        } else {
            format!("{:.0}k", freq / 1000.0)
        }
    } else {
        format!("{}", freq as i32)
    }
}

// Kind colour helper — used by the node drawing in `nodes.rs` to tint each
// band node by type. Kept here with the rest of the palette-adjacent logic.
pub fn color_for_kind(kind: BandKind) -> egui::Color32 {
    match kind {
        BandKind::Bell => theme::ACCENT,
        BandKind::LowShelf => theme::GOOD,
        BandKind::HighShelf => theme::GOOD,
        BandKind::LowCut => theme::WARN,
        BandKind::HighCut => theme::WARN,
    }
}
