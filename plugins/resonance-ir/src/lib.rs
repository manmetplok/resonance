/// Resonance IR - An impulse response convolution CLAP plugin for cab and room emulation.

use nih_plug::prelude::*;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

pub mod convolver;
pub mod editor;
pub mod ir_loader;
pub mod params;

use convolver::StereoConvolver;
use editor::EditorFlags;
use params::IrParams;

#[derive(Clone)]
pub enum IrTask {
    LoadIr(String),
}

/// Scan a directory for .wav files, returning sorted paths.
fn scan_directory(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("wav"))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    files.sort();
    files
}

pub struct ResonanceIr {
    params: Arc<IrParams>,
    active_convolver: Option<StereoConvolver>,
    convolver_mailbox: Arc<Mutex<Option<StereoConvolver>>>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    file_list: Arc<Mutex<Vec<String>>>,
    last_file_index: i32,
    sample_rate: Arc<Mutex<f32>>,
}

impl Default for ResonanceIr {
    fn default() -> Self {
        Self {
            params: Arc::new(IrParams::default()),
            active_convolver: None,
            convolver_mailbox: Arc::new(Mutex::new(None)),
            ir_name: Arc::new(Mutex::new(String::new())),
            ir_info: Arc::new(Mutex::new(String::new())),
            file_list: Arc::new(Mutex::new(Vec::new())),
            last_file_index: -1,
            sample_rate: Arc::new(Mutex::new(44100.0)),
        }
    }
}

impl ResonanceIr {
    fn rescan_directory(&self, path: &str) -> usize {
        if let Some(dir) = Path::new(path).parent() {
            let files = scan_directory(dir);
            let idx = files.iter().position(|f| f == path).unwrap_or(0);
            *self.file_list.lock() = files;
            idx
        } else {
            0
        }
    }
}

impl Plugin for ResonanceIr {
    const NAME: &'static str = "Resonance IR";
    const VENDOR: &'static str = "Resonance";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    type SysExMessage = ();
    type BackgroundTask = IrTask;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let mailbox = self.convolver_mailbox.clone();
        let ir_name = self.ir_name.clone();
        let ir_info = self.ir_info.clone();
        let sample_rate = self.sample_rate.clone();

        Box::new(move |task| match task {
            IrTask::LoadIr(path) => {
                let sr = *sample_rate.lock();
                match ir_loader::load_ir(&path, sr) {
                    Ok(ir_data) => {
                        let name = Path::new(&path)
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();

                        let duration_ms = ir_data.left.len() as f32 / sr * 1000.0;
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
                        nih_plug::nih_log!("Failed to load IR: {e}");
                        *ir_name.lock() = format!("Error: {e}");
                        *ir_info.lock() = String::new();
                    }
                }
            }
        })
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let mailbox = self.convolver_mailbox.clone();
        let ir_name_clone = self.ir_name.clone();
        let ir_info_clone = self.ir_info.clone();
        let file_list = self.file_list.clone();
        let sample_rate = self.sample_rate.clone();
        let params = self.params.clone();

        let task_sender: Arc<dyn Fn(IrTask) + Send + Sync> = {
            let ir_name = self.ir_name.clone();
            let ir_info = self.ir_info.clone();
            let file_list = self.file_list.clone();
            Arc::new(move |task| match task {
                IrTask::LoadIr(path) => {
                    if let Some(dir) = Path::new(&path).parent() {
                        let files = scan_directory(dir);
                        *file_list.lock() = files;
                    }

                    // Load IR on background thread to avoid blocking GUI
                    let mailbox = mailbox.clone();
                    let ir_name = ir_name.clone();
                    let ir_info = ir_info.clone();
                    let sample_rate = sample_rate.clone();
                    std::thread::spawn(move || {
                        let sr = *sample_rate.lock();
                        match ir_loader::load_ir(&path, sr) {
                            Ok(ir_data) => {
                                let name = Path::new(&path)
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_default();

                                let duration_ms =
                                    ir_data.left.len() as f32 / sr * 1000.0;
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
                                nih_plug::nih_log!("Failed to load IR: {e}");
                                *ir_name.lock() = format!("Error: {e}");
                                *ir_info.lock() = String::new();
                            }
                        }
                    });
                }
            })
        };

        editor::create(EditorFlags {
            params,
            ir_name: ir_name_clone,
            ir_info: ir_info_clone,
            file_list,
            task_sender,
        })
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        *self.sample_rate.lock() = buffer_config.sample_rate;

        let path = self.params.ir_path.lock().clone();
        if !path.is_empty() {
            let idx = self.rescan_directory(&path);
            self.last_file_index = idx as i32;

            context.execute(IrTask::LoadIr(path));
            if let Some(conv) = self.convolver_mailbox.lock().take() {
                self.active_convolver = Some(conv);
            }
        }

        // Report convolution latency to host
        if self.active_convolver.is_some() {
            context.set_latency_samples(convolver::BLOCK_SIZE as u32);
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
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Flush denormals to zero to prevent CPU spikes
        #[cfg(target_arch = "x86_64")]
        unsafe {
            std::arch::x86_64::_mm_setcsr(std::arch::x86_64::_mm_getcsr() | 0x8040);
        }
        #[cfg(target_arch = "x86")]
        unsafe {
            std::arch::x86::_mm_setcsr(std::arch::x86::_mm_getcsr() | 0x8040);
        }

        // Check mailbox for newly loaded convolver
        if let Some(mut guard) = self.convolver_mailbox.try_lock() {
            if guard.is_some() {
                let was_none = self.active_convolver.is_none();
                self.active_convolver = guard.take();
                // Report latency change to host when convolver is first loaded
                if was_none {
                    context.set_latency_samples(convolver::BLOCK_SIZE as u32);
                }
            }
        }

        // Detect file_select param change from host/DAW
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            if let Some(file_list) = self.file_list.try_lock() {
                if !file_list.is_empty() {
                    let idx = (current_index as usize).min(file_list.len() - 1);
                    let path = file_list[idx].clone();
                    drop(file_list);
                    if let Some(mut ir_path) = self.params.ir_path.try_lock() {
                        *ir_path = path.clone();
                    }
                    context.execute_background(IrTask::LoadIr(path));
                }
            }
        }

        let num_channels = buffer.channels();

        match &mut self.active_convolver {
            Some(conv) => {
                for mut channel_samples in buffer.iter_samples() {
                    let dry_wet = self.params.dry_wet.smoothed.next();
                    let output_gain = self.params.output_gain.smoothed.next();

                    let Some(sample_l) = channel_samples.get_mut(0) else { continue; };
                    let dry_l = *sample_l;
                    let dry_r = if num_channels >= 2 {
                        let Some(sample_r) = channel_samples.get_mut(1) else { continue; };
                        *sample_r
                    } else {
                        dry_l
                    };

                    let (wet_l, wet_r) = conv.process_sample(dry_l, dry_r);

                    let dry_amount = 1.0 - dry_wet;
                    let out_l = (dry_l * dry_amount + wet_l * dry_wet) * output_gain;
                    let out_r = (dry_r * dry_amount + wet_r * dry_wet) * output_gain;

                    let Some(out_sample_l) = channel_samples.get_mut(0) else { continue; };
                    *out_sample_l = out_l;
                    if num_channels >= 2 {
                        let Some(out_sample_r) = channel_samples.get_mut(1) else { continue; };
                        *out_sample_r = out_r;
                    }
                }
            }
            None => {
                for channel_samples in buffer.iter_samples() {
                    let output_gain = self.params.output_gain.smoothed.next();
                    for sample in channel_samples {
                        *sample *= output_gain;
                    }
                }
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for ResonanceIr {
    const CLAP_ID: &'static str = "com.resonance.ir";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Impulse response convolution for cabinet and room emulation");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Custom("cabinet_simulator"),
        ClapFeature::Custom("reverb"),
    ];
}

nih_export_clap!(ResonanceIr);
