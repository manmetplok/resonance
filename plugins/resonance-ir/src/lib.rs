/// Resonance IR - An impulse response convolution CLAP plugin for cab and room emulation.

use resonance_plugin::*;
use parking_lot::Mutex;
use std::path::Path;
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

pub struct ResonanceIr {
    params: IrParams,
    active_convolver: Option<StereoConvolver>,
    convolver_mailbox: Arc<Mutex<Option<StereoConvolver>>>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    last_file_index: i32,
    sample_rate: f32,
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

    /// Load an IR in the background via a spawned thread.
    /// The result is placed in the convolver_mailbox for pickup in process().
    pub fn spawn_load_ir(&self, path: String) {
        let mailbox = self.convolver_mailbox.clone();
        let ir_name = self.ir_name.clone();
        let ir_info = self.ir_info.clone();
        let sample_rate = self.sample_rate;

        std::thread::spawn(move || {
            Self::do_load_ir(&path, sample_rate, &mailbox, &ir_name, &ir_info);
        });
    }

    /// Load an IR by index from the file list in the background.
    fn spawn_load_by_index(&self, idx: usize) {
        let mailbox = self.convolver_mailbox.clone();
        let ir_name = self.ir_name.clone();
        let ir_info = self.ir_info.clone();
        let sample_rate = self.sample_rate;
        let file_list = self.params.file_list.clone();
        let ir_path_param = self.params.ir_path.clone();

        std::thread::spawn(move || {
            let path = {
                let list = file_list.lock();
                if list.is_empty() {
                    return;
                }
                let clamped = idx.min(list.len() - 1);
                let p = list[clamped].clone();
                drop(list);
                if let Some(mut ip) = ir_path_param.try_lock() {
                    *ip = p.clone();
                }
                p
            };
            Self::do_load_ir(&path, sample_rate, &mailbox, &ir_name, &ir_info);
        });
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
    const OUTPUT_CHANNELS: u32 = 2;

    fn new() -> Self {
        Self {
            params: IrParams::default(),
            active_convolver: None,
            convolver_mailbox: Arc::new(Mutex::new(None)),
            ir_name: Arc::new(Mutex::new(String::new())),
            ir_info: Arc::new(Mutex::new(String::new())),
            last_file_index: -1,
            sample_rate: 44100.0,
        }
    }

    fn params(&self) -> Vec<&dyn Param> {
        vec![
            &self.params.file_select,
            &self.params.dry_wet,
            &self.params.output_gain,
        ]
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.sample_rate = sample_rate;

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

            // Block on IR loading during initialize so it's ready before processing
            let mailbox = self.convolver_mailbox.clone();
            let ir_name = self.ir_name.clone();
            let ir_info = self.ir_info.clone();
            let sr = self.sample_rate;

            let handle = std::thread::spawn(move || {
                Self::do_load_ir(&path, sr, &mailbox, &ir_name, &ir_info);
            });
            let _ = handle.join();

            if let Some(conv) = self.convolver_mailbox.lock().take() {
                self.active_convolver = Some(conv);
            }
        }

        true
    }

    fn reset(&mut self) {
        if let Some(conv) = &mut self.active_convolver {
            conv.reset();
        }
    }

    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        _events: &mut EventIterator,
    ) {
        resonance_common::flush_denormals();

        // Check mailbox for newly loaded convolver
        if let Some(mut guard) = self.convolver_mailbox.try_lock() {
            if guard.is_some() {
                self.active_convolver = guard.take();
            }
        }

        // Detect file_select param change from host/DAW
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            self.spawn_load_by_index(current_index as usize);
        }

        // Set smoother targets from current param values
        self.params.dry_wet.smoother.set_target(self.params.dry_wet.value());
        self.params.output_gain.smoother.set_target(self.params.output_gain.value());

        match &mut self.active_convolver {
            Some(conv) => {
                for i in 0..frames {
                    let dry_wet = self.params.dry_wet.smoother.next();
                    let output_gain = self.params.output_gain.smoother.next();

                    let dry_l = left[i];
                    let dry_r = right[i];

                    let (wet_l, wet_r) = conv.process_sample(dry_l, dry_r);

                    let dry_amount = 1.0 - dry_wet;
                    left[i] = (dry_l * dry_amount + wet_l * dry_wet) * output_gain;
                    right[i] = (dry_r * dry_amount + wet_r * dry_wet) * output_gain;
                }
            }
            None => {
                for i in 0..frames {
                    let output_gain = self.params.output_gain.smoother.next();
                    left[i] *= output_gain;
                    right[i] *= output_gain;
                }
            }
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut json = resonance_plugin::state::params_to_json(&self.params());
        json["ir_path"] = serde_json::Value::String(self.params.ir_path.lock().clone());
        serde_json::to_vec(&json).unwrap_or_default()
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        if let Ok(state) = serde_json::from_slice::<serde_json::Value>(data) {
            resonance_plugin::state::load_params_from_json(&self.params(), &state);
            if let Some(path) = state.get("ir_path").and_then(|v| v.as_str()) {
                *self.params.ir_path.lock() = path.to_string();
            }
            true
        } else {
            false
        }
    }

    fn latency_samples(&self) -> u32 {
        if self.active_convolver.is_some() {
            convolver::BLOCK_SIZE as u32
        } else {
            0
        }
    }
}

#[cfg(not(feature = "ui"))]
resonance_plugin::export_clap!(ResonanceIr);
