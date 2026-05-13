pub mod canvas;

use iced::widget::{container, Canvas};
use iced::{Element, Length};

use resonance_audio::types::{ClipId, TrackId};

use crate::compose::{SectionDefinitionState, SectionPlacementState};
use crate::message::Message;
use crate::state::InstrumentType;
use crate::Resonance;

pub use canvas::{ComposeDrumCanvas, DRUM_TRACK_HEIGHT};

/// Build the drumroll block. Returns an empty 0-height container when the
/// project has no drum tracks so synth-only projects pay no visual cost.
pub fn view<'a>(
    app: &'a Resonance,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let drum_track_count = app
        .registry
        .tracks
        .iter()
        .filter(|t| {
            matches!(t.track_type, resonance_audio::types::TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type == InstrumentType::Drum
        })
        .count();

    if drum_track_count == 0 {
        return container(iced::widget::Space::with_height(0))
            .width(Length::Fill)
            .into();
    }

    let section_start = app.tempo_map.bar_to_sample(placement.start_bar);
    let section_end = app.tempo_map.bar_to_sample(placement.start_bar + definition.length_bars);

    let canvas_prog = ComposeDrumCanvas {
        tracks: &app.registry.tracks,
        midi_clips: &app.midi_clips,
        pad_map: &app.compose.drumroll.pad_map,
        section_start,
        section_end,
        section_length_bars: definition.length_bars,
        steps_per_bar: app.compose.drumroll.steps_per_bar,
        sample_rate: app.sample_rate,
        tempo_map: &app.tempo_map,
        start_bar: placement.start_bar,
        scroll_offset_y: app.viewport.scroll_offset_y,
        details_track_id: app.compose.details_track_id(),
        selected_pad: app.compose.drumroll.selected_pad,
    };

    let total_height = drum_track_count as f32 * DRUM_TRACK_HEIGHT;
    let width = super::workspace_width(
        &app.tempo_map,
        placement.start_bar,
        definition.length_bars,
    );
    container(
        Canvas::new(canvas_prog)
            .width(Length::Fixed(width))
            .height(Length::Fixed(total_height)),
    )
    .width(Length::Fixed(width))
    .height(Length::Fixed(total_height))
    .into()
}

/// Look up the first MIDI clip on `track_id` that overlaps the current
/// section. Used by the sidebar controls to decide whether the
/// Apply/Clear buttons can target a clip yet.
pub fn clip_for_track(
    app: &Resonance,
    placement: &SectionPlacementState,
    definition: &SectionDefinitionState,
    track_id: TrackId,
) -> Option<ClipId> {
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
