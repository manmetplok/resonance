/// Core drum sampler engine: sample loading, voice management, and audio rendering.

use crossbeam_channel::{Receiver, TryRecvError};

use crate::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use crate::kit::{self, LoadedPad, LoadedSample, VelocityLayer};
use crate::voice::{Voice, VoiceState, MAX_VOICES, RELEASE_SAMPLES};

/// Maximum velocity layer count we track per pad in the round-robin counter
/// array. Drummica's deepest pad has 28 layers, so 32 is a comfortable cap.
const MAX_LAYERS: usize = 32;

pub struct DrumSampler {
    pub pads: Vec<LoadedPad>,
    pub voices: Vec<Voice>,
    voice_counter: u64,
    /// Monotonic round-robin counter per (pad, layer). Advanced on each
    /// note_on; indexed modulo the layer's RR count to pick the next take.
    rr_counters: [[u32; MAX_LAYERS]; NUM_PADS],
    /// Receives new kit versions from the loader thread. The audio thread is
    /// the sole consumer; `try_recv` at the top of each process block swaps
    /// in a freshly loaded kit without blocking.
    kit_receiver: Receiver<Vec<LoadedPad>>,
}

impl DrumSampler {
    pub fn new(kit_receiver: Receiver<Vec<LoadedPad>>) -> Self {
        Self {
            pads: Vec::new(),
            voices: (0..MAX_VOICES).map(|_| Voice::new()).collect(),
            voice_counter: 0,
            rr_counters: [[0; MAX_LAYERS]; NUM_PADS],
            kit_receiver,
        }
    }

    /// Load the embedded default samples as a single-layer kit. Called once
    /// from `initialize()` so the plugin always boots with sound.
    pub fn load_defaults(&mut self, sample_rate: f32) {
        self.pads.clear();

        for mapping in &PAD_MAPPINGS {
            let pad = match kit::decode_wav(mapping.default_sample, sample_rate) {
                Ok(data) => LoadedPad {
                    note: mapping.note,
                    name: mapping.name.to_string(),
                    layers: vec![VelocityLayer {
                        round_robins: vec![LoadedSample::from_data(data)],
                    }],
                    choke_group: mapping.choke_group,
                },
                Err(e) => {
                    eprintln!("Failed to load sample for {}: {}", mapping.name, e);
                    // Push an empty pad so indices stay aligned.
                    LoadedPad {
                        note: mapping.note,
                        name: mapping.name.to_string(),
                        layers: Vec::new(),
                        choke_group: mapping.choke_group,
                    }
                }
            };
            self.pads.push(pad);
        }
    }

    /// Audio-thread: check for a freshly loaded kit and swap it in if one is
    /// waiting. Called once per `process()` call from `lib.rs`. Silences all
    /// active voices so no read references the old pad data.
    pub fn try_swap_kit(&mut self) {
        loop {
            match self.kit_receiver.try_recv() {
                Ok(new_pads) => {
                    for voice in &mut self.voices {
                        voice.active = false;
                    }
                    self.rr_counters = [[0; MAX_LAYERS]; NUM_PADS];
                    // The old Vec<LoadedPad> is dropped on the audio thread.
                    // Acceptable trade for v1: kit swaps are rare and
                    // user-initiated. A janitor thread can take this over
                    // later if it shows up in profiling.
                    self.pads = new_pads;
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Trigger a note-on event. Finds the matching pad, allocates a voice,
    /// and handles choke groups.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        let pad_index = match drum_map::pad_index_for_note(note) {
            Some(i) => i,
            None => return,
        };

        if pad_index >= self.pads.len() {
            return;
        }
        let pad = &self.pads[pad_index];
        if pad.layers.is_empty() {
            return;
        }

        // Pick the velocity layer: equal-width buckets across [0, 1].
        let n_layers = pad.layers.len();
        let layer_index = ((velocity.clamp(0.0, 1.0) * n_layers as f32) as usize)
            .min(n_layers - 1);
        let layer = &pad.layers[layer_index];
        if layer.round_robins.is_empty() {
            return;
        }

        // Pick the next round robin via the per-(pad, layer) counter.
        let counter_slot = layer_index.min(MAX_LAYERS - 1);
        let n_rrs = layer.round_robins.len();
        let rr_index = (self.rr_counters[pad_index][counter_slot] as usize) % n_rrs;
        self.rr_counters[pad_index][counter_slot] =
            self.rr_counters[pad_index][counter_slot].wrapping_add(1);

        // Baseline playback gain: for multi-layer pads the layer itself
        // captures the dynamics, so no velocity gain. For single-layer
        // fallback pads, keep the old behaviour.
        let trigger_gain = if n_layers > 1 { 1.0 } else { velocity };

        // Read pad data before borrowing self mutably
        let choke_group = pad.choke_group;

        // Handle choke groups: release any active voices in the same choke group
        if let Some(group) = choke_group {
            self.choke_group(group);
        }

        // Find a free voice, or steal the oldest one for the same pad,
        // or steal the oldest voice overall.
        let voice_idx = self.find_free_voice(pad_index);

        self.voice_counter += 1;
        let voice = &mut self.voices[voice_idx];
        voice.active = true;
        voice.pad_index = pad_index;
        voice.note = note;
        voice.velocity = trigger_gain;
        voice.layer_index = layer_index;
        voice.rr_index = rr_index;
        voice.position = 0;
        voice.choke_group = choke_group;
        voice.state = VoiceState::Playing;
        voice.release_gain = 1.0;
        voice.release_pos = 0;
        voice.age = self.voice_counter;
    }

    /// Trigger note-off for a given note. For drums, this triggers release
    /// (fade-out) on matching voices.
    pub fn note_off(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.active && voice.note == note && voice.state == VoiceState::Playing {
                voice.trigger_release();
            }
        }
    }

    /// Render a single stereo frame, summing all active voices.
    pub fn render_frame(
        &mut self,
        left: &mut f32,
        right: &mut f32,
        pad_volumes: &[f32; NUM_PADS],
        pad_pans: &[f32; NUM_PADS],
    ) {
        *left = 0.0;
        *right = 0.0;

        if self.pads.is_empty() {
            return;
        }

        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }

            let pad = &self.pads[voice.pad_index];
            if voice.layer_index >= pad.layers.len() {
                voice.active = false;
                continue;
            }
            let layer = &pad.layers[voice.layer_index];
            if voice.rr_index >= layer.round_robins.len() {
                voice.active = false;
                continue;
            }
            let sample = &layer.round_robins[voice.rr_index];

            // Check if voice has played past the end of the sample
            if voice.position >= sample.frames {
                voice.active = false;
                continue;
            }

            // Check if release envelope has completed
            if voice.state == VoiceState::Releasing && voice.release_pos >= RELEASE_SAMPLES {
                voice.active = false;
                continue;
            }

            // Read stereo sample at current position
            let idx = voice.position * 2;
            let sample_l = sample.data[idx];
            let sample_r = sample.data[idx + 1];

            // Apply voice gain (velocity + release envelope)
            let gain = voice.current_gain() * pad_volumes[voice.pad_index];

            // Constant-power pan law
            let (pan_l, pan_r) = resonance_dsp::constant_power_pan(pad_pans[voice.pad_index]);

            *left += sample_l * gain * pan_l;
            *right += sample_r * gain * pan_r;

            // Advance position
            voice.position += 1;
            if voice.state == VoiceState::Releasing {
                voice.release_pos += 1;
            }
        }
    }

    /// Release all voices in the given choke group.
    fn choke_group(&mut self, group: u8) {
        for voice in &mut self.voices {
            if voice.active && voice.choke_group == Some(group) {
                voice.trigger_release();
            }
        }
    }

    /// Find the best voice slot to use: free voice > oldest same-pad > oldest overall.
    fn find_free_voice(&self, pad_index: usize) -> usize {
        // Prefer an inactive voice
        if let Some(idx) = self.voices.iter().position(|v| !v.active) {
            return idx;
        }

        // Steal the oldest voice playing the same pad
        if let Some(idx) = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.pad_index == pad_index)
            .min_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
        {
            return idx;
        }

        // Steal the oldest voice overall
        self.voices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Kill all active voices immediately.
    pub fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.active = false;
        }
    }
}
