//! Drum-kit loader: parses a `drum_samples.json` manifest, decodes the
//! referenced WAV files into `LoadedPad`s, and hands the result back to the
//! audio thread via a `crossbeam_channel`.
//!
//! The loader runs on a dedicated background thread — never on the audio
//! thread and never on the editor/UI thread — and decodes on a small pool
//! of scoped workers it starts per load. It only touches:
//!   * the filesystem (read JSON + WAVs), through the process-wide
//!     [`cache`] of decoded takes (E5),
//!   * the bridge's reporting state (`kit_path`, `kit_status`, progress,
//!     memory figures, the last build for incremental reloads),
//!   * the one-slot mailbox (for publishing the new pad set).

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::TrySendError;

use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::pad_map::KitPads;
use crate::kit::{LoadedPad, SampleData};
use crate::mic_catalog::ManifestMicCatalog;
use crate::KitBridge;

pub mod banks;
pub mod cache;
pub mod decode;
pub mod fallback;
pub mod manifest;
pub mod progress;

pub use banks::{BankRequest, MicBankSetups, MIC_BANKS_STATE_KEY};
pub use cache::{SampleCache, SampleKey};
pub use fallback::{build_fallback_pad, build_fallback_pad_sourced};
pub use manifest::{parse_vel_index, KitManifest, MicSetup, PadMicChoices};
pub use progress::{KitLoadProgress, LoadPhase, ProgressSnapshot};

use decode::{assemble_pad, plan_bank_for_position, plan_overhead_bank, plan_setup, Jobs, Tally};

// The pad -> piece mapping (the Drummica table, `_meta.pads` overrides,
// articulation pairs, display names) is `crate::pad_map`'s.
pub use crate::pad_map::{DRUMMICA_ARTICULATION_ALT, DRUMMICA_MAPPING};

/// Default overhead setup key. Matches the pre-multi-output loader so
/// existing projects load with no audible change.
pub const DEFAULT_OVERHEAD_SETUP: &str = "23_OHsAB_e914";

// ---------------------------------------------------------------------------
// Requests, builds and the status reported by the loader thread.
// ---------------------------------------------------------------------------

/// Everything a kit is decoded from except the sample rate: the manifest,
/// the mic / articulation choices and the streaming preload. Two loads
/// with equal requests at the same rate decode the same kit.
#[derive(Debug, Clone, PartialEq)]
pub struct KitRequest {
    pub path: PathBuf,
    pub overhead_setup_key: String,
    pub pad_choices: [PadMicChoices; NUM_PADS],
    pub articulations: [bool; NUM_PADS],
    /// Frames of each take kept resident; the rest of a longer take
    /// streams from disk (E14). 0 keeps every take whole.
    pub preload: u32,
    /// The E15 banks: overhead slots 2 and 3, bleed, room. Off by default.
    pub banks: BankRequest,
}

impl KitRequest {
    /// What pad `pad` of this request is built from, besides the kit.
    /// `has_overhead`: whether the pad's piece has an overhead to pick
    /// with the overhead key at all (see [`piece_uses_overhead_key`]);
    /// if not, the key is no part of the pad, and changing it must not
    /// rebuild it.
    pub fn pad_request(&self, pad: usize, has_overhead: bool) -> PadRequest {
        PadRequest {
            articulation: self.articulations[pad],
            close_setups: self.pad_choices[pad].clone(),
            overhead_setup_key: has_overhead.then(|| self.overhead_setup_key.clone()),
            extra_banks: Vec::new(),
        }
    }
}

/// What one pad is built from, besides the kit and the rate. Two builds
/// of a pad with equal `PadRequest`s from the same manifest at the same
/// rate are the same pad, which is what lets a reload keep it (E4).
#[derive(Debug, Clone, PartialEq)]
pub struct PadRequest {
    pub articulation: bool,
    pub close_setups: PadMicChoices,
    /// `None` for a pad the overhead key cannot change: the built-in
    /// pads, and pieces with no overhead setup.
    pub overhead_setup_key: Option<String>,
    /// The E15 banks the pad plays, as resolved against its piece
    /// ([`banks::resolve_extra_banks`]): empty unless one is on and the
    /// piece has it, so turning a bank on rebuilds only the pads that
    /// gain one.
    pub extra_banks: Vec<(crate::kit::BankKind, String)>,
}

/// Whether the overhead setup key can change what `piece` loads as its
/// overhead: it has an overhead setup — the key's, or one the overhead
/// falls back to ([`banks::resolve_overhead_slot1`]). A piece with none
/// plays no overhead whatever the key names.
pub fn piece_uses_overhead_key(
    piece: &std::collections::BTreeMap<String, MicSetup>,
    kinds: &banks::MicKinds,
) -> bool {
    piece.values().any(|setup| kinds.is_overhead(&setup.position))
}

/// The manifest file as it was when a kit was built from it, so a reload
/// can tell an untouched manifest from an edited one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestStamp {
    pub modified: Option<std::time::SystemTime>,
    pub len: u64,
}

impl ManifestStamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// A finished kit build, kept on the bridge ([`KitBridge::built_kit`]) so
/// the next load of the same kit at the same rate rebuilds only the pads
/// whose [`PadRequest`] changed (E4) and clones the rest. Its pads share
/// their sample memory with the kit the sampler plays while the sampler
/// plays it; once it does not, the build alone keeps that memory alive,
/// which is why `initialize` drops a build that cannot donate.
#[derive(Clone)]
pub struct BuiltKit {
    pub path: PathBuf,
    pub manifest: Option<ManifestStamp>,
    pub sample_rate: f32,
    /// The preload the takes were split at (E14).
    pub preload: u32,
    pub pad_requests: Vec<PadRequest>,
    pub pads: Vec<LoadedPad>,
    /// What building each pad found, in pad order.
    pub pad_builds: Vec<PadBuild>,
}

/// What building one pad found, kept with the build so a reload that
/// reuses the pad still reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PadBuild {
    /// The pad's files that could not be read or decoded.
    pub unreadable: usize,
    /// The first few of them.
    pub unreadable_paths: Vec<PathBuf>,
    /// Bytes of the pad's takes that another instance held when this one
    /// got them (see [`is_shared`]).
    pub shared_bytes: u64,
    /// Those takes, by [`take_addr`].
    pub shared_takes: HashSet<usize>,
}

/// A kit this instance holds outside its last build — the built-in kit
/// `initialize` installed — and which of its takes it found shared.
/// Kept on the bridge ([`KitBridge::builtin_kit`]) while the sampler
/// plays it, so a load can tell those takes from another instance's.
#[derive(Clone, Default)]
pub struct HeldKit {
    pub pads: Vec<LoadedPad>,
    pub shared_takes: HashSet<usize>,
}

/// Takes an instance already holds, by [`take_addr`], each with whether
/// it was shared with another instance when this instance got it.
pub type HeldTakes = HashMap<usize, bool>;

/// A take's identity in [`HeldTakes`]: its address. Only compared while
/// the take is held, so the address cannot have been reused.
pub fn take_addr(sample: &Arc<SampleData>) -> usize {
    Arc::as_ptr(sample) as usize
}

/// Whether a take fetched from `source` is memory shared with another
/// instance. A decode is this instance's alone. A cache hit is shared —
/// unless this instance already held the take, in which case it is
/// whatever it was when this instance first got it: a reload must
/// neither count the instance's own takes as shared nor forget that a
/// take it reuses is shared.
pub fn is_shared(sample: &Arc<SampleData>, source: cache::Source, held: &HeldTakes) -> bool {
    match source {
        cache::Source::Decoded => false,
        cache::Source::Cached => held.get(&take_addr(sample)).copied().unwrap_or(true),
    }
}

/// Add every take of `pads` to `held`, marked shared when its address
/// is in `shared`.
fn hold_takes<'a>(
    held: &mut HeldTakes,
    pads: impl IntoIterator<Item = (&'a LoadedPad, &'a HashSet<usize>)>,
) {
    for (pad, shared) in pads {
        for take in pad
            .banks()
            .flat_map(|bank| bank.layers.iter())
            .flat_map(|layer| layer.round_robins.iter())
        {
            let addr = take_addr(take.shared());
            held.insert(addr, shared.contains(&addr));
        }
    }
}

/// The built-in kit `pads`, measured: the bytes of it another instance
/// already held, and the kit to keep as [`KitBridge::builtin_kit`].
/// `sources` says where each pad's take came from (`None`: the pad has
/// none), as [`fallback::build_fallback_pad_sourced`] reported it; `held`
/// is what this instance held before.
pub fn measure_builtin_kit(
    pads: &[LoadedPad],
    sources: &[Option<cache::Source>],
    held: &HeldTakes,
) -> (u64, HeldKit) {
    let mut shared_bytes = 0;
    let mut kit = HeldKit::default();
    for (pad, source) in pads.iter().zip(sources) {
        if let Some(source) = source {
            for take in pad
                .close_mics
                .iter()
                .flat_map(|bank| bank.layers.iter())
                .flat_map(|layer| layer.round_robins.iter())
            {
                if is_shared(take.shared(), *source, held) {
                    shared_bytes += take.bytes() as u64;
                    kit.shared_takes.insert(take_addr(take.shared()));
                }
            }
        }
        kit.pads.push(pad.clone());
    }
    (shared_bytes, kit)
}

impl HeldKit {
    /// This kit's takes as [`HeldTakes`].
    pub fn held_takes(&self, held: &mut HeldTakes) {
        hold_takes(held, self.pads.iter().map(|pad| (pad, &self.shared_takes)));
    }
}

impl BuiltKit {
    /// This build's takes as [`HeldTakes`].
    pub fn held_takes(&self, held: &mut HeldTakes) {
        hold_takes(
            held,
            self.pads
                .iter()
                .zip(self.pad_builds.iter().map(|b| &b.shared_takes)),
        );
    }
}

impl BuiltKit {
    /// Pad `pad` of this build, if a build of `request` at `sample_rate`
    /// would give the same pad.
    ///
    /// A pad that lost takes is never reused: the files may be readable
    /// now (a kit still extracting, a disk remounted), and a reload is
    /// the moment to try them again.
    fn reusable_pad(
        &self,
        path: &Path,
        manifest: Option<&ManifestStamp>,
        sample_rate: f32,
        preload: u32,
        pad: usize,
        request: &PadRequest,
    ) -> Option<(&LoadedPad, &PadBuild)> {
        let same_kit = self.path == path
            && manifest.is_some()
            && self.manifest.as_ref() == manifest
            && self.sample_rate.to_bits() == sample_rate.to_bits()
            && self.preload == preload;
        if !same_kit || self.pad_requests.get(pad) != Some(request) {
            return None;
        }
        let build = self.pad_builds.get(pad)?;
        if build.unreadable > 0 {
            return None;
        }
        Some((self.pads.get(pad)?, build))
    }
}

/// The kit a loader last put in the audio thread's mailbox, and the rate
/// it was decoded at. Recorded under [`KitBridge::kit_handoff`]; `None`
/// once nobody can say what the sampler holds (a direct
/// [`hand_off_kit`], or `initialize` reverting to the built-in kit).
#[derive(Debug, Clone, PartialEq)]
pub struct HandedOffKit {
    pub request: KitRequest,
    pub sample_rate: f32,
    /// What each of its pads was built from ([`BuiltKit::pad_requests`]).
    pub pad_requests: Vec<PadRequest>,
}

#[derive(Debug, Clone, Default)]
pub enum KitStatus {
    #[default]
    Empty,
    Loading { path: PathBuf },
    /// The kit is loaded. `unreadable` sample files could not be read or
    /// decoded and were left out (E6) — the editor shows "3 samples
    /// unreadable"; `unreadable_paths` holds the first few of them.
    Loaded {
        name: String,
        num_pads: usize,
        unreadable: usize,
        unreadable_paths: Vec<PathBuf>,
    },
    Error { message: String },
}

/// What one load did: the decode counter (a test hook as much as a
/// readout) and the partial-kit tally.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadStats {
    /// Pads carried over unchanged from the previous build (E4).
    pub reused_pads: usize,
    /// Pads built by this load (from the manifest or the built-in kit).
    pub rebuilt_pads: usize,
    /// Sample files the rebuilt pads reference.
    pub files: usize,
    /// Files decoded from disk by this load.
    pub decoded: usize,
    /// Files the shared cache already held (another instance, or this
    /// instance's previous kit).
    pub cached: usize,
    /// The kit's files that could not be read or decoded, and were left
    /// out — every pad's, not only the pads this load rebuilt.
    pub unreadable: usize,
    /// The first few unreadable files.
    pub unreadable_paths: Vec<PathBuf>,
    /// Bytes of decoded audio the kit holds.
    pub kit_bytes: u64,
    /// Of `kit_bytes`, what was already decoded by another instance when
    /// this instance got it — memory shared with another instance. Over
    /// the whole kit: a reused pad carries its figure from the load that
    /// built it.
    pub shared_bytes: u64,
}

/// Output of a successful kit load — the decoded pads, a snapshot of the
/// manifest's mic catalog for the GUI, and what the load did.
pub struct LoadedKit {
    pub pads: Vec<LoadedPad>,
    /// Which piece each pad plays, its name, articulation pair and the
    /// kit's port / choke hints (E10). Absent pads are in `pads` with no
    /// banks.
    pub kit_pads: KitPads,
    pub catalog: ManifestMicCatalog,
    pub stats: LoadStats,
    /// The build, for the next load to reuse pads from.
    pub built: BuiltKit,
}

// ---------------------------------------------------------------------------
// Public loader entrypoints.
// ---------------------------------------------------------------------------

/// Parse the manifest at `manifest_path`, decode every referenced sample at
/// `target_sr`, and return the assembled pad list + catalog of available
/// mic setups for the GUI. A full load: nothing is reused from an earlier
/// build (the shared cache still serves takes another kit holds).
///
/// `articulations` is a per-pad boolean: when true, the loader uses the
/// pad's alternate piece instead of its primary one, if the kit pairs it
/// with one ([`crate::pad_map`]).
pub fn load_kit_from_manifest(
    manifest_path: &Path,
    target_sr: f32,
    overhead_setup_key: &str,
    pad_choices: &[PadMicChoices; NUM_PADS],
    articulations: &[bool; NUM_PADS],
) -> Result<LoadedKit, String> {
    let request = KitRequest {
        path: manifest_path.to_path_buf(),
        overhead_setup_key: overhead_setup_key.to_string(),
        pad_choices: pad_choices.clone(),
        articulations: *articulations,
        preload: crate::stream::DEFAULT_PRELOAD,
        banks: BankRequest::default(),
    };
    load_kit(
        &request,
        target_sr,
        None,
        None,
        cache::global(),
        &|| {},
        &|_| {},
        &|| false,
    )
}

/// The error a [`load_kit`] of a kit with no piece on any pad returns.
pub const NO_MAPPABLE_PADS: &str =
    "no pads this plugin can map — the kit needs _meta.pads or Drummica piece names";

/// The error a [`load_kit`] that was cancelled mid-decode returns.
pub const LOAD_CANCELLED: &str = "load cancelled";

/// Load `request` at `target_sr`.
///
/// - Pads whose [`PadRequest`] is unchanged from `previous` (same kit,
///   same manifest file, same rate) are cloned from it — no file is
///   touched for them (E4).
/// - Every other pad's files are fetched through `cache`, decoding only
///   what no one holds yet (E5), on [`decode::decode_workers`] threads.
/// - A file that cannot be read or decoded drops that take's cell (its
///   velocity and round robin) from every bank of the pad, so the banks
///   stay aligned; a layer left empty, a bank left empty — or never
///   readable at all — goes too (see [`decode::assemble_pad`]). The kit
///   still loads and the count is in
///   [`LoadStats::unreadable`] (E6). A pad whose every file failed is
///   silent: it keeps its slot with no banks, and plays nothing. Only a
///   kit in which *no* file could be read fails.
///
/// `builtin` is the built-in kit this instance plays, if it does: with
/// `previous`, what tells this instance's own takes from another's when
/// the cache serves them ([`LoadStats::shared_bytes`]).
///
/// `set_total` is told how many files the load reads once it knows, and
/// `file_done` runs once per file finished (on a decode worker).
/// `cancelled` is asked before each file: once it says yes, the decode
/// stops starting files and the load fails with [`LOAD_CANCELLED`].
#[allow(clippy::too_many_arguments)]
pub fn load_kit(
    request: &KitRequest,
    target_sr: f32,
    previous: Option<&BuiltKit>,
    builtin: Option<&HeldKit>,
    cache: &SampleCache,
    file_done: &(dyn Fn() + Sync),
    set_total: &dyn Fn(usize),
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<LoadedKit, String> {
    let manifest_path = request.path.as_path();
    let manifest_stamp = ManifestStamp::of(manifest_path);
    let bytes = std::fs::read(manifest_path).map_err(|e| format!("read manifest: {e}"))?;

    // Two-phase parse: first as raw JSON so we can take out the optional
    // `_meta` key (which has a different shape than a drum piece, and is
    // read by the kit library's parser), then deserialize the remaining
    // entries as the usual KitManifest.
    let mut raw: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse manifest JSON: {e}"))?;
    let meta = raw
        .as_object_mut()
        .and_then(|obj| obj.remove(resonance_common::drumkit_library::META_KEY))
        .map(|meta| resonance_common::drumkit_library::KitMeta::from_value(&meta))
        .unwrap_or_default();
    let manifest: KitManifest =
        serde_json::from_value(raw).map_err(|e| format!("parse manifest pieces: {e}"))?;
    let kit_pads = KitPads::resolve(|piece| manifest.contains_key(piece), &meta);
    // A kit none of whose pieces lands on a pad would "load" as 30 silent
    // pads, replacing whatever played with nothing.
    if !kit_pads.pads.iter().any(|pad| pad.present) {
        return Err(NO_MAPPABLE_PADS.to_string());
    }

    let kit_dir = manifest_path
        .parent()
        .ok_or_else(|| "manifest path has no parent directory".to_string())?;

    // What each mic position is: the kit's `_meta.mic_kinds`, else a
    // guess from its name (E15).
    let kinds = banks::MicKinds::from_meta(&meta);
    let catalog = ManifestMicCatalog::with_bleed(&manifest, &kit_pads, &kinds);

    // 1. Plan: reuse, absent (D7: silent, never the built-in sample), or
    // the piece's banks per pad.
    enum PadPlan {
        Reuse(LoadedPad, PadBuild),
        Absent,
        Piece {
            close: Vec<decode::BankPlan>,
            overhead: Option<decode::BankPlan>,
            extras: Vec<(crate::kit::BankKind, decode::BankPlan)>,
        },
    }
    let pad_requests: Vec<PadRequest> = (0..NUM_PADS)
        .map(|i| {
            let piece = kit_pads
                .piece_for(i, request.articulations[i])
                .and_then(|piece| manifest.get(piece));
            let has_overhead = piece
                .is_some_and(|piece| {
                    piece_uses_overhead_key(piece, &kinds)
                });
            let mut pad = request.pad_request(i, has_overhead);
            if let Some(piece) = piece {
                pad.extra_banks = banks::resolve_extra_banks(
                    i,
                    piece,
                    &request.overhead_setup_key,
                    &request.pad_choices,
                    &request.banks,
                    &kinds,
                );
            }
            // The parameter only means something on a pad the kit pairs:
            // elsewhere both values play the same piece, and moving it
            // must not rebuild the pad.
            pad.articulation &= kit_pads.pads[i].articulation.is_some();
            pad
        })
        .collect();
    let mut jobs = Jobs::default();
    let mut plans = Vec::with_capacity(NUM_PADS);
    for (pad_idx, mapping) in PAD_MAPPINGS.iter().enumerate() {
        if let Some((pad, build)) = previous.and_then(|prev| {
            prev.reusable_pad(
                manifest_path,
                manifest_stamp.as_ref(),
                target_sr,
                request.preload,
                pad_idx,
                &pad_requests[pad_idx],
            )
        }) {
            plans.push(PadPlan::Reuse(pad.clone(), build.clone()));
            continue;
        }
        let Some((piece_name, piece)) = kit_pads
            .piece_for(pad_idx, request.articulations[pad_idx])
            .and_then(|name| manifest.get_key_value(name))
            .map(|(name, piece)| (name.as_str(), piece))
        else {
            plans.push(PadPlan::Absent);
            continue;
        };
        let mut close = Vec::with_capacity(mapping.close_mic_positions.len());
        for position in mapping.close_mic_positions {
            if let Some(bank) = plan_bank_for_position(
                piece_name,
                piece,
                kit_dir,
                position,
                request.pad_choices[pad_idx]
                    .close_setups
                    .get(*position)
                    .map(String::as_str),
                &mut jobs,
            )? {
                close.push(bank);
            }
        }
        // Overhead bank: look up the global overhead setup key directly. If
        // the piece doesn't have that specific setup, fall back to any
        // overhead setup the piece does have so the pad still makes sound.
        let overhead = plan_overhead_bank(
            piece_name,
            piece,
            kit_dir,
            &request.overhead_setup_key,
            &kinds,
            &mut jobs,
        )?;
        // The E15 banks, as the pad's request resolved them.
        let mut extras = Vec::with_capacity(pad_requests[pad_idx].extra_banks.len());
        for (kind, key) in &pad_requests[pad_idx].extra_banks {
            if let Some(plan) = plan_setup(piece_name, piece, kit_dir, key, &mut jobs)? {
                extras.push((*kind, plan));
            }
        }
        plans.push(PadPlan::Piece {
            close,
            overhead,
            extras,
        });
    }

    // 2. Decode.
    set_total(jobs.paths.len());
    let results = decode::decode_all(
        &jobs.paths,
        target_sr,
        request.preload,
        cache,
        file_done,
        cancelled,
    );
    if cancelled() {
        return Err(LOAD_CANCELLED.to_string());
    }

    // 3. Assemble. Takes this instance already held (its previous build,
    // the built-in kit it plays) are not "shared" with anyone else just
    // because the cache served them — nor stop being shared because a
    // reload served them again (see `is_shared`).
    let mut held = HeldTakes::new();
    if let Some(prev) = previous {
        prev.held_takes(&mut held);
    }
    if let Some(builtin) = builtin {
        builtin.held_takes(&mut held);
    }
    let mut tally = Tally::default();
    let mut stats = LoadStats {
        files: jobs.paths.len(),
        ..LoadStats::default()
    };
    let mut pads = Vec::with_capacity(NUM_PADS);
    let mut pad_builds = Vec::with_capacity(NUM_PADS);
    for (pad_idx, (plan, mapping)) in plans.into_iter().zip(PAD_MAPPINGS.iter()).enumerate() {
        let (pad, build) = match plan {
            PadPlan::Reuse(pad, build) => {
                stats.reused_pads += 1;
                (pad, build)
            }
            PadPlan::Absent => {
                stats.rebuilt_pads += 1;
                (
                    LoadedPad {
                        name: kit_pads.pads[pad_idx].name.clone(),
                        choke_group: mapping.choke_group,
                        output_group: mapping.output_group,
                        close_mics: Vec::new(),
                        extra_banks: Vec::new(),
                        overhead: None,
                    },
                    PadBuild::default(),
                )
            }
            PadPlan::Piece {
                close,
                overhead,
                extras,
            } => {
                stats.rebuilt_pads += 1;
                let mut pad_tally = Tally::default();
                let (close_mics, overhead, extra_banks) = assemble_pad(
                    &close,
                    overhead.as_ref(),
                    &extras,
                    &results,
                    &jobs.paths,
                    &held,
                    &mut pad_tally,
                );
                tally.decoded += pad_tally.decoded;
                tally.cached += pad_tally.cached;
                let kit_pad = &kit_pads.pads[pad_idx];
                let pad = LoadedPad {
                    name: kit_pad.name.clone(),
                    choke_group: kit_pad.choke_group(pad_idx),
                    output_group: kit_pad.output_group(pad_idx),
                    close_mics,
                    overhead,
                    extra_banks,
                };
                let build = PadBuild {
                    unreadable: pad_tally.unreadable,
                    unreadable_paths: pad_tally.unreadable_paths,
                    shared_bytes: pad_tally.shared_bytes,
                    shared_takes: pad_tally.shared_takes,
                };
                (pad, build)
            }
        };
        // Every pad's unreadable files and shared bytes count, reused or
        // rebuilt: the kit's figures are the kit's, not this load's.
        tally.unreadable += build.unreadable;
        tally.shared_bytes += build.shared_bytes;
        for path in &build.unreadable_paths {
            if tally.unreadable_paths.len() < decode::UNREADABLE_PATHS_KEPT {
                tally.unreadable_paths.push(path.clone());
            }
        }
        pads.push(pad);
        pad_builds.push(build);
    }

    if tally.unreadable > 0 && tally.decoded + tally.cached == 0 && stats.reused_pads == 0 {
        return Err(format!(
            "none of the kit's {} sample files could be read (first: {})",
            tally.unreadable,
            tally
                .unreadable_paths
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ));
    }

    stats.decoded = tally.decoded;
    stats.cached = tally.cached;
    stats.unreadable = tally.unreadable;
    stats.unreadable_paths = tally.unreadable_paths;
    stats.shared_bytes = tally.shared_bytes;
    stats.kit_bytes = crate::sample_info::total_sample_bytes(&pads) as u64;

    let built = BuiltKit {
        path: request.path.clone(),
        manifest: manifest_stamp,
        sample_rate: target_sr,
        preload: request.preload,
        pad_requests,
        pads: pads.clone(),
        pad_builds,
    };
    Ok(LoadedKit {
        pads,
        kit_pads,
        catalog,
        stats,
        built,
    })
}

/// Whether `built` — a build of `request` at `sample_rate` — is the kit
/// `handed` already put in the mailbox: the same kit file at the same
/// rate and preload, every pad built from the same resolved
/// [`PadRequest`].
///
/// Per pad, not per [`KitRequest`]: a request differs in ways that build
/// nothing — an articulation on a pad the kit does not pair, `room_on`
/// on a kit with no room mics, bleed on a kit without bleed, a room
/// setup while room is off, an overhead slot naming the setup slot 1
/// already plays — and handing the same kit off again for those would
/// fade every voice and restart the round robins.
pub fn same_kit_built(
    handed: &HandedOffKit,
    request: &KitRequest,
    sample_rate: f32,
    built: &BuiltKit,
) -> bool {
    handed.sample_rate.to_bits() == sample_rate.to_bits()
        && handed.request.path == request.path
        && handed.request.preload == request.preload
        && handed.pad_requests == built.pad_requests
}

/// Spawn a background loader thread. Writes status updates and the kit path
/// to `bridge`, and publishes the finished pad vec through `bridge.kit_sender`.
///
/// Each call bumps `bridge.load_generation`; in-flight older loads check
/// the stamp before writing state and become no-ops if a newer load has
/// started, so last-click-wins status is preserved even under spam.
/// Loader panics are caught and converted to `KitStatus::Error`.
///
/// The requested path is recorded in [`KitBridge::pending_kit`] at once,
/// not on success, so a re-activation in the middle of the decode reloads
/// *this* kit rather than the last one that finished. And a decode only
/// reaches the audio thread if the host is still running at `target_sr`
/// when it lands: one that straddles a deactivation is dropped, and the
/// `initialize` that follows reloads the pending kit at the new rate.
///
/// The load is incremental (E4): pads unchanged since the last build
/// ([`KitBridge::built_kit`]) are reused, so a mic or articulation change
/// decodes only the pads it touches. Progress is published on
/// [`KitBridge::load_progress`].
pub fn spawn_loader(
    manifest_path: PathBuf,
    target_sr: f32,
    bridge: &KitBridge,
    overhead_setup_key: String,
    pad_choices: [PadMicChoices; NUM_PADS],
    articulations: [bool; NUM_PADS],
) {
    let bridge = bridge.clone();
    // Stamp and record under one lock, so the pending entry always
    // belongs to the newest generation.
    let stamp = {
        let mut pending = bridge.pending_kit.lock();
        let stamp = bridge.load_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *pending = Some((stamp, manifest_path.clone()));
        bridge.load_progress.begin(stamp);
        stamp
    };
    // Record what this load is being built from, in call order, so the
    // articulation watcher compares the params against the kit that is
    // actually (being) decoded rather than re-triggering every poll.
    *bridge.loaded_articulations.lock() = articulations;
    // The E15 banks the kit is built with: their setups (state) and
    // on/off params, recorded like the articulations so the watcher
    // reloads when a param moves.
    let banks = bridge.bank_request();
    *bridge.loaded_bank_flags.lock() = (banks.bleed, banks.room);

    let request = KitRequest {
        path: manifest_path,
        overhead_setup_key,
        pad_choices,
        articulations,
        preload: bridge.stream_preload.load(Ordering::Relaxed),
        banks,
    };

    std::thread::Builder::new()
        .name("resonance-drums-loader".to_string())
        .spawn(move || {
            // Publish "Loading" only if we're still the latest load.
            if bridge.load_generation.load(Ordering::Acquire) == stamp {
                *bridge.kit_status.lock() = KitStatus::Loading {
                    path: request.path.clone(),
                };
            }

            // Test hook: hold the decode until the test says go.
            let gate = bridge.decode_gate.lock().clone();
            if let Some(gate) = gate {
                let _ = gate.recv();
            }

            // A superseded load does not decode at all.
            if bridge.load_generation.load(Ordering::Acquire) != stamp {
                return;
            }

            let previous = bridge.built_kit.lock().clone();
            let builtin = bridge.builtin_kit.lock().clone();
            let progress = &bridge.load_progress;
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                load_kit(
                    &request,
                    target_sr,
                    previous.as_ref(),
                    builtin.as_ref(),
                    cache::global(),
                    &|| progress.file_done(stamp),
                    &|total| progress.set_total(stamp, total.min(u32::MAX as usize) as u32),
                    // Superseded: a newer pick (or a state load) wants
                    // another kit; stop decoding this one.
                    &|| bridge.load_generation.load(Ordering::Acquire) != stamp,
                )
            }));
            drop(previous);
            drop(builtin);

            // Only the newest load is allowed to write final state. The
            // check and the hand-off happen under one lock, so an older
            // load cannot pass the check and then send after a newer one.
            let handoff = bridge.kit_handoff.lock();
            if bridge.load_generation.load(Ordering::Acquire) != stamp {
                return;
            }
            // Decoded for a rate the host is no longer running at (or for
            // an activation that has ended): it would play at the wrong
            // pitch. Leave it pending — `initialize` reloads it.
            if bridge.sample_rate.load(Ordering::Acquire) != target_sr.to_bits() {
                return;
            }

            let kit_loaded = matches!(outcome, Ok(Ok(_)));
            match outcome {
                Ok(Ok(kit)) => {
                    let num_pads = kit.pads.len();
                    let name = kit_display_name(&request.path, drumkits_root().as_deref());
                    *bridge.catalog.lock() = Arc::new(kit.catalog);
                    // The pads go with the hand-off, under `kit_handoff`:
                    // the editor and the articulation text describe the
                    // kit the sampler takes, never one still decoding. A
                    // label change asks the host for a text rescan.
                    let kit_pads = Arc::new(kit.kit_pads);
                    crate::pad_map::publish(&bridge, kit_pads.clone());
                    // Measure the kit before handing it over: the status
                    // bar's memory readout and the inspector's SAMPLE stage
                    // both describe the takes this load actually decoded.
                    let infos = crate::sample_info::infos_for_pads(&kit.pads, target_sr);
                    // A reload that rebuilt nothing, of the kit the
                    // sampler already holds, hands nothing off: a swap
                    // would fade every voice out and restart the round
                    // robins to play the same samples. The load completes
                    // with the kit already sent.
                    let same_kit = kit.stats.rebuilt_pads == 0
                        && bridge.handed_off.lock().as_ref().is_some_and(|h| {
                            same_kit_built(h, &request, target_sr, &kit.built)
                        });
                    let ordinal = if same_kit {
                        bridge.load_progress.last_sent()
                    } else {
                        hand_off_kit_locked(&bridge, kit.pads)
                    };
                    bridge.load_progress.handed_off(stamp, ordinal);
                    *bridge.handed_off.lock() = Some(HandedOffKit {
                        request: request.clone(),
                        sample_rate: target_sr,
                        pad_requests: kit.built.pad_requests.clone(),
                    });
                    *bridge.built_kit.lock() = Some(kit.built);
                    // The sampler moves off the built-in kit (if it was
                    // on it); its takes are no longer this instance's.
                    *bridge.builtin_kit.lock() = None;
                    bridge
                        .kit_bytes
                        .store(kit.stats.kit_bytes, Ordering::Relaxed);
                    bridge
                        .kit_shared_bytes
                        .store(kit.stats.shared_bytes, Ordering::Relaxed);
                    *bridge.pad_samples.lock() = Arc::new(infos);
                    *bridge.kit_path.lock() = Some(request.path.clone());
                    *bridge.kit_status.lock() = KitStatus::Loaded {
                        name,
                        num_pads,
                        unreadable: kit.stats.unreadable,
                        unreadable_paths: kit.stats.unreadable_paths.clone(),
                    };
                    *bridge.load_stats.lock() = kit.stats;
                }
                Ok(Err(message)) => {
                    bridge.load_progress.failed(stamp);
                    *bridge.kit_status.lock() = KitStatus::Error { message };
                }
                Err(_) => {
                    bridge.load_progress.failed(stamp);
                    *bridge.kit_status.lock() = KitStatus::Error {
                        message: "loader panicked".to_string(),
                    };
                }
            }
            // The project's last-good kit, if it named one: tried when
            // this — the first load since the project opened — failed.
            // Either way it has served its purpose.
            let fallback = bridge
                .kit_fallback
                .lock()
                .take()
                .filter(|path| *path != request.path && !kit_loaded);
            if let Some(path) = fallback {
                // Started before this load lets go of `pending_kit`, so
                // the kit is never "settled" on the failure in between.
                spawn_loader(
                    path,
                    target_sr,
                    &bridge,
                    request.overhead_setup_key.clone(),
                    request.pad_choices.clone(),
                    request.articulations,
                );
            }
            // Finished, one way or the other: nothing is pending any
            // more (unless a newer load has recorded itself meanwhile).
            {
                let mut pending = bridge.pending_kit.lock();
                if pending.as_ref().map(|(s, _)| *s) == Some(stamp) {
                    *pending = None;
                }
            }
            drop(handoff);
            // Takes no kit holds any more (the kit this one replaced may
            // still be playing out; a later load sweeps those).
            cache::global().sweep();
        })
        .expect("spawn drums kit loader thread");
}

/// The per-user directory installed kits live in:
/// `$XDG_DATA_HOME/resonance/drumkits`, next to the shared registry's
/// `installed.json`. `None` when no data directory can be determined.
pub fn drumkits_root() -> Option<PathBuf> {
    resonance_common::registry::registry_path()
        .and_then(|p| p.parent().map(|dir| dir.join("drumkits")))
}

/// The name a kit is shown under, from its manifest's location.
///
/// A kit inside `drumkits_root` is named after the directory directly
/// under the root, however deep the manifest sits: a downloaded zip
/// extracts to `drumkits/Drummica/drummica/drum_samples.json`, and the
/// kit is "Drummica" — the name it was installed (and is listed) under —
/// not the inner "drummica". Anywhere else the manifest's own directory
/// names the kit.
pub fn kit_display_name(manifest_path: &Path, drumkits_root: Option<&Path>) -> String {
    let under_root = |path: &Path, root: &Path| -> Option<String> {
        let rel = path.strip_prefix(root).ok()?;
        let mut parts = rel.components();
        let top = parts.next()?;
        // The manifest itself directly in the root has no kit directory.
        parts.next()?;
        Some(top.as_os_str().to_string_lossy().into_owned())
    };
    if let Some(root) = drumkits_root {
        if let Some(name) = under_root(manifest_path, root) {
            return name;
        }
        // The same check on resolved paths, for a root or manifest given
        // through a symlink or with `..` in it.
        if let (Ok(path), Ok(root)) = (manifest_path.canonicalize(), root.canonicalize()) {
            if let Some(name) = under_root(&path, &root) {
                return name;
            }
        }
    }
    manifest_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "kit".to_string())
}

/// Put `pads` in the audio thread's one-slot kit mailbox, latest wins.
///
/// If the slot still holds a kit the audio thread has not taken yet, that
/// kit is stale: take it back out and drop it here, on the calling
/// (loader) thread, then send the new one. The audio thread only ever
/// `try_recv`s, so it sees either the stale kit (taken before we got to
/// it — then our retry finds the slot empty) or the new one; never
/// nothing in place of the newest.
///
/// Takes [`KitBridge::kit_handoff`] itself, so it cannot race a loader's
/// hand-off. Must not be called with that lock already held (it is not
/// reentrant) — the loader, which holds it across its generation check,
/// calls [`hand_off_kit_locked`] instead.
pub fn hand_off_kit(bridge: &KitBridge, pads: Vec<LoadedPad>) {
    let _handoff = bridge.kit_handoff.lock();
    let ordinal = hand_off_kit_locked(bridge, pads);
    bridge.load_progress.handed_off_directly(ordinal);
    // Not a loader's kit: what the sampler will hold is no longer known.
    *bridge.handed_off.lock() = None;
    *bridge.built_kit.lock() = None;
    *bridge.builtin_kit.lock() = None;
}

/// Hand `pads` to the audio thread as load `generation`'s kit — unless a
/// newer load has begun since, or the host now runs another rate — and run
/// `publish` (the kit's facts for the editor) under the same lock, so a
/// newer load's facts never land first. For a kit no loader builds: the
/// built-in one `selection::play_builtin` swaps in. Returns whether it was
/// sent; a kit that was not is dropped here, on the caller's thread.
pub fn hand_off_kit_if_current(
    bridge: &KitBridge,
    pads: Vec<LoadedPad>,
    generation: u64,
    rate: f32,
    publish: impl FnOnce(),
) -> bool {
    let _handoff = bridge.kit_handoff.lock();
    if bridge.load_generation.load(Ordering::Acquire) != generation
        || bridge.sample_rate.load(Ordering::Acquire) != rate.to_bits()
    {
        return false;
    }
    let ordinal = hand_off_kit_locked(bridge, pads);
    bridge.load_progress.handed_off(generation, ordinal);
    *bridge.handed_off.lock() = None;
    *bridge.built_kit.lock() = None;
    publish();
    true
}

/// [`hand_off_kit`] for a caller already holding
/// [`KitBridge::kit_handoff`]. Returns the kit's send ordinal (see
/// [`progress`]).
fn hand_off_kit_locked(bridge: &KitBridge, pads: Vec<LoadedPad>) -> u64 {
    let mut pads = pads;
    loop {
        match bridge.kit_sender.try_send(pads) {
            Ok(()) => return bridge.load_progress.note_sent(),
            Err(TrySendError::Full(back)) => {
                pads = back;
                // Freed here, off the audio thread. Empty if the audio
                // thread took it in the meantime; the retry then fits.
                if let Ok(stale) = bridge.kit_reclaim.try_recv() {
                    bridge.load_progress.note_reclaimed();
                    drop(stale);
                }
            }
            // Unreachable while the bridge holds `kit_reclaim`: the
            // channel cannot disconnect under it.
            Err(TrySendError::Disconnected(_)) => return bridge.load_progress.note_sent(),
        }
    }
}
