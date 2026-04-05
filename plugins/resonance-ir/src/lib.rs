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

pub struct ResonanceIr {
    params: Arc<IrParams>,
    active_convolver: Option<StereoConvolver>,
    convolver_mailbox: Arc<Mutex<Option<StereoConvolver>>>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    sample_rate: f32,
}

impl Default for ResonanceIr {
    fn default() -> Self {
        Self {
            params: Arc::new(IrParams::default()),
            active_convolver: None,
            convolver_mailbox: Arc::new(Mutex::new(None)),
            ir_name: Arc::new(Mutex::new(String::new())),
            ir_info: Arc::new(Mutex::new(String::new())),
            sample_rate: 44100.0,
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
        // Stereo in, stereo out
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        // Mono in, stereo out
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
        let sample_rate = self.sample_rate;

        Box::new(move |task| match task {
            IrTask::LoadIr(path) => match ir_loader::load_ir(&path, sample_rate) {
                Ok(ir_data) => {
                    let name = Path::new(&path)
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
                    nih_plug::nih_log!("Failed to load IR: {e}");
                    *ir_name.lock() = format!("Error: {e}");
                    *ir_info.lock() = String::new();
                }
            },
        })
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let mailbox = self.convolver_mailbox.clone();
        let ir_name_clone = self.ir_name.clone();
        let ir_info_clone = self.ir_info.clone();
        let sample_rate = self.sample_rate;

        let task_sender: Arc<dyn Fn(IrTask) + Send + Sync> = {
            let ir_name = self.ir_name.clone();
            let ir_info = self.ir_info.clone();
            Arc::new(move |task| match task {
                IrTask::LoadIr(path) => match ir_loader::load_ir(&path, sample_rate) {
                    Ok(ir_data) => {
                        let name = Path::new(&path)
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
                        nih_plug::nih_log!("Failed to load IR: {e}");
                        *ir_name.lock() = format!("Error: {e}");
                        *ir_info.lock() = String::new();
                    }
                },
            })
        };

        editor::create(EditorFlags {
            params: self.params.clone(),
            ir_name: ir_name_clone,
            ir_info: ir_info_clone,
            task_sender,
        })
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;

        // Reload persisted IR path
        let path = self.params.ir_path.lock().clone();
        if !path.is_empty() {
            context.execute(IrTask::LoadIr(path));
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
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Check mailbox for newly loaded convolver
        if let Some(mut guard) = self.convolver_mailbox.try_lock() {
            if guard.is_some() {
                self.active_convolver = guard.take();
            }
        }

        let num_channels = buffer.channels();

        match &mut self.active_convolver {
            Some(conv) => {
                for mut channel_samples in buffer.iter_samples() {
                    let dry_wet = self.params.dry_wet.smoothed.next();
                    let output_gain = self.params.output_gain.smoothed.next();

                    // Read input
                    let dry_l = *channel_samples.get_mut(0).unwrap();
                    let dry_r = if num_channels >= 2 {
                        *channel_samples.get_mut(1).unwrap()
                    } else {
                        dry_l
                    };

                    // Convolve
                    let (wet_l, wet_r) = conv.process_sample(dry_l, dry_r);

                    // Mix dry/wet and apply output gain
                    let dry_amount = 1.0 - dry_wet;
                    let out_l = (dry_l * dry_amount + wet_l * dry_wet) * output_gain;
                    let out_r = (dry_r * dry_amount + wet_r * dry_wet) * output_gain;

                    // Write output
                    *channel_samples.get_mut(0).unwrap() = out_l.clamp(-1.0, 1.0);
                    if num_channels >= 2 {
                        *channel_samples.get_mut(1).unwrap() = out_r.clamp(-1.0, 1.0);
                    }
                }
            }
            None => {
                // Bypass: apply output gain only
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
