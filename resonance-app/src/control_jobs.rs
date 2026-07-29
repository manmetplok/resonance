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
use std::collections::HashMap;
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
    /// Completes when the SVS render for this lane lands (todo #1156).
    VocalRender { definition_id: u64, track_id: u64 },
    /// Completes on bounce/export completion (todo #1157).
    Export { path: std::path::PathBuf },
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

    fn newest_live_with_token(&self, token: &JobToken) -> Option<u64> {
        let table = self.table.lock().expect("job table poisoned");
        table
            .jobs
            .iter()
            .filter(|(_, e)| !e.state.is_terminal() && e.token.as_ref() == Some(token))
            .map(|(id, _)| *id)
            .max()
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
