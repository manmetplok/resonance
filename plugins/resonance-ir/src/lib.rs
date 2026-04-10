/// Resonance IR - An impulse response convolution CLAP plugin for cab and room emulation.

use resonance_plugin::*;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

pub mod convolver;
pub mod ir_loader;
pub mod params;

#[cfg(feature = "ui")]
pub mod ui;

use convolver::StereoConvolver;
use params::IrParams;

/// Scan a directory for .wav files, returning sorted paths.
fn scan_directory(dir: &Path) -> Vec<String> {
    resonance_common::scan_directory(dir, "wav")
}

/// Crossfade length in samples (~1.5ms at 44.1kHz) to avoid pops on convolver swap.
const SWAP_FADE_SAMPLES: u32 = 64;

pub struct ResonanceIr {
    params: IrParams,
    active_convolver: Option<StereoConvolver>,
    convolver_mailbox: Arc<Mutex<Option<StereoConvolver>>>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    last_file_index: i32,
    sample_rate: f32,
    /// Atomic load request for the persistent loader thread (-1 = no request).
    load_request: Arc<AtomicI32>,
    /// Signal the loader thread to stop.
    loader_stop: Arc<AtomicBool>,
    /// Handle to the persistent loader thread.
    loader_handle: Option<std::thread::JoinHandle<()>>,
    /// Bypass delay lines to compensate for reported latency when no convolver is active.
    bypass_delay_l: resonance_dsp::DelayLine,
    bypass_delay_r: resonance_dsp::DelayLine,
    /// Convolver waiting to be swapped in after fade-out completes.
    pending_convolver: Option<StereoConvolver>,
    /// Samples remaining in fade-out before convolver swap.
    fade_out_remaining: u32,
    /// Samples remaining in fade-in after convolver swap.
    fade_in_remaining: u32,
}

impl ResonanceIr {
    fn rescan_directory(&self, path: &str) -> usize {
        if let Some(dir) = Path::new(path).parent() {
            let files = scan_directory(dir);
            let idx = files.iter().position(|f| f == path).unwrap_or(0);
            *self.params.file_list.lock() = files;
            idx
        } else {
            0
        }
    }

    /// Load an IR in the background via the persistent loader thread.
    pub fn request_load_ir(&self, path: String) {
        // For direct path loading (from UI), store path and trigger a rescan+load
        *self.params.ir_path.lock() = path.clone();
        if let Some(dir) = Path::new(&path).parent() {
            let files = scan_directory(dir);
            let idx = files.iter().position(|f| f == &path).unwrap_or(0);
            *self.params.file_list.lock() = files;
            self.load_request.store(idx as i32, Ordering::Release);
        }
    }

    /// Start the persistent loader thread that polls `load_request`.
    fn start_loader_thread(&mut self) {
        self.stop_loader_thread();

        let load_request = self.load_request.clone();
        let stop_flag = self.loader_stop.clone();
        let file_list = self.params.file_list.clone();
        let ir_path_param = self.params.ir_path.clone();
        let mailbox = self.convolver_mailbox.clone();
        let ir_name = self.ir_name.clone();
        let ir_info = self.ir_info.clone();
        let sample_rate = self.sample_rate;

        self.loader_handle = Some(
            std::thread::Builder::new()
                .name("ir-loader".into())
                .spawn(move || {
                    while !stop_flag.load(Ordering::Relaxed) {
                        let idx = load_request.swap(-1, Ordering::AcqRel);
                        if idx >= 0 {
                            let path = {
                                let list = file_list.lock();
                                if list.is_empty() {
                                    continue;
                                }
                                let clamped = (idx as usize).min(list.len() - 1);
                                let p = list[clamped].clone();
                                drop(list);
                                if let Some(mut ip) = ir_path_param.try_lock() {
                                    *ip = p.clone();
                                }
                                p
                            };
                            Self::do_load_ir(&path, sample_rate, &mailbox, &ir_name, &ir_info);
                        } else {
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                    }
                })
                .expect("failed to spawn ir-loader thread"),
        );
    }

    fn stop_loader_thread(&mut self) {
        self.loader_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.loader_handle.take() {
            let _ = handle.join();
        }
        self.loader_stop.store(false, Ordering::Relaxed);
    }

    /// Shared IR loading logic used by both spawn methods.
    fn do_load_ir(
        path: &str,
        sample_rate: f32,
        mailbox: &Mutex<Option<StereoConvolver>>,
        ir_name: &Mutex<String>,
        ir_info: &Mutex<String>,
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

                let right_ir = if ir_data.stereo {
                    Some(ir_data.right.as_slice())
                } else {
                    None
                };
                let conv = StereoConvolver::new(&ir_data.left, right_ir);

                *ir_name.lock() = name;
                *ir_info.lock() = info;
                *mailbox.lock() = Some(conv);
            }
            Err(e) => {
                eprintln!("Failed to load IR: {e}");
                *ir_name.lock() = format!("Error: {e}");
                *ir_info.lock() = String::new();
            }
        }
    }
}

impl ResonancePlugin for ResonanceIr {
    const CLAP_ID: &'static str = "com.resonance.ir";
    const NAME: &'static str = "Resonance IR";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Impulse response convolution for cabinet and room emulation";
    const FEATURES: &'static [&'static str] = &[
        "audio-effect",
        "stereo",
        "cabinet_simulator",
        "reverb",
    ];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: IrParams::default(),
            active_convolver: None,
            convolver_mailbox: Arc::new(Mutex::new(None)),
            ir_name: Arc::new(Mutex::new(String::new())),
            ir_info: Arc::new(Mutex::new(String::new())),
            last_file_index: -1,
            sample_rate: 44100.0,
            load_request: Arc::new(AtomicI32::new(-1)),
            loader_stop: Arc::new(AtomicBool::new(false)),
            loader_handle: None,
            bypass_delay_l: resonance_dsp::DelayLine::new(convolver::BLOCK_SIZE),
            bypass_delay_r: resonance_dsp::DelayLine::new(convolver::BLOCK_SIZE),
            pending_convolver: None,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
        }
    }

    fn param_count(&self) -> usize { 3 }

    fn param(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.params.file_select,
            1 => &self.params.dry_wet,
            2 => &self.params.output_gain,
            _ => unreachable!("invalid param index {index}"),
        }
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.sample_rate = sample_rate;
        self.bypass_delay_l = resonance_dsp::DelayLine::new(convolver::BLOCK_SIZE);
        self.bypass_delay_r = resonance_dsp::DelayLine::new(convolver::BLOCK_SIZE);

        // Set smoother sample rates
        self.params.dry_wet.smoother.set_sample_rate(sample_rate);
        self.params.output_gain.smoother.set_sample_rate(sample_rate);

        // Initialize smoother targets to current values
        self.params.dry_wet.smoother.reset(self.params.dry_wet.value());
        self.params.output_gain.smoother.reset(self.params.output_gain.value());

        let path = self.params.ir_path.lock().clone();
        if !path.is_empty() {
            let idx = self.rescan_directory(&path);
            self.last_file_index = idx as i32;
            self.params.file_select.set_value(idx as i32);

            // Block on IR loading during initialize so it's ready before processing
            let mailbox = &self.convolver_mailbox;
            let ir_name = &self.ir_name;
            let ir_info = &self.ir_info;
            Self::do_load_ir(&path, sample_rate, mailbox, ir_name, ir_info);

            if let Some(conv) = self.convolver_mailbox.lock().take() {
                self.active_convolver = Some(conv);
            }
        }

        // Start persistent loader thread for runtime file_select changes
        self.start_loader_thread();

        true
    }

    fn reset(&mut self) {
        if let Some(conv) = &mut self.active_convolver {
            conv.reset();
        }
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
    ) {
        let main = outputs
            .first_mut()
            .expect("resonance-ir always has a main output");
        let left = &mut *main.left;
        let right = &mut *main.right;
        resonance_common::flush_denormals();

        // Check mailbox for newly loaded convolver — start crossfade
        if let Some(mut guard) = self.convolver_mailbox.try_lock() {
            if guard.is_some() {
                self.pending_convolver = guard.take();
                if self.active_convolver.is_some() {
                    self.fade_out_remaining = SWAP_FADE_SAMPLES;
                    self.fade_in_remaining = 0;
                } else {
                    // No previous convolver — swap directly with fade-in
                    self.active_convolver = self.pending_convolver.take();
                    self.fade_in_remaining = SWAP_FADE_SAMPLES;
                }
            }
        }

        // Detect file_select param change from host/DAW
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            // Signal the persistent loader thread (no allocation, no spawn)
            self.load_request.store(current_index, Ordering::Release);
        }

        // Set smoother targets from current param values
        self.params.dry_wet.smoother.set_target(self.params.dry_wet.value());
        self.params.output_gain.smoother.set_target(self.params.output_gain.value());

        for i in 0..frames {
            let dry_wet = self.params.dry_wet.smoother.next();
            let output_gain = self.params.output_gain.smoother.next();

            // Crossfade envelope: fade out old convolver, swap, fade in new convolver
            let fade_gain = if self.fade_out_remaining > 0 {
                self.fade_out_remaining -= 1;
                let g = self.fade_out_remaining as f32 / SWAP_FADE_SAMPLES as f32;
                if self.fade_out_remaining == 0 {
                    self.active_convolver = self.pending_convolver.take();
                    self.fade_in_remaining = SWAP_FADE_SAMPLES;
                }
                g
            } else if self.fade_in_remaining > 0 {
                self.fade_in_remaining -= 1;
                1.0 - self.fade_in_remaining as f32 / SWAP_FADE_SAMPLES as f32
            } else {
                1.0
            };

            match &mut self.active_convolver {
                Some(conv) => {
                    let dry_l = left[i];
                    let dry_r = right[i];

                    let (wet_l, wet_r) = conv.process_sample(dry_l, dry_r);

                    let dry_amount = 1.0 - dry_wet;
                    left[i] = (dry_l * dry_amount + wet_l * dry_wet) * output_gain * fade_gain;
                    right[i] = (dry_r * dry_amount + wet_r * dry_wet) * output_gain * fade_gain;
                }
                None => {
                    let delayed_l = self.bypass_delay_l.tap(convolver::BLOCK_SIZE);
                    let delayed_r = self.bypass_delay_r.tap(convolver::BLOCK_SIZE);
                    self.bypass_delay_l.push(left[i]);
                    self.bypass_delay_r.push(right[i]);
                    left[i] = delayed_l * output_gain * fade_gain;
                    right[i] = delayed_r * output_gain * fade_gain;
                }
            }
        }
    }

    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::plugin::ExtraStateSaver>> {
        Some(Arc::new(IrExtraState {
            ir_path: self.params.ir_path.clone(),
        }))
    }

    fn latency_samples(&self) -> u32 {
        convolver::BLOCK_SIZE as u32
    }
}

/// Persists the IR file path alongside the plugin's params. Holds only
/// the shared `Arc<Mutex<String>>` so the CLAP bridge can serialize it
/// while the plugin is in the audio processor.
struct IrExtraState {
    ir_path: Arc<Mutex<String>>,
}

impl resonance_plugin::plugin::ExtraStateSaver for IrExtraState {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "ir_path".to_string(),
            serde_json::Value::String(self.ir_path.lock().clone()),
        );
        map
    }

    fn load(&self, state: &serde_json::Value) {
        if let Some(path) = state.get("ir_path").and_then(|v| v.as_str()) {
            *self.ir_path.lock() = path.to_string();
        }
    }
}

impl Drop for ResonanceIr {
    fn drop(&mut self) {
        self.stop_loader_thread();
    }
}

#[cfg(not(feature = "ui"))]
resonance_plugin::export_clap!(ResonanceIr);
