//! Reconstruct GUI state from a `LoadedProject` and replay every required
//! engine command. Called after `AudioEvent::AllCleared` confirms the
//! engine has been emptied. Side-effecting end to end — sends ~20
//! `AudioCommand` variants and mutates almost every sub-state of `Resonance`.

use resonance_audio::types::*;

use crate::compose::ComposeState;
use crate::project::{
    fade_curve_from_tag, LoadedProject, ProjectBus, ProjectFile, ProjectPlugin, ProjectTrack,
};
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
    let _ = r.engine
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
    r.markers = crate::state::ArrangementMarkers::from(project.arrangement_markers.clone());

    // Restore the project's drum pattern bank (with legacy promotion),
    // keeping the `ComposeState::default()` bank in place when the
    // project predates drum groups entirely. Afterwards point the
    // right-rail / modal focus at a pattern that actually exists.
    restore_drum_patterns(&mut r.compose, project, false);
    let first_group_id = r
        .compose
        .drum_patterns
        .first()
        .and_then(|p| p.groups.first().map(|g| g.id));
    r.compose.drumroll.selected_group_id = first_group_id;
    r.compose.drumroll.managing_group_id = first_group_id;
    r.compose.drumroll.managing_pattern_id = r.compose.default_drum_pattern_id;

    restore_tempo_events(r, project);

    let _ = r.engine.send(AudioCommand::SetBpm {
        bpm: r.transport.bpm,
    });
    r.rebuild_and_send_tempo();
    let _ = r.engine.send(AudioCommand::SetTimeSignature {
        numerator: r.transport.time_sig_num,
        denominator: r.transport.time_sig_den,
    });
    let _ = r.engine.send(AudioCommand::SetMetronomeEnabled {
        enabled: r.transport.metronome_enabled,
    });
    let _ = r.engine.send(AudioCommand::SetMasterVolume {
        volume: db_to_gain(r.master_volume),
    });

    // Restore MIDI clock settings. The engine treats `enabled=false`
    // as a no-op port-wise, so it's safe to send for legacy projects.
    r.midi_clock_send_enabled = project.midi_clock_send_enabled;
    r.midi_clock_send_device = project.midi_clock_send_device.clone();
    r.midi_clock_recv_enabled = project.midi_clock_recv_enabled;
    r.midi_clock_recv_device = project.midi_clock_recv_device.clone();
    let _ = r.engine.send(AudioCommand::SetMidiClockOutput {
        device: r.midi_clock_send_device.clone(),
        enabled: r.midi_clock_send_enabled,
    });
    let _ = r.engine.send(AudioCommand::SetMidiClockInput {
        device: r.midi_clock_recv_device.clone(),
        enabled: r.midi_clock_recv_enabled,
    });
    let _ = r.engine.send(AudioCommand::SetLoopRange {
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
    // restored ones. Saved projects from buggier prior versions may have
    // *non-sub-track* ids that fell into the sub-track range (engine's
    // monotonic counter ran past 1_000_000_000 after a long session);
    // include every id, not just `sub_track.is_some()`, so the next
    // `allocate_sub_track_id` skip loop has fewer iterations to do.
    for pt in &project.tracks {
        if pt.id >= r.registry.next_sub_track_id {
            r.registry.next_sub_track_id = pt.id + 1;
        }
    }

    // Stash saved plugin-slot order per track / bus / master so we can
    // re-apply it after all replays + late `PluginAdded` events have
    // resolved. `track_added` / `bus_added` / `master_added` push a new
    // slot at the end whenever they see an `instance_id` that isn't
    // already in the slot list — a `PluginAdded` event arriving for a
    // track whose placeholder hasn't been pushed yet (or for a plugin
    // whose engine-side id differs from the saved hint for any reason)
    // would silently scramble the saved chain order. The post-replay
    // sort below restores it.
    let mut saved_track_plugin_order: std::collections::HashMap<TrackId, Vec<u64>> =
        std::collections::HashMap::with_capacity(project.tracks.len());
    let mut saved_bus_plugin_order: std::collections::HashMap<BusId, Vec<u64>> =
        std::collections::HashMap::with_capacity(project.busses.len());
    for pt in &project.tracks {
        saved_track_plugin_order
            .insert(pt.id, pt.plugins.iter().map(|p| p.instance_id).collect());
    }
    for pb in &project.busses {
        saved_bus_plugin_order
            .insert(pb.id, pb.plugins.iter().map(|p| p.instance_id).collect());
    }
    let saved_master_plugin_order: Vec<u64> = project
        .master_plugins
        .iter()
        .map(|p| p.instance_id)
        .collect();

    for pt in &project.tracks {
        replay_track(r, pt, &loaded);
    }
    // Defensive: older project files weren't guaranteed to be saved in
    // .order sequence, and replay relies on the registry staying sorted
    // by .order for the view layer's invariant.
    r.registry.resort_tracks();
    r.compose.refresh_track_count(&r.registry.tracks);

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
            let _ = r.engine.send(AudioCommand::SetTrackOutput {
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
        let _ = r.engine.send(AudioCommand::LoadClipFromWav {
            clip_id: pc.id,
            track_id: pc.track_id,
            start_sample: pc.start_sample,
            path: abs_path,
            name: pc.name.clone(),
            trim_start_frames: pc.trim_start_frames,
            trim_end_frames: pc.trim_end_frames,
        });

        // Fades & per-clip gain (epic #18, doc #156). `LoadClipFromWav`
        // carries no fade/gain, so push them explicitly after the clip
        // exists; the engine clamps and echoes them back. Only emit when
        // non-default to keep legacy/unfaded projects quiet.
        let fade_in_frames = pc.fade_in_frames;
        let fade_in_curve = fade_curve_from_tag(&pc.fade_in_curve);
        let fade_out_frames = pc.fade_out_frames;
        let fade_out_curve = fade_curve_from_tag(&pc.fade_out_curve);
        let gain_db = pc.gain_db;
        if fade_in_frames != 0 || fade_out_frames != 0 {
            let _ = r.engine.send(AudioCommand::SetClipFade {
                clip_id: pc.id,
                fade_in_frames,
                fade_in_curve,
                fade_out_frames,
                fade_out_curve,
            });
        }
        if gain_db != 0.0 {
            let _ = r.engine.send(AudioCommand::SetClipGain {
                clip_id: pc.id,
                gain_db,
            });
        }

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
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
            gain_db,
            waveform_peaks: Vec::new(), // Will be populated by ClipImported event
            vocal_tuning: None, // Re-derived on demand when the pitch editor opens.
            // Link to the pool asset this clip was placed from, if any
            // (doc #175). Persisted on `ProjectClip`; rebuilt here so
            // imported audio survives reload. `restore_pool` reconciles
            // it against the pool (and recomputes usage) after every clip
            // is in place.
            asset_ref: pc.asset_ref.map(crate::state::pool::AssetRef::new),
        });
    }

    // Replay MIDI clips from the parsed `.mid` files.
    for pmc in &project.midi_clips {
        let notes: Vec<MidiNote> = loaded.midi_notes.get(&pmc.id).cloned().unwrap_or_default();

        let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
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

    // Defensive: re-impose the saved plugin-chain order on every track,
    // bus, and the master chain. See `saved_*_plugin_order` setup above
    // for the race this guards against. Sorting in place is cheap
    // (Rust's sort is adaptive — already-sorted slices are O(n)) and
    // safely no-ops in the common case where placeholders + events
    // landed in the expected order.
    for track in &mut r.registry.tracks {
        if let Some(saved) = saved_track_plugin_order.get(&track.id) {
            sort_plugins_by_saved_order(&mut track.plugins, saved);
        }
    }
    for bus in &mut r.registry.busses {
        if let Some(saved) = saved_bus_plugin_order.get(&bus.id) {
            sort_plugins_by_saved_order(&mut bus.plugins, saved);
        }
    }
    sort_plugins_by_saved_order(&mut r.master_plugins, &saved_master_plugin_order);

    // Re-populate the `with_plugin_mut` side-index from the wholesale
    // replay we just performed. Per-slot inserts would also work but
    // a single rebuild is simpler and keeps `replay_track` / `_bus` /
    // `_master` focused on their own concern.
    r.rebuild_plugin_index();

    // Reference A/B block: re-decode each saved reference and restore the
    // panel settings. References live entirely outside the render/export
    // path (see the engine-side gate in `engine::bounce`), so this never
    // touches the bounced/exported mix.
    restore_references(r, project);

    // Media pool (doc #175): rebuild the imported-asset list, flag any
    // whose backing WAV is missing, and recompute usage counts from the
    // clips' restored asset refs. Resolve relative asset paths against
    // the loaded project's directory — `r.io.project_path` is still
    // `None` here (replay cleared it; the caller restores it afterwards).
    restore_pool(r, project, &loaded.project_dir);

    // Quantize state (ba todo #395): the user's groove library and the
    // last-used quantize/humanize settings. Pure app-side data — no
    // engine command — so it's restored verbatim.
    restore_quantize(r, project);
}

/// Restore the MIDI quantize state (ba todo #395) from a saved project:
/// the user-extracted groove library and the last-used quantize/humanize
/// settings. Pure app-side data with no engine counterpart, so it's a
/// straight copy. Legacy projects (no fields) restore to an empty library
/// and neutral default settings via the `#[serde(default)]` on the file.
pub(crate) fn restore_quantize(r: &mut Resonance, project: &ProjectFile) {
    r.quantize.groove_library = project.groove_library.clone();
    r.quantize.settings = project.quantize_settings.clone();
}

/// Restore the media pool from a saved project (doc #175). Wipes the
/// previous project's assets, then re-adds each persisted asset, marking
/// it [`PoolAsset::missing`](crate::state::pool::PoolAsset::missing) when
/// its backing WAV is no longer present in the project's `audio/`
/// directory — a missing asset is **kept, not dropped**, so its clips
/// survive offline and can be relinked later. Finally recomputes usage
/// counts from the clips' asset refs.
///
/// `project_dir` is the absolute `.rproj` directory used to resolve each
/// asset's project-relative WAV path for the existence check.
///
/// Favourites and recent folders are *not* touched here: they are
/// project-independent user state persisted in `settings.json`, loaded
/// into the pool once at startup.
pub(crate) fn restore_pool(
    r: &mut Resonance,
    project: &ProjectFile,
    project_dir: &std::path::Path,
) {
    use crate::state::pool::PoolAsset;

    // Drop the prior project's assets + usage; keep favourites / recent.
    r.pool.clear_assets();

    for pa in &project.pool_assets {
        // Resolve the project-relative WAV path against the project dir.
        // An absolute `project_relative_path` (shouldn't happen, but be
        // defensive) is used as-is by `Path::join`.
        let abs_path = project_dir.join(&pa.project_relative_path);
        let missing = !abs_path.exists();

        r.pool.add(PoolAsset {
            id: pa.id,
            project_relative_path: pa.project_relative_path.clone(),
            original_path: pa.original_path.clone(),
            format: crate::project::audio_format_from_tag(&pa.format),
            channels: pa.channels,
            source_sample_rate: pa.source_sample_rate,
            duration_frames: pa.duration_frames,
            // Thumbnail peaks are rebuilt off-thread when the pool/browser
            // renders the asset; not persisted, so start empty.
            thumbnail_peaks: Vec::new(),
            missing,
        });
    }

    // Recompute per-asset usage from the clips loaded above (their
    // `asset_ref`s were set during the clip replay). A clip pointing at
    // an asset that didn't load simply isn't counted.
    r.recompute_pool_usage();
}

/// Restore the reference A/B block from a saved project. Wipes any prior
/// project's references, then for each saved entry re-issues
/// `LoadReferenceTrack` (so the PCM / waveform are rebuilt) and re-seeds
/// the GUI mirror with the durable facts — name, path, cached loudness and
/// the user's markers — that the re-decode does not itself carry back.
///
/// A reference whose file has gone missing is kept as a `Missing` entry
/// (name + path preserved) and is *not* sent to the engine, so the panel
/// can show it without crashing or losing the user's markers.
///
/// Engine ids are reallocated here: the engine's reference id counter was
/// reset by `ClearAll`, so present entries take ids `1..=K` in load order,
/// exactly mirroring what the engine allocates as it registers each
/// `LoadReferenceTrack`. Missing entries — which the engine never hears
/// about — take ids from a high, disjoint base so a later in-session load
/// can never collide with one.
pub(crate) fn restore_references(r: &mut Resonance, project: &ProjectFile) {
    use crate::reference::{ReferenceEntry, ReferenceMarkerState, ReferenceStatus};

    /// Base for ids handed to missing (never-registered) references. The
    /// engine allocates reference ids sequentially from 1, so it would
    /// take ~1e9 loads in a single session to reach this — i.e. never.
    const MISSING_ID_BASE: u32 = 1_000_000_000;

    // Drop the previous project's references (entries + settings + any
    // in-flight load bookkeeping). Nothing here talks to the engine — the
    // engine's own reference state was already emptied by `ClearAll`.
    r.reference = crate::reference::ReferenceState::default();

    let settings = &project.reference_settings;

    let mut next_present_id: u32 = 1;
    let mut next_missing_id: u32 = MISSING_ID_BASE;
    for pr in &project.references {
        let exists = std::path::Path::new(&pr.path).exists();
        let markers: Vec<ReferenceMarkerState> = pr
            .markers
            .iter()
            .map(|m| ReferenceMarkerState {
                id: m.id,
                position_samples: m.position_samples,
                label: m.label.clone(),
            })
            .collect();

        let (id, status) = if exists {
            let id = ReferenceId(next_present_id);
            next_present_id += 1;
            // Re-decode: the engine registers the entry under this id
            // synchronously and streams analysis + `ReferenceLoaded` back,
            // which the folding layer reconciles onto the entry we seed
            // below (preserving its markers).
            let _ = r.engine.send(AudioCommand::LoadReferenceTrack {
                id_hint: Some(id),
                path: std::path::PathBuf::from(&pr.path),
            });
            (id, ReferenceStatus::Analyzing(ReferenceAnalysisStage::Decoding))
        } else {
            let id = ReferenceId(next_missing_id);
            next_missing_id += 1;
            r.reference.last_error =
                Some(format!("Reference file not found: {}", pr.path));
            (id, ReferenceStatus::Missing)
        };

        r.reference.entries.push(ReferenceEntry {
            id,
            name: pr.name.clone(),
            path: pr.path.clone(),
            status,
            integrated_lufs: pr.integrated_lufs,
            waveform_peaks: Vec::new(),
            markers,
            position_samples: 0,
            // Filled in by the re-decode's `ReferenceLoaded` echo; a
            // missing file simply never reports one.
            length_samples: 0,
        });
    }

    // Restore the panel settings. The active selection was saved as an
    // index into the (ordered) reference list; map it back to the entry's
    // freshly-allocated id and only engage it on the engine when that
    // reference actually loaded (a Missing one was never registered).
    let active = settings
        .active
        .and_then(|idx| r.reference.entries.get(idx))
        .map(|e| (e.id, e.status != ReferenceStatus::Missing));
    r.reference.active_id = active.map(|(id, _)| id);
    if let Some((id, present)) = active {
        if present {
            let _ = r.engine.send(AudioCommand::SetActiveReference { id });
        }
    }

    r.reference.ab_source = if settings.ab_source_is_reference {
        ABSource::Reference
    } else {
        ABSource::Mix
    };
    r.reference.loudness_match = settings.loudness_match;
    r.reference.trim_db = settings.trim_db;
    r.reference.loop_to_mix = settings.loop_to_mix;

    let _ = r.engine.send(AudioCommand::SetABSource {
        source: r.reference.ab_source,
    });
    let _ = r.engine.send(AudioCommand::SetRefLoudnessMatch {
        enabled: settings.loudness_match,
    });
    let _ = r.engine.send(AudioCommand::SetRefTrim {
        db: settings.trim_db,
    });
    let _ = r.engine.send(AudioCommand::SetRefLoopToMix {
        enabled: settings.loop_to_mix,
    });
}

/// Restore the drum pattern bank from a saved project file (or undo
/// snapshot). Three legacy paths:
///
/// 1. Modern project: `drum_patterns` populated → use it directly.
/// 2. Legacy v2 project: `drum_groups` populated (single flat list) →
///    promote into a one-entry pattern bank named "Main", and point any
///    definition that has no pattern id at it so the lane resolves
///    identically to how the legacy project rendered.
/// 3. Pre-grouped legacy: both fields empty → `clear_on_empty` decides:
///    the full project load keeps the default bank seeded by
///    `ComposeState::default()` in place, while diff replay clears the
///    bank to mirror the snapshot exactly.
///
/// After the bank is hydrated, the project default pattern id is
/// refreshed and `next_id` is bumped past every saved pattern and group
/// id so the manager's "+ New" actions never collide with reserved ids.
pub(super) fn restore_drum_patterns(
    compose: &mut ComposeState,
    file: &ProjectFile,
    clear_on_empty: bool,
) {
    if !file.drum_patterns.is_empty() {
        compose.drum_patterns = file.drum_patterns.clone();
    } else if !file.drum_groups.is_empty() {
        let (patterns, _id) = crate::compose::drumroll::legacy_groups_to_pattern(
            file.drum_groups.clone(),
            &mut compose.next_id,
        );
        compose.drum_patterns = patterns;
        let main_id = compose.drum_patterns.first().map(|p| p.id);
        for def in &mut compose.definitions {
            if def.primary_pattern_id().is_none() {
                def.set_primary_pattern(main_id);
            }
        }
    } else if clear_on_empty {
        compose.drum_patterns.clear();
    }

    compose.default_drum_pattern_id = compose.drum_patterns.first().map(|p| p.id);
    let max_id = compose
        .drum_patterns
        .iter()
        .flat_map(|p| std::iter::once(p.id).chain(p.groups.iter().map(|g| g.id)))
        .max();
    if let Some(m) = max_id {
        compose.next_id = compose.next_id.max(m + 1);
    }
}

/// Restore tempo/signature events from a saved project file (or undo
/// snapshot). If the project has none (legacy), create a single event
/// at bar 0 from the global BPM/sig. Does not talk to the engine — the
/// caller is responsible for `rebuild_and_send_tempo`.
pub(super) fn restore_tempo_events(r: &mut Resonance, file: &ProjectFile) {
    if file.tempo_events.is_empty() {
        r.tempo_events = vec![crate::state::TempoEvent {
            bar: 0,
            bpm: file.bpm,
        }];
    } else {
        r.tempo_events = file.tempo_events.clone();
    }
    if file.signature_events.is_empty() {
        r.signature_events = vec![crate::state::SignatureEvent {
            bar: 0,
            numerator: file.time_sig_num,
            denominator: file.time_sig_den,
        }];
    } else {
        r.signature_events = file.signature_events.clone();
    }
}

/// Reorder `plugins` to match the saved instance-id sequence, leaving
/// any slot whose `instance_id` isn't in `saved` at the end in its
/// current relative order. Missing entries in `saved` (plugins that
/// failed to load and therefore have no live slot) are silently
/// filtered — the absent ids never reach the comparator. Stable, so a
/// chain already in saved order is unchanged.
fn sort_plugins_by_saved_order(plugins: &mut [PluginSlotState], saved: &[u64]) {
    if plugins.len() < 2 || saved.is_empty() {
        return;
    }
    // O(n) index lookup; `saved` is bounded by the per-chain plugin
    // count (single digits in practice, dozens worst-case).
    let position = |id: u64| -> usize {
        saved
            .iter()
            .position(|&s| s == id)
            .unwrap_or(usize::MAX)
    };
    plugins.sort_by_key(|p| position(p.instance_id));
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
        let _ = r.engine.send(AudioCommand::CreateSubTrack {
            sub_id: track_id,
            parent_track_id: link.parent_track_id,
            output_port_index: link.output_port_index,
            name: pt.name.clone(),
        });
    } else if pt.track_type == "instrument" {
        let _ = r.engine.send(AudioCommand::AddInstrumentTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    } else if pt.track_type == "vocal" {
        let _ = r.engine.send(AudioCommand::AddVocalTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    } else {
        let _ = r.engine.send(AudioCommand::AddTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    }

    // Set track properties
    let _ = r.engine.send(AudioCommand::SetTrackVolume {
        track_id,
        volume: db_to_gain(pt.volume),
    });
    let _ = r.engine.send(AudioCommand::SetTrackPan {
        track_id,
        pan: pt.pan,
    });
    let _ = r.engine.send(AudioCommand::SetTrackMute {
        track_id,
        muted: pt.muted,
    });
    let _ = r.engine.send(AudioCommand::SetTrackSolo {
        track_id,
        soloed: pt.soloed,
    });
    let _ = r.engine.send(AudioCommand::SetTrackRecordArm {
        track_id,
        armed: pt.record_armed,
    });
    let _ = r.engine.send(AudioCommand::SetTrackMonitor {
        track_id,
        enabled: pt.monitor_enabled,
    });
    let _ = r.engine.send(AudioCommand::SetTrackMono {
        track_id,
        mono: pt.mono,
    });
    let _ = r.engine.send(AudioCommand::SetTrackFxBypass {
        track_id,
        bypassed: pt.fx_bypassed,
    });
    if let Some(ref device) = pt.input_device_name {
        let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
            track_id,
            device_name: Some(device.clone()),
        });
    }
    if let Some(port_index) = pt.input_port_index {
        let _ = r.engine.send(AudioCommand::SetTrackInputPort {
            track_id,
            port_index,
        });
    }
    if pt.midi_input_device.is_some() {
        let _ = r.engine.send(AudioCommand::SetTrackMidiInput {
            track_id,
            device: pt.midi_input_device.clone(),
            channel: pt.midi_input_channel,
        });
    }
    if pt.midi_output_device.is_some() {
        let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
            track_id,
            device: pt.midi_output_device.clone(),
            channel: pt.midi_output_channel,
        });
    }

    // Build GUI track state.
    let gui_plugins = replay_plugins(r, &pt.plugins, loaded, |pp| AudioCommand::AddPlugin {
        track_id,
        clap_file_path: pp.clap_file_path.clone(),
        clap_plugin_id: pp.clap_plugin_id.clone(),
        id_hint: Some(pp.instance_id),
    });

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
    let _ = r.engine.send(AudioCommand::AddBus {
        id_hint: Some(pb.id),
        name: Some(pb.name.clone()),
    });
    let _ = r.engine.send(AudioCommand::SetBusVolume {
        bus_id: pb.id,
        volume: db_to_gain(pb.volume),
    });
    let _ = r.engine.send(AudioCommand::SetBusPan {
        bus_id: pb.id,
        pan: pb.pan,
    });
    let _ = r.engine.send(AudioCommand::SetBusMute {
        bus_id: pb.id,
        muted: pb.muted,
    });
    let _ = r.engine.send(AudioCommand::SetBusFxBypass {
        bus_id: pb.id,
        bypassed: pb.fx_bypassed,
    });

    let gui_plugins = replay_plugins(r, &pb.plugins, loaded, |pp| AudioCommand::AddPluginToBus {
        bus_id: pb.id,
        clap_file_path: pp.clap_file_path.clone(),
        clap_plugin_id: pp.clap_plugin_id.clone(),
        id_hint: Some(pp.instance_id),
    });

    let mut bus = BusState::new(pb.id, r.registry.next_bus_order, pb.name.clone());
    bus.volume = pb.volume;
    bus.pan = pb.pan;
    bus.muted = pb.muted;
    bus.fx_bypassed = pb.fx_bypassed;
    bus.plugins = gui_plugins;
    r.registry.busses.push(bus);
    r.registry.next_bus_order += 1;
}

fn replay_master(r: &mut Resonance, project: &ProjectFile, loaded: &LoadedProject) {
    r.master_fx_bypassed = project.master_fx_bypassed;
    let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
        bypassed: project.master_fx_bypassed,
    });

    r.master_plugins = replay_plugins(r, &project.master_plugins, loaded, |pp| {
        AudioCommand::AddPluginToMaster {
            clap_file_path: pp.clap_file_path.clone(),
            clap_plugin_id: pp.clap_plugin_id.clone(),
            id_hint: Some(pp.instance_id),
        }
    });
}

/// Replay one saved plugin chain: instantiate each plugin on the engine
/// (via the target-specific `add_command`), restore its saved state
/// blob, and collect placeholder GUI slots. The placeholders' params +
/// has_gui are overwritten when the subsequent PluginAdded event
/// arrives from the engine.
fn replay_plugins(
    r: &mut Resonance,
    plugins: &[ProjectPlugin],
    loaded: &LoadedProject,
    mut add_command: impl FnMut(&ProjectPlugin) -> AudioCommand,
) -> Vec<PluginSlotState> {
    let mut gui_plugins = Vec::with_capacity(plugins.len());
    for pp in plugins {
        let _ = r.engine.send(add_command(pp));
        if let Some(state_data) = loaded.plugin_states.get(&pp.instance_id) {
            let _ = r.engine.send(AudioCommand::LoadPluginState {
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
    gui_plugins
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
// private `migrate_auto_name` / `sort_plugins_by_saved_order` helpers
// aren't reachable from a `tests/` file. See ARCHITECTURE.md → Test
// Layout → Binary-crate exception.
#[cfg(test)]
mod tests {
    use super::{migrate_auto_name, sort_plugins_by_saved_order};
    use crate::state::PluginSlotState;

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

    /// Helper: build a minimal `PluginSlotState` carrying only the
    /// `instance_id` we care about for the order assertions.
    fn slot(id: u64) -> PluginSlotState {
        PluginSlotState::new(
            id,
            format!("plugin_{id}"),
            String::new(),
            String::new(),
            Vec::new(),
            false,
        )
    }

    fn ids(plugins: &[PluginSlotState]) -> Vec<u64> {
        plugins.iter().map(|p| p.instance_id).collect()
    }

    #[test]
    fn restores_saved_order_when_chain_is_scrambled() {
        let mut plugins = vec![slot(30), slot(10), slot(20)];
        sort_plugins_by_saved_order(&mut plugins, &[10, 20, 30]);
        assert_eq!(ids(&plugins), vec![10, 20, 30]);
    }

    #[test]
    fn already_sorted_chain_is_unchanged() {
        let mut plugins = vec![slot(10), slot(20), slot(30)];
        sort_plugins_by_saved_order(&mut plugins, &[10, 20, 30]);
        assert_eq!(ids(&plugins), vec![10, 20, 30]);
    }

    #[test]
    fn appended_plugins_not_in_saved_go_to_end_in_arrival_order() {
        // PluginAdded events for ids 40 and 50 raced ahead of replay
        // and were appended to the chain; saved order is [10, 20, 30].
        let mut plugins = vec![slot(40), slot(20), slot(10), slot(50), slot(30)];
        sort_plugins_by_saved_order(&mut plugins, &[10, 20, 30]);
        assert_eq!(ids(&plugins), vec![10, 20, 30, 40, 50]);
    }

    #[test]
    fn saved_ids_missing_from_chain_are_tolerated() {
        // Plugin 20 failed to load — its placeholder was filtered out.
        // The remaining [30, 10] should still resort to [10, 30].
        let mut plugins = vec![slot(30), slot(10)];
        sort_plugins_by_saved_order(&mut plugins, &[10, 20, 30]);
        assert_eq!(ids(&plugins), vec![10, 30]);
    }

    #[test]
    fn empty_saved_order_is_a_noop() {
        let mut plugins = vec![slot(30), slot(10), slot(20)];
        sort_plugins_by_saved_order(&mut plugins, &[]);
        assert_eq!(ids(&plugins), vec![30, 10, 20]);
    }

    #[test]
    fn single_plugin_chain_is_a_noop() {
        let mut plugins = vec![slot(42)];
        sort_plugins_by_saved_order(&mut plugins, &[1, 2, 3]);
        assert_eq!(ids(&plugins), vec![42]);
    }
}
