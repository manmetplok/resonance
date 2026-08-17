use parking_lot::Mutex;
/// Resonance IR - An impulse response convolution CLAP plugin for cab and room emulation.
use resonance_plugin::*;
use std::ffi::CStr;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

pub mod dsp;
pub mod ir_loader;
pub mod latency;
pub mod loader;
pub mod params;
pub mod state;
pub mod viz;

#[cfg(feature = "editor")]
mod editor;

use dsp::{IrEngine, LatencyMode, StereoConvolver};
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
    /// Which preset is loaded and whether it has been edited since,
    /// chained in front of this plugin's own `IrExtraState` so both ride
    /// along in `save_state` (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Audio-thread-only smoothers. Lives outside the shared `Arc<IrParams>`
    /// so the audio thread can mutate smoother state through `&mut self`.
    smoothers: IrSmoothers,
    /// Lock-free meters + precomputed IR snapshot shared with the editor.
    viz: Arc<IrViz>,

    /// Block-based wet/dry engine: convolvers, bypass delay alignment, and
    /// the swap-crossfade state machine all live in `dsp::IrEngine`.
    engine: IrEngine,
    convolver_mailbox: Mailbox<StereoConvolver>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    last_file_index: i32,
    sample_rate: f32,
    /// Atomic load request for the persistent loader thread (-1 = no request).
    load_request: Arc<AtomicI32>,
    /// Handle to the persistent loader thread; dropped on plugin drop.
    loader_handle: Option<LoaderHandle>,
    /// Handle back to the host, from `set_host`. The only way to report a
    /// latency change once the plugin is active (ba todo #1296) — which is
    /// exactly what a latency-mode change is. `None` outside a CLAP host
    /// (unit tests, benches).
    host: Option<Arc<HostHandle>>,
}

impl ResonanceIr {
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

    /// The latency mode the parameter currently selects.
    fn latency_mode(&self) -> LatencyMode {
        LatencyMode::from_index(self.params.latency_mode.value())
    }

    /// The convolution block size the selected mode asks for at the current
    /// sample rate — the plugin's *target* latency. Equals
    /// `self.engine.block_size()` except between a mode change and the
    /// reactivation that applies it.
    fn target_block_size(&self) -> usize {
        dsp::block_size_for(self.sample_rate, self.latency_mode())
    }
}

impl ResonancePlugin for ResonanceIr {
    const CLAP_ID: &'static str = "com.resonance.ir";
    const NAME: &'static str = "Resonance IR";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "Impulse response convolution for cabinet and room emulation";
    // `cabinet_simulator` (underscore) was silently dropped by the old
    // whitelist; CLAP has no cabinet-simulator feature at all, so the
    // namespaced constant carries that intent and `reverb` is the
    // category a host can actually file a convolver under (ba todo #1298).
    const FEATURES: &'static [&'static CStr] = &[
        features::AUDIO_EFFECT,
        features::REVERB,
        features::CABINET_SIMULATOR,
        features::STEREO,
    ];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        let block_size = dsp::block_size_for(44100.0, LatencyMode::default());
        let params = Arc::new(IrParams::default());
        let load_request = Arc::new(AtomicI32::new(-1));
        // The preset identity wraps the IR-path saver rather than
        // replacing it: chaining is why `with_extra` exists.
        let presets = resonance_plugin::presets::PresetSession::with_extra(Arc::new(
            IrExtraState {
                ir_path: params.ir_path.clone(),
                file_list: params.file_list.clone(),
                load_request: load_request.clone(),
            },
        ));
        Self {
            params,
            presets,
            smoothers: IrSmoothers::new(),
            viz: IrViz::new(),
            engine: IrEngine::new(block_size),
            convolver_mailbox: Mailbox::new(),
            ir_name: Arc::new(Mutex::new(String::new())),
            ir_info: Arc::new(Mutex::new(String::new())),
            last_file_index: -1,
            sample_rate: 44100.0,
            load_request,
            loader_handle: None,
            host: None,
        }
    }

    fn param_count(&self) -> usize {
        params::PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn set_host(&mut self, host: Arc<HostHandle>) {
        self.host = Some(host);
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.sample_rate = sample_rate;
        // This is where a latency-mode change becomes real: the host has
        // just (re)activated us — the one moment the reported latency is
        // allowed to move and the delay lines may be reallocated. The
        // block size the parameter asks for is what the whole rest of
        // this function, and `latency_samples()`, then agree on.
        let block_size = self.target_block_size();
        self.engine.set_block_size(block_size);
        self.viz.store_engine_block(block_size, sample_rate);
        self.smoothers.prepare(sample_rate, &self.params);

        let path = self.params.ir_path.lock().clone();
        if !path.is_empty() {
            let idx = rescan_directory(&path, "wav", &self.params.file_list);
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
            if let Some(conv) = self.convolver_mailbox.take() {
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
        if let Some(conv) = self.convolver_mailbox.try_take() {
            self.engine.begin_swap(conv);
        }

        // Detect file_select param change from host/DAW.
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            self.load_request.store(current_index, Ordering::Release);
        }

        // Detect a latency-mode change (host automation, the editor's
        // picker, `track.set_plugin_param`). The block size cannot change
        // here — new delay lines and a re-partitioned convolver are both
        // allocations, and CLAP forbids the reported latency moving while
        // active — so report it and let the host cycle us: it deactivates,
        // reactivates, and `initialize()` above applies it. Reporting the
        // same figure twice is a no-op inside the handle, so this costs two
        // atomic loads a block once the change has landed.
        if self.target_block_size() != self.engine.block_size() {
            if let Some(host) = &self.host {
                host.set_latency_samples(self.target_block_size() as u32);
            }
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
        Some(self.presets.clone())
    }

    /// The convolution block size, which is exactly this plugin's
    /// algorithmic delay (one hop of the partitioned convolver), and the
    /// dry path is delayed to match — see `dsp::IrEngine`.
    ///
    /// Derived from the *parameter*, not from the engine's current block
    /// size, so the two answers this can be asked always agree:
    ///
    /// * the bridge reads it at every activation, right after
    ///   `initialize()` has applied the mode — so it is the engine's
    ///   figure;
    /// * a host may read it while the plugin is inactive, where the honest
    ///   answer is what the plugin *will* impose when it is next
    ///   activated, which is the mode the parameter selects.
    ///
    /// It also keeps [`HostHandle::set_latency_samples`]'s contract: the
    /// figure we push is already the one this returns.
    fn latency_samples(&self) -> u32 {
        self.target_block_size() as u32
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::IrEditorFactory::new(
            self.params.clone(),
            self.ir_name.clone(),
            self.ir_info.clone(),
            self.load_request.clone(),
            self.viz.clone(),
            self.presets.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceIr);

