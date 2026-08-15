//! App state -> control-wire projection (ba todo #1256).
//!
//! One place where `Resonance` becomes the compact, LLM-oriented view
//! types of `resonance-control`: real app ids, musical + sample
//! positions, lowercase enums, no UI/view state. It is a *library*, not
//! a namespace handler — `song.*` reads it, but so do `transport.*`,
//! `clip.*`, `meter.*` and the `track.*` families, which is exactly why
//! it does not live inside [`song`](super::song) anymore.
//!
//! Split by the entity being projected:
//!
//! | module | projects |
//! |---|---|
//! | [`position`] | playhead / clip positions, transport state, song length |
//! | [`section`] | section placements, definitions in arrangement order |
//! | [`track`] | track + bus summaries, routing, plugin chain, track detail |
//! | [`clip`] | a track's audio + MIDI clips, clip counts |
//! | [`lane`] | vocal lanes: clip, note count, articulation, render state |
//!
//! The unit conversions the wire protocol fixes (dB -> linear gain,
//! 0.0..=1.0 velocity -> MIDI 0..=127, the app's `Scale` -> `KeyScale`)
//! sit here at the root, since every projection above may need them and
//! none of them owns one.
//!
//! Nothing in here mutates or dispatches: every function takes
//! `&Resonance` and returns a wire type.

use crate::Resonance;
use resonance_control::KeyScale;

mod clip;
mod lane;
mod position;
mod section;
mod track;

// Flat re-exports so a caller writes `view_model::song_position(..)` and
// never has to know which entity file it lives in. Only the projections
// the handlers actually call are re-exported; the rest (`clip_count`,
// `track_clip_views`, `track_summary`, `lane_clip`) are ingredients the
// entity modules combine for each other.
pub(in crate::update::control) use lane::{
    lane_articulation, lane_note_count, vocal_render_state,
};
pub(in crate::update::control) use position::{song_end_sample, song_position, transport_state};
pub(in crate::update::control) use section::{definitions_in_placement_order, placement_views};
pub(in crate::update::control) use track::{
    param_view, plugin_entries, track_detail, track_summaries, unknown_plugin_on_track,
};

/// Ticks per quarter note — the app's MIDI clock resolution, and the
/// divisor that turns every tick on the wire into beats.
pub(in crate::update::control) const TPQ: f64 =
    resonance_audio::types::TICKS_PER_QUARTER_NOTE as f64;

/// The song key: the first (lowest-sample) key change on the global
/// chord track, per its "song key" convention.
pub(in crate::update::control) fn song_key(app: &Resonance) -> Option<KeyScale> {
    app.chord_track.key_changes.first().map(|k| key_scale(&k.scale))
}

/// The app's `Scale` as its wire form.
pub(in crate::update::control) fn key_scale(scale: &resonance_music_theory::Scale) -> KeyScale {
    KeyScale {
        tonic: scale.root.to_string(),
        scale: scale.mode.as_str().to_owned(),
    }
}

/// dB fader value -> linear gain (protocol convention: 1.0 = unity).
pub(in crate::update::control) fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The app stores velocity as `0.0..=1.0`; the wire uses MIDI `0..=127`.
pub(in crate::update::control) fn velocity_to_midi(velocity: f32) -> u8 {
    (velocity.clamp(0.0, 1.0) * 127.0).round() as u8
}
