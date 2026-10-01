//! Voice management for polyphonic drum sample playback.

pub const MAX_VOICES: usize = 64;

/// Choke / release fade, in milliseconds: what a choke group (the open
/// hat cut by a closed or pedal hat) and a host choke fade over. Long
/// enough that a ringing cymbal is cut without a click, short enough
/// that the cut still reads as a cut.
pub const RELEASE_FADE_MS: f32 = 25.0;

/// Fade for voices still sounding when a new kit is swapped in, in
/// milliseconds. Kept short: those voices read the retired kit, which
/// is held in memory until they end. (CLAP `reset` does not fade: see
/// `DrumSampler::reset`.)
pub const SWAP_FADE_MS: f32 = 5.0;

/// A fade length in milliseconds as a whole number of frames at
/// `sample_rate`, never less than one — so a fade lasts the same time at
/// every rate rather than the same number of samples.
pub fn fade_frames(ms: f32, sample_rate: f32) -> u32 {
    ((ms * sample_rate / 1000.0).round() as u32).max(1)
}

/// The equal-power fade-out curve: `cos(t · π/2)` for `t` in `0..=1`.
/// Its slope is zero where the fade starts, so the gain has no corner at
/// the moment a choke lands, and the power `cos²` falls linearly.
pub fn fade_out_gain(t: f32) -> f32 {
    if t >= 1.0 {
        0.0
    } else {
        (t * std::f32::consts::FRAC_PI_2).cos()
    }
}

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
    /// Overhead mic bank, scaled by the per-pad `oh_blend` param.
    ///
    /// `output_port` is normally the shared Overhead port
    /// (`kit::OVERHEAD_PORT_INDEX`). The exception is a pad the library
    /// ships **no close mic for** — every cymbal, ride and china piece in
    /// Drummica is recorded on the overheads only. For those pads the
    /// overhead bank is the pad's *only* signal, so it is routed to the
    /// pad's own group port instead; otherwise that group's output port
    /// (and the sub-track the host creates for it) would be permanently
    /// silent. See `DrumSampler::note_on`.
    Overhead { output_port: u8 },
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
    /// True when this voice predates a kit swap and is fading out
    /// against the retired kit's sample data instead of the current
    /// `pads`.
    pub retired: bool,
    /// Which retired kit a `retired` voice reads (an index into the
    /// sampler's retired-kit slots). Meaningless while `retired` is false.
    pub retired_slot: u8,
    pub state: VoiceState,
    /// The gain at the moment release was triggered (for fade-out).
    pub release_gain: f32,
    /// Number of samples elapsed since release was triggered.
    pub release_pos: usize,
    /// Length of the release fade in samples, fixed when it starts (from
    /// a time in ms and the sample rate, see [`fade_frames`]).
    pub release_len: usize,
    /// Monotonic counter for voice-stealing (oldest first).
    pub age: u64,
}

impl Default for Voice {
    fn default() -> Self {
        Self::new()
    }
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
            retired: false,
            retired_slot: 0,
            state: VoiceState::Playing,
            release_gain: 0.0,
            release_pos: 0,
            release_len: 1,
            age: 0,
        }
    }

    /// Trigger release on this voice (fade-out to avoid clicks), over
    /// `len` frames. A voice already releasing keeps its fade.
    pub fn trigger_release(&mut self, len: u32) {
        if self.state == VoiceState::Playing {
            self.state = VoiceState::Releasing;
            self.release_gain = self.base_gain;
            self.release_pos = 0;
            self.release_len = len.max(1) as usize;
        }
    }

    /// Fade this voice out over at most `len` frames, starting from the
    /// gain it is at now. A voice already releasing restarts its fade
    /// from where it stands, never lengthening it: a 25 ms choke that a
    /// 3 ms steal lands on ends within 3 ms, and one with 1 ms left still
    /// ends in 1 ms.
    pub fn force_fade(&mut self, len: u32) {
        let len = len.max(1) as usize;
        let len = match self.state {
            VoiceState::Playing => len,
            VoiceState::Releasing => len.min(self.release_len.saturating_sub(self.release_pos)),
        };
        self.release_gain = self.current_gain();
        self.state = VoiceState::Releasing;
        self.release_pos = 0;
        self.release_len = len.max(1);
    }

    /// True once a releasing voice has run its fade to the end.
    pub fn release_done(&self) -> bool {
        self.state == VoiceState::Releasing && self.release_pos >= self.release_len
    }

    /// Compute the current gain for this voice, accounting for release envelope.
    /// Returns 0.0 if the voice should be deactivated.
    pub fn current_gain(&self) -> f32 {
        match self.state {
            VoiceState::Playing => self.base_gain,
            VoiceState::Releasing => {
                if self.release_pos >= self.release_len {
                    0.0
                } else {
                    let t = self.release_pos as f32 / self.release_len as f32;
                    self.release_gain * fade_out_gain(t)
                }
            }
        }
    }
}
