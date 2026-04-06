/// Voice management for polyphonic drum sample playback.

pub const MAX_VOICES: usize = 32;

/// Fade-out length in samples to avoid clicks when a voice is choked or released.
pub const RELEASE_SAMPLES: usize = 1024;

#[derive(Clone, Copy, PartialEq)]
pub enum VoiceState {
    Playing,
    Releasing,
}

#[derive(Clone)]
pub struct Voice {
    pub active: bool,
    pub pad_index: usize,
    pub note: u8,
    pub velocity: f32,
    /// Current read position in the sample (in stereo frames).
    pub position: usize,
    pub choke_group: Option<u8>,
    pub state: VoiceState,
    /// The gain at the moment release was triggered (for fade-out).
    pub release_gain: f32,
    /// Number of samples elapsed since release was triggered.
    pub release_pos: usize,
    /// Monotonic counter for voice-stealing (oldest first).
    pub age: u64,
}

impl Voice {
    pub fn new() -> Self {
        Self {
            active: false,
            pad_index: 0,
            note: 0,
            velocity: 0.0,
            position: 0,
            choke_group: None,
            state: VoiceState::Playing,
            release_gain: 1.0,
            release_pos: 0,
            age: 0,
        }
    }

    /// Trigger release on this voice (fade-out to avoid clicks).
    pub fn trigger_release(&mut self) {
        if self.state == VoiceState::Playing {
            self.state = VoiceState::Releasing;
            self.release_gain = self.velocity;
            self.release_pos = 0;
        }
    }

    /// Compute the current gain for this voice, accounting for release envelope.
    /// Returns 0.0 if the voice should be deactivated.
    pub fn current_gain(&self) -> f32 {
        match self.state {
            VoiceState::Playing => self.velocity,
            VoiceState::Releasing => {
                if self.release_pos >= RELEASE_SAMPLES {
                    0.0
                } else {
                    let t = self.release_pos as f32 / RELEASE_SAMPLES as f32;
                    self.release_gain * (1.0 - t)
                }
            }
        }
    }
}
