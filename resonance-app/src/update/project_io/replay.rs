//! Reconstruct GUI state from a `LoadedProject` and replay every required
//! engine command. Called after `AudioEvent::AllCleared` confirms the
//! engine has been emptied. Side-effecting end to end — sends ~20
//! `AudioCommand` variants and mutates almost every sub-state of `Resonance`.

use resonance_audio::types::*;

use crate::project::{LoadedProject, ProjectBus, ProjectTrack};
use crate::state::*;
use crate::util::db_to_gain;
use crate::Resonance;

/// Replay a loaded project into the engine and rebuild GUI state. Called
/// after `AudioEvent::AllCleared` confirms the engine is empty.
pub fn replay_loaded_project(r: &mut Resonance, loaded: Box<LoadedProject>) {
    let project = &loaded.file;
    r.io.project_path = None; // Will be set by the caller (OpenPathSelected)

    // Wipe runtime-only vocal side-tables (clip_lyrics, render_epoch)
    // before re-installing entries from the project. Without this,
    // loading a project on top of an existing one keeps stale lyrics
    // for clips that no longer exist, and stale render epochs that
    // make the next Generate Vocal think a section is already up to
    // date.
    r.compose.vocal_audio.clear();

    // Point the engine at the loaded project's directory so that
    // subsequent imports and recordings stream into it.
    r.engine
        .send(AudioCommand::SetProjectDir(loaded.project_dir.clone()));

    // Restore global settings
    r.transport.bpm = project.bpm;
    r.transport.time_sig_num = project.time_sig_num;
    r.transport.time_sig_den = project.time_sig_den;
    r.transport.metronome_enabled = project.metronome_enabled;
    r.master_volume = project.master_volume;
    r.transport.loop_enabled = project.loop_enabled;
    r.transport.loop_in = project.loop_in;
    r.transport.loop_out = project.loop_out;
    r.transport.playhead = 0;
    r.viewport.scroll_offset = 0.0;
    r.viewport.scroll_offset_y = 0.0;
    r.interaction.selected_clip = None;
    r.mixer.selected_plugin = None;
    r.interaction.clip_drag = None;
    r.interaction.clip_trim = None;
    r.confirm_delete_track = None;
    r.confirm_quit = None;
    r.compose
        .load_from_project(&project.section_definitions, &project.section_placements);

    // Restore the project's drum pattern bank. Three legacy paths:
    //
    // 1. Modern project: `drum_patterns` populated → use it directly.
    // 2. Legacy v2 project: `drum_groups` populated (single flat list) →
    //    promote into a one-entry pattern bank named "Main" so the
    //    section selector has something to point at.
    // 3. Pre-grouped legacy: both fields empty → the default seeded by
    //    `ComposeState::default()` stays in place.
    //
    // After the bank is hydrated we bump `next_id` past every saved id
    // (patterns + groups + pads) so the in-memory allocator never
    // collides with the project's reserved ids.
    if !project.drum_patterns.is_empty() {
        r.compose.drum_patterns = project.drum_patterns.clone();
    } else if !project.drum_groups.is_empty() {
        let (patterns, _id) = crate::compose::drumroll::legacy_groups_to_pattern(
            project.drum_groups.clone(),
            &mut r.compose.next_id,
        );
        r.compose.drum_patterns = patterns;
        // Point any pre-existing definition that has no pattern id at
        // the freshly-promoted "Main" pattern so the lane resolves
        // identically to how the legacy project rendered.
        let main_id = r.compose.drum_patterns.first().map(|p| p.id);
        for def in &mut r.compose.definitions {
            if def.drum_pattern_id.is_none() {
                def.drum_pattern_id = main_id;
            }
        }
    }

    // Refresh the project default pattern id and the right-rail / modal
    // focus to land on a pattern that actually exists.
    r.compose.default_drum_pattern_id = r.compose.drum_patterns.first().map(|p| p.id);
    // Bump `next_id` past every saved pattern and group id so the
    // manager's "+ New" actions never collide with reserved ids.
    let max_id = r
        .compose
        .drum_patterns
        .iter()
        .flat_map(|p| std::iter::once(p.id).chain(p.groups.iter().map(|g| g.id)))
        .max();
    if let Some(m) = max_id {
        r.compose.next_id = r.compose.next_id.max(m + 1);
    }
    let first_group_id = r
        .compose
        .drum_patterns
        .first()
        .and_then(|p| p.groups.first().map(|g| g.id));
    r.compose.drumroll.selected_group_id = first_group_id;
    r.compose.drumroll.managing_group_id = first_group_id;
    r.compose.drumroll.managing_pattern_id = r.compose.default_drum_pattern_id;

    // Restore tempo/signature events. If the project has none (legacy),
    // create a single event at bar 0 from the global BPM/sig.
    if project.tempo_events.is_empty() {
        r.tempo_events = vec![crate::state::TempoEvent {
            bar: 0,
            bpm: project.bpm,
        }];
    } else {
        r.tempo_events = project.tempo_events.clone();
    }
    if project.signature_events.is_empty() {
        r.signature_events = vec![crate::state::SignatureEvent {
            bar: 0,
            numerator: project.time_sig_num,
            denominator: project.time_sig_den,
        }];
    } else {
        r.signature_events = project.signature_events.clone();
    }

    r.engine.send(AudioCommand::SetBpm {
        bpm: r.transport.bpm,
    });
    r.rebuild_and_send_tempo();
    r.engine.send(AudioCommand::SetTimeSignature {
        numerator: r.transport.time_sig_num,
        denominator: r.transport.time_sig_den,
    });
    r.engine.send(AudioCommand::SetMetronomeEnabled {
        enabled: r.transport.metronome_enabled,
    });
    r.engine.send(AudioCommand::SetMasterVolume {
        volume: db_to_gain(r.master_volume),
    });

    // Restore MIDI clock settings. The engine treats `enabled=false`
    // as a no-op port-wise, so it's safe to send for legacy projects.
    r.midi_clock_send_enabled = project.midi_clock_send_enabled;
    r.midi_clock_send_device = project.midi_clock_send_device.clone();
    r.midi_clock_recv_enabled = project.midi_clock_recv_enabled;
    r.midi_clock_recv_device = project.midi_clock_recv_device.clone();
    r.engine.send(AudioCommand::SetMidiClockOutput {
        device: r.midi_clock_send_device.clone(),
        enabled: r.midi_clock_send_enabled,
    });
    r.engine.send(AudioCommand::SetMidiClockInput {
        device: r.midi_clock_recv_device.clone(),
        enabled: r.midi_clock_recv_enabled,
    });
    r.engine.send(AudioCommand::SetLoopRange {
        enabled: r.transport.loop_enabled,
        loop_in: r.transport.loop_in,
        loop_out: r.transport.loop_out,
    });

    // Clear GUI state
    r.registry.tracks.clear();
    r.registry.busses.clear();
    r.master_plugins.clear();
    r.master_fx_bypassed = false;
    r.clips.clear();
    r.midi_clips.clear();
    r.registry.next_track_order = 0;
    r.registry.next_bus_order = 0;
    r.plugin_index.clear();

    // Bump the app-side sub-track id counter past any persisted ids so
    // new sub-tracks allocated after this load don't collide with
    // restored ones.
    for pt in &project.tracks {
        if pt.sub_track.is_some() && pt.id >= r.registry.next_sub_track_id {
            r.registry.next_sub_track_id = pt.id + 1;
        }
    }

    for pt in &project.tracks {
        replay_track(r, pt, &loaded);
    }
    // Defensive: older project files weren't guaranteed to be saved in
    // .order sequence, and replay relies on the registry staying sorted
    // by .order for the view layer's invariant.
    r.registry.resort_tracks();

    // Migrate old generate_params + track roles to lane_generators for
    // projects predating the unified lane generator system.
    r.compose.migrate_old_generate_params(&r.registry.tracks);

    // Replay busses (must come before SetTrackOutput so the target bus
    // exists at the time the routing is set).
    for pb in &project.busses {
        replay_bus(r, pb, &loaded);
    }
    r.registry.resort_busses();
    // Output-destination picker depends on the bus list.
    r.view_caches.rebuild_output(&r.registry.busses);

    // Replay master FX chain + bypass state.
    replay_master(r, project, &loaded);

    // Now that all busses exist, resolve track → bus routing.
    for pt in &project.tracks {
        if let Some(bus_id) = pt.output_bus {
            r.engine.send(AudioCommand::SetTrackOutput {
                track_id: pt.id,
                output: TrackOutput::Bus(bus_id),
            });
        }
    }

    // Replay audio clips: each clip's WAV file already exists on disk
    // inside the project directory; hand the engine an absolute path
    // and let it mmap the file itself.
    for pc in &project.clips {
        let abs_path = loaded.project_dir.join(&pc.audio_file);
        r.engine.send(AudioCommand::LoadClipFromWav {
            clip_id: pc.id,
            track_id: pc.track_id,
            start_sample: pc.start_sample,
            path: abs_path,
            name: pc.name.clone(),
            trim_start_frames: pc.trim_start_frames,
            trim_end_frames: pc.trim_end_frames,
        });

        let duration_samples = pc
            .total_frames
            .saturating_sub(pc.trim_start_frames)
            .saturating_sub(pc.trim_end_frames);
        r.clips.push(ClipState {
            id: pc.id,
            track_id: pc.track_id,
            start_sample: pc.start_sample,
            duration_samples,
            name: pc.name.clone(),
            total_frames: pc.total_frames,
            trim_start_frames: pc.trim_start_frames,
            trim_end_frames: pc.trim_end_frames,
            waveform_peaks: Vec::new(), // Will be populated by ClipImported event
        });
    }

    // Replay MIDI clips from the parsed `.mid` files.
    for pmc in &project.midi_clips {
        let notes: Vec<MidiNote> = loaded.midi_notes.get(&pmc.id).cloned().unwrap_or_default();

        r.engine.send(AudioCommand::LoadMidiClipDirect {
            clip_id: pmc.id,
            track_id: pmc.track_id,
            start_sample: pmc.start_sample,
            duration_ticks: pmc.duration_ticks,
            notes: notes.clone(),
            name: pmc.name.clone(),
            trim_start_ticks: pmc.trim_start_ticks,
            trim_end_ticks: pmc.trim_end_ticks,
        });

        let note_count = notes.len();
        r.midi_clips.push(MidiClipState {
            id: pmc.id,
            track_id: pmc.track_id,
            start_sample: pmc.start_sample,
            duration_ticks: pmc.duration_ticks,
            name: pmc.name.clone(),
            notes,
            trim_start_ticks: pmc.trim_start_ticks,
            trim_end_ticks: pmc.trim_end_ticks,
        });

        // Re-install the lyric side-table. Serializer strips trailing
        // empties; pad back to the clip's note count so every parallel
        // walker stays correctly aligned. Skip entirely when the saved
        // vec is all-empty (legacy projects + non-vocal clips).
        if !pmc.vocal_lyrics.is_empty() {
            let mut lyrics = pmc.vocal_lyrics.clone();
            lyrics.resize(note_count, String::new());
            r.compose.vocal_audio.clip_lyrics.insert(pmc.id, lyrics);
        }
    }

    let samples_per_beat = r.sample_rate as f64 * 60.0 / r.transport.bpm as f64;
    let samples_per_bar = (samples_per_beat * r.transport.time_sig_num as f64) as u64;
    r.compose
        .rebuild_derived_clips(&r.midi_clips, samples_per_bar);

    // Rebuild the vocal audio clip map so subsequent regen tear-downs
    // find the loaded clips and clean them up — otherwise the next
    // Generate Vocal stacks a new clip on top of the old one and the
    // mixer plays both summed together.
    use std::collections::{HashMap, HashSet};
    let vocal_track_ids: HashSet<resonance_audio::types::TrackId> = r
        .registry
        .tracks
        .iter()
        .filter(|t| t.track_type == resonance_audio::types::TrackType::Vocal)
        .map(|t| t.id)
        .collect();
    let audio_clip_paths: HashMap<resonance_audio::types::ClipId, std::path::PathBuf> = project
        .clips
        .iter()
        .map(|pc| (pc.id, loaded.project_dir.join(&pc.audio_file)))
        .collect();
    r.compose.rebuild_vocal_audio_clips(
        &r.clips,
        &audio_clip_paths,
        &vocal_track_ids,
        samples_per_bar,
    );

    r.transport.loop_range_set = r.transport.loop_enabled;

    // Re-populate the `with_plugin_mut` side-index from the wholesale
    // replay we just performed. Per-slot inserts would also work but
    // a single rebuild is simpler and keeps `replay_track` / `_bus` /
    // `_master` focused on their own concern.
    r.rebuild_plugin_index();
}

fn replay_track(r: &mut Resonance, pt: &ProjectTrack, loaded: &LoadedProject) {
    // Repair sub-track id collisions left by buggier prior versions. If
    // the saved id is already in use by an earlier-loaded track, allocate
    // a fresh sub-track id from `next_sub_track_id` (which the pre-loop
    // bump already advanced past every saved sub-track id, so this won't
    // collide with later siblings either).
    let track_id = if pt.sub_track.is_some()
        && r.registry.tracks.iter().any(|t| t.id == pt.id)
    {
        let new_id = r.registry.allocate_sub_track_id();
        eprintln!(
            "replay_track: sub-track {:?} id {} collided with existing track; remapped to {}",
            pt.name, pt.id, new_id
        );
        new_id
    } else {
        pt.id
    };

    // Register the track / sub-track / instrument-track with the engine.
    if let Some(link) = pt.sub_track {
        r.engine.send(AudioCommand::CreateSubTrack {
            sub_id: track_id,
            parent_track_id: link.parent_track_id,
            output_port_index: link.output_port_index,
            name: pt.name.clone(),
        });
    } else if pt.track_type == "instrument" {
        r.engine.send(AudioCommand::AddInstrumentTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    } else if pt.track_type == "vocal" {
        r.engine.send(AudioCommand::AddVocalTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    } else {
        r.engine.send(AudioCommand::AddTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    }

    // Set track properties
    r.engine.send(AudioCommand::SetTrackVolume {
        track_id,
        volume: db_to_gain(pt.volume),
    });
    r.engine.send(AudioCommand::SetTrackPan {
        track_id,
        pan: pt.pan,
    });
    r.engine.send(AudioCommand::SetTrackMute {
        track_id,
        muted: pt.muted,
    });
    r.engine.send(AudioCommand::SetTrackSolo {
        track_id,
        soloed: pt.soloed,
    });
    r.engine.send(AudioCommand::SetTrackRecordArm {
        track_id,
        armed: pt.record_armed,
    });
    r.engine.send(AudioCommand::SetTrackMonitor {
        track_id,
        enabled: pt.monitor_enabled,
    });
    r.engine.send(AudioCommand::SetTrackMono {
        track_id,
        mono: pt.mono,
    });
    r.engine.send(AudioCommand::SetTrackFxBypass {
        track_id,
        bypassed: pt.fx_bypassed,
    });
    if let Some(ref device) = pt.input_device_name {
        r.engine.send(AudioCommand::SetTrackInputDevice {
            track_id,
            device_name: Some(device.clone()),
        });
    }
    if let Some(port_index) = pt.input_port_index {
        r.engine.send(AudioCommand::SetTrackInputPort {
            track_id,
            port_index,
        });
    }
    if pt.midi_input_device.is_some() {
        r.engine.send(AudioCommand::SetTrackMidiInput {
            track_id,
            device: pt.midi_input_device.clone(),
            channel: pt.midi_input_channel,
        });
    }
    if pt.midi_output_device.is_some() {
        r.engine.send(AudioCommand::SetTrackMidiOutput {
            track_id,
            device: pt.midi_output_device.clone(),
            channel: pt.midi_output_channel,
        });
    }

    // Build GUI track state. Plugin slots are placeholders — their
    // params + has_gui are overwritten when the subsequent
    // PluginAdded event arrives from the engine.
    let mut gui_plugins = Vec::new();
    for pp in &pt.plugins {
        r.engine.send(AudioCommand::AddPlugin {
            track_id,
            clap_file_path: pp.clap_file_path.clone(),
            clap_plugin_id: pp.clap_plugin_id.clone(),
            id_hint: Some(pp.instance_id),
        });
        if let Some(state_data) = loaded.plugin_states.get(&pp.instance_id) {
            r.engine.send(AudioCommand::LoadPluginState {
                instance_id: pp.instance_id,
                data: state_data.clone(),
            });
        }
        gui_plugins.push(PluginSlotState::new(
            pp.instance_id,
            pp.plugin_name.clone(),
            pp.clap_plugin_id.clone(),
            pp.clap_file_path.clone(),
            Vec::new(),
            false,
        ));
    }

    let order = r.registry.next_track_order;
    let mut track = if let Some(link) = pt.sub_track {
        // Sub-tracks are always instrument-typed regardless of what the
        // saved `track_type` says. Earlier buggy saves could land a
        // sub-track in a colliding-id slot whose surviving entry had
        // track_type "vocal"; re-typing here keeps the inspector / mixer
        // rendering the correct controls after the remap above.
        TrackState::new_sub_track(
            track_id,
            order,
            pt.name.clone(),
            link.parent_track_id,
            link.output_port_index,
        )
    } else if pt.track_type == "instrument" {
        TrackState::new_instrument(track_id, order)
    } else if pt.track_type == "vocal" {
        TrackState::new_vocal(track_id, order)
    } else {
        TrackState::new_audio(track_id, order)
    };
    // Projects saved before the order-based default-naming fix
    // (commit ~late-2026) stored auto-generated names like
    // "Track 1000000006" derived from the engine TrackId. Replace
    // those on load with the new order-based form; user-chosen
    // names containing digits are left alone.
    track.name = migrate_auto_name(&pt.name, pt.track_type == "instrument", order);
    track.volume = pt.volume;
    track.pan = pt.pan;
    track.muted = pt.muted;
    track.soloed = pt.soloed;
    track.fx_bypassed = pt.fx_bypassed;
    track.record_armed = pt.record_armed;
    track.monitor_enabled = pt.monitor_enabled;
    track.mono = pt.mono;
    track.input_device_name = pt.input_device_name.clone();
    track.plugins = gui_plugins;
    track.output = pt
        .output_bus
        .map(TrackOutput::Bus)
        .unwrap_or(TrackOutput::Master);
    track.instrument_type = pt.instrument_type;
    track.instrument_icon = pt.instrument_icon;
    track.role = pt.role;
    track.sub_track = pt.sub_track;
    track.input_port_index = pt.input_port_index.unwrap_or(0);
    track.midi_input_device = pt.midi_input_device.clone();
    track.midi_input_channel = pt.midi_input_channel;
    track.midi_output_device = pt.midi_output_device.clone();
    track.midi_output_channel = pt.midi_output_channel;
    r.registry.tracks.push(track);
    r.registry.next_track_order += 1;
}

fn replay_bus(r: &mut Resonance, pb: &ProjectBus, loaded: &LoadedProject) {
    r.engine.send(AudioCommand::AddBus {
        id_hint: Some(pb.id),
        name: Some(pb.name.clone()),
    });
    r.engine.send(AudioCommand::SetBusVolume {
        bus_id: pb.id,
        volume: db_to_gain(pb.volume),
    });
    r.engine.send(AudioCommand::SetBusPan {
        bus_id: pb.id,
        pan: pb.pan,
    });
    r.engine.send(AudioCommand::SetBusMute {
        bus_id: pb.id,
        muted: pb.muted,
    });
    r.engine.send(AudioCommand::SetBusFxBypass {
        bus_id: pb.id,
        bypassed: pb.fx_bypassed,
    });

    let mut gui_plugins = Vec::new();
    for pp in &pb.plugins {
        r.engine.send(AudioCommand::AddPluginToBus {
            bus_id: pb.id,
            clap_file_path: pp.clap_file_path.clone(),
            clap_plugin_id: pp.clap_plugin_id.clone(),
            id_hint: Some(pp.instance_id),
        });
        if let Some(state_data) = loaded.plugin_states.get(&pp.instance_id) {
            r.engine.send(AudioCommand::LoadPluginState {
                instance_id: pp.instance_id,
                data: state_data.clone(),
            });
        }
        gui_plugins.push(PluginSlotState::new(
            pp.instance_id,
            pp.plugin_name.clone(),
            pp.clap_plugin_id.clone(),
            pp.clap_file_path.clone(),
            Vec::new(),
            false,
        ));
    }

    let mut bus = BusState::new(pb.id, r.registry.next_bus_order, pb.name.clone());
    bus.volume = pb.volume;
    bus.pan = pb.pan;
    bus.muted = pb.muted;
    bus.fx_bypassed = pb.fx_bypassed;
    bus.plugins = gui_plugins;
    r.registry.busses.push(bus);
    r.registry.next_bus_order += 1;
}

fn replay_master(r: &mut Resonance, project: &crate::project::ProjectFile, loaded: &LoadedProject) {
    r.master_fx_bypassed = project.master_fx_bypassed;
    r.engine.send(AudioCommand::SetMasterFxBypass {
        bypassed: project.master_fx_bypassed,
    });

    r.master_plugins.clear();
    for pp in &project.master_plugins {
        r.engine.send(AudioCommand::AddPluginToMaster {
            clap_file_path: pp.clap_file_path.clone(),
            clap_plugin_id: pp.clap_plugin_id.clone(),
            id_hint: Some(pp.instance_id),
        });
        if let Some(state_data) = loaded.plugin_states.get(&pp.instance_id) {
            r.engine.send(AudioCommand::LoadPluginState {
                instance_id: pp.instance_id,
                data: state_data.clone(),
            });
        }
        r.master_plugins.push(PluginSlotState::new(
            pp.instance_id,
            pp.plugin_name.clone(),
            pp.clap_plugin_id.clone(),
            pp.clap_file_path.clone(),
            Vec::new(),
            false,
        ));
    }
}

/// Rewrite legacy auto-generated track names like "Track 1000000006" or
/// "Instrument 1234567890" to the new order-based form ("Track 4").
/// Names that aren't a single Track/Instrument + a 7-or-more digit
/// number pass through unchanged so user labels like "Track 2 (lead)"
/// or short numbered names like "Track 12" are preserved.
fn migrate_auto_name(name: &str, is_instrument: bool, order: usize) -> String {
    let prefix = if is_instrument {
        "Instrument "
    } else {
        "Track "
    };
    if let Some(rest) = name.strip_prefix(prefix) {
        if rest.len() >= 7 && rest.chars().all(|c| c.is_ascii_digit()) {
            return format!("{}{}", prefix, order + 1);
        }
    }
    name.to_string()
}

// Inline tests: `resonance-app` is a binary crate with no `lib.rs`, so the
// private `migrate_auto_name` helper isn't reachable from a `tests/` file.
// See ARCHITECTURE.md → Test Layout → Binary-crate exception.
#[cfg(test)]
mod tests {
    use super::migrate_auto_name;

    #[test]
    fn migrates_engine_id_track_name() {
        assert_eq!(migrate_auto_name("Track 1000000006", false, 3), "Track 4");
        assert_eq!(
            migrate_auto_name("Instrument 1000000007", true, 5),
            "Instrument 6"
        );
    }

    #[test]
    fn leaves_short_numbered_names_alone() {
        assert_eq!(migrate_auto_name("Track 12", false, 7), "Track 12");
        assert_eq!(migrate_auto_name("Track 99", false, 7), "Track 99");
    }

    #[test]
    fn leaves_user_names_alone() {
        assert_eq!(migrate_auto_name("Bass", false, 1), "Bass");
        assert_eq!(migrate_auto_name("Lead synth", true, 2), "Lead synth");
        assert_eq!(
            migrate_auto_name("Track 1000000006 (vocals)", false, 1),
            "Track 1000000006 (vocals)"
        );
    }
}
