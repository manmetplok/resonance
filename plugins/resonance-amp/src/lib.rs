/// Resonance Amp - A guitar amp simulator CLAP plugin using NAM models.

use nih_plug::prelude::*;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

pub mod editor;
pub mod nam;
pub mod params;

use editor::EditorFlags;
use nam::NamInference;
use params::AmpParams;

/// Background task dispatched for model loading.
#[derive(Clone)]
pub enum AmpTask {
    LoadModel(String),
}

pub struct ResonanceAmp {
    params: Arc<AmpParams>,
    /// Currently active NAM model (None = bypass).
    active_model: Option<Box<dyn NamInference>>,
    /// Mailbox: background task places a loaded model here for the audio thread to pick up.
    model_mailbox: Arc<Mutex<Option<Box<dyn NamInference>>>>,
    /// Display name of the currently loaded model.
    model_name: Arc<Mutex<String>>,
}

impl Default for ResonanceAmp {
    fn default() -> Self {
        Self {
            params: Arc::new(AmpParams::default()),
            active_model: None,
            model_mailbox: Arc::new(Mutex::new(None)),
            model_name: Arc::new(Mutex::new(String::new())),
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
        // Mono in, stereo out
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        // Stereo in, stereo out (fallback)
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

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let mailbox = self.model_mailbox.clone();
        let model_name_clone = self.model_name.clone();

        // Create a task sender that puts models directly into the mailbox
        // (task_executor runs on a background thread, but we can also load inline)
        let task_sender: Arc<dyn Fn(AmpTask) + Send + Sync> = {
            let model_name = self.model_name.clone();
            Arc::new(move |task| match task {
                AmpTask::LoadModel(path) => {
                    match nam::parse::load_model_from_file(&path) {
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
                    }
                }
            })
        };

        editor::create(EditorFlags {
            params: self.params.clone(),
            model_name: model_name_clone,
            task_sender,
        })
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        // Reload persisted model path
        let path = self.params.model_path.lock().clone();
        if !path.is_empty() {
            context.execute(AmpTask::LoadModel(path));
            // After synchronous execute, model should be in mailbox
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
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Check mailbox for newly loaded model (non-blocking)
        if let Some(mut guard) = self.model_mailbox.try_lock() {
            if guard.is_some() {
                self.active_model = guard.take();
            }
        }

        match &mut self.active_model {
            Some(model) => {
                for mut channel_samples in buffer.iter_samples() {
                    let input_gain = self.params.input_gain.smoothed.next();
                    let output_gain = self.params.output_gain.smoothed.next();

                    // Read mono input (first channel)
                    let input = *channel_samples.get_mut(0).unwrap() * input_gain;

                    // Process through NAM model
                    let output = model.process_sample(input) * output_gain;
                    let output = output.clamp(-1.0, 1.0);

                    // Write to all output channels
                    for sample in channel_samples {
                        *sample = output;
                    }
                }
            }
            None => {
                // Bypass: apply gains only
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
