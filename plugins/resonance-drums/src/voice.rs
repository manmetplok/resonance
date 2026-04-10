//! Voice management for polyphonic drum sample playback.

pub const MAX_VOICES: usize = 64;

/// Fade-out length in samples to avoid clicks when a voice is choked or released.
pub const RELEASE_SAMPLES: usize = 1024;

#[derive(Clone, Copy, PartialEq)]
pub enum VoiceState {
    Playing,
    Releasing,
}

/// Which bank this voice is reading from and where it should be summed
/// at render time.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VoiceDestination {
    /// One of the pad's close-mic banks. `bank_index` is the index into
    /// `pad.close_mics`. `output_port` is the plugin output port this
    /// bank routes to (from `pad.output_group`). `balance_side` determines
    /// how the kick In/Out or snare Top/Btm balance slider scales this
    /// voice.
    CloseMic {
        bank_index: usize,
        output_port: u8,
        balance_side: BalanceSide,
    },
    /// Overhead mic bank. Always routes to the shared Overhead output
    /// port (6) and is scaled by the per-pad `oh_blend` param.
    Overhead,
}

/// Which "side" of a balance slider this close-mic voice represents.
/// For kick: `Left` = KickIn, `Right` = KickOut. For snare: `Left` = SNTop,
/// `Right` = SNBtm. `None` for pads with only one close mic position
/// (toms, hats) — the balance slider doesn't apply.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BalanceSide {
    None,
    Left,
    Right,
}

#[derive(Clone)]
pub struct Voice {
    pub active: bool,
    pub pad_index: usize,
    pub note: u8,
    /// Baseline gain applied throughout playback. For multi-layer pads this
    /// is 1.0 because the chosen velocity layer already captures the
    /// dynamics; for single-layer fallback pads it's the MIDI velocity so
    /// the embedded defaults still scale with how hard the note was hit.
    pub base_gain: f32,
    /// Where this voice's audio should be summed.
    pub destination: VoiceDestination,
    /// Index into the selected bank's `layers`.
    pub layer_index: usize,
    /// Index into `layers[layer_index].round_robins`.
    pub rr_index: usize,
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
            base_gain: 0.0,
            destination: VoiceDestination::CloseMic {
                bank_index: 0,
                output_port: 0,
                balance_side: BalanceSide::None,
            },
            layer_index: 0,
            rr_index: 0,
            position: 0,
            choke_group: None,
            state: VoiceState::Playing,
            release_gain: 0.0,
            release_pos: 0,
            age: 0,
        }
    }

    /// Trigger release on this voice (fade-out to avoid clicks).
    pub fn trigger_release(&mut self) {
        if self.state == VoiceState::Playing {
            self.state = VoiceState::Releasing;
            self.release_gain = self.base_gain;
            self.release_pos = 0;
        }
    }

    /// Compute the current gain for this voice, accounting for release envelope.
    /// Returns 0.0 if the voice should be deactivated.
    pub fn current_gain(&self) -> f32 {
        match self.state {
            VoiceState::Playing => self.base_gain,
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
