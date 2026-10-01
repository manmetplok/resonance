//! Turning a manifest piece into mic banks, in three steps:
//!
//! 1. **Plan** ([`plan_bank_for_position`], [`plan_overhead_bank`]): pick
//!    the mic setup and list its files, layer by layer. No I/O.
//! 2. **Decode** ([`decode_all`]): fetch every planned file through the
//!    shared [`SampleCache`] on a small worker pool. Results come back
//!    indexed by job, so the kit is the same whatever order the workers
//!    finish in.
//! 3. **Assemble** ([`assemble_pad`]): build a pad's banks from the
//!    results, dropping (and counting) every take that could not be read,
//!    with the banks kept aligned (E6).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use crate::kit::{LoadedMicBank, LoadedSample, SampleData, VelocityLayer};

use super::cache::{SampleCache, Source};
use super::manifest::{parse_vel_index, MicSetup};
use super::{is_shared, take_addr, HeldTakes};

/// How many unreadable paths a load keeps for display; the count is exact
/// beyond that.
pub const UNREADABLE_PATHS_KEPT: usize = 5;

/// The files a load reads, in plan order. A job is an index into it.
#[derive(Default)]
pub(super) struct Jobs {
    pub paths: Vec<PathBuf>,
}

impl Jobs {
    fn push(&mut self, path: PathBuf) -> usize {
        self.paths.push(path);
        self.paths.len() - 1
    }
}

/// A bank before decoding: which setup, and its files per velocity layer
/// (soft → loud).
pub(super) struct BankPlan {
    position: String,
    setup_key: String,
    layers: Vec<PlannedLayer>,
}

/// One velocity layer of a planned bank: its manifest velocity, and its
/// takes as (round-robin key, job index), in round-robin key order.
///
/// The keys are what tie the banks of one pad together: a multi-mic
/// recording captured every strike on every mic at once, so `Vel03` /
/// `RR2` is the same strike in the KickIn bank as in the OH bank. A
/// (velocity, round robin) pair is a *cell* below.
struct PlannedLayer {
    vel: u32,
    takes: Vec<(String, usize)>,
}

/// Plan the mic bank for `position` from `piece`, picking the user's
/// preferred setup if one is supplied and available, otherwise taking
/// the first setup whose `position` field matches.
pub(super) fn plan_bank_for_position(
    piece_name: &str,
    piece: &BTreeMap<String, MicSetup>,
    kit_dir: &Path,
    position: &str,
    preferred_setup: Option<&str>,
    jobs: &mut Jobs,
) -> Result<Option<BankPlan>, String> {
    let chosen = preferred_setup
        .and_then(|key| piece.get(key).map(|setup| (key.to_string(), setup)))
        .or_else(|| {
            piece
                .iter()
                .find(|(_, setup)| setup.position == position)
                .map(|(k, v)| (k.clone(), v))
        });
    let Some((setup_key, setup)) = chosen else {
        return Ok(None);
    };
    let layers = plan_layers(piece_name, setup, kit_dir, jobs)?;
    Ok(Some(BankPlan {
        position: position.to_string(),
        setup_key,
        layers,
    }))
}

/// Plan the overhead bank for a piece using the globally selected OH
/// setup key, falling back to any OH-prefixed setup the piece supplies.
pub(super) fn plan_overhead_bank(
    piece_name: &str,
    piece: &BTreeMap<String, MicSetup>,
    kit_dir: &Path,
    overhead_setup_key: &str,
    jobs: &mut Jobs,
) -> Result<Option<BankPlan>, String> {
    let chosen = piece
        .get(overhead_setup_key)
        .map(|setup| (overhead_setup_key.to_string(), setup))
        .or_else(|| {
            piece
                .iter()
                .find(|(_, setup)| setup.position.starts_with("OH"))
                .map(|(k, v)| (k.clone(), v))
        });
    let Some((setup_key, setup)) = chosen else {
        return Ok(None);
    };
    let layers = plan_layers(piece_name, setup, kit_dir, jobs)?;
    Ok(Some(BankPlan {
        position: setup.position.clone(),
        setup_key,
        layers,
    }))
}

/// The files of every velocity layer / round robin of one mic setup.
/// A setup with no files at all plans no layers; the bank then simply
/// does not load (E6), instead of failing the kit.
fn plan_layers(
    piece_name: &str,
    setup: &MicSetup,
    kit_dir: &Path,
    jobs: &mut Jobs,
) -> Result<Vec<PlannedLayer>, String> {
    // Reshape rounds: {RR -> {Vel -> filename}} into {Vel -> [(RR, filename)]}.
    let mut layers_by_vel: BTreeMap<u32, Vec<(&String, &String)>> = BTreeMap::new();
    for (rr_name, vel_map) in &setup.rounds {
        for (vel_name, filename) in vel_map {
            let vel_num = parse_vel_index(vel_name).ok_or_else(|| {
                format!("piece '{piece_name}': unparseable velocity key '{vel_name}'")
            })?;
            layers_by_vel
                .entry(vel_num)
                .or_default()
                .push((rr_name, filename));
        }
    }
    Ok(layers_by_vel
        .into_iter()
        .map(|(vel, files)| PlannedLayer {
            vel,
            takes: files
                .into_iter()
                .map(|(rr, f)| (rr.clone(), jobs.push(kit_dir.join(f))))
                .collect(),
        })
        .collect())
}

/// One job's outcome.
pub(super) type Fetched = Result<(Arc<SampleData>, Source), String>;

/// Worker threads a load decodes on: half the cores, at least one, and
/// never more than there are files.
pub fn decode_workers(jobs: usize) -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    (cores / 2).max(1).min(jobs.max(1))
}

/// Fetch every job's file through `cache` at `sample_rate`, on
/// [`decode_workers`] threads. `on_done` runs once per finished file, on
/// whichever worker finished it. The result is in job order.
pub(super) fn decode_all(
    paths: &[PathBuf],
    sample_rate: f32,
    cache: &SampleCache,
    on_done: &(dyn Fn() + Sync),
) -> Vec<Fetched> {
    let slots: Vec<OnceLock<Fetched>> = (0..paths.len()).map(|_| OnceLock::new()).collect();
    let next = AtomicUsize::new(0);
    let work = || loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        let Some(path) = paths.get(i) else {
            return;
        };
        // A panicking decoder (a malformed file tripping a symphonia
        // assertion) costs that take, not the kit.
        let fetched = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.get_or_decode(path, sample_rate)
        }))
        .unwrap_or_else(|_| Err(format!("decode {}: decoder panicked", path.display())));
        let _ = slots[i].set(fetched);
        on_done();
    };
    let workers = decode_workers(paths.len());
    if workers <= 1 {
        work();
    } else {
        std::thread::scope(|scope| {
            for n in 0..workers {
                let spawned = std::thread::Builder::new()
                    .name(format!("resonance-drums-decode-{n}"))
                    .spawn_scoped(scope, work);
                if spawned.is_err() {
                    // Could not get a thread: the others (or this one,
                    // below) pick up its share.
                    break;
                }
            }
            work();
        });
    }
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(|| Err("decode: job never ran".to_string()))
        })
        .collect()
}

/// What a load counted while assembling its banks.
#[derive(Debug, Default)]
pub(super) struct Tally {
    pub decoded: usize,
    pub cached: usize,
    pub unreadable: usize,
    pub unreadable_paths: Vec<PathBuf>,
    /// Bytes of the kept takes shared with another instance (see
    /// [`super::is_shared`]).
    pub shared_bytes: u64,
    /// Those takes, by address.
    pub shared_takes: HashSet<usize>,
}

impl Tally {
    /// A take fetched from `source` goes into the kit. Counted per use,
    /// as [`crate::sample_info::total_sample_bytes`] counts the kit.
    pub fn note_kept(&mut self, sample: &Arc<SampleData>, source: Source, held: &HeldTakes) {
        if is_shared(sample, source, held) {
            self.shared_bytes += sample.bytes() as u64;
            self.shared_takes.insert(take_addr(sample));
        }
    }
}

/// The banks of one pad, built from their plans and the decode results.
///
/// Every file is counted in `tally` once. What is left out when some fail
/// keeps the banks *aligned* — the sampler picks one layer and one round
/// robin per hit, from the reference bank, and plays that cell in every
/// bank, so a cell must mean the same strike in all of them (E6):
///
/// - A take that failed drops its **cell** — that velocity and round
///   robin — from *every* bank of the pad, not just its own. Dropping it
///   from one bank alone would shift that bank's takes (or layers) one
///   place down: a hit would then sum two different strikes, or find no
///   take there and leave that mic silent on alternate hits.
/// - A layer left with no takes is dropped, from every bank alike (its
///   cells all went).
/// - A bank none of whose files could be read is dropped as a bank and
///   takes no cells with it: an unreadable overhead setup must not
///   silence the close mics. The pad plays its other banks.
///
/// Banks whose recordings legitimately differ in shape (a close mic with
/// two round robins where the overhead has one) keep their shapes; the
/// sampler maps a hit onto each by relative position (E7).
pub(super) fn assemble_pad(
    close: &[BankPlan],
    overhead: Option<&BankPlan>,
    results: &[Fetched],
    paths: &[PathBuf],
    held: &HeldTakes,
    tally: &mut Tally,
) -> (Vec<LoadedMicBank>, Option<LoadedMicBank>) {
    let banks = || close.iter().chain(overhead);
    let readable = |plan: &BankPlan| {
        plan.layers
            .iter()
            .flat_map(|l| &l.takes)
            .any(|&(_, job)| results[job].is_ok())
    };
    // Cells lost in a bank that is otherwise readable.
    let mut lost: HashSet<(u32, &str)> = HashSet::new();
    for plan in banks().filter(|plan| readable(plan)) {
        for layer in &plan.layers {
            for (rr, job) in &layer.takes {
                if results[*job].is_err() {
                    lost.insert((layer.vel, rr.as_str()));
                }
            }
        }
    }
    let mut build = |plan: &BankPlan| -> Option<LoadedMicBank> {
        let mut layers = Vec::with_capacity(plan.layers.len());
        for layer in &plan.layers {
            let mut round_robins = Vec::with_capacity(layer.takes.len());
            for (rr, job) in &layer.takes {
                match &results[*job] {
                    Ok((sample, source)) => {
                        match source {
                            Source::Decoded => tally.decoded += 1,
                            Source::Cached => tally.cached += 1,
                        }
                        if lost.contains(&(layer.vel, rr.as_str())) {
                            continue;
                        }
                        tally.note_kept(sample, *source, held);
                        round_robins.push(LoadedSample::from_shared(sample.clone()));
                    }
                    Err(_) => {
                        tally.unreadable += 1;
                        if tally.unreadable_paths.len() < UNREADABLE_PATHS_KEPT {
                            tally.unreadable_paths.push(paths[*job].clone());
                        }
                    }
                }
            }
            if !round_robins.is_empty() {
                layers.push(VelocityLayer { round_robins });
            }
        }
        if layers.is_empty() {
            return None;
        }
        Some(LoadedMicBank {
            position: plan.position.clone(),
            setup_key: plan.setup_key.clone(),
            layers,
        })
    };
    let close_banks = close.iter().filter_map(&mut build).collect();
    let overhead_bank = overhead.and_then(&mut build);
    (close_banks, overhead_bank)
}
