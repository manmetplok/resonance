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
//! evicted first, and a closing connection drops the jobs it owns, so
//! lost clients don't leak entries.

use crate::control_socket::ConnId;
use resonance_control::ids::JobId;
use resonance_control::job::{JobStarted, JobState, JobStatus};
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
/// the existing message handlers complete/fail the newest live job
/// carrying the matching token. Extended by the namespace todos as they
/// wire more long-running operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobToken {
    /// Completes on `ProjectIoMessage::ProjectSaved` (manual saves only —
    /// autosaves never resolve control jobs).
    ProjectSave,
    /// Completes on `ProjectIoMessage::ProjectLoaded`.
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
    /// thread and reports back per file, so a batch resolves one path at a
    /// time via [`JobBoard::tick_import_path`] and the job completes only
    /// once the LAST of them has landed — the same "don't resolve on the
    /// first sub-event" rule as [`JobToken::VocalRender`]. Entries are the
    /// source paths exactly as they were handed to the engine, which is
    /// what the import events echo back.
    PoolImport { paths: Vec<String> },
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
    },
}

#[derive(Debug)]
struct JobEntry {
    kind: String,
    description: String,
    state: JobState,
    progress: Option<f32>,
    result: Option<Value>,
    error: Option<String>,
    token: Option<JobToken>,
    /// Lanes of a [`JobToken::VocalRender`] batch whose audio hasn't
    /// landed yet. Seeded from the token at [`JobBoard::start`] and
    /// drained by [`JobBoard::complete_vocal_lane`]; the job completes
    /// when it empties. Always empty for every other token.
    remaining_lanes: HashSet<(u64, u64)>,
    /// Source paths of a [`JobToken::PoolImport`] batch whose import
    /// hasn't reported yet. Seeded from the token at [`JobBoard::start`]
    /// and drained by [`JobBoard::tick_import_path`]; the job resolves
    /// when it empties. Always empty for every other token.
    remaining_paths: HashSet<String>,
    /// First failure reported by any path of a [`JobToken::PoolImport`]
    /// batch. A batch with a failed file resolves as an error even when
    /// its other files imported fine — those assets are still in the pool
    /// and visible to `pool.list`.
    import_error: Option<String>,
    /// Connection that started the job; its close drops the entry.
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
    /// Register a new `pending` job and return its id (monotonic per
    /// app run).
    pub fn start(
        &self,
        kind: &str,
        description: &str,
        token: Option<JobToken>,
        owner: Option<ConnId>,
    ) -> JobStarted {
        let mut table = self.table.lock().expect("job table poisoned");
        table.next_id += 1;
        let id = table.next_id;
        let remaining_lanes = match &token {
            Some(JobToken::VocalRender { lanes }) => lanes.iter().copied().collect(),
            _ => HashSet::new(),
        };
        let remaining_paths = match &token {
            Some(JobToken::PoolImport { paths }) => paths.iter().cloned().collect(),
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
                remaining_paths,
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
        let mut table = self.table.lock().expect("job table poisoned");
        if let Some(entry) = table.jobs.get_mut(&id) {
            if !entry.state.is_terminal() {
                entry.state = JobState::Running;
                entry.progress = progress;
            }
        }
    }

    /// Update a live job's fractional progress (`0.0..=1.0`).
    pub fn set_progress(&self, id: u64, progress: f32) {
        let mut table = self.table.lock().expect("job table poisoned");
        if let Some(entry) = table.jobs.get_mut(&id) {
            if !entry.state.is_terminal() {
                entry.state = JobState::Running;
                entry.progress = Some(progress.clamp(0.0, 1.0));
            }
        }
    }

    /// Complete a job by id with its method-specific result payload.
    pub fn complete(&self, id: u64, result: Value) {
        let mut table = self.table.lock().expect("job table poisoned");
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

    /// Fail a job by id with a human-readable message.
    pub fn fail(&self, id: u64, error: impl Into<String>) {
        let mut table = self.table.lock().expect("job table poisoned");
        if let Some(entry) = table.jobs.get_mut(&id) {
            if !entry.state.is_terminal() {
                entry.state = JobState::Error;
                entry.error = Some(error.into());
            }
        }
        drop(table);
        self.terminal.notify_all();
    }

    /// Complete the newest live job carrying `token`. Returns whether a
    /// job matched — a `false` is normal (the operation wasn't
    /// control-initiated), never an error.
    pub fn complete_token(&self, token: &JobToken, result: Value) -> bool {
        match self.newest_live_with_token(token) {
            Some(id) => {
                self.complete(id, result);
                true
            }
            None => false,
        }
    }

    /// Fail the newest live job carrying `token`; see
    /// [`complete_token`](Self::complete_token).
    pub fn fail_token(&self, token: &JobToken, error: impl Into<String>) -> bool {
        match self.newest_live_with_token(token) {
            Some(id) => {
                self.fail(id, error);
                true
            }
            None => false,
        }
    }

    /// Tick off one source path of every live [`JobToken::PoolImport`]
    /// batch that covers it. `error` is `Some` when that file failed to
    /// import.
    ///
    /// Returns the batches whose LAST path just landed, as
    /// `(job_id, batch_error)` — still un-resolved, because the result
    /// payload is built from app state (the pool assets that appeared, the
    /// clip that was placed) which this board cannot see. The caller
    /// completes or fails each returned id. A `Vec` that comes back empty
    /// is the normal case for a GUI-driven import.
    pub fn tick_import_path(&self, path: &str, error: Option<&str>) -> Vec<(u64, Option<String>)> {
        let mut finished = Vec::new();
        let mut table = self.table.lock().expect("job table poisoned");
        for (id, entry) in table.jobs.iter_mut() {
            if entry.state.is_terminal() || !entry.remaining_paths.remove(path) {
                continue;
            }
            if let Some(error) = error {
                entry
                    .import_error
                    .get_or_insert_with(|| format!("{path}: {error}"));
            }
            if entry.remaining_paths.is_empty() {
                finished.push((*id, entry.import_error.clone()));
            }
        }
        finished
    }

    /// The method that started a job, without marking it fetched the way
    /// [`Self::status`] does — completion hooks use it to pick the result
    /// shape, and a peek must not count as the client collecting it.
    pub fn kind_of(&self, id: u64) -> Option<String> {
        let table = self.table.lock().expect("job table poisoned");
        table.jobs.get(&id).map(|e| e.kind.clone())
    }

    /// The source paths of a [`JobToken::PoolImport`] job, so the caller
    /// can collect exactly that batch's assets when building its result.
    /// Empty for any other job.
    pub fn import_batch_paths(&self, id: u64) -> Vec<String> {
        let table = self.table.lock().expect("job table poisoned");
        match table.jobs.get(&id).and_then(|e| e.token.as_ref()) {
            Some(JobToken::PoolImport { paths }) => paths.clone(),
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
    /// The result payload carries every track the batch rendered, so a
    /// whole-project render reports all of them.
    pub fn complete_vocal_lane(&self, definition_id: u64, track_id: u64, revision: u64) -> bool {
        let mut finished: Vec<(u64, Value)> = Vec::new();
        let mut matched = false;
        {
            let mut table = self.table.lock().expect("job table poisoned");
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

    /// Fail the newest live `VocalRender` job, whatever its lane. The
    /// `VocalAudioFailed` message (todo #1156) carries no lane identity,
    /// and control renders run one at a time through the update loop, so
    /// the newest live vocal-render job is the one that just failed.
    /// No-op when none is live (a GUI-driven render).
    pub fn fail_newest_vocal_render(&self, error: impl Into<String>) -> bool {
        let id = {
            let table = self.table.lock().expect("job table poisoned");
            table
                .jobs
                .iter()
                .filter(|(_, e)| {
                    !e.state.is_terminal()
                        && matches!(e.token, Some(JobToken::VocalRender { .. }))
                })
                .map(|(id, _)| *id)
                .max()
        };
        match id {
            Some(id) => {
                self.fail(id, error);
                true
            }
            None => false,
        }
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
        let table = self.table.lock().expect("job table poisoned");
        let entry = table.jobs.get(&job_id)?;
        if entry.state.is_terminal() {
            return None;
        }
        match &entry.token {
            Some(token @ JobToken::Measure { .. }) => Some(token.clone()),
            _ => None,
        }
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
        let table = self.table.lock().expect("job table poisoned");
        table.jobs.values().any(|e| {
            !e.state.is_terminal()
                && matches!(e.token, Some(JobToken::Measure { offline: true, .. }))
        })
    }

    fn newest_live_with_token(&self, token: &JobToken) -> Option<u64> {
        let table = self.table.lock().expect("job table poisoned");
        table
            .jobs
            .iter()
            .filter(|(_, e)| !e.state.is_terminal() && e.token.as_ref() == Some(token))
            .map(|(id, _)| *id)
            .max()
    }

    /// Fail every live [`JobToken::Export`] job with `error`. The engine's
    /// `BounceError` event carries no path to correlate on, but the
    /// render busy-guard forbids more than one bounce at a time, so at
    /// most one export job is ever live (todo #1157). Returns whether any
    /// matched.
    pub fn fail_export_jobs(&self, error: impl Into<String>) -> bool {
        let ids: Vec<u64> = {
            let table = self.table.lock().expect("job table poisoned");
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
            self.fail(*id, error.clone());
        }
        !ids.is_empty()
    }

    /// The job's current status, `None` for an unknown id. Marks a
    /// terminal status as fetched (eviction priority).
    pub fn status(&self, id: u64) -> Option<JobStatus> {
        let mut table = self.table.lock().expect("job table poisoned");
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
        let mut table = self.table.lock().expect("job table poisoned");
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
                .expect("job table poisoned");
            table = guard;
        }
    }

    /// Drop every job owned by a closing connection (terminal or not) —
    /// nobody can query them anymore. Wakes waiting readers so a
    /// blocked `job.wait` on a dropped job resolves to `not_found`.
    pub fn on_disconnect(&self, conn: ConnId) {
        let mut table = self.table.lock().expect("job table poisoned");
        table.jobs.retain(|_, e| e.owner != Some(conn));
        drop(table);
        self.terminal.notify_all();
    }

    /// Kind + description of a job, for diagnostics. `None` when unknown.
    pub fn describe(&self, id: u64) -> Option<(String, String)> {
        let table = self.table.lock().expect("job table poisoned");
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
