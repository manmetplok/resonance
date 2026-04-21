//! Viewport, scrolling, zoom, and the periodic `Tick` handler (which
//! drains engine events and decays VU meter levels).
use iced::Task;

use crate::message::Message;
use crate::theme;
use crate::Resonance;
use resonance_audio::types::AudioCommand;

/// Handle the per-frame subscription tick.
pub fn handle_tick(r: &mut Resonance) -> Task<Message> {
    let mut tasks = Vec::new();
    while let Some(event) = r.engine.try_recv() {
        let task = r.handle_engine_event(event);
        tasks.push(task);
    }
    update_vu_meters(r);
    sync_tempo_at_playhead(r);
    auto_follow_playhead(r);
    if tasks.is_empty() {
        Task::none()
    } else {
        Task::batch(tasks)
    }
}

fn update_vu_meters(r: &mut Resonance) {
    let (track_peaks, bus_peaks, master_peak_l, master_peak_r) =
        r.engine.read_and_clear_peaks();
    for track in &mut r.registry.tracks {
        track.level_l *= theme::PEAK_DECAY;
        track.level_r *= theme::PEAK_DECAY;
    }
    for bus in &mut r.registry.busses {
        bus.level_l *= theme::PEAK_DECAY;
        bus.level_r *= theme::PEAK_DECAY;
    }
    for (track_id, pl, pr) in track_peaks {
        r.with_track_mut(track_id, |t| {
            if pl > t.level_l {
                t.level_l = pl;
            }
            if pr > t.level_r {
                t.level_r = pr;
            }
        });
    }
    for (bus_id, pl, pr) in bus_peaks {
        r.with_bus_mut(bus_id, |b| {
            if pl > b.level_l {
                b.level_l = pl;
            }
            if pr > b.level_r {
                b.level_r = pr;
            }
        });
    }
    r.master_level_l = (r.master_level_l * theme::PEAK_DECAY).max(master_peak_l);
    r.master_level_r = (r.master_level_r * theme::PEAK_DECAY).max(master_peak_r);
}

/// During playback, update the transport BPM display from the tempo
/// map. The engine computes its own BPM from the shared tempo events
/// so no `SetBpm` commands are sent here.
fn sync_tempo_at_playhead(r: &mut Resonance) {
    if !r.transport.playing || r.tempo_events.len() <= 1 && r.signature_events.len() <= 1 {
        return;
    }
    let (bpm, num, den) = r.tempo_map.tempo_at_sample(
        r.transport.playhead,
        r.sample_rate,
    );
    // Display only — no engine command.
    r.transport.bpm = bpm;
    r.transport.bpm_input = format!("{:.1}", bpm);

    if num != r.transport.time_sig_num || den != r.transport.time_sig_den {
        r.transport.time_sig_num = num;
        r.transport.time_sig_den = den;
        r.engine.send(AudioCommand::SetTimeSignature {
            numerator: num,
            denominator: den,
        });
    }
}

fn auto_follow_playhead(r: &mut Resonance) {
    if !r.transport.playing {
        return;
    }
    let playhead_seconds = r.transport.playhead as f64 / r.sample_rate as f64;
    let playhead_x = playhead_seconds as f32 * r.viewport.zoom - r.viewport.scroll_offset;
    let visible_width = r.viewport.viewport_width;
    if playhead_x > visible_width * 0.8 {
        r.viewport.scroll_offset = playhead_seconds as f32 * r.viewport.zoom - visible_width * 0.5;
    } else if playhead_x < 0.0 {
        r.viewport.scroll_offset =
            (playhead_seconds as f32 * r.viewport.zoom - visible_width * 0.2).max(0.0);
    }
}

pub fn scroll_x_delta(r: &mut Resonance, delta: f32) {
    let max_x = (r.viewport.timeline_content_width - r.viewport.viewport_width).max(0.0);
    r.viewport.scroll_offset = (r.viewport.scroll_offset + delta).clamp(0.0, max_x);
}

pub fn scroll_y_delta(r: &mut Resonance, delta: f32) {
    r.viewport.scroll_offset_y = (r.viewport.scroll_offset_y + delta).max(0.0);
    let max_y = (r.registry.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
    r.viewport.scroll_offset_y = r.viewport.scroll_offset_y.min(max_y);
}

pub fn scroll_to_x(r: &mut Resonance, x: f32) {
    let max_x = (r.viewport.timeline_content_width - r.viewport.viewport_width).max(0.0);
    r.viewport.scroll_offset = x.clamp(0.0, max_x);
}

pub fn scroll_to_y(r: &mut Resonance, y: f32) {
    r.viewport.scroll_offset_y = y.max(0.0);
    let max_y = (r.registry.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
    r.viewport.scroll_offset_y = r.viewport.scroll_offset_y.min(max_y);
}

pub fn viewport_width(r: &mut Resonance, w: f32) {
    r.viewport.viewport_width = w;
}

pub fn timeline_content_size(r: &mut Resonance, w: f32, h: f32) {
    r.viewport.timeline_content_width = w;
    r.viewport.timeline_content_height = h;
    // Re-clamp scroll offsets if content shrank.
    let max_x = (w - r.viewport.viewport_width).max(0.0);
    if r.viewport.scroll_offset > max_x {
        r.viewport.scroll_offset = max_x;
    }
    let max_y = (h - 1.0).max(0.0);
    if r.viewport.scroll_offset_y > max_y {
        r.viewport.scroll_offset_y = max_y;
    }
}

pub fn zoom_in(r: &mut Resonance) {
    r.viewport.zoom = (r.viewport.zoom * 1.5).min(1000.0);
}

pub fn zoom_out(r: &mut Resonance) {
    r.viewport.zoom = (r.viewport.zoom / 1.5).max(10.0);
}
