//! Domain-specific restore helpers: performance, quantize, media pool,
//! track groups, take lanes, reference A/B, drum patterns, and tempo
//! events. Each is a self-contained side-effecting unit; the ones both
//! restore paths share whole are driven through the `Reconcile` domains in
//! `super::super::reconcile` (ARCH-01 A-13).

use crate::compose::ComposeState;
use crate::project::ProjectFile;
use crate::state::*;
use crate::Resonance;
use resonance_audio::types::*;

/// Restore the track-group (folder track) registry from a saved project
/// file (epic #36, todo #679/#690). Pure app-side state with no engine
/// counterpart — the macro mute/solo/level cascades re-derive from the
/// registry — so the registry is rebuilt wholesale from the saved list.
pub(crate) fn restore_track_groups(r: &mut Resonance, project: &ProjectFile) {
    r.track_groups = crate::state::TrackGroupRegistry::new();
    for tg in &project.track_groups {
        r.track_groups.add_group(tg.clone());
        // Groups share the track-id counter: advance it past every
        // loaded group, or the next Cmd-G / track add reuses a saved
        // group's id (code review STATE-04).
        if tg.id >= r.registry.next_track_id {
            r.registry.next_track_id = tg.id + 1;
        }
    }
}

/// Seed the cycle-record take lanes (epic #15, design doc #165) from a
/// saved project file (or an undo snapshot).
///
/// **Additive on purpose.** This only pushes what the
/// file carries; emptying the previous project's mirror is
/// the `TakeGroups` reconcile domain's job, just before it (via
/// [`TakeGroupState::clear`](crate::state::TakeGroupState::clear)). Keeping
/// the two apart is what makes that clear the single place a project load
/// drops stale take lanes, rather than a rebuild that happens to overwrite
/// them.
///
/// **The engine is told, once, at the end** (ba todo #1394). Take groups
/// are born in the engine — cycle recording calls `capture_take_event` per pass —
/// but a project load has no capture to be born from, so until
/// `AudioCommand::RestoreTakeGroups` existed the engine held no groups
/// after a reload: the lanes were drawn and the comp rendered **silence**,
/// on playback and on bounce alike. The command replaces the engine's
/// store wholesale and republishes the comp table, which is why a loaded
/// comp is audible without touching the transport.
///
/// It carries `r.take_groups.groups` — the mirror as it stands after this
/// function has seeded it — rather than `project.take_groups`. The two are
/// equal at both existing call sites, but sending the mirror means the
/// engine matches what the user is *looking at* whatever a caller did
/// beforehand, so a caller that forgot to clear produces an app-side bug
/// rather than a silent engine/app divergence.
///
/// The send is unconditional: a project with no take lanes must still
/// clear the engine's store, or opening it after a comped project would
/// leave the old comp governing clip ids the new project reuses.
///
/// Missing takes are sent too. Their `clip_ref` names a WAV the engine
/// will not find, so the comp renders that span silent — but the take
/// keeps its id, and every other segment of the cover keeps playing.
/// Dropping it would renumber nothing and break everything.
///
/// **The groups are only half of it** (ba todo #1402). `RestoreTakeGroups`
/// rebuilds the engine's take-group *store*; it does not rebuild the audio
/// those groups name, and nothing else does either — a take clip enters the
/// engine only through the capture path, where `roll_audio_pass` pushes the
/// `AudioClip` straight into the clip list and emits no clip command at
/// all. So a reloaded project still rendered **silence**: the comp table
/// resolved its spans to clip ids the engine did not hold. Each take whose
/// WAV is still on disk therefore gets an
/// [`AudioCommand::LoadTakeClipFromWav`] as well, sent *after* the restore
/// so the clip is governed by the published table from the moment it
/// exists.
///
/// **Missing recorded audio.** An audio take names its WAV by `clip_ref`,
/// resolved against the project directory through
/// [`clip_audio_file`](crate::project::clip_audio_file). A take whose file
/// is gone — the bundle was moved without its `audio/` folder, or a hand
/// edit deleted it — is **kept and flagged**
/// ([`TakeGroupState::mark_missing`](crate::state::TakeGroupState::mark_missing)),
/// never dropped, for the same reason
/// [`restore_pool`] keeps a missing asset: the comp cover references takes
/// by id, so silently dropping one would leave the comp pointing at a take
/// that no longer exists and quietly punch a hole in the composite. Doc
/// #165's acceptance is explicit that no take is ever silently lost.
/// MIDI takes carry their notes inline and can never go missing.
///
/// **One read decides both** (ba todo #1400). The flag used to come from
/// an `exists()` stat and the lane's waveform from a clip lookup that
/// never resolved; now
/// [`load_take_peaks`](crate::project::load_take_peaks) opens the file
/// once and its outcome answers both questions — peaks on success, the
/// missing flag on failure. That widens the flag from "the file is
/// absent" to "the app could not read the recording", which is the fact
/// the lane claims when it hatches a card, and covers a bundle whose WAV
/// survived truncated or in a format the engine cannot map. The engine
/// memory-maps the very same file to play the take, so a file this read
/// rejects is one the comp would render silent anyway.
///
/// **This is not only the load path.** The `TakeGroups` domain on the
/// undo/redo diff-replay path re-runs this function on *every* history
/// step, including one that touches no take, so the expensive half has to
/// be skippable — otherwise a hold-to-repeat undo re-mmaps and re-scans
/// every take WAV in the project (~3.2 ms per recorded minute). A take
/// whose peaks are already cached for the same `clip_ref` therefore skips
/// the read.
///
/// It does **not** skip the cheap half. The step still `exists()`-checks
/// the file, which is exactly what this path cost before todo #1400, so
/// a WAV deleted mid-session still lights the hatch on the next history
/// step rather than going quiet — the property the diff-path restore was
/// written for. The saving is precisely the mmap and the scan, and
/// nothing else changes hands.
pub(crate) fn replay_take_groups(
    r: &mut Resonance,
    project: &ProjectFile,
    project_dir: &std::path::Path,
) {
    // The take clips whose audio is still on disk, in restore order, as
    // `(clip_ref, track_id, start_sample)`. Collected while seeding the
    // mirror and sent after the group restore — see below.
    let mut present_clips: Vec<(resonance_audio::types::ClipId, TrackId, u64)> = Vec::new();

    for group in &project.take_groups {
        r.take_groups.groups.push(group.clone());
        for take in &group.takes {
            let resonance_common::TakeContent::Audio { clip_ref } = take.content else {
                continue;
            };
            // **One read decides both** (ba todo #1400 + #1402, resolved
            // here where the two changes met). `load_take_peaks` opens the
            // WAV once; its outcome says whether the lane can draw the
            // take *and* whether the engine should be asked for the clip.
            //
            // #1402 gated the load on an `exists()` stat, which a corrupt
            // or non-float WAV passes. That split the two answers: the
            // lane drew the take as present while the load worker raised a
            // global error banner and the comp rendered its span silent.
            // Routing the send off the same read collapses the corrupt
            // case into the missing case — one file, one verdict.
            let present = project_dir
                .join(crate::project::clip_audio_file(clip_ref))
                .exists();
            let readable = if present && r.take_groups.has_peaks(group.id, take.id, clip_ref) {
                // Already read this recording in this session, and it is
                // still there. A recording is immutable, so there is
                // nothing a re-read could learn — and this read is the
                // whole diff-replay path's cost. The `exists()` is kept:
                // it is what this step cost before #1400, and it is what
                // still catches a WAV deleted mid-session.
                true
            } else {
                match crate::project::load_take_peaks(project_dir, clip_ref) {
                    Ok(peaks) => {
                        r.take_groups.set_peaks(group.id, take.id, clip_ref, peaks);
                        true
                    }
                    Err(reason) => {
                        // Loud, because the take is otherwise
                        // indistinguishable from one that simply recorded
                        // silence.
                        tracing::warn!(
                            "project load: take {} of group {} has no usable recorded \
                             audio ({reason}) — kept in the lane so the comp stays intact",
                            take.id, group.id
                        );
                        // Drop any table read from this recording before
                        // it went: the take hatches now, and if the file
                        // comes back the next history step re-reads it
                        // rather than trusting a cache entry for a file
                        // that vanished.
                        r.take_groups.forget_peaks(group.id, take.id);
                        r.take_groups.mark_missing(group.id, take.id);
                        false
                    }
                }
            };
            if !readable {
                continue;
            }
            // **The cache skips the read, never the load.** A cache hit
            // still reaches this push, so the engine is told about every
            // readable take on every replay. That keeps a cache about
            // *pixels* from ever deciding what is *audible*: whether the
            // engine needs this clip is a question only the engine can
            // answer, and `handle_load_take_clip_from_wav` answers it by
            // early-returning when the clip is already in its list —
            // before any mmap or decimation. Skipping the push here would
            // instead rest on the app correctly predicting the engine's
            // contents, and would fail silently, as no playback if it were
            // ever wrong.
            //
            // `extent` is this pass's own `[start, +duration)` on the
            // timeline — `RolledAudioTake::extent` is defined as the
            // rolled clip's position — so it is an exact record of where
            // capture put the clip, including a punched-in pass 0 that
            // starts later than the slot.
            if !present_clips.iter().any(|(id, _, _)| *id == clip_ref) {
                present_clips.push((clip_ref, group.track_id, take.extent.start));
            }
        }
    }

    let _ = r.engine.send(AudioCommand::RestoreTakeGroups {
        groups: r.take_groups.groups.clone(),
    });

    // ...and then the audio those groups name (ba todo #1402). The groups
    // alone were never enough: a take clip enters the engine *only* on the
    // capture path, which pushes the `AudioClip` straight into the clip
    // list, so a reloaded project had `build_comp_table` resolving spans to
    // clip ids the engine did not hold and the comp rendered silence — on
    // playback and on bounce alike.
    //
    // **After the restore, not before.** The restore publishes the comp
    // table, which is what marks these clips *governed*; until it does,
    // any take clip in the engine's list is fair game for the ordinary
    // clip path, and a lane's overlapping passes would all play at once.
    // Sending the loads second means a take clip is governed from the
    // first instant it can exist. It does leave the mirror-image window —
    // a table naming a clip not yet loaded — but that one is benign
    // (`mix_track_comp` skips a span whose clip it cannot find) and
    // unavoidable anyway, because the load itself is asynchronous.
    //
    // A take whose WAV is gone is simply not asked for: it stays flagged
    // and stays in the group, so the rest of the cover keeps playing and
    // the load does not fail (ba todo #412's keep-and-flag rule). Asking
    // would only trade a silent span for an error banner.
    //
    // MIDI takes carry their notes inline and name no clip, so they
    // contribute nothing here.
    for (clip_id, track_id, start_sample) in present_clips {
        let _ = r.engine.send(AudioCommand::LoadTakeClipFromWav {
            clip_id,
            track_id,
            start_sample,
            path: project_dir.join(crate::project::clip_audio_file(clip_id)),
            // The name capture gives a take clip, so a restored one is not
            // distinguishable from a freshly recorded one anywhere.
            name: format!("Take {clip_id}"),
        });
    }
}

/// Restore the Performance-mode footer selection (epic #11, todo #312):
/// the instrument tuning and capo offset that drive the live fingering
/// diagrams. Pure app-side state with no engine counterpart, so it's
/// applied directly.
///
/// The persisted tuning is matched by name against
/// [`ALL_TUNINGS`](resonance_music_theory::ALL_TUNINGS); an unknown name —
/// or a legacy project with no `performance` block, which deserializes to
/// the default Guitar 6 / no-capo selection — falls back to the default
/// tuning. Both the tuning index and capo go through
/// [`PerformanceState`](crate::state::PerformanceState)'s setters, so a
/// stale or out-of-range value can never desync or panic the diagram
/// renderer.
pub(crate) fn restore_performance(r: &mut Resonance, project: &ProjectFile) {
    let mut performance = PerformanceState::default();
    if let Some(index) = resonance_music_theory::ALL_TUNINGS
        .iter()
        .position(|t| t.name == project.performance.tuning)
    {
        performance.set_tuning_index(index);
    }
    performance.set_capo(project.performance.capo);
    r.performance = performance;
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
    restore_pool_assets(r, project, Some(project_dir), true);
}

/// [`restore_pool`] for every origin (the `Pool` reconcile domain).
/// `project_dir` is `None` for an untitled project on the undo diff path
/// (which only records with a saved project, so not expected): no asset
/// is flagged missing then, rather than all of them. `reserve_engine_ids`
/// is set after a `ClearAll`, which reset the engine's allocator; the
/// diff path leaves the live allocator alone.
pub(crate) fn restore_pool_assets(
    r: &mut Resonance,
    project: &ProjectFile,
    project_dir: Option<&std::path::Path>,
    reserve_engine_ids: bool,
) {
    use crate::state::pool::PoolAsset;

    // Drop the prior project's assets + usage; keep favourites / recent.
    r.media.pool.clear_assets();

    for pa in &project.pool_assets {
        // Resolve the project-relative WAV path against the project dir.
        // An absolute `project_relative_path` (shouldn't happen, but be
        // defensive) is used as-is by `Path::join`.
        let missing = match project_dir {
            Some(dir) => !dir.join(&pa.project_relative_path).exists(),
            None => false,
        };

        r.media.pool.add(PoolAsset {
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

    // Push the engine's id allocator past every restored id (ba doc #276
    // BUG 2). It is engine-thread-local and starts at 1 each session,
    // and nothing else tells it about a loaded project's assets — so
    // without this the first `pool.import` after opening a project
    // handed out an id the project was already using, and every clip
    // referencing it silently started playing the newly imported file.
    if !reserve_engine_ids {
        return;
    }
    if let Some(above) = r.media.pool.max_asset_id() {
        let _ = r
            .engine
            .send(resonance_audio::types::AudioCommand::ReserveAssetIds { above });
    }
}

/// Where [`restore_references`] takes the A/B *monitor* state from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceMonitorSource {
    /// A disk load: the monitor the project was saved with.
    File,
    /// An undo/redo: the live monitor, untouched (the snapshot carries it
    /// cleared — monitor state is not undo state, ARCH-01 A-5).
    Live,
}

/// Restore the reference A/B block after a `ClearAll` — a disk load or
/// the full-replay undo path. Wipes the previous references, then for each
/// saved entry re-issues `LoadReferenceTrack` (so the PCM / waveform are
/// rebuilt) and re-seeds the GUI mirror with the durable facts — name,
/// path, cached loudness and the user's markers — that the re-decode does
/// not itself carry back. The content (entries, selection, loudness match,
/// trim) comes from `project`; the monitor state from `monitor`.
///
/// A reference whose file has gone missing is kept as a `Missing` entry
/// (name + path preserved) and is *not* sent to the engine, so the panel
/// can show it without crashing or losing the user's markers.
///
/// Engine ids are reallocated here, from the app's session-monotonic
/// allocator (hinted on each `LoadReferenceTrack`), not restarted at 1
/// with the engine's: a late echo from a load `ClearAll` superseded
/// carries an old id, which must not name a restored entry (FU-A5b).
/// Entries the restore does not bring back are dropped, in-flight loads
/// included, so their echoes are stale.
/// Missing entries — which the engine never hears about — take ids from
/// a high, disjoint base so a later in-session load can never collide
/// with one.
///
/// Everything the engine's `ReferencePlayer` holds is re-sent, monitor
/// state included: `ClearAll` reset it to its defaults.
pub(crate) fn restore_references(
    r: &mut Resonance,
    project: &ProjectFile,
    monitor: ReferenceMonitorSource,
) {
    // Drop the previous project's references (entries + settings + any
    // in-flight load bookkeeping), keeping only the live monitor state an
    // undo leaves alone. The engine's own reference state was already
    // emptied by `ClearAll`, allocator included.
    let live_monitor = std::mem::take(&mut r.reference.monitor);
    let next_engine_id = r.reference.next_engine_id;
    let next_marker_id = r.reference.next_marker_id;
    r.reference = crate::reference::ReferenceState::default();
    r.reference.next_engine_id = next_engine_id;
    r.reference.next_marker_id = next_marker_id;

    let mut next_missing_id: u32 = crate::state::ids::MISSING_REFERENCE_ID_BASE;
    for pr in &project.references {
        let entry = seed_reference_entry(r, pr, &mut next_missing_id);
        r.reference.entries.push(entry);
    }

    let settings = &project.reference_settings;
    restore_reference_selection(r, settings.active, None);
    r.reference.loudness_match = settings.loudness_match;
    r.reference.trim_db = settings.trim_db;
    let _ = r.engine.send(AudioCommand::SetRefLoudnessMatch {
        enabled: settings.loudness_match,
    });
    let _ = r.engine.send(AudioCommand::SetRefTrim {
        db: settings.trim_db,
    });

    r.reference.monitor = match monitor {
        ReferenceMonitorSource::Live => live_monitor,
        ReferenceMonitorSource::File => crate::reference::ReferenceMonitorState {
            ab_source: if settings.ab_source_is_reference {
                ABSource::Reference
            } else {
                ABSource::Mix
            },
            loop_to_mix: settings.loop_to_mix,
            ..Default::default()
        },
    };
    let _ = r.engine.send(AudioCommand::SetABSource {
        source: r.reference.monitor.ab_source,
    });
    let _ = r.engine.send(AudioCommand::SetRefLoopToMix {
        enabled: r.reference.monitor.loop_to_mix,
    });
}

/// Reconcile the reference A/B *content* to `project` without a
/// `ClearAll` — the diff-replay undo path, where the engine still holds
/// every live reference. The monitor state is not touched (ARCH-01 A-5).
///
/// Live entries are matched to the saved ones by path, in order. A match
/// keeps its engine id, decoded audio and analysis, and takes the saved
/// name and markers (and the saved loudness while its own analysis is
/// unfinished). A saved entry with no live match is loaded again, under an
/// id from the app's copy of the engine's allocator; a live entry with no
/// saved match is removed from the engine. The selection, loudness match
/// and trim are re-sent only when they changed.
pub(crate) fn reconcile_references(r: &mut Resonance, project: &ProjectFile) {
    use crate::reference::ReferenceStatus;

    let mut live = std::mem::take(&mut r.reference.entries);
    let mut next_missing_id = live
        .iter()
        .map(|e| e.id.0)
        .filter(|&id| id >= crate::state::ids::MISSING_REFERENCE_ID_BASE)
        .max()
        .map_or(crate::state::ids::MISSING_REFERENCE_ID_BASE, |id| id + 1);
    let mut entries = Vec::with_capacity(project.references.len());
    for pr in &project.references {
        let entry = match live.iter().position(|e| e.path == pr.path) {
            Some(i) => {
                let mut e = live.remove(i);
                e.name = pr.name.clone();
                e.markers = reference_markers(pr);
                if e.status != ReferenceStatus::Loaded {
                    e.integrated_lufs = pr.integrated_lufs;
                }
                e
            }
            None => seed_reference_entry(r, pr, &mut next_missing_id),
        };
        entries.push(entry);
    }
    for gone in live {
        if gone.status != ReferenceStatus::Missing {
            let _ = r
                .engine
                .send(AudioCommand::RemoveReferenceTrack { id: gone.id });
        }
    }
    r.reference.entries = entries;

    let settings = &project.reference_settings;
    let engine_active = r.reference.active_id;
    restore_reference_selection(r, settings.active, engine_active);
    if r.reference.loudness_match != settings.loudness_match {
        r.reference.loudness_match = settings.loudness_match;
        let _ = r.engine.send(AudioCommand::SetRefLoudnessMatch {
            enabled: settings.loudness_match,
        });
    }
    if r.reference.trim_db != settings.trim_db {
        r.reference.trim_db = settings.trim_db;
        let _ = r.engine.send(AudioCommand::SetRefTrim {
            db: settings.trim_db,
        });
    }
}

/// The GUI entry for a saved reference that is not live: re-registered
/// with the engine under a hinted id when its file exists, else a
/// `Missing` entry the engine never hears about.
fn seed_reference_entry(
    r: &mut Resonance,
    pr: &crate::project::ProjectReference,
    next_missing_id: &mut u32,
) -> crate::reference::ReferenceEntry {
    use crate::reference::{ReferenceEntry, ReferenceStatus};

    let (id, status) = if std::path::Path::new(&pr.path).exists() {
        let id = r.reference.alloc_engine_id();
        // Re-decode: the engine registers the entry under this id
        // synchronously and streams analysis + `ReferenceLoaded` back,
        // which the folding layer reconciles onto the entry seeded here
        // (preserving its markers).
        let _ = r.engine.send(AudioCommand::LoadReferenceTrack {
            id_hint: Some(id),
            path: std::path::PathBuf::from(&pr.path),
        });
        (id, ReferenceStatus::Analyzing(ReferenceAnalysisStage::Decoding))
    } else {
        let id = ReferenceId(*next_missing_id);
        *next_missing_id += 1;
        r.reference.last_error = Some(format!("Reference file not found: {}", pr.path));
        (id, ReferenceStatus::Missing)
    };
    ReferenceEntry {
        id,
        name: pr.name.clone(),
        path: pr.path.clone(),
        status,
        integrated_lufs: pr.integrated_lufs,
        waveform_peaks: Vec::new(),
        markers: reference_markers(pr),
        position_samples: 0,
        // Filled in by the re-decode's `ReferenceLoaded` echo; a missing
        // file simply never reports one.
        length_samples: 0,
    }
}

fn reference_markers(
    pr: &crate::project::ProjectReference,
) -> Vec<crate::reference::ReferenceMarkerState> {
    pr.markers
        .iter()
        .map(|m| crate::reference::ReferenceMarkerState {
            id: m.id,
            position_samples: m.position_samples,
            label: m.label.clone(),
        })
        .collect()
}

/// Select the saved active reference — an index into the (ordered)
/// entries, mapped back to that entry's id — and bring the engine's
/// selection along. `engine_active` is what the engine has selected now
/// (`None` after a `ClearAll`). Only a reference that actually loaded is
/// engaged on the engine (a Missing one was never registered); anything
/// else leaves the engine with nothing selected.
fn restore_reference_selection(
    r: &mut Resonance,
    active: Option<usize>,
    engine_active: Option<ReferenceId>,
) {
    use crate::reference::ReferenceStatus;

    let target = active
        .and_then(|idx| r.reference.entries.get(idx))
        .map(|e| (e.id, e.status != ReferenceStatus::Missing));
    r.reference.active_id = target.map(|(id, _)| id);
    let engine_target = target.and_then(|(id, present)| present.then_some(id));
    if engine_target == engine_active {
        return;
    }
    match engine_target {
        Some(id) => {
            let _ = r.engine.send(AudioCommand::SetActiveReference { id });
        }
        None => {
            let _ = r.engine.send(AudioCommand::ClearActiveReference);
        }
    }
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
pub(crate) fn restore_drum_patterns(
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
pub(crate) fn restore_tempo_events(r: &mut Resonance, file: &ProjectFile) {
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
