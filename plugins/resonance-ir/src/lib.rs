use parking_lot::Mutex;
/// Resonance IR - An impulse response convolution CLAP plugin for cab and room emulation.
use resonance_plugin::*;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

pub mod convolver;
pub mod ir_loader;
pub mod loader;
pub mod params;
pub mod state;
pub mod viz;

#[cfg(feature = "editor")]
mod editor;

use convolver::StereoConvolver;
use loader::{LoaderDeps, LoaderHandle};
use params::{IrParams, IrSmoothers};
use state::IrExtraState;
use viz::IrViz;

/// Crossfade length in samples (~1.5ms at 44.1kHz) to avoid pops on convolver swap.
const SWAP_FADE_SAMPLES: u32 = 64;

pub struct ResonanceIr {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. The `FloatParam` / `IntParam`
    /// fields use atomic storage internally, so `&IrParams` is safe to use
    /// concurrently from audio + UI.
    params: Arc<IrParams>,
    /// Audio-thread-only smoothers. Lives outside the shared `Arc<IrParams>`
    /// so the audio thread can mutate smoother state through `&mut self`.
    smoothers: IrSmoothers,
    /// Lock-free meters + precomputed IR snapshot shared with the editor.
    viz: Arc<IrViz>,

    active_convolver: Option<StereoConvolver>,
    convolver_mailbox: Arc<Mutex<Option<StereoConvolver>>>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    last_file_index: i32,
    sample_rate: f32,
    /// Atomic load request for the persistent loader thread (-1 = no request).
    load_request: Arc<AtomicI32>,
    /// Handle to the persistent loader thread; dropped on plugin drop.
    loader_handle: Option<LoaderHandle>,
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
            let files = resonance_common::scan_directory(dir, "wav");
            let idx = files.iter().position(|f| f == path).unwrap_or(0);
            *self.params.file_list.lock() = files;
            idx
        } else {
            0
        }
    }

    fn start_loader_thread(&mut self) {
        // Drop the old handle first so its thread joins before we
        // spawn a replacement.
        self.loader_handle = None;
        self.loader_handle = Some(loader::start(LoaderDeps {
            params: self.params.clone(),
            mailbox: self.convolver_mailbox.clone(),
            ir_name: self.ir_name.clone(),
            ir_info: self.ir_info.clone(),
            load_request: self.load_request.clone(),
            viz: self.viz.clone(),
            sample_rate: self.sample_rate,
        }));
    }
}

impl ResonancePlugin for ResonanceIr {
    const CLAP_ID: &'static str = "com.resonance.ir";
    const NAME: &'static str = "Resonance IR";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "Impulse response convolution for cabinet and room emulation";
    const FEATURES: &'static [&'static str] =
        &["audio-effect", "stereo", "cabinet_simulator", "reverb"];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(IrParams::default()),
            smoothers: IrSmoothers::new(),
            viz: IrViz::new(),
            active_convolver: None,
            convolver_mailbox: Arc::new(Mutex::new(None)),
            ir_name: Arc::new(Mutex::new(String::new())),
            ir_info: Arc::new(Mutex::new(String::new())),
            last_file_index: -1,
            sample_rate: 44100.0,
            load_request: Arc::new(AtomicI32::new(-1)),
            loader_handle: None,
            bypass_delay_l: resonance_dsp::DelayLine::new(convolver::BLOCK_SIZE),
            bypass_delay_r: resonance_dsp::DelayLine::new(convolver::BLOCK_SIZE),
            pending_convolver: None,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
        }
    }

    fn param_count(&self) -> usize {
        3
    }

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
        self.smoothers.prepare(sample_rate, &self.params);

        let path = self.params.ir_path.lock().clone();
        if !path.is_empty() {
            let idx = self.rescan_directory(&path);
            self.last_file_index = idx as i32;
            self.params.file_select.set_value(idx as i32);

            // Block on IR loading during initialize so it's ready before processing.
            loader::load_into(
                &path,
                sample_rate,
                &self.convolver_mailbox,
                &self.ir_name,
                &self.ir_info,
                &self.viz,
            );
            if let Some(conv) = self.convolver_mailbox.lock().take() {
                self.active_convolver = Some(conv);
            }
        }

        // Start persistent loader thread for runtime file_select changes.
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
        _tempo: Option<TempoInfo>,
    ) {
        let main = outputs
            .first_mut()
            .expect("resonance-ir always has a main output");
        let left = &mut *main.left;
        let right = &mut *main.right;
        resonance_common::flush_denormals();

        // Check mailbox for newly loaded convolver — start crossfade.
        if let Some(mut guard) = self.convolver_mailbox.try_lock() {
            if guard.is_some() {
                self.pending_convolver = guard.take();
                if self.active_convolver.is_some() {
                    self.fade_out_remaining = SWAP_FADE_SAMPLES;
                    self.fade_in_remaining = 0;
                } else {
                    // No previous convolver — swap directly with fade-in.
                    self.active_convolver = self.pending_convolver.take();
                    self.fade_in_remaining = SWAP_FADE_SAMPLES;
                }
            }
        }

        // Detect file_select param change from host/DAW.
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            self.load_request.store(current_index, Ordering::Release);
        }

        self.smoothers.retarget_from(&self.params);

        let mut in_peak_l = 0.0f32;
        let mut in_peak_r = 0.0f32;
        let mut out_peak_l = 0.0f32;
        let mut out_peak_r = 0.0f32;

        for i in 0..frames {
            let dry_wet = self.smoothers.dry_wet.next();
            let output_gain = self.smoothers.output_gain.next();

            // Crossfade envelope: fade out old convolver, swap, fade in new convolver.
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

            let dry_l = left[i];
            let dry_r = right[i];
            in_peak_l = in_peak_l.max(dry_l.abs());
            in_peak_r = in_peak_r.max(dry_r.abs());

            match &mut self.active_convolver {
                Some(conv) => {
                    let (wet_l, wet_r) = conv.process_sample(dry_l, dry_r);

                    let dry_amount = 1.0 - dry_wet;
                    left[i] = (dry_l * dry_amount + wet_l * dry_wet) * output_gain * fade_gain;
                    right[i] = (dry_r * dry_amount + wet_r * dry_wet) * output_gain * fade_gain;
                }
                None => {
                    let delayed_l = self.bypass_delay_l.tap(convolver::BLOCK_SIZE);
                    let delayed_r = self.bypass_delay_r.tap(convolver::BLOCK_SIZE);
                    self.bypass_delay_l.push(dry_l);
                    self.bypass_delay_r.push(dry_r);
                    left[i] = delayed_l * output_gain * fade_gain;
                    right[i] = delayed_r * output_gain * fade_gain;
                }
            }

            out_peak_l = out_peak_l.max(left[i].abs());
            out_peak_r = out_peak_r.max(right[i].abs());
        }

        let to_db = |v: f32| {
            if v <= 1e-6 {
                f32::NEG_INFINITY
            } else {
                20.0 * v.log10()
            }
        };
        self.viz.store_peaks(
            to_db(in_peak_l),
            to_db(in_peak_r),
            to_db(out_peak_l),
            to_db(out_peak_r),
        );
    }

    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::plugin::ExtraStateSaver>> {
        Some(Arc::new(IrExtraState {
            ir_path: self.params.ir_path.clone(),
            file_list: self.params.file_list.clone(),
            load_request: self.load_request.clone(),
        }))
    }

    fn latency_samples(&self) -> u32 {
        convolver::BLOCK_SIZE as u32
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::IrEditorFactory::new(
            self.params.clone(),
            self.ir_name.clone(),
            self.ir_info.clone(),
            self.load_request.clone(),
            self.viz.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceIr);

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_plugin::ResonancePlugin;

    /// Full save_state → load_state round-trip preserves the persisted IR
    /// path. Exercises the trait-default `save_state` / `load_state` that
    /// the CLAP bridge calls on the owned plugin instance.
    #[test]
    fn state_roundtrip_preserves_ir_path() {
        let src = ResonanceIr::new();
        *src.params.ir_path.lock() = "/some/cabs/resonance_cab.wav".to_string();

        let bytes = src.save_state();

        let mut dst = ResonanceIr::new();
        assert!(dst.load_state(&bytes), "load_state should succeed");
        assert_eq!(
            dst.params.ir_path.lock().clone(),
            "/some/cabs/resonance_cab.wav"
        );
    }
}
