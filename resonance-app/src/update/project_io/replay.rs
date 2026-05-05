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

    // Migrate old generate_params + track roles to lane_generators for
    // projects predating the unified lane generator system.
    r.compose.migrate_old_generate_params(&r.registry.tracks);

    // Replay busses (must come before SetTrackOutput so the target bus
    // exists at the time the routing is set).
    for pb in &project.busses {
        replay_bus(r, pb, &loaded);
    }

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
    }

    let samples_per_beat = r.sample_rate as f64 * 60.0 / r.transport.bpm as f64;
    let samples_per_bar = (samples_per_beat * r.transport.time_sig_num as f64) as u64;
    r.compose
        .rebuild_derived_clips(&r.midi_clips, samples_per_bar);

    r.transport.loop_range_set = r.transport.loop_enabled;
}

fn replay_track(r: &mut Resonance, pt: &ProjectTrack, loaded: &LoadedProject) {
    // Register the track / sub-track / instrument-track with the engine.
    if let Some(link) = pt.sub_track {
        r.engine.send(AudioCommand::CreateSubTrack {
            sub_id: pt.id,
            parent_track_id: link.parent_track_id,
            output_port_index: link.output_port_index,
            name: pt.name.clone(),
        });
    } else if pt.track_type == "instrument" {
        r.engine.send(AudioCommand::AddInstrumentTrack {
            id_hint: Some(pt.id),
            name: Some(pt.name.clone()),
        });
    } else {
        r.engine.send(AudioCommand::AddTrack {
            id_hint: Some(pt.id),
            name: Some(pt.name.clone()),
        });
    }

    // Set track properties
    r.engine.send(AudioCommand::SetTrackVolume {
        track_id: pt.id,
        volume: db_to_gain(pt.volume),
    });
    r.engine.send(AudioCommand::SetTrackPan {
        track_id: pt.id,
        pan: pt.pan,
    });
    r.engine.send(AudioCommand::SetTrackMute {
        track_id: pt.id,
        muted: pt.muted,
    });
    r.engine.send(AudioCommand::SetTrackSolo {
        track_id: pt.id,
        soloed: pt.soloed,
    });
    r.engine.send(AudioCommand::SetTrackRecordArm {
        track_id: pt.id,
        armed: pt.record_armed,
    });
    r.engine.send(AudioCommand::SetTrackMonitor {
        track_id: pt.id,
        enabled: pt.monitor_enabled,
    });
    r.engine.send(AudioCommand::SetTrackMono {
        track_id: pt.id,
        mono: pt.mono,
    });
    r.engine.send(AudioCommand::SetTrackFxBypass {
        track_id: pt.id,
        bypassed: pt.fx_bypassed,
    });
    if let Some(ref device) = pt.input_device_name {
        r.engine.send(AudioCommand::SetTrackInputDevice {
            track_id: pt.id,
            device_name: Some(device.clone()),
        });
    }
    if let Some(port_index) = pt.input_port_index {
        r.engine.send(AudioCommand::SetTrackInputPort {
            track_id: pt.id,
            port_index,
        });
    }
    if pt.midi_input_device.is_some() {
        r.engine.send(AudioCommand::SetTrackMidiInput {
            track_id: pt.id,
            device: pt.midi_input_device.clone(),
            channel: pt.midi_input_channel,
        });
    }
    if pt.midi_output_device.is_some() {
        r.engine.send(AudioCommand::SetTrackMidiOutput {
            track_id: pt.id,
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
            track_id: pt.id,
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

    let mut track = if pt.track_type == "instrument" {
        TrackState::new_instrument(pt.id, r.registry.next_track_order)
    } else {
        TrackState::new_audio(pt.id, r.registry.next_track_order)
    };
    track.name = pt.name.clone();
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
