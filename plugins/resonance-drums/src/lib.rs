/// Resonance Drums - A drum sampler instrument CLAP plugin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crossbeam_channel::{bounded, Receiver, Sender};
use resonance_plugin::*;

#[cfg(feature = "editor")]
mod editor;
mod drum_map;
mod kit;
mod kit_loader;
mod params;
mod sampler;
mod voice;

use kit::LoadedPad;
use kit_loader::KitStatus;
use params::DrumParams;
use sampler::DrumSampler;

/// Shared state the loader thread, editor, and audio-thread plugin all need
/// handles to. Cheap to clone (all Arcs + one channel sender clone).
#[derive(Clone)]
pub(crate) struct KitBridge {
    /// Path to the currently loaded (or last-loaded) kit manifest. Set by
    /// the loader on success; persisted in `save_state`.
    pub kit_path: Arc<Mutex<Option<PathBuf>>>,
    /// Status reported by the loader, rendered by the editor.
    pub kit_status: Arc<Mutex<KitStatus>>,
    /// Host sample rate, captured in `initialize()`. Stored as `f32::to_bits`.
    pub sample_rate: Arc<AtomicU32>,
    /// Audio-thread kit handoff. Clones go to the editor and loader thread.
    pub kit_sender: Sender<Vec<LoadedPad>>,
}

pub struct ResonanceDrums {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. All `FloatParam` / `BoolParam`
    /// fields use atomic storage internally, so `&DrumParams` is safe to use
    /// concurrently from audio + UI.
    params: Arc<DrumParams>,
    sampler: DrumSampler,
    bridge: KitBridge,
}

impl ResonancePlugin for ResonanceDrums {
    const CLAP_ID: &'static str = "com.resonance.drums";
    const NAME: &'static str = "Resonance Drums";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "A drum sampler instrument";
    const FEATURES: &'static [&'static str] = &["instrument", "sampler", "drum", "stereo"];

    const INPUT_CHANNELS: Option<u32> = None;
    const OUTPUT_CHANNELS: u32 = 2;
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
            sample_rate: Arc::new(AtomicU32::new(44100f32.to_bits())),
            kit_sender,
        };
        Self {
            params: Arc::new(DrumParams::default()),
            sampler: DrumSampler::new(kit_receiver),
            bridge,
        }
    }

    fn param_count(&self) -> usize {
        1 + drum_map::NUM_PADS * 3 // master_volume + (volume, pan, mute) per pad
    }

    fn param(&self, index: usize) -> &dyn Param {
        if index == 0 {
            return &self.params.master_volume;
        }
        let pad_idx = (index - 1) / 3;
        let field = (index - 1) % 3;
        let pad = &self.params.pads[pad_idx];
        match field {
            0 => &pad.volume,
            1 => &pad.pan,
            2 => &pad.mute,
            _ => unreachable!(),
        }
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.bridge
            .sample_rate
            .store(sample_rate.to_bits(), Ordering::Relaxed);
        self.sampler.load_defaults(sample_rate);

        // If a kit path was set (either by a prior session via load_state or
        // by the editor) re-kick the loader at the current sample rate so the
        // kit decodes to the host's rate.
        let path = self.bridge.kit_path.lock().unwrap().clone();
        if let Some(path) = path {
            kit_loader::spawn_loader(
                path,
                sample_rate,
                self.bridge.kit_path.clone(),
                self.bridge.kit_status.clone(),
                self.bridge.kit_sender.clone(),
            );
        }

        true
    }

    fn reset(&mut self) {
        self.sampler.reset();
    }

    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        events: &mut EventIterator<'_>,
    ) {
        resonance_common::flush_denormals();

        // Swap in a freshly loaded kit if one is waiting.
        self.sampler.try_swap_kit();

        // Read per-pad parameters
        let mut pad_volumes = [0.0f32; drum_map::NUM_PADS];
        let mut pad_pans = [0.0f32; drum_map::NUM_PADS];
        for (i, pad) in self.params.pads.iter().enumerate() {
            pad_volumes[i] = if pad.mute.value() {
                0.0
            } else {
                pad.volume.value()
            };
            pad_pans[i] = pad.pan.value();
        }
        let master_vol = self.params.master_volume.value();

        // Sample-accurate MIDI processing
        let mut next_event = events.next_event();

        for sample_id in 0..frames {
            // Process all MIDI events at this sample position
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.sampler.note_on(note, velocity);
                    }
                    NoteEvent::NoteOff { note, .. } => {
                        self.sampler.note_off(note);
                    }
                    NoteEvent::Choke { note, .. } => {
                        self.sampler.note_off(note);
                    }
                }

                next_event = events.next_event();
            }

            // Render one stereo frame from the sampler
            let mut frame_l = 0.0f32;
            let mut frame_r = 0.0f32;
            self.sampler
                .render_frame(&mut frame_l, &mut frame_r, &pad_volumes, &pad_pans);

            // Write to output with master volume
            left[sample_id] = frame_l * master_vol;
            right[sample_id] = frame_r * master_vol;
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut json = resonance_plugin::state::params_to_json(&self.params());
        let kit_path = self
            .bridge
            .kit_path
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        if let Some(obj) = json.as_object_mut() {
            obj.insert(
                "kit_path".to_string(),
                match kit_path {
                    Some(s) => serde_json::Value::String(s),
                    None => serde_json::Value::Null,
                },
            );
        }
        serde_json::to_vec(&json).unwrap_or_default()
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        let Ok(state) = serde_json::from_slice::<serde_json::Value>(data) else {
            return false;
        };
        let params_ok =
            resonance_plugin::state::load_params_from_json(&self.params(), &state);

        if let Some(path_str) = state.get("kit_path").and_then(|v| v.as_str()) {
            *self.bridge.kit_path.lock().unwrap() = Some(PathBuf::from(path_str));
            // Don't spawn the loader yet — sample rate isn't known until
            // `initialize()`. The initialize hook picks this up and kicks the
            // load with the correct rate.
        }

        params_ok
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::DrumsEditorFactory::new(
            self.params.clone(),
            self.bridge.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceDrums);
