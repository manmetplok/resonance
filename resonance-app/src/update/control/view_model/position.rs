//! Time projections: where the transport is, and how far the song runs.
//!
//! Shared by `song.*`, `transport.*`, `clip.*` and `meter.*` — every
//! method that reports or clamps a position has to agree with the
//! others, so they all come through here.

use crate::Resonance;
use resonance_control::{SongPosition, TransportState};

/// Resolve a sample position into the protocol's bar/beat/sample triple
/// (1-based bar, 1-based fractional beat).
pub(in crate::update::control) fn song_position(app: &Resonance, sample: u64) -> SongPosition {
    let (bar, beat, frac) = app.tempo_map.position_to_bars(sample, app.sample_rate);
    SongPosition {
        bar,
        beat: beat as f64 + frac,
        sample,
    }
}

/// The transport's wire state. Recording outranks playing: a recording
/// transport is also rolling, and the client needs the stronger fact.
pub(in crate::update::control) fn transport_state(app: &Resonance) -> TransportState {
    if app.transport.recording {
        TransportState::Recording
    } else if app.transport.playing {
        TransportState::Playing
    } else {
        TransportState::Stopped
    }
}

/// Last sample of the song: the furthest end over audio clips, MIDI
/// clips, and placed sections. 0 for an empty project.
///
/// Shared with `meter.*` (todo #1219), which clamps a measurement range
/// to it, so "the whole song" means the same thing to a reader and to a
/// measurement.
pub(in crate::update::control) fn song_end_sample(app: &Resonance) -> u64 {
    let mut end: u64 = 0;
    for c in &app.clips {
        end = end.max(c.start_sample + c.duration_samples);
    }
    for m in &app.midi_clips {
        end = end.max(app.tempo_map.tick_to_abs_sample(
            m.start_sample,
            m.duration_ticks,
            app.sample_rate,
        ));
    }
    for p in &app.compose.placements {
        if let Some(def) = app.compose.definitions.iter().find(|d| d.id == p.definition_id) {
            end = end.max(app.tempo_map.bar_to_sample(p.start_bar + def.length_bars));
        }
    }
    end
}
