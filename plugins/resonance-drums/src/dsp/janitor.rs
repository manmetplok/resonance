//! Voice cleanup, stealing, and off-audio-thread heap-free utilities.
//!
//! The heap-free janitor runs on a dedicated background thread; when the
//! audio thread swaps a freshly loaded kit in, it hands the old
//! `Vec<LoadedPad>` to this janitor so the (potentially large) heap free
//! happens off-audio. The voice-cleanup helpers run on the audio thread
//! itself and deal with stealing voice slots when all MAX_VOICES are in
//! use, releasing groups for chokes, and resetting state on host stop.

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::kit::LoadedPad;
use crate::voice::{Voice, VoiceState};

/// How many retired kits can wait for the janitor at once. The channel
/// is `bounded`, so its slots are allocated here, up front: a send from
/// the audio thread is a copy into a preallocated ring and never
/// allocates. (An `unbounded` channel allocates a new block every 31
/// sends — on the sending thread.) The audio thread retires at most one
/// kit per swap and holds at most `RETIRED_KITS` fading ones, so eight
/// is more than it can produce while the janitor is draining; a full
/// channel only means the kit stays parked a block longer.
pub const JANITOR_DEPTH: usize = 8;

/// Spawn the heap-free janitor thread and return the sender used to ship
/// retired kits to it. Called once from `DrumSampler::new`. The janitor
/// exits cleanly once all `Sender`s drop.
pub fn spawn() -> Sender<Vec<LoadedPad>> {
    let (janitor_sender, janitor_receiver): (
        Sender<Vec<LoadedPad>>,
        Receiver<Vec<LoadedPad>>,
    ) = bounded(JANITOR_DEPTH);
    std::thread::Builder::new()
        .name("resonance-drums-janitor".to_string())
        .spawn(move || {
            // Block on recv; each received Vec is dropped here, off the
            // audio thread. Exits when all senders disconnect.
            while janitor_receiver.recv().is_ok() {}
        })
        .expect("spawn drums janitor thread");
    janitor_sender
}

/// Find the best voice slot to use: free voice > oldest same-pad > oldest overall.
///
/// `max_voices` is the polyphony ceiling (ba todo #1326). A free slot is
/// only taken while fewer than that many voices are sounding; past the
/// ceiling the same stealing rules apply as when every slot is busy, so
/// turning polyphony down thins the kit rather than dropping hits. At
/// the default (`MAX_VOICES`) this is exactly the old behaviour: there
/// is a free slot if and only if fewer than `MAX_VOICES` are active.
pub(super) fn find_free_voice(voices: &[Voice], pad_index: usize, max_voices: usize) -> usize {
    let active_count = voices.iter().filter(|v| v.active).count();

    // Prefer an inactive voice, as long as we're under the ceiling
    if active_count < max_voices {
        if let Some(idx) = voices.iter().position(|v| !v.active) {
            return idx;
        }
    }

    // Steal the oldest voice playing the same pad. Only *active* voices
    // are candidates: below the ceiling an idle slot would otherwise win
    // on age and quietly lift the polyphony limit. When every slot is
    // busy — the only case before the limit existed — every voice is
    // active, so this filter changes nothing.
    if let Some(idx) = voices
        .iter()
        .enumerate()
        .filter(|(_, v)| v.active && v.pad_index == pad_index)
        .min_by_key(|(_, v)| v.age)
        .map(|(i, _)| i)
    {
        return idx;
    }

    // Steal the oldest voice overall
    voices
        .iter()
        .enumerate()
        .filter(|(_, v)| v.active)
        .min_by_key(|(_, v)| v.age)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Release all voices in the given choke group, fading over `len` frames.
pub(super) fn choke_group(voices: &mut [Voice], group: u8, len: u32) {
    for voice in voices.iter_mut() {
        if voice.active && voice.choke_group == Some(group) {
            voice.trigger_release(len);
        }
    }
}

/// Kill all active voices immediately (CLAP `reset`; see
/// `DrumSampler::reset` for why this one does not fade).
pub(super) fn reset_all(voices: &mut [Voice]) {
    for voice in voices.iter_mut() {
        voice.active = false;
    }
}

/// Host-level "silence this note now" — used by the CLAP host when
/// playback stops or a track is muted mid-hit. Fades the matching
/// voices out over `len` frames rather than clicking them off.
pub(super) fn choke_note(voices: &mut [Voice], note: u8, len: u32) {
    for voice in voices.iter_mut() {
        if voice.active && voice.note == note && voice.state == VoiceState::Playing {
            voice.trigger_release(len);
        }
    }
}
