//! Pure serialization: build a `ProjectFile` from current GUI state.
//! No I/O, no engine commands, no state mutation — just a transformation
//! from the runtime model to the on-disk shape.

use resonance_audio::types::*;

use crate::project::{
    audio_format_tag, fade_curve_tag, send_source_tag, ProjectBus, ProjectClip,
    ProjectExternalInstrument, ProjectFile, ProjectMidiClip, ProjectPerformance, ProjectPlugin,
    ProjectPoolAsset, ProjectReference, ProjectReferenceMarker, ProjectReferenceSettings,
    ProjectSend, ProjectTrack, PROJECT_FORMAT_VERSION,
};
use crate::Resonance;

/// Serialize one plugin slot: identity, the path of its opaque CLAP
/// state blob, and the parameter values that differ from the plugin's own
/// defaults.
///
/// The explicit parameter list is what makes a saved project able to
/// reproduce its own bounce. The state blob alone could not: nothing
/// re-reads a plugin's params after `LoadPluginState`, so the app-side
/// mirror every reader consults (`track.plugin_params`, the mixer panel,
/// automation, the freeze fingerprint) reverted to instantiation-time
/// defaults on load. See [`crate::project::ProjectPlugin::params`].
///
/// Only non-defaults are written — an untouched chain adds nothing to
/// `project.json`, and a plugin that grows new parameters in a later
/// version picks up its own new defaults for them.
///
/// **A slot with no live instance** (the `.clap` is missing on this
/// machine, so `PluginAdded` never arrived) has an empty `params` mirror
/// — there is no parameter list to diff against defaults, because the
/// host never learned one. Deriving from the mirror would then write
/// `params: []` and silently destroy the user's settings on the first
/// Save As. The values the load parked in
/// [`Resonance::pending_plugin_param_overrides`] are still the file's own,
/// so they are written straight back out, names and all (ba doc #275, P5).
fn project_plugin(r: &Resonance, p: &crate::state::PluginSlotState) -> ProjectPlugin {
    // Present only while an instance is still unaccounted for; consumed
    // by `PluginAdded`, after which the live mirror is authoritative.
    let params = match r.pending_plugin_param_overrides.get(&p.instance_id) {
        Some(parked) => parked.clone(),
        None => p
            .params
            .iter()
            .filter(|param| param.current_value != param.default_value)
            .map(|param| crate::project::ProjectPluginParam {
                id: param.id,
                name: param.name.clone(),
                value: param.current_value,
            })
            .collect(),
    };
    ProjectPlugin {
        instance_id: p.instance_id,
        bypassed: p.bypassed,
        plugin_name: p.plugin_name.clone(),
        clap_plugin_id: p.clap_plugin_id.clone(),
        clap_file_path: p.clap_file_path.clone(),
        state_file: format!("plugins/plugin_{}.bin", p.instance_id),
        params,
    }
}

/// The plugin-state blobs a save must write: every blob the engine just
/// reported for a live instance, plus the app-side copy for every slot
/// the engine said nothing about.
///
/// The engine can only report instances it actually created, so a slot
/// whose `.clap` is missing is absent from `engine_states` — and used to
/// be absent from the written bundle too, which is what destroyed its
/// opaque state on the first Save As (ba doc #275, P5). The fallback
/// comes from [`Resonance::plugin_state_cache`], seeded from the project
/// file at load time, so what gets written back is byte-for-byte what was
/// read.
///
/// Only blobs for slots that are still in a chain are written: a plugin
/// the user removed has already been dropped from the cache, and this
/// second filter keeps a stale entry from resurrecting a `plugin_*.bin`
/// nothing references.
///
/// Shared by every path that writes project state — the async save
/// collector (Save, Save As, autosave) and template capture — so none of
/// them can regress independently.
pub fn plugin_states_for_save(
    r: &Resonance,
    engine_states: Vec<(PluginInstanceId, Vec<u8>)>,
) -> Vec<(PluginInstanceId, Vec<u8>)> {
    let reported: std::collections::HashSet<PluginInstanceId> =
        engine_states.iter().map(|(id, _)| *id).collect();
    let mut out = engine_states;

    let preserve = |slots: &[crate::state::PluginSlotState],
                    out: &mut Vec<(PluginInstanceId, Vec<u8>)>| {
        for slot in slots {
            if reported.contains(&slot.instance_id) {
                continue;
            }
            if let Some(blob) = r.plugin_state_cache.get(&slot.instance_id) {
                out.push((slot.instance_id, blob.to_vec()));
            }
        }
    };
    for track in &r.registry.tracks {
        preserve(&track.plugins, &mut out);
    }
    for bus in &r.registry.busses {
        preserve(&bus.plugins, &mut out);
    }
    preserve(&r.master_plugins, &mut out);

    out
}

/// Serialize current GUI state to the on-disk `ProjectFile` shape.
pub fn build_project_file(r: &Resonance) -> ProjectFile {
    // Ids of the read-only devices shipped in the app binary. A selected
    // *bundled* device is re-resolved from the registry on load, so we never
    // embed a copy of it; a *user-authored* device (id not in this set) is
    // embedded verbatim so the project reopens on another machine (doc #201
    // §5). Computed once so the per-track closure below stays O(1).
    let bundled_device_ids: std::collections::HashSet<String> =
        resonance_common::bundled_definitions()
            .into_iter()
            .map(|d| d.id)
            .collect();

    let tracks = r
        .sorted_tracks()
        .iter()
        .map(|t| ProjectTrack {
            id: t.id,
            name: t.name.clone(),
            order: t.order,
            volume: t.volume,
            pan: t.pan,
            muted: t.muted,
            soloed: t.soloed,
            fx_bypassed: t.fx_bypassed,
            record_armed: t.record_armed,
            monitor_enabled: t.monitor_enabled,
            playback_source: t.playback_source,
            mono: t.mono,
            input_device_name: t.input_device_name.clone(),
            plugins: t.plugins.iter().map(|p| project_plugin(r, p)).collect(),
            track_type: match t.track_type {
                TrackType::Audio => "audio".to_string(),
                TrackType::Instrument => "instrument".to_string(),
                TrackType::Vocal => "vocal".to_string(),
            },
            output_bus: match t.output {
                TrackOutput::Master => None,
                TrackOutput::Bus(id) => Some(id),
            },
            instrument_type: t.instrument_type,
            instrument_icon: t.instrument_icon,
            role: t.role,
            sub_track: t.sub_track,
            input_port_index: Some(t.input_port_index),
            midi_input_device: t.midi_input_device.clone(),
            midi_input_channel: t.midi_input_channel,
            midi_output_device: t.midi_output_device.clone(),
            midi_output_channel: t.midi_output_channel,
            // Project the live freeze status onto its persisted shape.
            // Transient states (Idle / Freezing / Failed) round-trip as
            // "not frozen"; Frozen / Stale persist their cache ref so the
            // load path can re-attach the cache (ba todo #577).
            freeze: r.freeze.status(t.id).to_persisted(),
            // External-instrument extras (bank/program/latency offset). The
            // *presence* of an entry in `external_instruments` marks the
            // track external; the route + monitor/arm already serialize via
            // the track fields above. Runtime offline flags are not saved.
            external_instrument: r.external_instruments.get(&t.id).map(|ext| {
                // Embed a copy of the selected definition only when it's
                // user-authored (not bundled) and still resolvable, so a
                // portable project carries unsupported gear with it while
                // bundled devices stay lean (re-resolved on load).
                let device_definition = ext.device_id.as_ref().and_then(|id| {
                    if bundled_device_ids.contains(id) {
                        None
                    } else {
                        r.device_registry.get(id).cloned()
                    }
                });
                ProjectExternalInstrument {
                    device_id: ext.device_id.clone(),
                    device_definition,
                    bank: ext.bank,
                    program: ext.program,
                    latency_offset_samples: ext.latency_offset_samples,
                }
            }),
        })
        .collect();

    let busses = r
        .sorted_busses()
        .iter()
        .map(|b| ProjectBus {
            id: b.id,
            name: b.name.clone(),
            order: b.order,
            volume: b.volume,
            pan: b.pan,
            muted: b.muted,
            fx_bypassed: b.fx_bypassed,
            plugins: b.plugins.iter().map(|p| project_plugin(r, p)).collect(),
            is_return: b.is_return,
        })
        .collect();

    // Aux-send graph (ba doc #273). The GUI mirror is the engine's own
    // resolved view of the graph — ids allocated, levels clamped — so
    // saving it verbatim saves what is actually playing. Sorted by id for
    // a stable on-disk order that doesn't depend on the order the
    // engine's echoes happened to arrive in.
    let sends = {
        // Only sends whose endpoints still exist. A send is an edge, so
        // it is meaningless once either end is gone -- and writing a
        // dangling one is durable damage rather than a cosmetic wart:
        // the loader refuses it (the engine rejects a missing source or
        // destination), yet the entry stays in the file and every later
        // save rewrites it, accumulating one per deleted track.
        let live_source = |source: SendSource| match source {
            SendSource::Track(id) => r.registry.tracks.iter().any(|t| t.id == id),
            SendSource::Bus(id) => r.registry.busses.iter().any(|b| b.id == id),
        };
        let mut sends: Vec<ProjectSend> = r
            .aux
            .sends
            .iter()
            .filter(|s| live_source(s.source) && r.registry.busses.iter().any(|b| b.id == s.dest))
            .map(|s| {
                let (source_kind, source_id) = send_source_tag(s.source);
                ProjectSend {
                    id: s.id,
                    source_kind: source_kind.to_string(),
                    source_id,
                    dest_bus: s.dest,
                    level_db: s.level_db,
                    pre_fader: s.pre_fader,
                    enabled: s.enabled,
                }
            })
            .collect();
        sends.sort_by_key(|s| s.id);
        sends
    };

    // External sidechain (key) routes (ba doc #157/#159, todo #1311).
    // Same edge discipline as the sends above, with one extra end to
    // check: a route names a *plugin instance* as well as a source, and
    // the plugin can be anywhere — a track chain, a bus chain, or master.
    // `plugin_index` is precisely the "every live plugin instance,
    // wherever it lives" view, so it is the membership test.
    //
    // Writing a dangling route out is durable damage for the same reason
    // a dangling send is: the loader refuses it on reopen, but the entry
    // stays in the file and every later save rewrites it.
    let sidechain_routes = {
        let live_source = |source: SendSource| match source {
            SendSource::Track(id) => r.registry.tracks.iter().any(|t| t.id == id),
            SendSource::Bus(id) => r.registry.busses.iter().any(|b| b.id == id),
        };
        let mut routes: Vec<crate::project::ProjectSidechainRoute> = r
            .sidechain
            .routes
            .iter()
            .filter(|route| {
                r.plugin_index.contains_key(&route.plugin) && live_source(route.source)
            })
            .map(|route| {
                let (source_kind, source_id) = send_source_tag(route.source);
                crate::project::ProjectSidechainRoute {
                    plugin_instance_id: route.plugin,
                    source_kind: source_kind.to_string(),
                    source_id,
                    enabled: route.enabled,
                }
            })
            .collect();
        // Stable on-disk order that doesn't depend on the order the
        // engine's echoes happened to arrive in.
        routes.sort_by_key(|route| route.plugin_instance_id);
        routes
    };

    let clips = r
        .clips
        .iter()
        .map(|c| ProjectClip {
            id: c.id,
            track_id: c.track_id,
            start_sample: c.start_sample,
            name: c.name.clone(),
            total_frames: c.total_frames,
            trim_start_frames: c.trim_start_frames,
            trim_end_frames: c.trim_end_frames,
            audio_file: crate::project::clip_audio_file(c.id),
            // Pool-asset provenance (doc #175): persist the link so an
            // imported+placed clip reconnects to its pool asset on reload.
            asset_ref: c.asset_ref.map(|r| r.asset_id),
            // Fades & per-clip gain (epic #18, doc #156). Curves are
            // stored as tags since `FadeCurve` has no serde derive.
            fade_in_frames: c.fade_in_frames,
            fade_in_curve: fade_curve_tag(c.fade_in_curve).to_string(),
            fade_out_frames: c.fade_out_frames,
            fade_out_curve: fade_curve_tag(c.fade_out_curve).to_string(),
            gain_db: c.gain_db,
        })
        .collect();

    let midi_clips = r
        .midi_clips
        .iter()
        .map(|mc| {
            // Per-note lyric annotations from the side-table, in their
            // canonical file form (trailing empties stripped, so a clip
            // without slurs / overrides doesn't bloat the project file);
            // the replay paths pad back to `notes.len()` on load.
            let vocal_lyrics = r.compose.vocal_audio.file_lyrics(mc.id, mc.notes.len());
            ProjectMidiClip {
                id: mc.id,
                track_id: mc.track_id,
                start_sample: mc.start_sample,
                duration_ticks: mc.duration_ticks,
                name: mc.name.clone(),
                trim_start_ticks: mc.trim_start_ticks,
                trim_end_ticks: mc.trim_end_ticks,
                midi_file: format!("midi/clip_{}.mid", mc.id),
                vocal_lyrics,
                notes: None,
            }
        })
        .collect();

    let master_plugins = r
        .master_plugins
        .iter()
        .map(|p| project_plugin(r, p))
        .collect();

    // Reference A/B block. Persist only the durable facts (path, name,
    // cached loudness, markers); the decoded PCM / waveform are rebuilt by
    // re-issuing `LoadReferenceTrack` on load. The active reference is
    // addressed by index so a reload's reallocated engine ids don't matter.
    let references: Vec<ProjectReference> = r
        .reference
        .entries
        .iter()
        .map(|e| ProjectReference {
            path: e.path.clone(),
            name: e.name.clone(),
            integrated_lufs: e.integrated_lufs,
            markers: e
                .markers
                .iter()
                .map(|m| ProjectReferenceMarker {
                    id: m.id,
                    position_samples: m.position_samples,
                    label: m.label.clone(),
                })
                .collect(),
        })
        .collect();
    let reference_settings = ProjectReferenceSettings {
        monitor_only: true,
        active: r
            .reference
            .active_id
            .and_then(|id| r.reference.index_of(id)),
        ab_source_is_reference: r.reference.ab_source == ABSource::Reference,
        loudness_match: r.reference.loudness_match,
        trim_db: r.reference.trim_db,
        loop_to_mix: r.reference.loop_to_mix,
    };

    // Media pool (doc #175). Persist the durable facts about each
    // imported asset — its project-relative WAV path, source provenance,
    // and the project-rate duration — in import order. The waveform
    // thumbnail and live usage counts are runtime-derived and rebuilt on
    // load, so they're left out of the file.
    let pool_assets: Vec<ProjectPoolAsset> = r
        .pool
        .assets
        .iter()
        .map(|a| ProjectPoolAsset {
            id: a.id,
            project_relative_path: a.project_relative_path.clone(),
            original_path: a.original_path.clone(),
            format: audio_format_tag(a.format).to_string(),
            channels: a.channels,
            source_sample_rate: a.source_sample_rate,
            duration_frames: a.duration_frames,
        })
        .collect();

    ProjectFile {
        version: PROJECT_FORMAT_VERSION,
        sample_rate: r.sample_rate,
        bpm: r.transport.bpm,
        time_sig_num: r.transport.time_sig_num,
        time_sig_den: r.transport.time_sig_den,
        metronome_enabled: r.transport.metronome_enabled,
        master_volume: r.master_volume,
        master_plugins,
        master_fx_bypassed: r.master_fx_bypassed,
        loop_enabled: r.transport.loop_enabled,
        loop_in: r.transport.loop_in,
        loop_out: r.transport.loop_out,
        tracks,
        clips,
        midi_clips,
        busses,
        sends,
        sidechain_routes,
        section_definitions: r.compose.to_project_definitions(),
        section_placements: r.compose.to_project_placements(),
        tempo_events: r.tempo_events.clone(),
        signature_events: r.signature_events.clone(),
        midi_clock_send_enabled: r.midi_clock_send_enabled,
        midi_clock_send_device: r.midi_clock_send_device.clone(),
        midi_clock_recv_enabled: r.midi_clock_recv_enabled,
        midi_clock_recv_device: r.midi_clock_recv_device.clone(),
        // Legacy field — current code persists the full pattern bank
        // below. Kept empty here so projects authored by this build skip
        // straight to the new shape, and the legacy loader only kicks in
        // for files written by older builds.
        drum_groups: Vec::new(),
        drum_patterns: r.compose.drum_patterns.clone(),
        track_groups: r
            .track_groups
            .get_all_groups_sorted()
            .into_iter()
            .cloned()
            .collect(),
        references,
        reference_settings,
        arrangement_markers: r.markers.markers.clone(),
        pool_assets,
        // MIDI quantize state (ba todo #395): the user's groove library
        // and last-used quantize/humanize settings.
        groove_library: r.quantize.groove_library.clone(),
        quantize_settings: r.quantize.settings.clone(),
        // Parameter-automation lanes (epic #14 / epic #40). Sorted by lane id
        // for a stable on-disk order (the mirror is a HashMap). DeviceParam
        // lanes ride along here and are re-applied on load after the owning
        // track's `SetTrackDeviceParams`.
        automation_lanes: {
            let mut lanes: Vec<_> = r.automation.lanes.values().cloned().collect();
            lanes.sort_by_key(|l| l.id);
            lanes
        },
        // Performance-mode footer selection (epic #11): the instrument
        // tuning (stored by stable name) and capo offset for the live
        // fingering diagrams.
        performance: ProjectPerformance {
            tuning: r.performance.tuning().name.to_string(),
            capo: r.performance.capo,
        },
        // Cycle-record take lanes (epic #15, doc #165). The GUI mirror is
        // rebuilt purely from engine events, so saving it verbatim saves
        // the takes the engine actually captured. Sorted by group id for a
        // stable on-disk order that doesn't depend on which order the
        // engine's `TakeCaptured` echoes happened to arrive in.
        //
        // Deliberately NOT filtered the way `sends` / `sidechain_routes`
        // are: a group whose track has since been deleted is written out
        // anyway. Those two are routing *edges*, meaningless without both
        // ends, and a dangling one accumulates in the file forever. A take
        // is recorded content — dropping it on save is exactly the silent
        // loss doc #165 forbids ("no take is ever silently lost"), and it
        // would be unrecoverable, whereas an orphaned group is inert until
        // something references its track again.
        take_groups: {
            let mut groups = r.take_groups.groups.clone();
            groups.sort_by_key(|g| g.id);
            groups
        },
        // Global chord track (epic #33). Pure app-side metadata, never
        // sent to the engine; the transient parse-error banner is left out.
        chord_track: crate::project::ProjectChordTrack::from(&r.chord_track),
    }
}
