/// Core drum sampler engine: sample loading, voice management, and audio rendering.

use crate::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use crate::kit::{self, LoadedPad};
use crate::voice::{Voice, VoiceState, MAX_VOICES, RELEASE_SAMPLES};

pub struct DrumSampler {
    pub pads: Vec<LoadedPad>,
    pub voices: Vec<Voice>,
    voice_counter: u64,
}

impl DrumSampler {
    pub fn new() -> Self {
        Self {
            pads: Vec::new(),
            voices: (0..MAX_VOICES).map(|_| Voice::new()).collect(),
            voice_counter: 0,
        }
    }

    /// Load the embedded default samples, resampled to the host sample rate.
    pub fn load_defaults(&mut self, sample_rate: f32) {
        self.pads.clear();

        for mapping in &PAD_MAPPINGS {
            match kit::decode_wav(mapping.default_sample, sample_rate) {
                Ok(data) => {
                    let frames = data.len() / 2;
                    self.pads.push(LoadedPad {
                        note: mapping.note,
                        name: mapping.name.to_string(),
                        sample_data: data,
                        sample_frames: frames,
                        choke_group: mapping.choke_group,
                    });
                }
                Err(e) => {
                    eprintln!("Failed to load sample for {}: {}", mapping.name, e);
                    // Push an empty pad so indices stay aligned
                    self.pads.push(LoadedPad {
                        note: mapping.note,
                        name: mapping.name.to_string(),
                        sample_data: Vec::new(),
                        sample_frames: 0,
                        choke_group: mapping.choke_group,
                    });
                }
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

        if pad_index >= self.pads.len() || self.pads[pad_index].sample_frames == 0 {
            return;
        }

        // Read pad data before borrowing self mutably
        let choke_group = self.pads[pad_index].choke_group;

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
        voice.velocity = velocity;
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

            // Check if voice has played past the end of the sample
            if voice.position >= pad.sample_frames {
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
            let sample_l = pad.sample_data[idx];
            let sample_r = pad.sample_data[idx + 1];

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
