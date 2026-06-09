use parking_lot::Mutex;
/// Resonance IR - An impulse response convolution CLAP plugin for cab and room emulation.
use resonance_plugin::*;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

pub mod dsp;
pub mod ir_loader;
pub mod loader;
pub mod params;
pub mod state;
pub mod viz;

#[cfg(feature = "editor")]
mod editor;

use dsp::{IrEngine, StereoConvolver};
use loader::{LoaderDeps, LoaderHandle};
use params::{IrParams, IrSmoothers};
use state::IrExtraState;
use viz::IrViz;

pub struct ResonanceIr {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. The `FloatParam` / `IntParam`
    /// fields use atomic storage internally, so `&IrParams` is safe to use
    /// concurrently from audio + UI.
    pub params: Arc<IrParams>,
    /// Audio-thread-only smoothers. Lives outside the shared `Arc<IrParams>`
    /// so the audio thread can mutate smoother state through `&mut self`.
    smoothers: IrSmoothers,
    /// Lock-free meters + precomputed IR snapshot shared with the editor.
    viz: Arc<IrViz>,

    /// Block-based wet/dry engine: convolvers, bypass delay alignment, and
    /// the swap-crossfade state machine all live in `dsp::IrEngine`.
    engine: IrEngine,
    convolver_mailbox: Arc<Mutex<Option<StereoConvolver>>>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    last_file_index: i32,
    sample_rate: f32,
    /// Atomic load request for the persistent loader thread (-1 = no request).
    load_request: Arc<AtomicI32>,
    /// Handle to the persistent loader thread; dropped on plugin drop.
    loader_handle: Option<LoaderHandle>,
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
            block_size: self.engine.block_size(),
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
        let block_size = dsp::block_size_for_sample_rate(44100.0);
        Self {
            params: Arc::new(IrParams::default()),
            smoothers: IrSmoothers::new(),
            viz: IrViz::new(),
            engine: IrEngine::new(block_size),
            convolver_mailbox: Arc::new(Mutex::new(None)),
            ir_name: Arc::new(Mutex::new(String::new())),
            ir_info: Arc::new(Mutex::new(String::new())),
            last_file_index: -1,
            sample_rate: 44100.0,
            load_request: Arc::new(AtomicI32::new(-1)),
            loader_handle: None,
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
            _ => &self.params.file_select,
        }
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.sample_rate = sample_rate;
        self.engine
            .set_block_size(dsp::block_size_for_sample_rate(sample_rate));
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
                self.engine.block_size(),
                &self.convolver_mailbox,
                &self.ir_name,
                &self.ir_info,
                &self.viz,
            );
            if let Some(conv) = self.convolver_mailbox.lock().take() {
                self.engine.install(conv);
            }
        }

        // Start persistent loader thread for runtime file_select changes.
        self.start_loader_thread();

        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut *main.left;
        let right = &mut *main.right;
        resonance_common::flush_denormals();

        // Check mailbox for newly loaded convolver — start crossfade.
        if let Some(mut guard) = self.convolver_mailbox.try_lock() {
            if let Some(conv) = guard.take() {
                self.engine.begin_swap(conv);
            }
        }

        // Detect file_select param change from host/DAW.
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            self.load_request.store(current_index, Ordering::Release);
        }

        self.smoothers.retarget_from(&self.params);

        let peaks = self.engine.process_block(
            &mut left[..frames],
            &mut right[..frames],
            &mut self.smoothers.dry_wet,
            &mut self.smoothers.output_gain,
        );

        let to_db = |v: f32| {
            if v <= 1e-6 {
                f32::NEG_INFINITY
            } else {
                20.0 * v.log10()
            }
        };
        self.viz.store_peaks(
            to_db(peaks.in_l),
            to_db(peaks.in_r),
            to_db(peaks.out_l),
            to_db(peaks.out_r),
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
        self.engine.block_size() as u32
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

