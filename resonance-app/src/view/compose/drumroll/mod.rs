pub mod canvas;

use iced::widget::{column, container, Canvas, Space};
use iced::{Element, Length};

use resonance_audio::types::{ClipId, TrackId};

use crate::compose::{SectionDefinitionState, SectionPlacementState};
use crate::message::Message;
use crate::state::InstrumentType;
use crate::Resonance;

pub use canvas::{drum_lane_height, sorted_drum_tracks, ComposeDrumCanvas};

/// Build the drumroll block. Returns an empty 0-height container when the
/// project has no drum tracks so synth-only projects pay no visual cost.
///
/// Each drum track gets its own grouped canvas; the project-scoped drum
/// groups are shared so kick/snare/hat/toms/perc read consistently across
/// tracks.
pub fn view<'a>(
    app: &'a Resonance,
    _placement: &'a SectionPlacementState,
    _definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let drum_tracks = sorted_drum_tracks(&app.registry.tracks);

    if drum_tracks.is_empty() {
        return container(Space::with_height(0)).width(Length::Fill).into();
    }

    let width = super::workspace_width(
        &app.tempo_map,
        _placement.start_bar,
        _definition.length_bars,
    );

    let total_height = drum_lane_height(&app.compose.drum_groups);
    let track_selected = matches!(
        app.compose.selected_lane,
        crate::compose::SelectedLane::Drums(_)
    );
    let selected_track_id = match app.compose.selected_lane {
        crate::compose::SelectedLane::Drums(id) => Some(id),
        _ => None,
    };

    let mut rows: Vec<Element<'a, Message>> = Vec::with_capacity(drum_tracks.len());
    for track in &drum_tracks {
        let canvas_prog = ComposeDrumCanvas {
            track,
            groups: &app.compose.drum_groups,
            selected_group_id: app.compose.drumroll.selected_group_id,
            track_selected: track_selected && selected_track_id == Some(track.id),
        };
        rows.push(
            container(
                Canvas::new(canvas_prog)
                    .width(Length::Fixed(width))
                    .height(Length::Fixed(total_height)),
            )
            .width(Length::Fixed(width))
            .height(Length::Fixed(total_height))
            .into(),
        );
    }

    column(rows).into()
}

/// Look up the first MIDI clip on `track_id` that overlaps the current
/// section. Used by the sidebar controls to decide whether the
/// Apply/Clear buttons can target a clip yet. (Kept for compatibility
/// with the old API — the grouped lane no longer requires a clip to
/// operate, but the inspector still uses this to enable optional
/// per-clip actions.)
pub fn clip_for_track(
    app: &Resonance,
    placement: &SectionPlacementState,
    definition: &SectionDefinitionState,
    track_id: TrackId,
) -> Option<ClipId> {
    let _ = InstrumentType::Drum;
    let section_start = app.tempo_map.bar_to_sample(placement.start_bar);
    let section_end = app.tempo_map.bar_to_sample(placement.start_bar + definition.length_bars);
    app.midi_clips.iter().find_map(|clip| {
        if clip.track_id != track_id {
            return None;
        }
        let clip_end = app.tempo_map.tick_to_abs_sample(
            clip.start_sample,
            clip.duration_ticks,
            app.sample_rate,
        );
        (clip_end > section_start && clip.start_sample < section_end).then_some(clip.id)
    })
}
