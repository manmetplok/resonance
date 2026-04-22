pub mod canvas;

use iced::widget::{container, Canvas};
use iced::{Element, Length};

use resonance_audio::types::{ClipId, TrackId, TICKS_PER_QUARTER_NOTE};

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

    let samples_per_bar = samples_per_bar(app);
    let section_start = placement.start_bar as u64 * samples_per_bar;
    let section_end = section_start + definition.length_bars as u64 * samples_per_bar;

    let canvas_prog = ComposeDrumCanvas {
        tracks: &app.registry.tracks,
        midi_clips: &app.midi_clips,
        pad_map: &app.compose.drumroll.pad_map,
        section_start,
        section_end,
        section_length_bars: definition.length_bars,
        steps_per_bar: app.compose.drumroll.steps_per_bar,
        sample_rate: app.sample_rate,
        bpm: app.transport.bpm,
        time_sig_num: app.transport.time_sig_num,
        scroll_offset_y: app.viewport.scroll_offset_y,
        details_track_id: app.compose.details_track_id(),
        selected_pad: app.compose.drumroll.selected_pad,
    };

    let total_height = drum_track_count as f32 * DRUM_TRACK_HEIGHT;
    container(
        Canvas::new(canvas_prog)
            .width(Length::Fill)
            .height(Length::Fixed(total_height)),
    )
    .width(Length::Fill)
    .height(Length::Fixed(total_height))
    .into()
}

fn samples_per_bar(app: &Resonance) -> u64 {
    let samples_per_beat = app.sample_rate as f64 * 60.0 / app.transport.bpm as f64;
    (samples_per_beat * app.transport.time_sig_num as f64) as u64
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
    let samples_per_bar = samples_per_bar(app);
    let section_start = placement.start_bar as u64 * samples_per_bar;
    let section_end = section_start + definition.length_bars as u64 * samples_per_bar;
    let samples_per_beat = app.sample_rate as f64 * 60.0 / app.transport.bpm as f64;
    let samples_per_tick = samples_per_beat / TICKS_PER_QUARTER_NOTE as f64;
    app.midi_clips.iter().find_map(|clip| {
        if clip.track_id != track_id {
            return None;
        }
        let clip_end = clip.start_sample + (clip.duration_ticks as f64 * samples_per_tick) as u64;
        (clip_end > section_start && clip.start_sample < section_end).then_some(clip.id)
    })
}
