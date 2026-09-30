//! User wavetables: a WAV file imported per oscillator, selected with the
//! `oscN_wavetable` value one past the bundled tables
//! ([`USER_WAVETABLE_INDEX`](crate::dsp::wavetable::USER_WAVETABLE_INDEX)).
//!
//! The pieces follow the file-loading plugins (resonance-ir, resonance-amp):
//!
//! * **Import and build off the audio thread.** The editor's "Load…" runs the
//!   file dialog on the UI thread and hands the path to a loader thread
//!   ([`UserWavetables::request_file`]); a restored project builds in the
//!   state-load call itself (main thread — the bridge expects a wavetable
//!   rebuild there). Either way [`import`] slices the WAV into frames and
//!   [`UserTable::build`] makes the band-limited mips.
//! * **Hand-off through a [`Mailbox`].** The finished table is posted to the
//!   oscillator's single-slot mailbox; the audio thread collects it with a
//!   non-blocking `try_take` at the top of `process` ([`UserTableSwap::apply`])
//!   and installs it with two slot writes.
//! * **Retire off the audio thread.** The displaced table goes to a janitor
//!   thread through a pre-sized channel (`SwapFader`'s
//!   `spawn_retire_janitor`, the resonance-drums idiom), with a few inline
//!   parking slots for when the channel is momentarily full — so the audio
//!   thread never frees a 25 MB table.
//! * **Persist the frames, not just the path** ([`state`]).

pub mod import;
pub mod state;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_plugin::Mailbox;

use crate::dsp::engine::{SynthEngine, NUM_OSCS};
use crate::dsp::user_table::UserTable;
use crate::dsp::wavetable::WAVETABLE_SIZE;

/// What the audio thread is asked to do with an oscillator's user slot.
pub enum UserTableMsg {
    Install(Box<UserTable>),
    /// Drop back to bundled table 0 (a project without a user table, or
    /// whose table could not be found).
    Clear,
}

/// What the editor and the state saver know about one oscillator's user
/// table. Never touched by the audio thread.
#[derive(Clone, Default)]
pub struct UserSlotInfo {
    /// Source file, or empty. Kept after a failed restore, so the project
    /// still names the file it wants.
    pub path: String,
    /// Display name (the file stem).
    pub name: String,
    /// The imported frames (`num_frames × WAVETABLE_SIZE`, normalised) —
    /// what state embeds and what the editor draws. `None` when no table is
    /// loaded.
    pub frames: Option<Arc<[f32]>>,
    /// Why the last load failed, if it did.
    pub error: Option<String>,
    /// A background load is in flight.
    pub loading: bool,
    /// The load request this reflects (see [`UserWavetables::request_file`]).
    pub generation: u64,
}

impl UserSlotInfo {
    pub fn num_frames(&self) -> usize {
        self.frames.as_ref().map_or(0, |f| f.len() / WAVETABLE_SIZE)
    }

    pub fn is_loaded(&self) -> bool {
        self.frames.is_some()
    }
}

/// Both oscillators' user-table state, shared between the plugin (audio
/// side), the editor and the state saver.
pub struct UserWavetables {
    slots: [Mutex<UserSlotInfo>; NUM_OSCS],
    mailboxes: [Mailbox<UserTableMsg>; NUM_OSCS],
    /// Generation of the newest load request per oscillator. A load that
    /// finishes after a newer one was requested is discarded, so two quick
    /// "Load…"s can't land out of order.
    requests: [AtomicU64; NUM_OSCS],
    /// Bumped whenever a slot's content may have changed (a request, a
    /// finished load, a clear): the state saver's revision, so a preset
    /// comparison re-serialises the (large) frames only after a change.
    revision: AtomicU64,
}

/// A finished import: everything [`UserWavetables::finish`] installs.
struct Built {
    path: String,
    name: String,
    frames: Arc<[f32]>,
    table: Box<UserTable>,
}

impl UserWavetables {
    pub fn new() -> Self {
        Self {
            slots: Default::default(),
            mailboxes: Default::default(),
            requests: Default::default(),
            revision: AtomicU64::new(0),
        }
    }

    /// A snapshot of oscillator `osc`'s slot.
    pub fn info(&self, osc: usize) -> UserSlotInfo {
        self.slots[osc].lock().clone()
    }

    /// Import `path` into oscillator `osc` on a background thread. Returns
    /// the request's generation: the slot's [`UserSlotInfo::generation`]
    /// reaches it once the load has finished, successfully or not. A failed
    /// load leaves the previous table playing and records the error.
    pub fn request_file(self: &Arc<Self>, osc: usize, path: String) -> u64 {
        let generation = self.begin(osc);
        self.slots[osc].lock().loading = true;
        let shared = self.clone();
        let spawned = std::thread::Builder::new()
            .name("wavetable-import".into())
            .spawn(move || {
                let built = build_from_file(&path);
                let _ = shared.finish(osc, generation, &path, built, false);
            });
        if let Err(e) = spawned {
            let _ = self.finish(
                osc,
                generation,
                "",
                Err(format!("could not start the import thread: {e}")),
                false,
            );
        }
        generation
    }

    /// Import `path` into oscillator `osc` on the calling thread (never the
    /// audio thread). On failure the slot is *cleared* — the oscillator falls
    /// back to bundled table 0 — but keeps naming `path`: this is the
    /// restore path, where a missing file must not leave the previous
    /// project's table playing.
    pub fn restore_file(&self, osc: usize, path: &str) -> Result<(), String> {
        let generation = self.begin(osc);
        self.finish(osc, generation, path, build_from_file(path), true)
    }

    /// Install already-sliced `frames` into oscillator `osc` on the calling
    /// thread (never the audio thread) — how a project's embedded table comes
    /// back. The frames are taken verbatim, so a re-save reproduces them bit
    /// for bit. On failure the slot is cleared, as for [`Self::restore_file`].
    pub fn restore_frames(
        &self,
        osc: usize,
        path: &str,
        name: &str,
        frames: Vec<f32>,
    ) -> Result<(), String> {
        let generation = self.begin(osc);
        let built = UserTable::build(&frames).map(|table| Built {
            path: path.to_string(),
            name: name.to_string(),
            frames: frames.into(),
            table: Box::new(table),
        });
        self.finish(osc, generation, path, built, true)
    }

    /// Remove oscillator `osc`'s user table.
    pub fn clear(&self, osc: usize) {
        let generation = self.begin(osc);
        let mut slot = self.slots[osc].lock();
        *slot = UserSlotInfo {
            generation,
            ..Default::default()
        };
        self.mailboxes[osc].post(UserTableMsg::Clear);
    }

    /// Audio thread: collect oscillator `osc`'s pending message, if any.
    /// Non-blocking, allocation-free.
    #[inline]
    pub fn take_message(&self, osc: usize) -> Option<UserTableMsg> {
        self.mailboxes[osc].try_take()
    }

    fn begin(&self, osc: usize) -> u64 {
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.requests[osc].fetch_add(1, Ordering::AcqRel) + 1
    }

    /// See the `revision` field.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    /// Publish a finished load — unless a newer request superseded it.
    fn finish(
        &self,
        osc: usize,
        generation: u64,
        path: &str,
        built: Result<Built, String>,
        clear_on_error: bool,
    ) -> Result<(), String> {
        // The slot lock orders this against a newer request's publish, so
        // the info and the mailbox always agree on which load won.
        let mut slot = self.slots[osc].lock();
        if self.requests[osc].load(Ordering::Acquire) != generation {
            return Err("superseded by a newer load".to_string());
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        match built {
            Ok(b) => {
                *slot = UserSlotInfo {
                    path: b.path,
                    name: b.name,
                    frames: Some(b.frames),
                    error: None,
                    loading: false,
                    generation,
                };
                self.mailboxes[osc].post(UserTableMsg::Install(b.table));
                Ok(())
            }
            Err(e) => {
                tracing::warn!("user wavetable for osc {}: {e}", osc + 1);
                if clear_on_error {
                    *slot = UserSlotInfo {
                        path: path.to_string(),
                        name: file_stem(path),
                        frames: None,
                        error: Some(e.clone()),
                        loading: false,
                        generation,
                    };
                    self.mailboxes[osc].post(UserTableMsg::Clear);
                } else {
                    slot.error = Some(e.clone());
                    slot.loading = false;
                    slot.generation = generation;
                }
                Err(e)
            }
        }
    }
}

impl Default for UserWavetables {
    fn default() -> Self {
        Self::new()
    }
}

fn build_from_file(path: &str) -> Result<Built, String> {
    let imported = import::import_wav_file(Path::new(path))?;
    let table = UserTable::build(&imported.frames)?;
    Ok(Built {
        path: path.to_string(),
        name: file_stem(path),
        frames: imported.frames.into(),
        table: Box::new(table),
    })
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// How many displaced tables the swap can hold inline while the janitor's
/// channel is full — the same margin as `SwapFader`'s.
const PARKED_SLOTS: usize = 4;

/// The audio-thread half: installs posted tables into the engine and ships
/// the displaced ones to a janitor thread.
pub struct UserTableSwap {
    retire_tx: Option<SyncSender<Box<UserTable>>>,
    parked: [Option<Box<UserTable>>; PARKED_SLOTS],
}

impl UserTableSwap {
    pub fn new() -> Self {
        Self {
            retire_tx: None,
            parked: Default::default(),
        }
    }

    /// Start the janitor thread, once. Non-audio-thread (`initialize()`).
    pub fn prepare(&mut self) {
        if self.retire_tx.is_none() {
            self.retire_tx = Some(resonance_dsp::SwapFader::<Box<UserTable>>::spawn_retire_janitor(
                "wavetable-retire",
            ));
        }
    }

    /// Audio thread, top of `process`: install whatever the loader posted.
    /// No allocation, no free, no blocking.
    #[inline]
    pub fn apply(&mut self, shared: &UserWavetables, engine: &mut SynthEngine) {
        for osc in 0..NUM_OSCS {
            let Some(msg) = shared.take_message(osc) else {
                continue;
            };
            let table = match msg {
                UserTableMsg::Install(t) => Some(t),
                UserTableMsg::Clear => None,
            };
            if let Some(old) = engine.install_user_table(osc, table) {
                self.retire(old);
            }
        }
    }

    /// Send a displaced table to the janitor, parking it inline when the
    /// channel is full. Only if every parking slot is taken too — a janitor
    /// that has been unreachable for several swaps — does it drop here.
    fn retire(&mut self, table: Box<UserTable>) {
        let Some(tx) = &self.retire_tx else {
            self.park(table);
            return;
        };
        for slot in &mut self.parked {
            if let Some(parked) = slot.take() {
                if let Err(TrySendError::Full(p) | TrySendError::Disconnected(p)) =
                    tx.try_send(parked)
                {
                    *slot = Some(p);
                    break;
                }
            }
        }
        if let Err(TrySendError::Full(t) | TrySendError::Disconnected(t)) = tx.try_send(table) {
            self.park(t);
        }
    }

    fn park(&mut self, table: Box<UserTable>) {
        if let Some(slot) = self.parked.iter_mut().find(|s| s.is_none()) {
            *slot = Some(table);
        }
        // else: every slot is full and `table` drops here — the last resort.
    }
}

impl Default for UserTableSwap {
    fn default() -> Self {
        Self::new()
    }
}
