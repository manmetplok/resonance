//! Canvas drawing for the timeline. These are the pure-draw methods
//! that take a `&mut Frame` and render bar/beat grids, rulers, audio
//! clips, and MIDI clips. Split into focused submodules:
//!
//! - [`chrome`]: shared clip primitives (chrome, fade helpers, utility fns)
//! - [`grid`]: bar/beat grid, ruler, markers
//! - [`chord_lane`]: chord lane and section band
//! - [`global_tracks`]: global-tracks shelf
//! - [`clip`]: audio clip drawing
//! - [`midi_notes`]: MIDI clip drawing
//! - [`drag`]: drag-to-timeline placement affordances

pub use chrome::{gain_tinted_body, format_gain_db, overlap_range, fade_envelope};
pub(in crate::view::timeline) use chrome::clip_lane_rect;

use super::TimelineCanvas;

mod chrome;
mod group_overview;
mod grid;
mod chord_lane;
mod global_tracks;
mod clip;
mod midi_notes;
mod drag;
