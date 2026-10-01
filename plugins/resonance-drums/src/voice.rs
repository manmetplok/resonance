//! Voice management for polyphonic drum sample playback.

pub const MAX_VOICES: usize = 64;

/// Extra slots a stolen voice is moved into so it can fade out instead of
/// being overwritten mid-sample (E1). They sit outside the polyphony
/// count: a steal still frees the main slot at once, and the victim only
/// needs [`STEAL_FADE_MS`] to die away.
///
/// Steals are not spread out in time: they bunch on the frames hits land
/// on. One hit takes up to three voices (two close mics + overhead), a
/// host delivers a flam, a fill or a pad "chord" as several hits on the
/// same frame, and a low polyphony ceiling makes every one of those a
/// steal. So the count that matters is how many *sounding* voices can be
/// stolen within one fade (144 frames at 48 kHz), and that is bounded by
/// how many were sounding — up to [`MAX_VOICES`] — not by a hit rate.
/// Thirty-two covers half the voice pool going at once; past that the
/// quietest tail is reused (of equally loud ones, the least heard), which
/// is the least audible cut there is.
pub const TAIL_SLOTS: usize = 32;

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

/// Fade for a voice stolen to make room for a new hit, in milliseconds.
/// The shortest fade: the victim shares the output with the hit
/// replacing it, and 3 ms of equal-power fade is already well clear of
/// a click.
pub const STEAL_FADE_MS: f32 = 3.0;

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
    /// `pad.close_mics`, and picks the trim that scales the voice
    /// (`pad_N_mic1_trim` for bank 0, `pad_N_mic2_trim` for bank 1).
    /// `output_port` is the plugin output port this bank routes to.
    CloseMic { bank_index: usize, output_port: u8 },
    /// Overhead mic bank, scaled by the per-pad `pad_N_oh_trim` param.
    ///
    /// `output_port` is the shared Overhead port
    /// (`kit::OVERHEAD_PORT_INDEX`) in Multi output mode for a pad with
    /// close mics; a pad with none (the overhead-only cymbals) keeps its
    /// overhead take on its own port, since it is the pad's sound (E11).
    /// Main in Stereo. See `DrumSampler::note_on`.
    Overhead { output_port: u8 },
}

/// `Copy`: a voice is plain data, so moving a stolen one into a tail slot
/// is a struct copy on the audio thread, never an allocation.
#[derive(Clone, Copy)]
pub struct Voice {
    pub active: bool,
    pub pad_index: usize,
    pub note: u8,
    /// Baseline gain applied throughout playback. For multi-layer pads it
    /// is what puts the chosen layer at the level the velocity asks for
    /// (E7, `dsp::pick_layer_by_level`: 1.0 at the ends of the range, at
    /// most half a layer gap either way between); for single-layer
    /// fallback pads it's the MIDI velocity so the embedded defaults
    /// still scale with how hard the note was hit.
    pub base_gain: f32,
    /// Where this voice's audio should be summed.
    pub destination: VoiceDestination,
    /// Index into the selected bank's `layers`.
    pub layer_index: usize,
    /// Index into `layers[layer_index].round_robins`.
    pub rr_index: usize,
    /// Current read position in the sample (in stereo frames): the whole
    /// part, for a pitched voice (E8).
    pub position: usize,
    /// Take frames read per output frame (E8, from `pad_N_tune`). Exactly
    /// `1.0` at pitch, which plays on the integer path, bit for bit.
    pub rate: f32,
    /// The fractional part of a pitched voice's position, `0..1`.
    pub frac: f32,
    /// Output frames since the hit — the AHD envelope's clock (E8).
    pub env_pos: u32,
    /// The AHD envelope (E8), in output frames: the hold, then the decay
    /// to silence. `decay_frames == 0` is no envelope (the whole sample).
    pub hold_frames: u32,
    pub decay_frames: u32,
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
    /// The stream ring this voice reads its take's tail from (E14), or
    /// [`crate::stream::NO_RING`] for a take that is wholly in memory —
    /// or a streamed one that found no free ring, which then ends with
    /// its head. Moves with the voice when it is stolen to a tail slot.
    pub ring: u8,
    /// The voice's tail will not come — its hit found no ring, or its
    /// stream failed — so it fades out where its frames end. Set (and
    /// counted as one underrun) once.
    pub stream_lost: bool,
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
            },
            layer_index: 0,
            rr_index: 0,
            position: 0,
            rate: 1.0,
            frac: 0.0,
            env_pos: 0,
            hold_frames: 0,
            decay_frames: 0,
            choke_group: None,
            retired: false,
            retired_slot: 0,
            state: VoiceState::Playing,
            release_gain: 0.0,
            release_pos: 0,
            release_len: 1,
            age: 0,
            ring: crate::stream::NO_RING,
            stream_lost: false,
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

    /// Make sure the voice has faded out `left` frames from now: one that
    /// would still be sounding then starts (or shortens) its fade to end
    /// there. For a voice whose frames run out early (E14).
    #[inline]
    pub fn end_within(&mut self, left: usize) {
        let ends_in_time = self.state == VoiceState::Releasing
            && self.release_len.saturating_sub(self.release_pos) <= left;
        if !ends_in_time {
            self.force_fade(left.min(u32::MAX as usize) as u32);
        }
    }

    /// The AHD envelope's gain now (E8): 1 through the hold, then a
    /// cubic fall, `(1 − t)³`, that reaches silence with a zero slope at
    /// the end of the decay (−18 dB half way). Only called with a decay
    /// set.
    #[inline]
    pub fn ahd_gain(&self) -> f32 {
        let Some(into) = self.env_pos.checked_sub(self.hold_frames) else {
            return 1.0;
        };
        if into >= self.decay_frames {
            return 0.0;
        }
        let left = 1.0 - into as f32 / self.decay_frames as f32;
        left * left * left
    }

    /// True once the AHD envelope has decayed to silence.
    #[inline]
    pub fn ahd_done(&self) -> bool {
        self.decay_frames > 0 && self.env_pos >= self.hold_frames.saturating_add(self.decay_frames)
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

    /// What the voice plays at now, envelope and all: [`current_gain`]
    /// times the AHD envelope when a decay is set (E8). What "quietest"
    /// means when a voice must be cut.
    ///
    /// [`current_gain`]: Voice::current_gain
    #[inline]
    pub fn audible_gain(&self) -> f32 {
        let gain = self.current_gain();
        if self.decay_frames > 0 {
            gain * self.ahd_gain()
        } else {
            gain
        }
    }
}
