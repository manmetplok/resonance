//! Resonance Drums - A drum sampler instrument CLAP plugin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
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
use resonance_plugin::plugin::ExtraStateSaver;
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
    /// Sentinel `0` means "not yet initialized — no audio rate is known".
    pub sample_rate: Arc<AtomicU32>,
    /// Audio-thread kit handoff. Clones go to the editor and loader thread.
    pub kit_sender: Sender<Vec<LoadedPad>>,
    /// Monotonic load stamp. Incremented each time a new loader is spawned;
    /// in-flight loaders check this before writing status/kit_path so a
    /// stale load can't clobber a newer one.
    pub load_generation: Arc<AtomicU64>,
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
            sample_rate: Arc::new(AtomicU32::new(0)),
            kit_sender,
            load_generation: Arc::new(AtomicU64::new(0)),
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
            .store(sample_rate.to_bits(), Ordering::Release);
        self.sampler.load_defaults(sample_rate);

        // If a kit path was set (either by a prior session via load_state or
        // by the editor) re-kick the loader at the current sample rate so the
        // kit decodes to the host's rate.
        let path = self.bridge.kit_path.lock().unwrap().clone();
        if let Some(path) = path {
            kit_loader::spawn_loader(path, sample_rate, &self.bridge);
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

    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        Some(Arc::new(DrumsExtraState {
            kit_path: self.bridge.kit_path.clone(),
        }))
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::DrumsEditorFactory::new(
            self.params.clone(),
            self.bridge.clone(),
        )))
    }
}

/// Persists the currently-loaded kit path alongside the plugin's params.
/// The saver holds only the shared `kit_path` Arc, so the CLAP bridge can
/// call `save`/`load` from the main thread while the plugin is in the
/// audio processor without touching audio-thread state.
struct DrumsExtraState {
    kit_path: Arc<Mutex<Option<PathBuf>>>,
}

impl ExtraStateSaver for DrumsExtraState {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        let path = self
            .kit_path
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        map.insert(
            "kit_path".to_string(),
            match path {
                Some(s) => serde_json::Value::String(s),
                None => serde_json::Value::Null,
            },
        );
        map
    }

    fn load(&self, state: &serde_json::Value) {
        // Always reassign so a null/missing `kit_path` clears any
        // previously remembered path on this instance. The actual loader
        // is spawned from `initialize()` because the sample rate isn't
        // known until the host activates the plugin.
        *self.kit_path.lock().unwrap() = state
            .get("kit_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
    }
}

resonance_plugin::export_clap!(ResonanceDrums);

#[cfg(test)]
mod tests {
    use super::*;

    /// save_state → load_state round-trip preserves a kit path.
    /// Exercises the main-thread path where the host calls save_state /
    /// load_state on the owned plugin instance.
    #[test]
    fn state_roundtrip_preserves_kit_path() {
        let src = ResonanceDrums::new();
        *src.bridge.kit_path.lock().unwrap() =
            Some(PathBuf::from("/some/kit/drum_samples.json"));

        let bytes = src.save_state();

        let mut dst = ResonanceDrums::new();
        assert!(dst.load_state(&bytes));
        let restored = dst.bridge.kit_path.lock().unwrap().clone();
        assert_eq!(
            restored,
            Some(PathBuf::from("/some/kit/drum_samples.json"))
        );
    }

    /// save_state with no kit followed by load_state clears any prior path.
    #[test]
    fn load_state_null_clears_kit_path() {
        let src = ResonanceDrums::new();
        let bytes = src.save_state(); // kit_path is None, serializes as null

        let mut dst = ResonanceDrums::new();
        // Pre-populate a stale path; load_state should clear it.
        *dst.bridge.kit_path.lock().unwrap() = Some(PathBuf::from("/stale/path.json"));

        assert!(dst.load_state(&bytes));
        assert_eq!(*dst.bridge.kit_path.lock().unwrap(), None);
    }

    /// Round-trip through the `ExtraStateSaver` interface directly. This
    /// simulates what the CLAP bridge does when the plugin is in the audio
    /// processor and the host asks for a state save — the owned plugin
    /// isn't reachable, so the bridge talks to the cached saver instead.
    /// This is exactly the path that used to silently drop kit_path at
    /// project save time before the framework fix.
    #[test]
    fn extra_saver_roundtrip_active_path() {
        // Construct the saver the same way editor_factory / new() would,
        // holding a shared Arc<Mutex<Option<PathBuf>>>.
        let kit_path = Arc::new(Mutex::new(Some(PathBuf::from(
            "/active/path/drum_samples.json",
        ))));
        let saver = DrumsExtraState {
            kit_path: kit_path.clone(),
        };

        // Serialize — this is what clap_bridge::save() would do on the
        // plugin-is-None branch.
        let mut json = serde_json::json!({ "params": {} });
        for (k, v) in saver.save() {
            json.as_object_mut().unwrap().insert(k, v);
        }

        // New instance with a different shared storage — clear to start.
        let restored_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let restored_saver = DrumsExtraState {
            kit_path: restored_path.clone(),
        };

        // Load from the serialized state.
        restored_saver.load(&json);

        assert_eq!(
            *restored_path.lock().unwrap(),
            Some(PathBuf::from("/active/path/drum_samples.json")),
            "kit_path should round-trip through the saver"
        );
    }

    /// A loaded null kit_path through the saver clears previously stored path.
    #[test]
    fn extra_saver_null_clears_active_path() {
        let kit_path = Arc::new(Mutex::new(Some(PathBuf::from("/stale.json"))));
        let saver = DrumsExtraState {
            kit_path: kit_path.clone(),
        };

        // State without a kit_path (simulating a save with no kit loaded).
        let state = serde_json::json!({ "params": {}, "kit_path": serde_json::Value::Null });
        saver.load(&state);
        assert_eq!(*kit_path.lock().unwrap(), None);
    }
}
