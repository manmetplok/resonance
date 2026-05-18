//! Voice cleanup, stealing, and off-audio-thread heap-free utilities.
//!
//! The heap-free janitor runs on a dedicated background thread; when the
//! audio thread swaps a freshly loaded kit in, it hands the old
//! `Vec<LoadedPad>` to this janitor so the (potentially large) heap free
//! happens off-audio. The voice-cleanup helpers run on the audio thread
//! itself and deal with stealing voice slots when all MAX_VOICES are in
//! use, releasing groups for chokes, and resetting state on host stop.

use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::kit::LoadedPad;
use crate::voice::{Voice, VoiceState};

/// Spawn the heap-free janitor thread and return the sender used to ship
/// retired kits to it. Called once from `DrumSampler::new`. The janitor
/// exits cleanly once all `Sender`s drop.
pub fn spawn() -> Sender<Vec<LoadedPad>> {
    let (janitor_sender, janitor_receiver): (
        Sender<Vec<LoadedPad>>,
        Receiver<Vec<LoadedPad>>,
    ) = unbounded();
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
pub(super) fn find_free_voice(voices: &[Voice], pad_index: usize) -> usize {
    // Prefer an inactive voice
    if let Some(idx) = voices.iter().position(|v| !v.active) {
        return idx;
    }

    // Steal the oldest voice playing the same pad
    if let Some(idx) = voices
        .iter()
        .enumerate()
        .filter(|(_, v)| v.pad_index == pad_index)
        .min_by_key(|(_, v)| v.age)
        .map(|(i, _)| i)
    {
        return idx;
    }

    // Steal the oldest voice overall
    voices
        .iter()
        .enumerate()
        .min_by_key(|(_, v)| v.age)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Release all voices in the given choke group.
pub(super) fn choke_group(voices: &mut [Voice], group: u8) {
    for voice in voices.iter_mut() {
        if voice.active && voice.choke_group == Some(group) {
            voice.trigger_release();
        }
    }
}

/// Kill all active voices immediately.
pub(super) fn reset_all(voices: &mut [Voice]) {
    for voice in voices.iter_mut() {
        voice.active = false;
    }
}

/// Host-level "silence this note now" — used by the CLAP host when
/// playback stops or a track is muted mid-hit. Fades the matching
/// voices out rather than clicking them off.
pub(super) fn choke_note(voices: &mut [Voice], note: u8) {
    for voice in voices.iter_mut() {
        if voice.active && voice.note == note && voice.state == VoiceState::Playing {
            voice.trigger_release();
        }
    }
}
