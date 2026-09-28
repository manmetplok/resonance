//! Harmonic bars H1–H7 and the THD readout, from the crate's probe
//! ([`crate::probe`]: a 1 kHz sine at −18 dBFS through the current
//! settings) — so the bars read the same numbers `tests/harmonics.rs`
//! pins and the presets were voiced to.
//!
//! "Live" means the bars follow every knob as it moves: the probe is a
//! function of the settings, so it re-runs when they change — never per
//! frame, never on the audio thread, and never on the GUI thread either.
//! A probe renders 19 200 samples through the whole chain (up to ~24 ms
//! in Tape HQ), which is a dropped frame or two if `ui()` waits for it.
//! [`ProbeCache`] hands the settings to a worker thread that holds only
//! the latest request (a drag coalesces to its newest position) and
//! probes on one reused [`Prober`]; `ui()` reads the newest result
//! without ever waiting on the worker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::thread::JoinHandle;

use plugin_gui_core::egui;

use crate::dsp::Settings;
use crate::editor::theme;
use crate::probe::{probe_settings, HarmonicSignature, Prober, BAR_ORDERS, PROBE_LEVEL_DBFS};

/// The bars' floor, in dBc.
pub const FLOOR_DBC: f64 = -100.0;

/// What the editor and the probe worker share.
#[derive(Default)]
struct Shared {
    /// The newest settings to probe, taken by the worker.
    request: Mutex<Option<Settings>>,
    wake: Condvar,
    /// The newest finished probe and the settings it was for.
    result: Mutex<Option<(Settings, HarmonicSignature)>>,
    quit: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `try_lock` that treats poisoning as success (the data is plain values).
fn try_lock<T>(m: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match m.try_lock() {
        Ok(g) => Some(g),
        Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

fn worker(shared: Arc<Shared>) {
    let mut prober = Prober::new();
    loop {
        let settings = {
            let mut req = lock(&shared.request);
            loop {
                if shared.quit.load(Ordering::Acquire) {
                    return;
                }
                if let Some(s) = req.take() {
                    break s;
                }
                req = shared.wake.wait(req).unwrap_or_else(|e| e.into_inner());
            }
        };
        let sig = prober.probe(&settings, PROBE_LEVEL_DBFS);
        *lock(&shared.result) = Some((settings, sig));
    }
}

/// The editor's side of the probe worker. The thread starts on the first
/// request and is stopped and joined on drop, which waits for at most
/// the one probe in flight.
#[derive(Default)]
pub struct ProbeCache {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    /// The probe settings last handed to the worker.
    requested: Option<Settings>,
    /// The newest result read back, and what it was for.
    shown: Option<(Settings, HarmonicSignature)>,
}

impl ProbeCache {
    /// The newest signature available for the bars; asks the worker for
    /// `s` when a setting the probe sees has changed. Never blocks: while
    /// a probe runs it returns the previous result (`None` before the
    /// first one lands).
    pub fn signature(&mut self, s: &Settings) -> Option<HarmonicSignature> {
        let key = probe_settings(s);
        if self.requested != Some(key) && self.ensure_worker() {
            // The worker holds this lock only to take a request, so
            // contention is rare; on it, try again next frame.
            if let Some(mut req) = try_lock(&self.shared.request) {
                *req = Some(key);
                self.requested = Some(key);
                self.shared.wake.notify_one();
            }
        }
        if let Some(res) = try_lock(&self.shared.result) {
            if let Some(r) = *res {
                self.shown = Some(r);
            }
        }
        self.shown.map(|(_, sig)| sig)
    }

    /// Whether the signature [`Self::signature`] last returned was probed
    /// for `s` (rather than for earlier settings, still being replaced).
    pub fn is_current(&self, s: &Settings) -> bool {
        self.shown.is_some_and(|(k, _)| k == probe_settings(s))
    }

    fn ensure_worker(&mut self) -> bool {
        if self.thread.is_none() {
            let shared = Arc::clone(&self.shared);
            self.thread = std::thread::Builder::new()
                .name("color-probe".into())
                .spawn(move || worker(shared))
                .ok();
        }
        self.thread.is_some()
    }
}

impl Drop for ProbeCache {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::Release);
        {
            // Under the request lock, so the store cannot land between the
            // worker's quit check and its wait (a lost wake-up).
            let _req = lock(&self.shared.request);
            self.shared.wake.notify_all();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
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
