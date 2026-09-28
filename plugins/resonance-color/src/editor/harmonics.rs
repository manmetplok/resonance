//! Harmonic bars H1–H7 and the THD readout, from the crate's probe
//! ([`crate::probe`]: a 1 kHz sine at −18 dBFS through the current
//! settings) — so the bars read the same numbers `tests/harmonics.rs`
//! pins and the presets were voiced to.
//!
//! "Live" means the bars follow every knob as it moves: the probe is a
//! function of the settings, so it re-runs when they change (at most
//! every [`MIN_INTERVAL`]), never per frame and never on the audio thread.

use std::time::{Duration, Instant};

use plugin_gui_core::egui;

use crate::dsp::Settings;
use crate::editor::theme;
use crate::probe::{probe, probe_settings, HarmonicSignature, BAR_ORDERS, PROBE_LEVEL_DBFS};

/// Shortest time between two probe renders while a knob is dragged.
pub const MIN_INTERVAL: Duration = Duration::from_millis(60);

/// The bars' floor, in dBc.
pub const FLOOR_DBC: f64 = -100.0;

#[derive(Default)]
pub struct ProbeCache {
    key: Option<Settings>,
    result: Option<HarmonicSignature>,
    last_run: Option<Instant>,
}

impl ProbeCache {
    /// The signature for `s`, re-probing when a setting the probe sees
    /// has changed and the throttle allows.
    pub fn signature(&mut self, s: &Settings) -> Option<HarmonicSignature> {
        let key = probe_settings(s);
        let stale = self.key != Some(key);
        let due = self
            .last_run
            .is_none_or(|t| t.elapsed() >= MIN_INTERVAL);
        if stale && due {
            self.result = Some(probe(&key, PROBE_LEVEL_DBFS));
            self.key = Some(key);
            self.last_run = Some(Instant::now());
        }
        self.result
    }
}

/// Bar height fraction for a level in dBc: 0 dBc full, [`FLOOR_DBC`] empty.
pub fn bar_fraction(dbc: f64) -> f32 {
    ((dbc - FLOOR_DBC) / -FLOOR_DBC).clamp(0.0, 1.0) as f32
}

pub fn draw(painter: &egui::Painter, rect: egui::Rect, sig: Option<HarmonicSignature>) {
    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    let pad = 10.0;
    let header_h = 16.0;
    let label_h = 26.0;
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top() + pad + header_h),
        egui::pos2(rect.right() - pad, rect.bottom() - pad - label_h),
    );

    // Grid every 20 dB.
    for i in 0..=5 {
        let dbc = FLOOR_DBC * i as f64 / 5.0;
        let y = plot.bottom() - bar_fraction(dbc) * plot.height();
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        painter.text(
            egui::pos2(plot.left(), y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{dbc:.0}"),
            egui::FontId::proportional(8.0),
            theme::TEXT_DIM,
        );
    }

    let Some(sig) = sig else {
        return;
    };

    painter.text(
        egui::pos2(rect.left() + pad, rect.top() + pad),
        egui::Align2::LEFT_TOP,
        format!(
            "HARMONICS  1 kHz @ {PROBE_LEVEL_DBFS:.0} dBFS   THD {:.2} %   H2−H3 {:+.1} dB",
            sig.thd_pct,
            sig.h2_h3_db()
        ),
        egui::FontId::proportional(10.0),
        theme::TEXT,
    );

    let slot = plot.width() / BAR_ORDERS as f32;
    let bar_w = slot * 0.55;
    for k in 1..=BAR_ORDERS {
        let dbc = sig.h_dbc[k];
        let cx = plot.left() + slot * (k as f32 - 0.5);
        let top = plot.bottom() - bar_fraction(dbc) * plot.height();
        let color = if k == 1 {
            theme::FUNDAMENTAL
        } else if k % 2 == 0 {
            theme::EVEN
        } else {
            theme::ODD
        };
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(cx - bar_w / 2.0, top),
                egui::pos2(cx + bar_w / 2.0, plot.bottom()),
            ),
            2.0,
            color,
        );
        painter.text(
            egui::pos2(cx, plot.bottom() + 3.0),
            egui::Align2::CENTER_TOP,
            format!("H{k}"),
            egui::FontId::proportional(9.0),
            theme::TEXT_DIM,
        );
        let value = if k == 1 {
            "0".to_string()
        } else if dbc <= FLOOR_DBC {
            "—".to_string()
        } else {
            format!("{dbc:.0}")
        };
        painter.text(
            egui::pos2(cx, plot.bottom() + 14.0),
            egui::Align2::CENTER_TOP,
            value,
            egui::FontId::proportional(8.0),
            theme::TEXT_DIM,
        );
    }
}
