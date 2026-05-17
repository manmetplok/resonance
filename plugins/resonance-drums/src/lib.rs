//! Resonance Drums - A drum sampler instrument CLAP plugin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use drum_map::NUM_PADS;

use crossbeam_channel::{bounded, Receiver, Sender};
use resonance_plugin::*;

#[cfg(feature = "editor")]
pub(crate) mod download;
pub mod drum_map;
pub mod dsp;
#[cfg(feature = "editor")]
mod editor;
pub mod kit;
pub mod kit_loader;
mod mic_catalog;
pub mod params;
pub mod voice;

#[cfg(feature = "editor")]
use download::WorkerHandle;
use kit::LoadedPad;
use kit_loader::{KitStatus, PadMicChoices, DEFAULT_OVERHEAD_SETUP};
use mic_catalog::ManifestMicCatalog;
use params::{DrumParams, PARAMS_PER_PAD};
use resonance_plugin::plugin::ExtraStateSaver;
use dsp::DrumSampler;

/// Shared state the loader thread, editor, and audio-thread plugin all need
/// handles to. Cheap to clone (all Arcs + one channel sender clone).
#[doc(hidden)]
#[derive(Clone)]
pub struct KitBridge {
    /// Path to the currently loaded (or last-loaded) kit manifest. Set by
    /// the loader on success; persisted in `save_state`.
    pub kit_path: Arc<Mutex<Option<PathBuf>>>,
    /// Status reported by the loader, rendered by the editor.
    pub kit_status: Arc<Mutex<KitStatus>>,
    /// Host sample rate, captured in `initialize()`. Stored as `f32::to_bits`.
    /// Sentinel `0` means "not yet initialized — no audio rate is known".
    pub sample_rate: Arc<AtomicU32>,
    /// Audio-thread kit handoff. Clones go to the editor and loader thread.
    pub kit_sender: Sender<Vec<LoadedPad>>,
    /// Monotonic load stamp. Incremented each time a new loader is spawned;
    /// in-flight loaders check this before writing status/kit_path so a
    /// stale load can't clobber a newer one.
    pub load_generation: Arc<AtomicU64>,
    /// Index of mic setups available in the currently-loaded manifest.
    /// Rebuilt on each successful load and read by the editor to populate
    /// per-pad mic pickers.
    pub catalog: Arc<Mutex<ManifestMicCatalog>>,
    /// User-chosen setup keys per pad (key = position, value = setup_key).
    /// Wrapped in a Mutex so the editor can edit from the UI thread while
    /// the loader thread reads a snapshot when building a new kit.
    pub pad_choices: Arc<Mutex<[PadMicChoices; drum_map::NUM_PADS]>>,
    /// User-chosen global overhead setup key. Defaults to
    /// `DEFAULT_OVERHEAD_SETUP` and persists via plugin state.
    pub overhead_setup_key: Arc<Mutex<String>>,
    /// Per-pad articulation toggle state. When true, the loader uses the
    /// alternate piece name (e.g. "ohne Teppich"). Persisted via plugin state.
    pub articulations: Arc<Mutex<[bool; drum_map::NUM_PADS]>>,
    /// Last-played round-robin display state. Written by the audio thread
    /// after each `note_on`, read by the editor for per-pad RR indicators.
    /// Packed as `rr_index | (n_rrs << 16)`; zero means "never triggered".
    pub last_rr: Arc<[AtomicU32; NUM_PADS]>,
}

pub struct ResonanceDrums {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. All `FloatParam` / `BoolParam`
    /// fields use atomic storage internally, so `&DrumParams` is safe to use
    /// concurrently from audio + UI.
    params: Arc<DrumParams>,
    sampler: DrumSampler,
    #[doc(hidden)]
    pub bridge: KitBridge,
    /// Download worker for fetching drumkits from the server. Only present
    /// in editor builds.
    #[cfg(feature = "editor")]
    download_worker: Arc<WorkerHandle>,
}

impl ResonancePlugin for ResonanceDrums {
    const CLAP_ID: &'static str = "com.resonance.drums";
    const NAME: &'static str = "Resonance Drums";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "A drum sampler instrument";
    const FEATURES: &'static [&'static str] = &["instrument", "sampler", "drum", "stereo"];

    const INPUT_CHANNELS: Option<u32> = None;
    const MIDI_INPUT: bool = true;

    fn new() -> Self {
        // SPSC-style handoff: audio thread is the sole consumer. Bound of 1
        // coalesces in-flight swaps so if the user spams Load Kit only the
        // newest loaded kit reaches the audio thread.
        let (kit_sender, kit_receiver): (Sender<Vec<LoadedPad>>, Receiver<Vec<LoadedPad>>) =
            bounded(1);
        let bridge = KitBridge {
            kit_path: Arc::new(Mutex::new(None)),
            kit_status: Arc::new(Mutex::new(KitStatus::Empty)),
            sample_rate: Arc::new(AtomicU32::new(0)),
            kit_sender,
            load_generation: Arc::new(AtomicU64::new(0)),
            catalog: Arc::new(Mutex::new(ManifestMicCatalog::default())),
            pad_choices: Arc::new(Mutex::new(std::array::from_fn(|_| {
                PadMicChoices::default()
            }))),
            overhead_setup_key: Arc::new(Mutex::new(DEFAULT_OVERHEAD_SETUP.to_string())),
            articulations: Arc::new(Mutex::new([false; drum_map::NUM_PADS])),
            last_rr: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
        };
        let mut sampler = DrumSampler::new(kit_receiver);
        sampler.set_last_rr(bridge.last_rr.clone());
        Self {
            params: Arc::new(DrumParams::default()),
            sampler,
            bridge,
            #[cfg(feature = "editor")]
            download_worker: Arc::new(download::spawn()),
        }
    }

    fn param_count(&self) -> usize {
        // master_volume + (volume, pan, mute, oh_blend, balance, articulation) per pad
        1 + drum_map::NUM_PADS * PARAMS_PER_PAD
    }

    fn param(&self, index: usize) -> &dyn Param {
        if index == 0 {
            return &self.params.master_volume;
        }
        let pad_idx = (index - 1) / PARAMS_PER_PAD;
        let field = (index - 1) % PARAMS_PER_PAD;
        let pad = &self.params.pads[pad_idx];
        match field {
            0 => &pad.volume,
            1 => &pad.pan,
            2 => &pad.mute,
            3 => &pad.oh_blend,
            4 => &pad.balance,
            5 => &pad.articulation,
            _ => &pad.volume,
        }
    }

    fn output_layout(&self) -> Vec<resonance_plugin::OutputPortSpec> {
        // 7 stereo output ports: Main + 5 drum groups + Overhead. See the
        // pad mapping in `drum_map.rs` for which pad feeds which port.
        [
            "Main", "Kick", "Snare", "Toms", "Hats", "Cymbals", "Overhead",
        ]
        .iter()
        .map(|name| resonance_plugin::OutputPortSpec {
            name: std::borrow::Cow::Borrowed(name),
            channel_count: 2,
        })
        .collect()
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.bridge
            .sample_rate
            .store(sample_rate.to_bits(), Ordering::Release);
        self.sampler.load_defaults(sample_rate);

        // If a kit path was set (either by a prior session via load_state or
        // by the editor) re-kick the loader at the current sample rate so the
        // kit decodes to the host's rate.
        let path = self.bridge.kit_path.lock().clone();
        if let Some(path) = path {
            let overhead_key = self.bridge.overhead_setup_key.lock().clone();
            let choices = self.bridge.pad_choices.lock().clone();
            let articulations = *self.bridge.articulations.lock();
            kit_loader::spawn_loader(
                path,
                sample_rate,
                &self.bridge,
                overhead_key,
                choices,
                articulations,
            );
        }

        true
    }

    fn reset(&mut self) {
        self.sampler.reset();
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        resonance_common::flush_denormals();

        // Swap in a freshly loaded kit if one is waiting.
        self.sampler.try_swap_kit();

        // Drain every pending MIDI event *before* rendering the block.
        // The new multi-output sampler renders whole blocks per voice
        // (not per frame) so sample-accurate timing within a block is
        // coarsened to block granularity. This matches how most hosts
        // run DAW-side instruments and is audibly indistinguishable for
        // typical drum programming.
        while let Some(event) = events.next_event() {
            match event {
                NoteEvent::NoteOn { note, velocity, .. } => {
                    self.sampler.note_on(note, velocity);
                }
                NoteEvent::NoteOff { note, .. } => {
                    self.sampler.note_off(note);
                }
                NoteEvent::Choke { note, .. } => {
                    self.sampler.choke_note(note);
                }
            }
        }

        // Project the CLAP bridge's `OutputBuffer` slice into the sampler's
        // `PortBuffers` shape on the stack. The plugin declares exactly
        // `NUM_OUTPUT_PORTS` output ports in its layout so the bridge is
        // guaranteed to hand us at least that many; bail if it doesn't
        // rather than panic on the audio thread.
        if outputs.len() < kit::NUM_OUTPUT_PORTS {
            return;
        }
        let mut out_iter = outputs.iter_mut();
        let mut port_views: [dsp::PortBuffers<'_>; kit::NUM_OUTPUT_PORTS] =
            std::array::from_fn(|_| {
                let out = out_iter.next().expect("checked len above");
                dsp::PortBuffers {
                    left: &mut *out.left,
                    right: &mut *out.right,
                }
            });
        drop(out_iter);

        // `render_block` zeroes ports, sums voices, and applies master
        // volume in one pass so this method can stay a thin shim.
        self.sampler
            .render_block(&mut port_views, frames, &self.params);
    }

    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        Some(Arc::new(DrumsExtraState {
            kit_path: self.bridge.kit_path.clone(),
            overhead_setup_key: self.bridge.overhead_setup_key.clone(),
            pad_choices: self.bridge.pad_choices.clone(),
            articulations: self.bridge.articulations.clone(),
        }))
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::DrumsEditorFactory::new(
            self.params.clone(),
            self.bridge.clone(),
            self.download_worker.clone(),
        )))
    }
}

/// Persists the drum plugin's kit path, the globally selected overhead
/// setup, per-pad close-mic picks, and per-pad articulation toggles
/// alongside the plugin's params.
/// The saver holds only shared Arcs so the CLAP bridge can call save/load
/// from the main thread while the plugin is in the audio processor
/// without touching audio-thread state.
#[doc(hidden)]
pub struct DrumsExtraState {
    pub kit_path: Arc<Mutex<Option<PathBuf>>>,
    pub overhead_setup_key: Arc<Mutex<String>>,
    pub pad_choices: Arc<Mutex<[PadMicChoices; drum_map::NUM_PADS]>>,
    pub articulations: Arc<Mutex<[bool; drum_map::NUM_PADS]>>,
}

impl ExtraStateSaver for DrumsExtraState {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        let path = self
            .kit_path
            .lock()
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        map.insert(
            "kit_path".to_string(),
            match path {
                Some(s) => serde_json::Value::String(s),
                None => serde_json::Value::Null,
            },
        );
        map.insert(
            "overhead_setup_key".to_string(),
            serde_json::Value::String(self.overhead_setup_key.lock().clone()),
        );
        // Per-pad close-mic choices as an array of `{position: setup_key}` maps.
        let choices = self.pad_choices.lock();
        let pads_array: Vec<serde_json::Value> = choices
            .iter()
            .map(|pc| {
                let entries: serde_json::Map<String, serde_json::Value> = pc
                    .close_setups
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                serde_json::Value::Object(entries)
            })
            .collect();
        map.insert(
            "pad_mic_choices".to_string(),
            serde_json::Value::Array(pads_array),
        );
        // Per-pad articulation toggles as an array of booleans.
        let arts = self.articulations.lock();
        let arts_array: Vec<serde_json::Value> =
            arts.iter().map(|&v| serde_json::Value::Bool(v)).collect();
        map.insert(
            "articulations".to_string(),
            serde_json::Value::Array(arts_array),
        );
        map
    }

    fn load(&self, state: &serde_json::Value) {
        // Always reassign so a null/missing `kit_path` clears any
        // previously remembered path on this instance. The actual loader
        // is spawned from `initialize()` because the sample rate isn't
        // known until the host activates the plugin.
        *self.kit_path.lock() = state
            .get("kit_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);

        if let Some(s) = state.get("overhead_setup_key").and_then(|v| v.as_str()) {
            *self.overhead_setup_key.lock() = s.to_string();
        }

        if let Some(arr) = state.get("pad_mic_choices").and_then(|v| v.as_array()) {
            let mut guard = self.pad_choices.lock();
            for (i, pad_val) in arr.iter().enumerate().take(drum_map::NUM_PADS) {
                let mut choices = PadMicChoices::default();
                if let Some(obj) = pad_val.as_object() {
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            choices.close_setups.insert(k.clone(), s.to_string());
                        }
                    }
                }
                guard[i] = choices;
            }
        }

        if let Some(arr) = state.get("articulations").and_then(|v| v.as_array()) {
            let mut guard = self.articulations.lock();
            for (i, val) in arr.iter().enumerate().take(drum_map::NUM_PADS) {
                if let Some(b) = val.as_bool() {
                    guard[i] = b;
                }
            }
        }
    }
}

resonance_plugin::export_clap!(ResonanceDrums);

