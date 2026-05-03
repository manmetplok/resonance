//! Engine control thread: owns the command/event loop and the
//! per-command handler dispatch. All mutable engine state that must
//! outlive a single command lives in [`HandlerState`]; the shared
//! references to `Arc<RwLock<...>>` project state, the event sender, and
//! the retry-command sender live in [`HandlerCtx`].
//!
//! Handlers are free functions in the submodules (`transport`, `tracks`,
//! `clips`, `midi`, `plugins`, `busses`). They take `&HandlerCtx` +
//! `&mut HandlerState` + the command payload and execute synchronously.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::{ClapBundle, SyncClapInstance};
use crate::midi_hardware::{LiveMidiEvent, MidiInputRegistry, MidiOutputRegistry};
use crate::recording::RecordingState;
use crate::types::*;

use super::{bounce, busses, clips, master, midi, plugins, scan, tracks, transport, SharedState};

/// Read-only handle to shared project state and channels. Passed by
/// reference into every handler so they can lock the relevant maps and
/// emit events without taking ownership.
pub(crate) struct HandlerCtx<'a> {
    pub shared: &'a Arc<SharedState>,
    pub tracks: &'a Arc<RwLock<IndexMap<TrackId, Track>>>,
    pub busses: &'a Arc<RwLock<IndexMap<BusId, Bus>>>,
    pub master: &'a Arc<RwLock<MasterBus>>,
    pub clips: &'a Arc<RwLock<Vec<AudioClip>>>,
    pub midi_clips: &'a Arc<RwLock<Vec<MidiClip>>>,
    pub plugins: &'a Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    pub tempo_map: &'a Arc<RwLock<TempoMap>>,
    pub monitor_prod: &'a Arc<Mutex<ringbuf::HeapProd<f32>>>,
    pub event_tx: &'a Sender<AudioEvent>,
    pub cmd_tx_retry: &'a Sender<AudioCommand>,
    pub sample_rate: u32,
    pub buf_frames: usize,
    pub quantum: usize,
}

/// Per-track state for ongoing live MIDI recording. Kept on the
/// engine control thread so it never leaks into the audio callback.
pub(crate) struct RecordingMidiState {
    /// MIDI clip currently being recorded into.
    pub clip_id: ClipId,
    /// Absolute tick at the clip's start sample. Used to convert the
    /// per-event playhead tick into the clip-relative `start_tick`.
    pub clip_start_tick: u64,
    /// Currently held notes: pitch → index in the clip's `notes` vec.
    /// On NoteOff we look the note up here, set its `duration_ticks`,
    /// and remove the entry. Stuck NoteOns get closed at transport stop.
    pub open_notes: HashMap<u8, usize>,
}

/// Mutable engine-thread-local state that persists across command
/// dispatches: monotonic id counters, the recording session, the loaded
/// CLAP bundles, and the concurrent-import counter.
pub(crate) struct HandlerState {
    pub next_track_id: TrackId,
    pub next_bus_id: BusId,
    pub next_clip_id: ClipId,
    pub next_plugin_id: PluginInstanceId,
    pub rec: RecordingState,
    pub bundles: Vec<ClapBundle>,
    pub active_imports: Arc<AtomicUsize>,
    /// Current project directory. Set via `AudioCommand::SetProjectDir`
    /// whenever the app opens, creates, or saves-as a project.
    /// Recording and import refuse to run when this is `None`.
    pub project_dir: Option<PathBuf>,
    /// Hardware MIDI input registry. Owns one open midir connection
    /// per track configured for hardware input. The connection's
    /// callback runs on a midir-spawned thread and feeds
    /// [`LiveMidiEvent`]s into the engine thread via a bounded channel.
    pub midi_inputs: MidiInputRegistry,
    /// Hardware MIDI output registry. Refcounts midir output
    /// connections across tracks that share the same physical port.
    pub midi_outputs: MidiOutputRegistry,
    /// Per-track recording state for live MIDI. A fresh entry is
    /// created lazily on the first NoteOn for an armed instrument
    /// track during playback; cleared on transport stop.
    pub midi_recording: HashMap<TrackId, RecordingMidiState>,
    /// Notes currently sounding on hardware MIDI outputs from
    /// timeline playback. Keyed by `(track_id, note)`; value carries
    /// the note's end-sample plus the channel it was sent on (so a
    /// later channel change doesn't strand the stuck note).
    pub midi_outbound_held: HashMap<(TrackId, u8), (u64, u8)>,
    /// Last playhead seen by the timeline → output poll. The next
    /// poll iterates notes whose start/end fall in
    /// `(midi_outbound_last_playhead .. current_playhead]` and emits
    /// NoteOn/NoteOff for them.
    pub midi_outbound_last_playhead: u64,
}

/// Hard cap on concurrent clip decode threads. Import commands past this
/// bound get dropped with an error event.
pub(crate) const MAX_CONCURRENT_IMPORTS: usize = 4;

#[allow(clippy::too_many_arguments)]
pub(crate) fn engine_thread(
    cmd_rx: Receiver<AudioCommand>,
    cmd_tx_retry: Sender<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    shared: Arc<SharedState>,
    tracks_arc: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses_arc: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master_arc: Arc<RwLock<MasterBus>>,
    clips_arc: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips_arc: Arc<RwLock<Vec<MidiClip>>>,
    tempo_map: Arc<RwLock<TempoMap>>,
    plugins_arc: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    monitor_prod: Arc<Mutex<ringbuf::HeapProd<f32>>>,
    live_midi_tx: Sender<LiveMidiEvent>,
    live_midi_rx: Receiver<LiveMidiEvent>,
    sample_rate: u32,
    buf_frames: usize,
    quantum: usize,
) {
    let mut state = HandlerState {
        next_track_id: 1,
        next_bus_id: 1,
        next_clip_id: 1,
        next_plugin_id: 1,
        rec: RecordingState::new(sample_rate),
        bundles: Vec::new(),
        active_imports: Arc::new(AtomicUsize::new(0)),
        project_dir: None,
        midi_inputs: MidiInputRegistry::new(live_midi_tx),
        midi_outputs: MidiOutputRegistry::new(),
        midi_recording: HashMap::new(),
        midi_outbound_held: HashMap::new(),
        midi_outbound_last_playhead: 0,
    };
    let ctx = HandlerCtx {
        shared: &shared,
        tracks: &tracks_arc,
        busses: &busses_arc,
        master: &master_arc,
        clips: &clips_arc,
        midi_clips: &midi_clips_arc,
        plugins: &plugins_arc,
        tempo_map: &tempo_map,
        monitor_prod: &monitor_prod,
        event_tx: &event_tx,
        cmd_tx_retry: &cmd_tx_retry,
        sample_rate,
        buf_frames,
        quantum,
    };

    let mut last_playhead_report = std::time::Instant::now();

    // Report actual sample rate to GUI
    let _ = ctx
        .event_tx
        .send(AudioEvent::SampleRateDetected { sample_rate });

    // Add a default track
    {
        let id = state.next_track_id;
        let track = Track::new(id, "Track 1".to_string());
        ctx.tracks.write().insert(id, track);
        let _ = ctx.event_tx.send(AudioEvent::TrackAdded { track_id: id });
        state.next_track_id += 1;
    }

    loop {
        match cmd_rx.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(cmd) => dispatch(&ctx, &mut state, cmd),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }

        // Drain any hardware MIDI input that's queued up since the
        // previous iteration. Each event is dispatched into the same
        // queue_note_on/off path as `AudioCommand::SendNoteOn`, plus
        // optional record-into-clip and Thru-to-output.
        for ev in live_midi_rx.try_iter() {
            midi::handle_live_midi_event(&ctx, &mut state, ev);
        }

        // Step the timeline → MIDI output bridge: any note whose
        // start/end falls in (last_playhead..current_playhead] gets
        // sent to the configured hardware output. Granularity is
        // engine-thread cadence (~16 ms); precise enough for most
        // hardware-synth use cases and simpler than a lock-free
        // audio→engine queue.
        midi::poll_timeline_to_midi_output(&ctx, &mut state);

        // Advance any pending record count-in: once the playhead
        // catches up to the user's original record-start, the real
        // recording stream opens.
        transport::poll_precount(&ctx, &mut state);

        // Sync the stable `bpm` field from the tempo event table so
        // the mixer (audio thread) always sees the correct tempo for
        // the current playhead position.
        {
            let playhead = ctx
                .shared
                .playhead
                .load(std::sync::atomic::Ordering::Relaxed);
            let mut tm = ctx.tempo_map.write();
            tm.sync_bpm_at(playhead, ctx.sample_rate);
        }

        // Drain recording ring buffer into per-track buffers
        if ctx.shared.recording.load(Ordering::Relaxed) {
            state.rec.drain_ring_to_buffers();
        }

        // Report playhead position at ~60Hz using wall-clock time
        if ctx.shared.playing.load(Ordering::SeqCst)
            && last_playhead_report.elapsed() >= std::time::Duration::from_millis(16)
        {
            last_playhead_report = std::time::Instant::now();
            let pos = ctx.shared.playhead.load(Ordering::SeqCst);
            let _ = ctx.event_tx.send(AudioEvent::PlayheadMoved(pos));
        }
    }
}

fn dispatch(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        // -- Transport --
        AudioCommand::Play => transport::handle_play(ctx),
        AudioCommand::Record { precount_bars } => {
            transport::handle_record(ctx, state, precount_bars)
        }
        AudioCommand::Pause => transport::handle_pause(ctx, state),
        AudioCommand::Stop => transport::handle_stop(ctx, state),
        AudioCommand::SeekTo(pos) => transport::handle_seek_to(ctx, pos),
        AudioCommand::SetBpm { bpm } => transport::handle_set_bpm(ctx, bpm),
        AudioCommand::SetTempoEvents { tempo, signature } => {
            transport::handle_set_tempo_events(ctx, tempo, signature)
        }
        AudioCommand::SetTimeSignature {
            numerator,
            denominator,
        } => transport::handle_set_time_signature(ctx, numerator, denominator),
        AudioCommand::SetMetronomeEnabled { enabled } => {
            transport::handle_set_metronome_enabled(ctx, enabled)
        }
        AudioCommand::SetLoopRange {
            enabled,
            loop_in,
            loop_out,
        } => transport::handle_set_loop_range(ctx, state, enabled, loop_in, loop_out),

        // -- Audio clips --
        AudioCommand::ImportClip {
            track_id,
            path,
            start_sample,
        } => clips::handle_import_clip(ctx, state, track_id, path, start_sample),
        AudioCommand::MoveClip {
            clip_id,
            new_start_sample,
            new_track_id,
        } => clips::handle_move_clip(ctx, clip_id, new_start_sample, new_track_id),
        AudioCommand::TrimClip {
            clip_id,
            new_start_sample,
            trim_start_frames,
            trim_end_frames,
        } => clips::handle_trim_clip(
            ctx,
            clip_id,
            new_start_sample,
            trim_start_frames,
            trim_end_frames,
        ),
        AudioCommand::DeleteClip { clip_id } => clips::handle_delete_clip(ctx, clip_id),
        AudioCommand::SetProjectDir(dir) => {
            state.project_dir = Some(dir);
        }
        AudioCommand::LoadClipFromWav {
            clip_id,
            track_id,
            start_sample,
            path,
            name,
            trim_start_frames,
            trim_end_frames,
        } => clips::handle_load_clip_from_wav(
            ctx,
            state,
            clip_id,
            track_id,
            start_sample,
            path,
            name,
            trim_start_frames,
            trim_end_frames,
        ),
        AudioCommand::SaveClipsToProjectDir => clips::handle_save_clips_to_project_dir(ctx, state),

        // -- Tracks --
        AudioCommand::SetTrackVolume { track_id, volume } => {
            tracks::handle_set_track_volume(ctx, track_id, volume)
        }
        AudioCommand::SetTrackPan { track_id, pan } => {
            tracks::handle_set_track_pan(ctx, track_id, pan)
        }
        AudioCommand::SetTrackMute { track_id, muted } => {
            tracks::handle_set_track_mute(ctx, track_id, muted)
        }
        AudioCommand::SetMasterVolume { volume } => tracks::handle_set_master_volume(ctx, volume),
        AudioCommand::SetTrackSolo { track_id, soloed } => {
            tracks::handle_set_track_solo(ctx, track_id, soloed)
        }
        AudioCommand::AddTrack { id_hint, name } => {
            tracks::handle_add_track(ctx, state, id_hint, name)
        }
        AudioCommand::CreateSubTrack {
            sub_id,
            parent_track_id,
            output_port_index,
            name,
        } => tracks::handle_create_sub_track(ctx, sub_id, parent_track_id, output_port_index, name),
        AudioCommand::RemoveTrack { track_id } => tracks::handle_remove_track(ctx, state, track_id),
        AudioCommand::SetTrackRecordArm { track_id, armed } => {
            tracks::handle_set_track_record_arm(ctx, track_id, armed)
        }
        AudioCommand::SetTrackMono { track_id, mono } => {
            tracks::handle_set_track_mono(ctx, track_id, mono)
        }
        AudioCommand::SetTrackMonitor { track_id, enabled } => {
            tracks::handle_set_track_monitor(ctx, state, track_id, enabled)
        }
        AudioCommand::SetTrackInputDevice {
            track_id,
            device_name,
        } => tracks::handle_set_track_input_device(ctx, track_id, device_name),
        AudioCommand::SetTrackInputPort {
            track_id,
            port_index,
        } => tracks::handle_set_track_input_port(ctx, track_id, port_index),
        AudioCommand::ListInputDevices => tracks::handle_list_input_devices(ctx),
        AudioCommand::ClearAll => tracks::handle_clear_all(ctx, state),

        // -- Plugins --
        AudioCommand::AddPlugin {
            track_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => plugins::handle_add_plugin(
            ctx,
            state,
            track_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePlugin {
            track_id,
            instance_id,
        } => plugins::handle_remove_plugin(ctx, track_id, instance_id),
        AudioCommand::ScanPlugins => {
            scan::scan_plugins(ctx.plugins, ctx.tracks, &mut state.bundles, ctx.event_tx)
        }
        AudioCommand::SetPluginParam {
            instance_id,
            param_id,
            value,
        } => plugins::handle_set_plugin_param(ctx, instance_id, param_id, value),
        AudioCommand::OpenPluginEditor { instance_id } => {
            plugins::handle_open_plugin_editor(ctx, instance_id)
        }
        AudioCommand::ClosePluginEditor { instance_id } => {
            plugins::handle_close_plugin_editor(ctx, instance_id)
        }
        AudioCommand::SavePluginState { instance_id } => {
            plugins::handle_save_plugin_state(ctx, instance_id)
        }
        AudioCommand::LoadPluginState { instance_id, data } => {
            plugins::handle_load_plugin_state(ctx, instance_id, data)
        }
        AudioCommand::SaveAllPluginStates => plugins::handle_save_all_plugin_states(ctx),

        // -- Bounce --
        AudioCommand::BounceToWav { path } => bounce::to_wav(
            path,
            ctx.shared,
            ctx.tracks,
            ctx.busses,
            ctx.master,
            ctx.clips,
            ctx.midi_clips,
            ctx.plugins,
            ctx.tempo_map,
            ctx.sample_rate,
            ctx.event_tx,
        ),

        // -- Instrument tracks + MIDI --
        AudioCommand::AddInstrumentTrack { id_hint, name } => {
            midi::handle_add_instrument_track(ctx, state, id_hint, name)
        }
        AudioCommand::CreateMidiClip {
            track_id,
            start_sample,
            duration_ticks,
            name,
        } => {
            midi::handle_create_midi_clip(ctx, state, track_id, start_sample, duration_ticks, name)
        }
        AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        } => midi::handle_load_midi_clip_direct(
            ctx,
            state,
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        ),
        AudioCommand::MoveMidiClip {
            clip_id,
            new_start_sample,
            new_track_id,
        } => midi::handle_move_midi_clip(ctx, clip_id, new_start_sample, new_track_id),
        AudioCommand::TrimMidiClip {
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        } => midi::handle_trim_midi_clip(
            ctx,
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        ),
        AudioCommand::DeleteMidiClip { clip_id } => midi::handle_delete_midi_clip(ctx, clip_id),
        AudioCommand::AddMidiNote { clip_id, note } => {
            midi::handle_add_midi_note(ctx, clip_id, note)
        }
        AudioCommand::RemoveMidiNote {
            clip_id,
            note_index,
        } => midi::handle_remove_midi_note(ctx, clip_id, note_index),
        AudioCommand::MoveMidiNote {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        } => midi::handle_move_midi_note(ctx, clip_id, note_index, new_start_tick, new_note),
        AudioCommand::ResizeMidiNote {
            clip_id,
            note_index,
            new_duration_ticks,
        } => midi::handle_resize_midi_note(ctx, clip_id, note_index, new_duration_ticks),
        AudioCommand::SetMidiNoteVelocity {
            clip_id,
            note_index,
            velocity,
        } => midi::handle_set_midi_note_velocity(ctx, clip_id, note_index, velocity),
        AudioCommand::SendNoteOn {
            track_id,
            note,
            velocity,
        } => midi::handle_send_note_on(ctx, state, track_id, note, velocity),
        AudioCommand::SendNoteOff { track_id, note } => {
            midi::handle_send_note_off(ctx, state, track_id, note)
        }
        AudioCommand::ListMidiInputDevices => midi::handle_list_midi_inputs(ctx, state),
        AudioCommand::ListMidiOutputDevices => midi::handle_list_midi_outputs(ctx),
        AudioCommand::SetTrackMidiInput {
            track_id,
            device,
            channel,
        } => midi::handle_set_track_midi_input(ctx, state, track_id, device, channel),
        AudioCommand::SetTrackMidiOutput {
            track_id,
            device,
            channel,
        } => midi::handle_set_track_midi_output(ctx, state, track_id, device, channel),

        // -- Busses --
        AudioCommand::AddBus { id_hint, name } => busses::handle_add_bus(ctx, state, id_hint, name),
        AudioCommand::RemoveBus { bus_id } => busses::handle_remove_bus(ctx, bus_id),
        AudioCommand::SetBusVolume { bus_id, volume } => {
            busses::handle_set_bus_volume(ctx, bus_id, volume)
        }
        AudioCommand::SetBusPan { bus_id, pan } => busses::handle_set_bus_pan(ctx, bus_id, pan),
        AudioCommand::SetBusMute { bus_id, muted } => {
            busses::handle_set_bus_mute(ctx, bus_id, muted)
        }
        AudioCommand::SetBusName { bus_id, name } => busses::handle_set_bus_name(ctx, bus_id, name),
        AudioCommand::SetTrackOutput { track_id, output } => {
            busses::handle_set_track_output(ctx, track_id, output)
        }
        AudioCommand::AddPluginToBus {
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => busses::handle_add_plugin_to_bus(
            ctx,
            state,
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePluginFromBus {
            bus_id,
            instance_id,
        } => busses::handle_remove_plugin_from_bus(ctx, bus_id, instance_id),

        // -- Master FX chain + bypass --
        AudioCommand::AddPluginToMaster {
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => {
            master::handle_add_plugin_to_master(ctx, state, clap_file_path, clap_plugin_id, id_hint)
        }
        AudioCommand::RemovePluginFromMaster { instance_id } => {
            master::handle_remove_plugin_from_master(ctx, instance_id)
        }
        AudioCommand::SetTrackFxBypass { track_id, bypassed } => {
            tracks::handle_set_track_fx_bypass(ctx, track_id, bypassed)
        }
        AudioCommand::SetBusFxBypass { bus_id, bypassed } => {
            busses::handle_set_bus_fx_bypass(ctx, bus_id, bypassed)
        }
        AudioCommand::SetMasterFxBypass { bypassed } => {
            master::handle_set_master_fx_bypass(ctx, bypassed)
        }
    }
}
