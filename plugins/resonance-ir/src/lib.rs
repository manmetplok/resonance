/// Resonance IR - An impulse response convolution CLAP plugin for cab and room emulation.

use resonance_plugin::*;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

pub mod convolver;
pub mod ir_loader;
pub mod params;

#[cfg(feature = "editor")]
mod editor;

use convolver::StereoConvolver;
use params::IrParams;

/// Scan a directory for .wav files, returning sorted paths.
fn scan_directory(dir: &Path) -> Vec<String> {
    resonance_common::scan_directory(dir, "wav")
}

/// Crossfade length in samples (~1.5ms at 44.1kHz) to avoid pops on convolver swap.
const SWAP_FADE_SAMPLES: u32 = 64;

pub struct ResonanceIr {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. The `FloatParam` / `IntParam`
    /// fields use atomic storage internally, so `&IrParams` is safe to use
    /// concurrently from audio + UI.
    params: Arc<IrParams>,
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
    /// Plugin-local smoother for the dry/wet mix. Lives here (not on
    /// `IrParams`) so that `params` can be `Arc`-shared with the editor
    /// thread while the smoothers stay audio-thread mutable.
    dry_wet_smoother: Smoother,
    output_gain_smoother: Smoother,
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
            params: Arc::new(IrParams::default()),
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
            dry_wet_smoother: Smoother::new(SmoothingStyle::Linear(50.0)),
            output_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
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

        // Configure plugin-local smoothers and seed them with the
        // current parameter values.
        self.dry_wet_smoother.set_sample_rate(sample_rate);
        self.output_gain_smoother.set_sample_rate(sample_rate);
        self.dry_wet_smoother.reset(self.params.dry_wet.value());
        self.output_gain_smoother.reset(self.params.output_gain.value());

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
        self.dry_wet_smoother.set_target(self.params.dry_wet.value());
        self.output_gain_smoother.set_target(self.params.output_gain.value());

        for i in 0..frames {
            let dry_wet = self.dry_wet_smoother.next();
            let output_gain = self.output_gain_smoother.next();

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
        )))
    }
}

/// Persists the IR file path alongside the plugin's params, and — crucially
/// — rebuilds the in-memory file list and kicks the persistent loader
/// thread when the state is restored. Holds only shared Arcs so the CLAP
/// bridge can call save/load while the plugin is in the audio processor.
///
/// Why the rescan + load_request live here (instead of in `initialize`):
/// the CLAP bridge's load path runs **after** the plugin has been moved
/// into the audio processor and `initialize` has already returned, so by
/// the time the saved path shows up in `ir_path` the loader thread has no
/// file list to walk and process() has no way to kick it. This saver
/// closes that gap by publishing both the path and the matching directory
/// scan as a single synchronous step, then bumping the load-request atomic
/// so the loader thread rebuilds the convolver on its next poll.
struct IrExtraState {
    ir_path: Arc<Mutex<String>>,
    file_list: Arc<Mutex<Vec<String>>>,
    load_request: Arc<std::sync::atomic::AtomicI32>,
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
        let Some(path) = state.get("ir_path").and_then(|v| v.as_str()) else {
            return;
        };
        *self.ir_path.lock() = path.to_string();
        if path.is_empty() {
            return;
        }
        // Rescan the containing directory so Prev/Next in the editor and
        // the audio-thread's param-change detector both have a populated
        // list to work with.
        if let Some(dir) = Path::new(path).parent() {
            let files = scan_directory(dir);
            let idx = files.iter().position(|f| f == path).unwrap_or(0);
            *self.file_list.lock() = files;
            // Bump the loader thread so it rebuilds the convolver for the
            // restored path. Without this the plugin would sit silent
            // after a project reopen even though `ir_path` was set.
            self.load_request
                .store(idx as i32, std::sync::atomic::Ordering::Release);
        }
    }
}

impl Drop for ResonanceIr {
    fn drop(&mut self) {
        self.stop_loader_thread();
    }
}

resonance_plugin::export_clap!(ResonanceIr);

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_plugin::plugin::ExtraStateSaver;
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

    /// Direct round-trip through the `ExtraStateSaver` interface. This
    /// simulates the audio-processor code path in the CLAP bridge: the
    /// owned plugin has been moved into `ClapAudioProcessor`, so `save()`
    /// on the bridge side talks to the cached saver Arc instead of calling
    /// the plugin's trait default. The saver must still round-trip the
    /// path through the shared `Arc<Mutex<String>>` it holds.
    #[test]
    fn extra_saver_roundtrip_active_path() {
        use std::sync::atomic::AtomicI32;

        // The save side: pretend the user has loaded an IR from a
        // directory that happens not to exist on the test host. We only
        // exercise `save()` here, so the absent directory is fine — the
        // rescan only runs on `load()`.
        let src_path = Arc::new(Mutex::new(
            "/definitely/not/real/active_cab.wav".to_string(),
        ));
        let saver = IrExtraState {
            ir_path: src_path.clone(),
            file_list: Arc::new(Mutex::new(Vec::new())),
            load_request: Arc::new(AtomicI32::new(-1)),
        };

        // Serialize the same way clap_bridge::save() does on the
        // plugin-is-None branch: a JSON object with a "params" key plus
        // whatever the saver adds at the top level.
        let mut json = serde_json::json!({ "params": {} });
        for (k, v) in saver.save() {
            json.as_object_mut().unwrap().insert(k, v);
        }

        // A fresh saver with its own shared storage — representing the
        // plugin instance that'll be loaded back from the project file.
        // We use the same non-existent directory so `load()` skips the
        // rescan gracefully and we only verify the ir_path field.
        let dst_path = Arc::new(Mutex::new(String::new()));
        let restored_saver = IrExtraState {
            ir_path: dst_path.clone(),
            file_list: Arc::new(Mutex::new(Vec::new())),
            load_request: Arc::new(AtomicI32::new(-1)),
        };
        restored_saver.load(&json);

        assert_eq!(
            dst_path.lock().clone(),
            "/definitely/not/real/active_cab.wav",
            "ir_path should round-trip through the ExtraStateSaver"
        );
    }

    /// Restoring a project file that references an IR must leave the
    /// plugin in a state where the persistent loader thread will actually
    /// rebuild the convolver. Before the fix, the saver wrote the path
    /// into the shared storage but never populated the file list or
    /// nudged the loader, so the plugin sat silent after reopen even
    /// though the editor showed the correct filename.
    ///
    /// This test asserts the three load-side side-effects that together
    /// make the reload work: `ir_path`, `file_list`, and `load_request`.
    #[test]
    fn extra_saver_load_populates_file_list_and_queues_loader() {
        use std::sync::atomic::{AtomicI32, Ordering};

        // Unique temp directory so parallel test runs don't collide.
        let dir = std::env::temp_dir().join(format!(
            "resonance-ir-saver-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let wav_a = dir.join("aaa_first.wav");
        let wav_b = dir.join("bbb_second.wav");
        let wav_c = dir.join("ccc_third.wav");
        // The content doesn't matter for the directory scan — we're not
        // actually decoding the file, just walking directory entries.
        for p in [&wav_a, &wav_b, &wav_c] {
            std::fs::write(p, b"").unwrap();
        }
        let target = wav_b.to_string_lossy().into_owned();

        // Fresh shared storage, all empty — this is what the plugin's
        // `new()` produces before any state is loaded.
        let ir_path = Arc::new(Mutex::new(String::new()));
        let file_list = Arc::new(Mutex::new(Vec::<String>::new()));
        let load_request = Arc::new(AtomicI32::new(-1));

        let saver = IrExtraState {
            ir_path: ir_path.clone(),
            file_list: file_list.clone(),
            load_request: load_request.clone(),
        };

        let state = serde_json::json!({
            "params": {},
            "ir_path": target,
        });
        saver.load(&state);

        // The path is published.
        assert_eq!(ir_path.lock().clone(), target);
        // The file list contains every .wav in the directory, sorted.
        let files = file_list.lock().clone();
        assert_eq!(files.len(), 3, "all three .wav files should be listed");
        assert!(files.iter().any(|f| f == &target));
        // The load request points at the restored file's index so the
        // loader thread will pick it up on its next poll.
        let expected_idx = files.iter().position(|f| f == &target).unwrap() as i32;
        assert_eq!(load_request.load(Ordering::Acquire), expected_idx);

        // Clean up the temp directory so repeated test runs don't leave
        // litter behind. Best-effort — ignore errors.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
