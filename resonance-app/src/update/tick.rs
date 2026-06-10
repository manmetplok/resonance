//! The periodic `Tick` handler: drains engine events, decays VU meter
//! levels, syncs the tempo display, follows the playhead, and
//! re-enumerates MIDI devices.
use iced::Task;

use crate::message::Message;
use crate::theme;
use crate::Resonance;
use resonance_audio::types::{AudioCommand, BusId, TrackId};

/// Handle the per-frame subscription tick.
pub fn handle_tick(r: &mut Resonance) -> Task<Message> {
    let mut tasks = Vec::new();
    while let Some(event) = r.engine.try_recv() {
        let task = crate::engine_events::handle_engine_event(r, event);
        tasks.push(task);
    }
    update_vu_meters(r);
    sync_tempo_at_playhead(r);
    auto_follow_playhead(r);
    refresh_midi_devices_if_stale(r);
    if tasks.is_empty() {
        Task::none()
    } else {
        Task::batch(tasks)
    }
}

/// Re-enumerate hardware MIDI ports periodically so a freshly
/// plugged controller appears in pickers without a restart.
/// Cadence is intentionally low (every 2 s) — ALSA seq enumeration
/// is cheap, but doing it every frame would still be wasteful.
fn refresh_midi_devices_if_stale(r: &mut Resonance) {
    if r.midi_devices_last_refresh.elapsed() < std::time::Duration::from_secs(2) {
        return;
    }
    r.midi_devices_last_refresh = std::time::Instant::now();
    let _ = r.engine.send(AudioCommand::ListMidiInputDevices);
    let _ = r.engine.send(AudioCommand::ListMidiOutputDevices);
}

/// Per-tick VU step: decay current levels and ask the engine for a
/// fresh peak snapshot. The reply arrives on a later tick as
/// `AudioEvent::PeakSnapshot` and is folded in by `apply_peak_snapshot`.
/// Splitting the read across two ticks is fine for a meter; it keeps the
/// GUI thread from contending on engine RwLocks.
fn update_vu_meters(r: &mut Resonance) {
    for track in &mut r.registry.tracks {
        track.level_l *= theme::PEAK_DECAY;
        track.level_r *= theme::PEAK_DECAY;
    }
    for bus in &mut r.registry.busses {
        bus.level_l *= theme::PEAK_DECAY;
        bus.level_r *= theme::PEAK_DECAY;
    }
    r.master_level_l *= theme::PEAK_DECAY;
    r.master_level_r *= theme::PEAK_DECAY;
    let _ = r.engine.send(AudioCommand::PollPeaks);
}

/// Fold a peak snapshot from the engine into the VU state. Each level
/// rises to the new peak immediately and decays only via the per-tick
/// pass in `update_vu_meters`.
pub fn apply_peak_snapshot(
    r: &mut Resonance,
    track_peaks: Vec<(TrackId, f32, f32)>,
    bus_peaks: Vec<(BusId, f32, f32)>,
    master_peak_l: f32,
    master_peak_r: f32,
) {
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
    if master_peak_l > r.master_level_l {
        r.master_level_l = master_peak_l;
    }
    if master_peak_r > r.master_level_r {
        r.master_level_r = master_peak_r;
    }
}

/// During playback, update the transport BPM display from the tempo
/// map. The engine computes its own BPM from the shared tempo events
/// so no `SetBpm` commands are sent here.
fn sync_tempo_at_playhead(r: &mut Resonance) {
    if !r.transport.playing || r.tempo_events.len() <= 1 && r.signature_events.len() <= 1 {
        return;
    }
    let (bpm, num, den) = r
        .tempo_map
        .tempo_at_sample(r.transport.playhead, r.sample_rate);
    // Display only — no engine command.
    r.transport.bpm = bpm;
    r.transport.bpm_input = format!("{:.1}", bpm);

    if num != r.transport.time_sig_num || den != r.transport.time_sig_den {
        r.transport.time_sig_num = num;
        r.transport.time_sig_den = den;
        let _ = r.engine.send(AudioCommand::SetTimeSignature {
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
