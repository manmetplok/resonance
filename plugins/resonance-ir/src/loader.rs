//! Background IR loader thread + IR precompute helpers.
//!
//! Kept out of `lib.rs` so the plugin file stays focused on the audio
//! path. The loader's job is:
//!   1. Poll an `AtomicI32` load-request slot that the audio thread
//!      and editor both write to.
//!   2. Decode the requested `.wav` file and resample it to the host
//!      sample rate.
//!   3. Build a `StereoConvolver` from the IR samples.
//!   4. Precompute a waveform envelope and a log-spaced magnitude
//!      response for the editor visualisation.
//!   5. Publish the convolver through a `Mutex<Option<...>>` mailbox
//!      and push the visualisation snapshot into `IrViz`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::convolver::StereoConvolver;
use crate::ir_loader::{self, IrData};
use crate::params::IrParams;
use crate::viz::{IrSnapshot, IrViz, RESPONSE_POINTS, WAVEFORM_POINTS};

/// Max FFT size used for the one-shot response plot. Longer IRs are
/// truncated — the first 4096 samples capture the initial attack and
/// early reflections, which is what shapes the audible timbre.
const RESPONSE_FFT_SIZE: usize = 4096;

const RESPONSE_MIN_HZ: f32 = 20.0;
const RESPONSE_MAX_HZ: f32 = 20_000.0;

// ---------------------------------------------------------------------------
// Loader thread handle + deps.
// ---------------------------------------------------------------------------

pub struct LoaderHandle {
    handle: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl LoaderHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LoaderHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct LoaderDeps {
    pub params: Arc<IrParams>,
    pub mailbox: Arc<Mutex<Option<StereoConvolver>>>,
    pub ir_name: Arc<Mutex<String>>,
    pub ir_info: Arc<Mutex<String>>,
    pub load_request: Arc<AtomicI32>,
    pub viz: Arc<IrViz>,
    pub sample_rate: f32,
}

pub fn start(deps: LoaderDeps) -> LoaderHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let handle = std::thread::Builder::new()
        .name("ir-loader".into())
        .spawn(move || loader_loop(deps, stop_clone))
        .expect("failed to spawn ir-loader thread");

    LoaderHandle {
        handle: Some(handle),
        stop,
    }
}

fn loader_loop(deps: LoaderDeps, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let idx = deps.load_request.swap(-1, Ordering::AcqRel);
        if idx < 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }

        let path = {
            let list = deps.params.file_list.lock();
            if list.is_empty() {
                continue;
            }
            let clamped = (idx as usize).min(list.len() - 1);
            let p = list[clamped].clone();
            drop(list);
            if let Some(mut ip) = deps.params.ir_path.try_lock() {
                *ip = p.clone();
            }
            p
        };

        load_into(
            &path,
            deps.sample_rate,
            &deps.mailbox,
            &deps.ir_name,
            &deps.ir_info,
            &deps.viz,
        );
    }
}

/// Synchronous IR load path used from both `initialize()` and the
/// loader thread. On success the convolver is handed to the mailbox
/// and the visualisation snapshot is published to `viz`. On failure
/// the error is surfaced through `ir_name`.
pub fn load_into(
    path: &str,
    sample_rate: f32,
    mailbox: &Mutex<Option<StereoConvolver>>,
    ir_name: &Mutex<String>,
    ir_info: &Mutex<String>,
    viz: &IrViz,
) {
    match ir_loader::load_ir(path, sample_rate) {
        Ok(ir_data) => {
            let name = Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();

            let duration_ms = ir_data.left.len() as f32 / sample_rate * 1000.0;
            let ch_str = if ir_data.stereo { "stereo" } else { "mono" };
            let info = format!(
                "{} samples ({:.0}ms, {})",
                ir_data.left.len(),
                duration_ms,
                ch_str
            );

            let snapshot = build_snapshot(&ir_data, sample_rate);

            let right_ir = if ir_data.stereo {
                Some(ir_data.right.as_slice())
            } else {
                None
            };
            let conv = StereoConvolver::new(&ir_data.left, right_ir);

            *ir_name.lock() = name;
            *ir_info.lock() = info;
            viz.store_snapshot(snapshot);
            *mailbox.lock() = Some(conv);
        }
        Err(e) => {
            eprintln!("Failed to load IR: {e}");
            *ir_name.lock() = format!("Error: {e}");
            *ir_info.lock() = String::new();
            viz.clear_snapshot();
        }
    }
}

// ---------------------------------------------------------------------------
// IR precompute: waveform envelope + log-spaced magnitude response.
// ---------------------------------------------------------------------------

fn build_snapshot(ir: &IrData, sample_rate: f32) -> IrSnapshot {
    let wave_len = WAVEFORM_POINTS.min(ir.left.len().max(1));
    let mut wave_left = [0.0f32; WAVEFORM_POINTS];
    let mut wave_right = [0.0f32; WAVEFORM_POINTS];
    decimate_envelope(&ir.left, &mut wave_left[..wave_len]);
    if ir.stereo {
        decimate_envelope(&ir.right, &mut wave_right[..wave_len]);
    } else {
        wave_right[..wave_len].copy_from_slice(&wave_left[..wave_len]);
    }

    let response_db = compute_response_db(&ir.left, sample_rate);

    IrSnapshot {
        wave_left,
        wave_right,
        wave_len,
        response_db,
        response_min_hz: RESPONSE_MIN_HZ,
        response_max_hz: RESPONSE_MAX_HZ.min(sample_rate * 0.5),
    }
}

/// Decimate `samples` into `out` by taking the max absolute value of
/// each source bucket. Produces a loudness envelope that reads well
/// even when the IR is longer than the display width.
fn decimate_envelope(samples: &[f32], out: &mut [f32]) {
    if samples.is_empty() || out.is_empty() {
        return;
    }
    let n = samples.len();
    let m = out.len();
    for (i, slot) in out.iter_mut().enumerate() {
        let start = i * n / m;
        let end = ((i + 1) * n / m).max(start + 1).min(n);
        let mut peak = 0.0f32;
        for &s in &samples[start..end] {
            let a = s.abs();
            if a > peak {
                peak = a;
            }
        }
        *slot = peak;
    }
}

/// One-shot magnitude response of the IR, log-smoothed onto a
/// geometrically-spaced frequency axis. Returns dBFS values clamped
/// below at -80 dB so the plot has a stable floor.
fn compute_response_db(ir: &[f32], sample_rate: f32) -> [f32; RESPONSE_POINTS] {
    let mut out = [-80.0f32; RESPONSE_POINTS];
    if ir.is_empty() || sample_rate <= 0.0 {
        return out;
    }

    let n = ir.len().min(RESPONSE_FFT_SIZE);
    let fft_size = RESPONSE_FFT_SIZE;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_size);

    let mut buf = vec![Complex::new(0.0, 0.0); fft_size];
    for (i, &s) in ir[..n].iter().enumerate() {
        buf[i] = Complex::new(s, 0.0);
    }
    fft.process(&mut buf);

    // Linear bin → frequency. We only care about the real half.
    let half = fft_size / 2;
    let bin_hz = sample_rate / fft_size as f32;

    // For each log-spaced output point, average the magnitude of every
    // FFT bin that falls into its geometric window. Using the average
    // (rather than the peak) matches how EQ plots typically render.
    let max_hz = RESPONSE_MAX_HZ.min(sample_rate * 0.5);
    let log_min = RESPONSE_MIN_HZ.ln();
    let log_max = max_hz.ln();
    let step = (log_max - log_min) / RESPONSE_POINTS as f32;

    for (i, slot) in out.iter_mut().enumerate() {
        let f_center = (log_min + (i as f32 + 0.5) * step).exp();
        let f_low = (log_min + i as f32 * step).exp();
        let f_high = (log_min + (i as f32 + 1.0) * step).exp();

        let b_low = (f_low / bin_hz).floor() as usize;
        let b_high = ((f_high / bin_hz).ceil() as usize).max(b_low + 1);
        let b_low = b_low.min(half.saturating_sub(1));
        let b_high = b_high.min(half);

        let mut sum_sq = 0.0f64;
        let mut count = 0usize;
        for b in b_low..b_high {
            let m = buf[b].norm();
            sum_sq += (m as f64) * (m as f64);
            count += 1;
        }
        // Fall back on the nearest single bin if the window was empty
        // (happens at the very low end where bins are sparse).
        let mag = if count == 0 {
            let b = ((f_center / bin_hz).round() as usize).min(half - 1);
            buf[b].norm()
        } else {
            (sum_sq / count as f64).sqrt() as f32
        };
        let db = 20.0 * mag.max(1e-6).log10();
        *slot = db.max(-80.0);
    }

    // Normalise so the loudest bin sits at 0 dB — it's the *shape*
    // that matters, not the absolute level (which depends on IR
    // normalisation at bake time).
    let peak = out.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if peak.is_finite() {
        for v in out.iter_mut() {
            *v -= peak;
        }
    }
    out
}
