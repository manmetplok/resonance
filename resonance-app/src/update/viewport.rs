//! Viewport, scrolling, zoom, and the periodic `Tick` handler (which
//! drains engine events and decays VU meter levels).
use iced::Task;

use crate::message::Message;
use crate::theme;
use crate::Resonance;

/// Handle the per-frame subscription tick.
pub fn handle_tick(r: &mut Resonance) -> Task<Message> {
    let mut tasks = Vec::new();
    while let Some(event) = r.engine.try_recv() {
        let task = r.handle_engine_event(event);
        tasks.push(task);
    }
    update_vu_meters(r);
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
    for track in &mut r.tracks {
        track.level_l *= theme::PEAK_DECAY;
        track.level_r *= theme::PEAK_DECAY;
    }
    for bus in &mut r.busses {
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

fn auto_follow_playhead(r: &mut Resonance) {
    if !r.playing {
        return;
    }
    let playhead_seconds = r.playhead as f64 / r.sample_rate as f64;
    let playhead_x = playhead_seconds as f32 * r.zoom - r.scroll_offset;
    let visible_width = r.viewport_width;
    if playhead_x > visible_width * 0.8 {
        r.scroll_offset = playhead_seconds as f32 * r.zoom - visible_width * 0.5;
    } else if playhead_x < 0.0 {
        r.scroll_offset =
            (playhead_seconds as f32 * r.zoom - visible_width * 0.2).max(0.0);
    }
}

pub fn scroll_x_delta(r: &mut Resonance, delta: f32) {
    let max_x = (r.timeline_content_width - r.viewport_width).max(0.0);
    r.scroll_offset = (r.scroll_offset + delta).clamp(0.0, max_x);
}

pub fn scroll_y_delta(r: &mut Resonance, delta: f32) {
    r.scroll_offset_y = (r.scroll_offset_y + delta).max(0.0);
    let max_y = (r.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
    r.scroll_offset_y = r.scroll_offset_y.min(max_y);
}

pub fn scroll_to_x(r: &mut Resonance, x: f32) {
    let max_x = (r.timeline_content_width - r.viewport_width).max(0.0);
    r.scroll_offset = x.clamp(0.0, max_x);
}

pub fn scroll_to_y(r: &mut Resonance, y: f32) {
    r.scroll_offset_y = y.max(0.0);
    let max_y = (r.tracks.len() as f32 * theme::TRACK_HEIGHT).max(0.0);
    r.scroll_offset_y = r.scroll_offset_y.min(max_y);
}

pub fn viewport_width(r: &mut Resonance, w: f32) {
    r.viewport_width = w;
}

pub fn timeline_content_size(r: &mut Resonance, w: f32, h: f32) {
    r.timeline_content_width = w;
    r.timeline_content_height = h;
    // Re-clamp scroll offsets if content shrank.
    let max_x = (w - r.viewport_width).max(0.0);
    if r.scroll_offset > max_x {
        r.scroll_offset = max_x;
    }
    let max_y = (h - 1.0).max(0.0);
    if r.scroll_offset_y > max_y {
        r.scroll_offset_y = max_y;
    }
}

pub fn zoom_in(r: &mut Resonance) {
    r.zoom = (r.zoom * 1.5).min(1000.0);
}

pub fn zoom_out(r: &mut Resonance) {
    r.zoom = (r.zoom / 1.5).max(10.0);
}
