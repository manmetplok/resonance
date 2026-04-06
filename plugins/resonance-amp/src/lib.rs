/// Resonance Amp - A guitar amp simulator CLAP plugin using NAM models.

use nih_plug::prelude::*;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

pub mod nam;
pub mod params;

use nam::NamInference;
use params::AmpParams;

/// Background task dispatched for model loading.
#[derive(Clone)]
pub enum AmpTask {
    LoadModel(String),
}

/// Scan a directory for .nam files, returning sorted paths.
fn scan_directory(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("nam"))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    files.sort();
    files
}

pub struct ResonanceAmp {
    params: Arc<AmpParams>,
    active_model: Option<Box<dyn NamInference>>,
    model_mailbox: Arc<Mutex<Option<Box<dyn NamInference>>>>,
    model_name: Arc<Mutex<String>>,
    /// List of .nam files in the current directory.
    file_list: Arc<Mutex<Vec<String>>>,
    /// Last file_select param value we acted on (to detect changes).
    last_file_index: i32,
}

impl Default for ResonanceAmp {
    fn default() -> Self {
        Self {
            params: Arc::new(AmpParams::default()),
            active_model: None,
            model_mailbox: Arc::new(Mutex::new(None)),
            model_name: Arc::new(Mutex::new(String::new())),
            file_list: Arc::new(Mutex::new(Vec::new())),
            last_file_index: -1,
        }
    }
}

impl ResonanceAmp {
    /// Scan the directory of the given file and update the file list.
    /// Returns the index of `path` in the new list, or 0.
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

impl Plugin for ResonanceAmp {
    const NAME: &'static str = "Resonance Amp";
    const VENDOR: &'static str = "Resonance";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    type SysExMessage = ();
    type BackgroundTask = AmpTask;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let mailbox = self.model_mailbox.clone();
        let model_name = self.model_name.clone();

        Box::new(move |task| match task {
            AmpTask::LoadModel(path) => match nam::parse::load_model_from_file(&path) {
                Ok(model) => {
                    let name = Path::new(&path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    *model_name.lock() = name;
                    *mailbox.lock() = Some(model);
                }
                Err(e) => {
                    nih_plug::nih_log!("Failed to load NAM model: {e}");
                    *model_name.lock() = format!("Error: {e}");
                }
            },
        })
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        let path = self.params.model_path.lock().clone();
        if !path.is_empty() {
            let idx = self.rescan_directory(&path);
            self.last_file_index = idx as i32;

            context.execute(AmpTask::LoadModel(path));
            if let Some(model) = self.model_mailbox.lock().take() {
                self.active_model = Some(model);
            }
        }
        true
    }

    fn reset(&mut self) {
        if let Some(model) = &mut self.active_model {
            model.reset();
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

        // Check mailbox for newly loaded model
        if let Some(mut guard) = self.model_mailbox.try_lock() {
            if guard.is_some() {
                self.active_model = guard.take();
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
                    if let Some(mut model_path) = self.params.model_path.try_lock() {
                        *model_path = path.clone();
                    }
                    context.execute_background(AmpTask::LoadModel(path));
                }
            }
        }

        match &mut self.active_model {
            Some(model) => {
                for mut channel_samples in buffer.iter_samples() {
                    let input_gain = self.params.input_gain.smoothed.next();
                    let output_gain = self.params.output_gain.smoothed.next();

                    let Some(input_sample) = channel_samples.get_mut(0) else { continue; };
                    let input = *input_sample * input_gain;
                    let output = model.process_sample(input) * output_gain;

                    for sample in channel_samples {
                        *sample = output;
                    }
                }
            }
            None => {
                for channel_samples in buffer.iter_samples() {
                    let input_gain = self.params.input_gain.smoothed.next();
                    let output_gain = self.params.output_gain.smoothed.next();
                    let gain = input_gain * output_gain;
                    for sample in channel_samples {
                        *sample *= gain;
                    }
                }
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for ResonanceAmp {
    const CLAP_ID: &'static str = "com.resonance.amp";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Guitar amp simulator using Neural Amp Modeler profiles");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Mono,
        ClapFeature::Stereo,
    ];
}

nih_export_clap!(ResonanceAmp);
