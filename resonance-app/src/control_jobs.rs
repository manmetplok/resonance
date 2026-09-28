//! Control-endpoint job registry (ba doc #265, todo #1149).
//!
//! Long-running operations started over the control protocol (project
//! save/load, SVS vocal render, WAV/stem export) return a job id
//! immediately; clients poll `job.status` or block on `job.wait`. The
//! [`JobBoard`] is the shared ledger:
//!
//! - The **update loop** starts jobs ([`Resonance::start_control_job`])
//!   and completes them from the *existing* completion messages (e.g.
//!   `ProjectIoMessage::ProjectSaved` / `ProjectLoaded`), matching by
//!   the [`JobToken`] stored when the job started — never by guessing.
//! - The **socket threads** serve `job.wait` by blocking on the board's
//!   condvar (`control_socket` intercepts the method before the bridge),
//!   so the update loop is never blocked.
//!
//! Retention: jobs stay queryable after completion; the table is
//! bounded ([`MAX_RETAINED_JOBS`]) with terminal already-fetched entries
//! evicted first. A closing connection drops the *terminal* jobs it
//! owns and orphans its live ones (job ids are global, so a client that
//! reconnects after a transport hiccup can keep polling the operation
//! that is still running); orphans go terminal through the normal
//! completion hooks and are then reaped by the same LRU bound, so lost
//! clients still don't leak entries.

use crate::control_socket::ConnId;
use resonance_audio::types::AssetId;
use resonance_control::ids::JobId;
use resonance_control::job::{JobError, JobStarted, JobState, JobStatus};
use resonance_control::ErrorKind;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Upper bound on retained job entries (running + terminal).
pub const MAX_RETAINED_JOBS: usize = 64;

/// Safety cap for a `job.wait` without `timeout_ms`: "indefinitely" on
/// the wire, but the blocked reader thread must not outlive any
/// plausible operation, so it resolves with the then-current status
/// after this long.
pub const MAX_WAIT: Duration = Duration::from_secs(600);

/// Correlation token stored when a job starts. The completion hooks in
/// the existing message handlers complete/fail the oldest live job
/// carrying the matching token (FIFO — completions arrive in dispatch
/// order). Extended by the namespace todos as they wire more
/// long-running operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobToken {
    /// Completes on `ProjectIoMessage::ProjectSaved` (manual saves only —
    /// autosaves never resolve control jobs).
    ProjectSave,
    /// Fails on `ProjectIoMessage::ProjectLoaded(Err)`; completes once
    /// the engine confirms the clear and the loaded project replays
    /// (`engine_events::project_io::all_cleared`, disk path only).
    ProjectLoad,
    /// A `project.new` instantiation (todo #1151): completes once the
    /// engine confirms the clear and the fresh project replays
    /// (`engine_events::project_io::all_cleared`, untitled path only);
    /// fails on `ProjectIoMessage::TemplateLoaded(Err)`.
    ProjectNew,
    /// Completes when the SVS render for **every** lane in the batch has
    /// landed (todo #1156). A `vocal.render` addressing a track covers
    /// each of that track's vocal lanes, and one addressing no track
    /// covers the whole project — one job, N lanes, resolved by
    /// [`JobBoard::complete_vocal_lane`] as each lane's audio arrives.
    /// Entries are `(definition_id, track_id)`.
    VocalRender { lanes: Vec<(u64, u64)> },
    /// Completes on bounce/export completion (todo #1157).
    Export { path: std::path::PathBuf },
    /// A media-pool import started over the control endpoint: `pool.import`,
    /// or a `clip.place` whose file was not in the pool yet.
    ///
    /// The engine decodes/resamples/copies each source file on a worker
    /// thread and reports back per file, so a batch resolves one asset id
    /// at a time via [`JobBoard::tick_import_asset`] and the job completes
    /// only once the LAST of them has landed — the same "don't resolve on
    /// the first sub-event" rule as [`JobToken::VocalRender`]. Entries are
    /// the ids the app allocated for the batch before sending it to the
    /// engine (D-7a) — the same ids `AssetImported` / `ImportFailed` echo
    /// back. Keyed by id rather than source path: two files that share a
    /// path across overlapping batches never share an id, so there is
    /// nothing left to resolve out of order.
    PoolImport { asset_ids: Vec<AssetId> },
    /// A mix measurement (todo #1219): completes on
    /// `AudioEvent::MixMeasured`, fails on `MixMeasureError`.
    ///
    /// Both events echo the `measure_id` the command carried (ba todo
    /// #1243), and `meter.*` passes the JOB ID as that token — so the
    /// event names its own job outright and [`JobBoard::live_measure`]
    /// is a direct lookup rather than an inference.
    ///
    /// Before #1243 the events carried nothing and the job had to be
    /// guessed as "the newest live measure", which was correct only
    /// while at most one measurement could exist at a time. Nothing in
    /// the ENGINE guaranteed that, and it is worth not re-learning why
    /// (ba todo #1244): the engine's own refusal covers render-vs-render
    /// only, and the live path skips its render guard entirely, so a
    /// live read starting during a render measurement would have been
    /// completed with the render's numbers — one of the two blockers
    /// that bounced #1219. What actually held it together was the
    /// app-side guard in `update::control::meter::source_guard`, which
    /// had to refuse EVERY overlap to do so.
    ///
    /// The correlation id removes that burden. The guard now refuses
    /// only what genuinely conflicts — see `offline` below — and a live
    /// read runs alongside a render measurement, each receiving its own
    /// result.
    Measure {
        /// The control method that started the job: the same engine pass
        /// backs `meter.measure` and `meter.stems`, which read its
        /// results into different result shapes.
        method: &'static str,
        /// This measurement renders offline (rather than reading the
        /// live streaming tap), so it contends with bounce / export /
        /// freeze for the one offline renderer. Only offline
        /// measurements block each other; see
        /// [`JobBoard::has_live_offline_measure`].
        offline: bool,
        /// For a `meter.compare` that renders a `"current"` side: which
        /// snapshots the result is compared against, and how. `None` for
        /// every other method.
        compare: Option<ComparePlan>,
    },
    /// A `meter.probe` (warmth-width-depth.md §7.3): completes on
    /// `AudioEvent::ChainProbed`, fails on `ChainProbeError`, both of
    /// which echo the job id as their `probe_id`. It renders nothing
    /// shared — the engine probes a cloned chain — so it blocks nothing
    /// and nothing blocks it.
    Probe,
}

/// The sides of a `meter.compare` that is waiting on a render
/// (warmth-width-depth.md §7.2). `None` on a side means "current" — the
/// measurement the render is producing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComparePlan {
    /// Side A's snapshot id, or `None` for the render.
    pub a: Option<u64>,
    /// Side B's snapshot id, or `None` for the render.
    pub b: Option<u64>,
    /// Gain-match B to A's integrated loudness.
    pub match_lufs: bool,
}

#[derive(Debug)]
struct JobEntry {
    kind: String,
    description: String,
    state: JobState,
    progress: Option<f32>,
    result: Option<Value>,
    error: Option<JobError>,
    token: Option<JobToken>,
    /// Lanes of a [`JobToken::VocalRender`] batch whose audio hasn't
    /// landed yet. Seeded from the token at [`JobBoard::start`] and
    /// drained by [`JobBoard::complete_vocal_lane`]; the job completes
    /// when it empties. Always empty for every other token.
    remaining_lanes: HashSet<(u64, u64)>,
    /// Asset ids of a [`JobToken::PoolImport`] batch whose import hasn't
    /// reported yet. Seeded from the token at [`JobBoard::start`] and
    /// drained by [`JobBoard::tick_import_asset`]; the job resolves when
    /// it empties. Always empty for every other token.
    remaining_asset_ids: HashSet<AssetId>,
    /// First failure reported by any path of a [`JobToken::PoolImport`]
    /// batch. A batch with a failed file resolves as an error even when
    /// its other files imported fine — those assets are still in the pool
    /// and visible to `pool.list`.
    import_error: Option<String>,
    /// Connection that started the job. Its close drops the entry if the
    /// job is already terminal, and orphans it (`owner = None`) while it
    /// is live — see [`JobBoard::on_disconnect`]. Ownership gates nothing
    /// else: completion hooks resolve by token/id and `job.status` /
    /// `job.wait` serve any connection, so an orphan finishes normally
    /// and stays queryable by a reconnected client.
    owner: Option<ConnId>,
    /// A terminal status has been delivered at least once — the entry
    /// is first in line for LRU eviction.
    fetched: bool,
}

impl JobEntry {
    fn status(&self, id: u64) -> JobStatus {
        JobStatus {
            job_id: JobId(id),
            state: self.state,
            progress: self.progress,
            result: self.result.clone(),
            error: self.error.clone(),
        }
    }
}

#[derive(Default)]
struct Table {
    next_id: u64,
    jobs: HashMap<u64, JobEntry>,
}

/// Shared job ledger: a mutex-guarded table plus a condvar signalled on
/// every transition to a terminal state (what `job.wait` blocks on).
#[derive(Default)]
pub struct JobBoard {
    table: Mutex<Table>,
    terminal: Condvar,
}

impl JobBoard {
    /// Lock the table, recovering from a poisoned mutex. The table is
    /// locked from the per-connection reader threads (`job.wait`) *and*
    /// the main update loop; a panic on a reader thread would otherwise
    /// poison the lock and turn the next main-thread access into a
    /// second panic — one lost client taking the whole app down. The
    /// table holds no cross-panic invariants (every mutation leaves it
    /// consistent at each statement), so the data under a poisoned lock
    /// is safe to keep using.
    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Register a new `pending` job and return its id (monotonic per
    /// app run).
    pub fn start(
        &self,
        kind: &str,
        description: &str,
        token: Option<JobToken>,
        owner: Option<ConnId>,
    ) -> JobStarted {
        let mut table = self.table();
        table.next_id += 1;
        let id = table.next_id;
        let remaining_lanes = match &token {
            Some(JobToken::VocalRender { lanes }) => lanes.iter().copied().collect(),
            _ => HashSet::new(),
        };
        let remaining_asset_ids = match &token {
            Some(JobToken::PoolImport { asset_ids }) => asset_ids.iter().copied().collect(),
            _ => HashSet::new(),
        };
        table.jobs.insert(
            id,
            JobEntry {
                kind: kind.to_owned(),
                description: description.to_owned(),
                state: JobState::Pending,
                progress: None,
                result: None,
                error: None,
                token,
                remaining_lanes,
                remaining_asset_ids,
                import_error: None,
                owner,
                fetched: false,
            },
        );
        prune(&mut table);
        JobStarted { job_id: JobId(id) }
    }

    /// Move a pending job to `running`, optionally seeding progress.
    pub fn set_running(&self, id: u64, progress: Option<f32>) {
        let mut table = self.table();
        if let Some(entry) = table.jobs.get_mut(&id) {
            if !entry.state.is_terminal() {
                entry.state = JobState::Running;
                entry.progress = progress;
            }
        }
    }

    /// Update a live job's fractional progress (`0.0..=1.0`).
    pub fn set_progress(&self, id: u64, progress: f32) {
        let mut table = self.table();
        if let Some(entry) = table.jobs.get_mut(&id) {
            if !entry.state.is_terminal() {
                entry.state = JobState::Running;
                entry.progress = Some(progress.clamp(0.0, 1.0));
            }
        }
    }

    /// Complete a job by id with its method-specific result payload.
    pub fn complete(&self, id: u64, result: Value) {
        let mut table = self.table();
        if let Some(entry) = table.jobs.get_mut(&id) {
            if !entry.state.is_terminal() {
                entry.state = JobState::Done;
                entry.progress = Some(1.0);
                entry.result = Some(result);
            }
        }
        drop(table);
        self.terminal.notify_all();
    }

    /// Fail a job by id with a human-readable message and no `kind`
    /// (most failures today: project I/O, vocal render, pool import —
    /// nothing upstream of them classifies a failure cause yet).
    pub fn fail(&self, id: u64, error: impl Into<String>) {
        self.fail_with_kind(id, error, None);
    }

    /// Fail a job by id with a message and an optional [`ErrorKind`]
    /// (ARCH-05 / epic C, C-2) — a control client branches on `kind`
    /// instead of the message text. `None` when the failing engine event
    /// carries no typed classification.
    pub fn fail_with_kind(&self, id: u64, error: impl Into<String>, kind: Option<ErrorKind>) {
        let mut table = self.table();
        if let Some(entry) = table.jobs.get_mut(&id) {
            if !entry.state.is_terminal() {
                entry.state = JobState::Error;
                entry.error = Some(JobError::new(error, kind));
            }
        }
        drop(table);
        self.terminal.notify_all();
    }

    /// Complete the OLDEST live job carrying `token`. Returns whether a
    /// job matched — a `false` is normal (the operation wasn't
    /// control-initiated), never an error.
    ///
    /// Oldest, not newest: completion events for identical operations
    /// arrive in dispatch order (one update loop, one engine queue), so
    /// if two live jobs ever carry the same token the first completion
    /// belongs to the first job. Resolving the newest instead completed
    /// the later client's job and left the earlier one running until
    /// the [`MAX_WAIT`] cap.
    pub fn complete_token(&self, token: &JobToken, result: Value) -> bool {
        match self.oldest_live_with_token(token) {
            Some(id) => {
                self.complete(id, result);
                true
            }
            None => false,
        }
    }

    /// Fail the oldest live job carrying `token`; see
    /// [`complete_token`](Self::complete_token).
    pub fn fail_token(&self, token: &JobToken, error: impl Into<String>) -> bool {
        match self.oldest_live_with_token(token) {
            Some(id) => {
                self.fail(id, error);
                true
            }
            None => false,
        }
    }

    /// Is any live job carrying exactly `token`? The project-lifecycle
    /// busy guard uses this to refuse starting a duplicate of an
    /// operation whose in-flight window the app state doesn't expose
    /// (a `project.new` from a user template loads asynchronously
    /// before `io.loading` is set, so only its live job betrays it).
    pub fn has_live_token(&self, token: &JobToken) -> bool {
        self.oldest_live_with_token(token).is_some()
    }

    /// Tick off one asset id of ONE live [`JobToken::PoolImport`] batch
    /// that covers it — the oldest still awaiting that id (D-7a). `path`
    /// is only for the error message text; `error` is `Some` when that
    /// file failed to import.
    ///
    /// One batch, not every batch: the engine emits one import event per
    /// file it was handed, and — since D-7a — every file of every batch
    /// carries its own distinct asset id, so there is exactly one batch
    /// that can be waiting on a given id to begin with (ticking every
    /// batch that happened to share a *path* used to let a second batch
    /// resolve `done` off the first batch's event, before its own copy of
    /// the file had imported).
    ///
    /// Returns the batch whose LAST id just landed, as
    /// `(job_id, batch_error)` — still un-resolved, because the result
    /// payload is built from app state (the pool assets that appeared, the
    /// clip that was placed) which this board cannot see. The caller
    /// completes or fails each returned id. A `Vec` that comes back empty
    /// is the normal case for a GUI-driven import.
    pub fn tick_import_asset(
        &self,
        asset_id: AssetId,
        path: &str,
        error: Option<&str>,
    ) -> Vec<(u64, Option<String>)> {
        let mut finished = Vec::new();
        let mut table = self.table();
        let oldest = table
            .jobs
            .iter()
            .filter(|(_, e)| !e.state.is_terminal() && e.remaining_asset_ids.contains(&asset_id))
            .map(|(id, _)| *id)
            .min();
        if let Some(id) = oldest {
            let entry = table.jobs.get_mut(&id).expect("id came from the table");
            entry.remaining_asset_ids.remove(&asset_id);
            if let Some(error) = error {
                entry
                    .import_error
                    .get_or_insert_with(|| format!("{path}: {error}"));
            }
            if entry.remaining_asset_ids.is_empty() {
                finished.push((id, entry.import_error.clone()));
            }
        }
        finished
    }

    /// The method that started a job, without marking it fetched the way
    /// [`Self::status`] does — completion hooks use it to pick the result
    /// shape, and a peek must not count as the client collecting it.
    pub fn kind_of(&self, id: u64) -> Option<String> {
        let table = self.table();
        table.jobs.get(&id).map(|e| e.kind.clone())
    }

    /// The asset ids of a [`JobToken::PoolImport`] job, so the caller can
    /// collect exactly that batch's assets when building its result.
    /// Empty for any other job.
    pub fn import_batch_asset_ids(&self, id: u64) -> Vec<AssetId> {
        let table = self.table();
        match table.jobs.get(&id).and_then(|e| e.token.as_ref()) {
            Some(JobToken::PoolImport { asset_ids }) => asset_ids.clone(),
            _ => Vec::new(),
        }
    }

    /// Tick off one lane of every live [`JobToken::VocalRender`] batch
    /// that covers it, completing a job once its last lane has landed.
    ///
    /// A `vocal.render` addressing a track fans out over *all* of that
    /// track's vocal lanes, so the job must not resolve on the first
    /// lane's audio — a client that waited on it would then read a
    /// half-rendered track back as `done`. Returns whether any job was
    /// tracking this lane (a `false` is normal: a GUI-driven render).
    ///
    /// Call this only for a `VocalAudioReady` the render-epoch check has
    /// ACCEPTED. A superseded render's event used to tick lanes off
    /// every batch before that check ran, letting a later job resolve
    /// `done` off audio the install then discarded as stale.
    ///
    /// Every covering batch, not the oldest — deliberately unlike
    /// [`tick_import_asset`](Self::tick_import_asset). Two import batches
    /// naming the same file get one engine event each, keyed by each
    /// file's own distinct asset id; but two render jobs covering the same
    /// lane share a SINGLE surviving event: the later request bumps the
    /// lane's epoch, the earlier render is discarded on arrival, and the
    /// one accepted install is current for every job that asked. Ticking
    /// only the oldest would strand the newer job until [`MAX_WAIT`].
    ///
    /// The result payload carries every track the batch rendered, so a
    /// whole-project render reports all of them.
    pub fn complete_vocal_lane(&self, definition_id: u64, track_id: u64, revision: u64) -> bool {
        let mut finished: Vec<(u64, Value)> = Vec::new();
        let mut matched = false;
        {
            let mut table = self.table();
            for (id, entry) in table.jobs.iter_mut() {
                if entry.state.is_terminal()
                    || !entry.remaining_lanes.remove(&(definition_id, track_id))
                {
                    continue;
                }
                matched = true;
                if entry.remaining_lanes.is_empty() {
                    let mut track_ids: Vec<u64> = match &entry.token {
                        Some(JobToken::VocalRender { lanes }) => {
                            lanes.iter().map(|(_, t)| *t).collect()
                        }
                        _ => vec![track_id],
                    };
                    track_ids.sort_unstable();
                    track_ids.dedup();
                    finished.push((
                        *id,
                        serde_json::json!({ "track_ids": track_ids, "revision": revision }),
                    ));
                }
            }
        }
        for (id, result) in finished {
            self.complete(id, result);
        }
        matched
    }

    /// Fail every live [`JobToken::VocalRender`] batch still waiting on
    /// this lane. `VocalAudioFailed` names the lane that errored (it
    /// used to carry nothing, so the failure had to be *inferred* onto
    /// the oldest live render job — which killed a job covering only
    /// lane A when an unrelated GUI regeneration of lane B failed), and
    /// the caller has already epoch-checked it, so a failure here means
    /// the lane's audio is genuinely not coming: no batch waiting on it
    /// can ever complete. Batches not waiting on the lane — including
    /// ones that already received its audio and moved on — keep running.
    /// Returns whether any batch matched (a `false` is normal: a
    /// GUI-driven render, with no control job attached).
    pub fn fail_vocal_lane(
        &self,
        definition_id: u64,
        track_id: u64,
        error: impl Into<String>,
    ) -> bool {
        let ids: Vec<u64> = {
            let table = self.table();
            table
                .jobs
                .iter()
                .filter(|(_, e)| {
                    !e.state.is_terminal()
                        && e.remaining_lanes.contains(&(definition_id, track_id))
                })
                .map(|(id, _)| *id)
                .collect()
        };
        let error = error.into();
        for id in &ids {
            self.fail(*id, error.clone());
        }
        !ids.is_empty()
    }

    /// The token of `job_id` if it is a [`JobToken::Measure`] that is
    /// still running, else `None`.
    ///
    /// This is the correlation hook for `MixMeasured` / `MixMeasureError`
    /// (ba todo #1243): the events carry the job id back as their
    /// `measure_id`, so a result reaches the job that asked for it, and
    /// an event for a job that no longer exists — already completed,
    /// evicted, or never control-initiated — resolves NOTHING rather
    /// than landing on whichever job happens to be open.
    ///
    /// Unlike [`complete_token`](Self::complete_token) this matches on
    /// the *variant* rather than on equality, because the caller needs
    /// the token's contents (which method asked) to shape the result
    /// before it can complete the job.
    pub fn live_measure(&self, job_id: u64) -> Option<JobToken> {
        let table = self.table();
        let entry = table.jobs.get(&job_id)?;
        if entry.state.is_terminal() {
            return None;
        }
        match &entry.token {
            Some(token @ JobToken::Measure { .. }) => Some(token.clone()),
            _ => None,
        }
    }

    /// Is `job_id` a [`JobToken::Probe`] that is still running?
    pub fn is_live_probe(&self, job_id: u64) -> bool {
        let table = self.table();
        table.jobs.get(&job_id).is_some_and(|e| {
            !e.state.is_terminal() && matches!(e.token, Some(JobToken::Probe))
        })
    }

    /// Is an OFFLINE measurement job still running?
    ///
    /// The one thing measurements genuinely contend for is the offline
    /// renderer, which is shared with bounce, export and freeze — so this
    /// is what `meter.*` refuses a second render measurement on. A live
    /// measurement reads the streaming tap, renders nothing, and is
    /// therefore not counted here and not blocked by anything here (ba
    /// todo #1243; before the correlation id existed, `meter.*` had to
    /// refuse EVERY overlap, live included, purely to keep "the newest
    /// live measure job" naming the right job).
    pub fn has_live_offline_measure(&self) -> bool {
        let table = self.table();
        table.jobs.values().any(|e| {
            !e.state.is_terminal()
                && matches!(e.token, Some(JobToken::Measure { offline: true, .. }))
        })
    }

    /// Oldest (lowest-id) live job carrying exactly `token` — FIFO, to
    /// match the order completions arrive in; see
    /// [`complete_token`](Self::complete_token).
    fn oldest_live_with_token(&self, token: &JobToken) -> Option<u64> {
        let table = self.table();
        table
            .jobs
            .iter()
            .filter(|(_, e)| !e.state.is_terminal() && e.token.as_ref() == Some(token))
            .map(|(id, _)| *id)
            .min()
    }

    /// Fail every live [`JobToken::Export`] job with `error` and,
    /// optionally, its [`ErrorKind`] (ARCH-05 / epic C, C-2 — the engine's
    /// `BounceError` now carries an `ExportErrorKind`, mapped onto the
    /// control-side kind by the caller). The engine's `BounceError` event
    /// carries no path to correlate on, but the render busy-guard forbids
    /// more than one bounce at a time, so at most one export job is ever
    /// live (todo #1157). Returns whether any matched.
    pub fn fail_export_jobs(&self, error: impl Into<String>, kind: Option<ErrorKind>) -> bool {
        let ids: Vec<u64> = {
            let table = self.table();
            table
                .jobs
                .iter()
                .filter(|(_, e)| {
                    !e.state.is_terminal() && matches!(e.token, Some(JobToken::Export { .. }))
                })
                .map(|(id, _)| *id)
                .collect()
        };
        let error = error.into();
        for id in &ids {
            self.fail_with_kind(*id, error.clone(), kind);
        }
        !ids.is_empty()
    }

    /// The job's current status, `None` for an unknown id. Marks a
    /// terminal status as fetched (eviction priority).
    pub fn status(&self, id: u64) -> Option<JobStatus> {
        let mut table = self.table();
        let entry = table.jobs.get_mut(&id)?;
        if entry.state.is_terminal() {
            entry.fetched = true;
        }
        Some(entry.status(id))
    }

    /// Block until job `id` reaches a terminal state or the timeout
    /// elapses, returning the then-current status either way (`None`
    /// for an unknown id — including one dropped by a connection close
    /// mid-wait). Called from the socket threads only; the update loop
    /// answers `job.wait` as an immediate snapshot instead.
    pub fn wait(&self, id: u64, timeout: Option<Duration>) -> Option<JobStatus> {
        let deadline = Instant::now() + timeout.unwrap_or(MAX_WAIT).min(MAX_WAIT);
        let mut table = self.table();
        loop {
            let entry = table.jobs.get_mut(&id)?;
            if entry.state.is_terminal() {
                entry.fetched = true;
                return Some(entry.status(id));
            }
            let now = Instant::now();
            if now >= deadline {
                return Some(entry.status(id));
            }
            let (guard, _timeout) = self
                .terminal
                .wait_timeout(table, deadline - now)
                // Same poison recovery as [`Self::table`]: a panicking
                // reader thread must not wedge everyone else's waits.
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            table = guard;
        }
    }

    /// Clean up after a closing connection: drop the *terminal* jobs it
    /// owns (their results were for that client alone, and keeping them
    /// only competes with live clients for retention slots), but ORPHAN
    /// its live jobs — `owner = None` — instead of deleting them.
    ///
    /// The operation behind a live job keeps running in the app whether
    /// or not the socket that asked for it is still open, and the MCP
    /// client drops its connection on any transport-level failure (a
    /// read timeout on an unrelated call included) while telling the
    /// model to poll `job.status` with the job id after reconnecting.
    /// Deleting the live job here turned that recovery path into
    /// `not_found` for a render that was in fact still in flight. Job
    /// ids are global and status/wait check no ownership, so the
    /// reconnected client legitimately resumes polling the orphan; it
    /// completes or fails through the normal token-matched hooks and is
    /// then subject to the ordinary [`MAX_RETAINED_JOBS`] eviction.
    ///
    /// Wakes waiting readers so a blocked `job.wait` on a dropped
    /// terminal job resolves to `not_found`. (The dead connection's own
    /// reader threads die with the socket; waits from other connections
    /// on an orphaned job keep blocking until it turns terminal, as
    /// they should.)
    pub fn on_disconnect(&self, conn: ConnId) {
        let mut table = self.table();
        table.jobs.retain(|_, e| {
            if e.owner != Some(conn) {
                return true;
            }
            if e.state.is_terminal() {
                return false;
            }
            e.owner = None;
            true
        });
        drop(table);
        self.terminal.notify_all();
    }

    /// Panic while holding the table lock — poisoning it exactly the way
    /// a crashing `job.wait` reader thread would. Test support for the
    /// containment guarantee in [`Self::table`]; the caller runs it on a
    /// scratch thread it expects to die.
    #[doc(hidden)]
    pub fn panic_holding_table_for_test(&self) -> ! {
        let _guard = self.table();
        panic!("intentional test panic while holding the job table");
    }

    /// Kind + description of a job, for diagnostics. `None` when unknown.
    pub fn describe(&self, id: u64) -> Option<(String, String)> {
        let table = self.table();
        table
            .jobs
            .get(&id)
            .map(|e| (e.kind.clone(), e.description.clone()))
    }
}

/// Enforce [`MAX_RETAINED_JOBS`]: evict terminal fetched entries first,
/// then the oldest terminal entries. Live (pending/running) jobs are
/// never evicted — real operations bound their count.
fn prune(table: &mut Table) {
    while table.jobs.len() > MAX_RETAINED_JOBS {
        let victim = table
            .jobs
            .iter()
            .filter(|(_, e)| e.state.is_terminal())
            .min_by_key(|(id, e)| (!e.fetched, **id))
            .map(|(id, _)| *id);
        match victim {
            Some(id) => {
                table.jobs.remove(&id);
            }
            None => break, // all live — allow temporary growth
        }
    }
}

impl crate::Resonance {
    /// Register a control-initiated long-running operation. The caller
    /// dispatches the actual work through the normal message path and
    /// returns `{job_id}` to the client immediately; the matching
    /// completion hook resolves the job via its [`JobToken`].
    pub fn start_control_job(
        &self,
        kind: &str,
        description: &str,
        token: JobToken,
        owner: Option<ConnId>,
    ) -> JobStarted {
        self.control.jobs.start(kind, description, Some(token), owner)
    }

    /// Shared job board handle (the socket layer's `job.wait` and the
    /// integration tests read it directly).
    #[doc(hidden)]
    pub fn control_jobs(&self) -> &std::sync::Arc<JobBoard> {
        &self.control.jobs
    }
}
