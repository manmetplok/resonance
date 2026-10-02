//! The actual egui app: state and update/view orchestration for the drums editor.
//!
//! `DrumsEditorApp` is the `EditorApp` the runtime drives each frame. It
//! paints the chrome on the outside — the header (kit, Library…), the tab
//! bar (`[Pads | Mix | Setup]` and the preset bar) and the status bar —
//! and the selected tab's body in the middle (drums-plugin-rework.md §6):
//!
//! - **Pads** (`pads_tab.rs`): the 6×5 pad grid and the selected pad's
//!   inspector;
//! - **Mix** (`mix_tab.rs`): a meter strip per output port, master, the
//!   per-pad level/pan/mute/output table and the global playing settings;
//! - **Setup** (`setup_tab.rs`): mic banks, streaming, the pad routing /
//!   choke / note table and the kit's facts.
//!
//! The Library overlay (`library_panel.rs`) draws over everything when open.
//!
//! The kit library state lives here, not in the overlay, because the
//! header's kit dropdown and ◀/▶ walk the same view with the overlay
//! closed (drums-plugin-rework.md §6.1).

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use plugin_gui_core::{egui, EditorApp};
use resonance_common::drumkit_library::{self, Entry, EntryStatus, ImportOutcome};
use resonance_common::library_marks::{FreshnessPoll, BAR_POLL_INTERVAL, BROWSER_POLL_INTERVAL};
use resonance_plugin::kit_rows::KitRows;
use resonance_plugin::library_view::BrowserModel;

use crate::download::Status;
use crate::last_hit::LastHit;
use crate::library::SharedKitLibrary;
use crate::drum_map::NUM_PADS;
use crate::kit::NUM_OUTPUT_PORTS;
use crate::params::DrumParams;
use crate::KitBridge;

use super::jobs::{JobDone, JobKind, Jobs, Picker};
use super::kit_browser::{self, LoadKind};
use super::library_panel::{self, LibraryPanelState};
use super::missing_kit::{self, MissingKitState};
use super::controls::{self, Gestures, Labels};
use super::{chrome, mix_tab, pads_tab, setup_tab, theme};

/// The editor's three views (§6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    Pads,
    Mix,
    Setup,
}

impl Tab {
    pub(crate) const ALL: [Tab; 3] = [Tab::Pads, Tab::Mix, Tab::Setup];
    pub(crate) const LABELS: [&'static str; 3] = ["Pads", "Mix", "Setup"];

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

/// How long a pad cell stays lit after a hit, seconds.
pub(crate) const HIT_FLASH_SECS: f64 = 0.25;

/// Repaint interval while a cell's hit light fades: ~30 Hz is smooth for
/// a quarter-second fade, at half the frames of the display rate.
const LIT_REPAINT: std::time::Duration = std::time::Duration::from_millis(33);
/// Repaint interval while something moves — a meter falling, a load or a
/// library job under way, a gesture open: the readouts' ~10 Hz.
const LIVE_REPAINT: std::time::Duration = std::time::Duration::from_millis(100);
/// Repaint interval with nothing moving: only to notice the next hit or
/// output (the audio thread cannot wake the editor) and the library
/// polls. The meters at rest and a static view need no 10 Hz tick.
const IDLE_REPAINT: std::time::Duration = std::time::Duration::from_millis(250);

/// How often a deleted-looking kit folder is `stat`ed again.
const KIT_EXISTS_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);

pub(crate) struct DrumsEditorApp {
    pub(crate) params: Arc<DrumParams>,
    pub(crate) bridge: KitBridge,
    pub(crate) selected_pad: usize,
    /// The view the body shows.
    pub(crate) tab: Tab,
    /// The inspector's audition velocity, MIDI 1..=127.
    pub(crate) audition_velocity: u8,
    /// Param readouts, rebuilt only when a value moves.
    pub(crate) labels: Labels,
    /// Each pad's last hit sequence seen, and when the cell lit for it
    /// (egui time, seconds).
    pub(crate) hit_seen: [u16; NUM_PADS],
    pub(crate) hit_at: [f64; NUM_PADS],
    /// What the hit slots held when the playing kit last changed (each
    /// pad's sequence, and the latest's): a hit from the kit before is
    /// not this kit's — its layer and take index another kit's takes —
    /// so it reads as no hit until the pad is played again.
    hits_before_kit: ([u16; NUM_PADS], u16),
    /// The kit those hits belong to (its manifest; `None`: built-in).
    hits_kit: Option<PathBuf>,
    /// Displayed Mix-strip meter levels, one per output port, with the
    /// same ballistics as the OUT meter, and when they last fell (egui
    /// time).
    port_meter: [f32; NUM_OUTPUT_PORTS],
    port_meter_at: Option<f64>,
    /// The process-wide kit library and its download worker.
    pub(crate) library: Arc<SharedKitLibrary>,
    pub(crate) library_panel: LibraryPanelState,
    /// The Installed tab's view-state, which the header's dropdown and
    /// ◀/▶ walk too.
    pub(crate) browser: BrowserModel,
    /// The library as browser rows, rebuilt when the library or the marks
    /// change.
    pub(crate) rows: KitRows,
    /// The detail pane's `+ tag` text, and the row it was typed for.
    pub(crate) tag_draft: String,
    tag_draft_for: Option<String>,
    /// Change detection for other processes' marks writes.
    marks_poll: FreshnessPoll,
    /// Change detection for the library root and index (kits added or
    /// removed in a file manager, another process's rescan).
    library_poll: FreshnessPoll,
    /// Rescans, imports and deletes, off the editor thread.
    pub(crate) jobs: Jobs,
    /// The import's file dialog, on its own thread.
    pub(crate) picker: Picker,
    /// The kits THIS editor asked the shared worker to download, so only
    /// it jumps to the new row when one lands — or reports one that failed.
    pub(crate) my_downloads: HashSet<String>,
    /// Library actions the user asked for while a job held the library,
    /// started in order as each job finishes — a confirmed delete or a
    /// picked import folder is never dropped because a background rescan
    /// happened to be running.
    pub(crate) queued: VecDeque<Queued>,
    /// Kits whose sample files were checked this session
    /// (`check_missing_files`), so each is stat'ed once.
    missing_checked: HashSet<String>,
    /// The worker's install counter last seen.
    seen_installs: u64,
    /// The last kit load this editor started, so ◀/▶ step from the kit on
    /// its way rather than the one it replaces
    /// (`kit_browser::kit_path_for_stepping`).
    pub(crate) requested_kit: Option<kit_browser::RequestedKit>,
    /// The library name of the kit last seen playing, by manifest — what
    /// the header still calls it once its folder is deleted.
    pub(crate) last_loaded_name: Option<(PathBuf, String)>,
    /// Displayed OUT meter level per channel. Rises instantly to the peak
    /// the audio thread published and falls back with a fixed decay, so
    /// the bar tracks real output instead of sitting dead.
    out_meter: [f32; 2],
    out_meter_at: Option<f64>,
    /// `loaded_entry`'s answer, for the library revision and kit it was
    /// worked out for — it was a scan and a clone of the entry per frame.
    loaded_cache: Option<(LoadedKey, Option<Arc<Entry>>)>,
    /// Whether the playing kit's manifest was there when last `stat`ed,
    /// and when: the header's "deleted" check, once a second rather than
    /// every frame.
    kit_exists: Option<(PathBuf, std::time::Instant, bool)>,
    /// This plugin ships no factory presets, so the bank is the user's
    /// own directory alone (ba todo #1358).
    pub(crate) bank: resonance_plugin::presets::PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    pub(crate) presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: resonance_plugin::presets::PresetEditor,
    /// The missing-kit banner's state (§5.3).
    pub(crate) missing_kit: MissingKitState,
    /// Continuous gestures in flight (`controls::Gestures`), lent to the
    /// controls each frame and flushed when a widget cannot end its own.
    gestures: Arc<Gestures>,
}

impl DrumsEditorApp {
    pub(super) fn new(
        params: Arc<DrumParams>,
        bridge: KitBridge,
        library: Arc<SharedKitLibrary>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        let seen_installs = library.download().state.lock().installs;
        // Hits from before the editor opened are history, not news: seen
        // already, so opening the editor does not flash every pad played
        // since the instance started.
        let hit_seen = std::array::from_fn(|pad| bridge.last_hits.pad(pad).map_or(0, |h| h.seq));
        let hits_kit = bridge.kit_path.lock().clone();
        let mut app = Self {
            params,
            bridge,
            selected_pad: 0,
            tab: Tab::default(),
            audition_velocity: 100,
            labels: Labels::default(),
            hit_seen,
            hit_at: [f64::NEG_INFINITY; NUM_PADS],
            hits_before_kit: ([0; NUM_PADS], 0),
            hits_kit,
            port_meter: [0.0; NUM_OUTPUT_PORTS],
            port_meter_at: None,
            library,
            library_panel: LibraryPanelState::default(),
            browser: BrowserModel::new(),
            rows: KitRows::default(),
            tag_draft: String::new(),
            tag_draft_for: None,
            marks_poll: FreshnessPoll::new(Vec::new(), BAR_POLL_INTERVAL),
            library_poll: FreshnessPoll::new(Vec::new(), BAR_POLL_INTERVAL),
            jobs: Jobs::default(),
            picker: Picker::default(),
            my_downloads: HashSet::new(),
            queued: VecDeque::new(),
            missing_checked: HashSet::new(),
            seen_installs,
            requested_kit: None,
            last_loaded_name: None,
            out_meter: [0.0; 2],
            out_meter_at: None,
            loaded_cache: None,
            kit_exists: None,
            bank: resonance_plugin::presets::PresetBank::for_plugin::<crate::ResonanceDrums>(),
            presets,
            preset_editor: resonance_plugin::presets::PresetEditor::default(),
            missing_kit: MissingKitState::default(),
            gestures: Arc::new(Gestures::default()),
        };
        // Opening an editor is when the library is brought up to date: on
        // the job thread, so the first frame is not held up by hashing.
        app.start_rescan(false);
        app
    }

    /// Fold the audio thread's latest block peak into the displayed OUT
    /// meter and return the level to draw, at egui time `now`. The audio
    /// thread publishes every block and the editor reads whenever it
    /// repaints, so the peak is taken as an instant rise and a fall of
    /// 0.75× per tenth of a second of *time* — not per frame, which made
    /// the fall three times faster at 30 Hz than at 10 Hz.
    pub(crate) fn tick_out_meter(&mut self, now: f64) -> [f32; 2] {
        let dt = elapsed(&mut self.out_meter_at, now);
        for (channel, level) in self.out_meter.iter_mut().enumerate() {
            fold_peak(level, self.bridge.out_peak[channel].load(Ordering::Relaxed), dt);
        }
        self.out_meter
    }

    /// [`Self::tick_out_meter`] for the Mix tab's per-port strips.
    pub(crate) fn tick_port_meters(&mut self, now: f64) -> [f32; NUM_OUTPUT_PORTS] {
        let dt = elapsed(&mut self.port_meter_at, now);
        for (port, level) in self.port_meter.iter_mut().enumerate() {
            fold_peak(level, self.bridge.port_peak[port].load(Ordering::Relaxed), dt);
        }
        self.port_meter
    }

    /// Whether a meter still shows anything (and so has a fall to draw).
    fn meters_moving(&self) -> bool {
        self.out_meter.iter().chain(&self.port_meter).any(|&l| l > 0.0)
    }

    /// Note every pad hit the sampler published since the last frame, so
    /// its cell lights — unless the pad is muted: a muted pad's click
    /// plays nothing, and a light would say it did. Returns whether any
    /// cell is still lit (the caller keeps repainting while one is).
    pub(crate) fn tick_hits(&mut self, now: f64) -> bool {
        self.follow_kit_for_hits();
        let mut lit = false;
        for pad in 0..NUM_PADS {
            if let Some(hit) = self.bridge.last_hits.pad(pad) {
                if hit.seq != self.hit_seen[pad] {
                    self.hit_seen[pad] = hit.seq;
                    if !self.params.pads[pad].mute.value() {
                        self.hit_at[pad] = now;
                    }
                }
            }
            lit |= now - self.hit_at[pad] < HIT_FLASH_SECS;
        }
        lit
    }

    /// When the playing kit changed, take the hit slots' current contents
    /// as the old kit's (see `hits_before_kit`).
    fn follow_kit_for_hits(&mut self) {
        let kit = self.bridge.kit_path.lock().clone();
        if kit == self.hits_kit {
            return;
        }
        self.hits_kit = kit;
        let hits = &self.bridge.last_hits;
        self.hits_before_kit = (
            std::array::from_fn(|pad| hits.pad(pad).map_or(0, |h| h.seq)),
            hits.latest().map_or(0, |h| h.seq),
        );
    }

    /// The last hit on `pad` with the kit that plays now.
    pub(crate) fn pad_hit(&self, pad: usize) -> Option<LastHit> {
        self.bridge
            .last_hits
            .pad(pad)
            .filter(|h| h.seq != self.hits_before_kit.0[pad])
    }

    /// The most recent hit on any pad with the kit that plays now.
    pub(crate) fn latest_hit(&self) -> Option<LastHit> {
        self.bridge
            .last_hits
            .latest()
            .filter(|h| h.seq != self.hits_before_kit.1)
    }

    /// Open the Library overlay.
    pub(crate) fn open_library(&mut self) {
        library_panel::open(self);
    }

    /// Start a background rescan (plus measuring kits of unknown size)
    /// unless a job is running. `user`: the Rescan button, whose outcome
    /// is reported — including losing the library to another writer.
    pub(crate) fn start_rescan(&mut self, user: bool) -> bool {
        let library = self.library.clone();
        self.jobs
            .start(JobKind::Scan, "scanning…", false, move |ctx| {
                // `rescan_with` — not `rescan` — so closing the editor mid-job
                // never waits on the `installed.json` migration's index
                // fetch: the job's own cancel flag bounds it.
                let (result, skipped) = match library.rescan_with(&ctx.cancel) {
                    // Another writer (a download installing) rescans when done.
                    None => (Ok(()), true),
                    Some(Ok(_)) => (Ok(()), false),
                    Some(Err(e)) => (Err(e.to_string()), false),
                };
                if !skipped {
                    library.measure_unsized(&ctx.cancel);
                }
                JobDone::Rescanned {
                    result,
                    skipped,
                    user,
                }
            })
    }

    /// Run `action` now, or after the running job when there is one.
    pub(crate) fn run_or_queue(&mut self, action: Queued) {
        if self.jobs.busy() {
            if !self.queued.contains(&action) {
                self.queued.push_back(action);
            }
            return;
        }
        match action {
            Queued::Delete(key) => library_panel::start_delete(self, &key),
            Queued::Import(src) => {
                library_panel::start_import(self, src);
            }
            Queued::Rescan => {
                self.start_rescan(true);
            }
        }
    }

    /// Check the selected kit's (and the loaded kit's) sample files once,
    /// on a job, so a kit with files gone shows it before it is loaded.
    ///
    /// Runs on every idle frame, so it clones nothing until it has found
    /// a kit to check.
    fn check_missing_lazily(&mut self) {
        if self.jobs.busy() {
            return;
        }
        let loaded = self.loaded_entry();
        let unchecked = |e: &&Entry| {
            !self.missing_checked.contains(&e.id)
                && matches!(e.status, EntryStatus::Ok | EntryStatus::MissingFiles(_))
        };
        let selected = self
            .library_panel
            .open
            .then(|| self.browser.selected_row())
            .flatten()
            .map(|row| &self.rows.rows[row].entry);
        let Some((id, name)) = selected
            .filter(unchecked)
            .or(loaded.as_deref().filter(unchecked))
            .map(|e| (e.id.clone(), e.name.clone()))
        else {
            return;
        };
        self.missing_checked.insert(id.clone());
        let library = self.library.clone();
        self.jobs.start(
            JobKind::Check,
            format!("checking \"{name}\"…"),
            false,
            move |_| {
                let result = match library.try_mutate(|lib| lib.check_missing_files(&id)) {
                    None => Err(String::new()),
                    Some(Ok(n)) => Ok(n),
                    Some(Err(e)) => Err(e.to_string()),
                };
                JobDone::CheckedFiles { id, result }
            },
        );
    }

    /// The library's files were just written by this editor's own job (or
    /// the download worker): take their current state as seen, so the
    /// freshness poll does not answer with a rescan of its own — which
    /// held the job slot when the user's next action came.
    fn rebaseline_library_poll(&mut self) {
        self.library_poll = FreshnessPoll::new(Vec::new(), BAR_POLL_INTERVAL);
    }

    /// Rebuild the rows if the library or the marks changed, and refresh
    /// the view.
    pub(crate) fn refresh_rows(&mut self) {
        let revision = self.library.revision();
        let marks_gen = self.library.marks_generation();
        if self.rows.built_from != (revision, marks_gen) {
            let marks = self.library.marks().snapshot();
            let lib = self.library.read();
            self.rows = KitRows::build(&lib, Some(&marks), (revision, marks_gen));
        }
        self.browser.refresh(&self.rows, (revision, marks_gen));
        // A draft tag belongs to the row it was typed for.
        if self.browser.selected() != self.tag_draft_for.as_deref() {
            self.tag_draft.clear();
            self.tag_draft_for = self.browser.selected().map(str::to_string);
        }
    }

    /// Apply a finished background job, and a closed import dialog; then
    /// start what was queued behind it.
    pub(crate) fn poll_jobs(&mut self) {
        if let Some(answer) = self.picker.poll() {
            // One picker serves the Library's import and the missing-kit
            // banner's Locate; the banner said which it opened it for.
            let locating = std::mem::take(&mut self.missing_kit.locating);
            match (answer, locating) {
                (Some(dir), true) => missing_kit::picked(self, dir),
                (Some(src), false) => self.run_or_queue(Queued::Import(src)),
                (None, _) => {}
            }
        }
        if let Some(done) = self.jobs.poll() {
            self.apply_job(done);
        }
        self.start_queued();
    }

    /// Start the next queued action, else a lazy missing-files check.
    fn start_queued(&mut self) {
        if self.jobs.busy() {
            return;
        }
        match self.queued.pop_front() {
            Some(action) => self.run_or_queue(action),
            None => self.check_missing_lazily(),
        }
    }

    pub(crate) fn apply_job(&mut self, done: JobDone) {
        let wrote = !matches!(done, JobDone::CheckedFiles { .. } | JobDone::Located { .. });
        // An import started by the missing-kit banner's Locate.
        let relinking = matches!(done, JobDone::Imported(_))
            && std::mem::take(&mut self.missing_kit.relinking);
        match done {
            JobDone::Rescanned {
                result,
                skipped,
                user,
            } => match result {
                Err(e) => self.browser.set_error(format!("rescan failed: {e}")),
                Ok(()) if user && skipped => self.browser.set_error(format!(
                    "rescan skipped: {} It is rescanned when that finishes.",
                    library_panel::BUSY_WHY
                )),
                Ok(()) if user => {
                    let n = self.library.read().len();
                    self.browser.set_info(format!(
                        "rescanned: {n} kit{}",
                        if n == 1 { "" } else { "s" }
                    ));
                }
                Ok(()) => {}
            },
            JobDone::Imported(result) => match *result {
                Ok(outcome) => {
                    self.refresh_rows();
                    self.browser.select(outcome.entry().mark_key());
                    self.library_panel.tab = library_panel::Tab::Installed;
                    match &outcome {
                        // A kit imported on its own is what the user wants
                        // to hear next: load it, as the amp does.
                        ImportOutcome::Added(e) => {
                            let entry = self
                                .library
                                .read()
                                .entry(&e.id)
                                .cloned()
                                .unwrap_or_else(|| e.clone());
                            match kit_browser::load_library_kit(
                                &self.bridge,
                                &self.library,
                                &entry,
                                LoadKind::Pick,
                            ) {
                                Ok(req) => {
                                    self.requested_kit = Some(req);
                                    self.browser
                                        .set_info(format!("imported and loaded \"{}\"", e.name));
                                }
                                Err(why) => self.browser.set_error(format!(
                                    "imported \"{}\", but could not load it: {why}",
                                    e.name
                                )),
                            }
                        }
                        // Relinking a missing kit: the folder is (now) in
                        // the library either way, and is what to play.
                        ImportOutcome::AlreadyPresent(e) if relinking => {
                            let entry = self
                                .library
                                .read()
                                .entry(&e.id)
                                .cloned()
                                .unwrap_or_else(|| e.clone());
                            self.load_entry(&entry, LoadKind::Pick);
                        }
                        ImportOutcome::AlreadyPresent(e) => self
                            .browser
                            .set_info(format!("\"{}\" is already in the library", e.name)),
                    }
                }
                Err(e) if relinking => self.missing_kit.error = Some(e),
                Err(e) => self.browser.set_error(e),
            },
            JobDone::Deleted {
                name,
                result,
                view_pos,
            } => {
                self.refresh_rows();
                match result {
                    Ok(()) => {
                        // The row's neighbour takes the selection, so the
                        // detail pane is not left blank after a delete.
                        let view = self.browser.view();
                        match view_pos.filter(|_| !view.is_empty()) {
                            Some(pos) => {
                                let row = view[pos.min(view.len() - 1)];
                                let key = self.rows.rows[row].key.clone();
                                self.browser.select(key);
                            }
                            None => self.browser.clear_selection(),
                        }
                        self.browser.set_info(format!("deleted \"{name}\""));
                    }
                    Err(e) => self.browser.set_error(e),
                }
            }
            JobDone::CheckedFiles { id, result } => match result {
                Ok(_) => self.refresh_rows(),
                // Lost the library to another writer: try again later.
                Err(e) if e.is_empty() => {
                    self.missing_checked.remove(&id);
                }
                Err(e) => tracing::warn!("missing-files check of {id}: {e}"),
            },
            JobDone::Located { dir, id } => missing_kit::located(self, dir, id),
        }
        if wrote {
            self.rebaseline_library_poll();
        }
    }

    /// Block until the running job and everything queued behind it are
    /// done, applying each. For tests — only `TestEditor` (`test-hooks`)
    /// calls it.
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub(crate) fn finish_jobs(&mut self) {
        // Bounded: a check that keeps losing the library to a download
        // would otherwise retry for ever.
        for _ in 0..64 {
            if let Some(done) = self.jobs.wait() {
                self.apply_job(done);
            }
            if let Some(action) = self.queued.pop_front() {
                self.run_or_queue(action);
                continue;
            }
            self.check_missing_lazily();
            if !self.jobs.busy() {
                break;
            }
        }
    }

    /// React to a download the shared worker finished: when it is one this
    /// editor asked for, select the new row on the Installed tab and offer
    /// Load (§4.1); when one of them stopped without installing (an error,
    /// a cancel), say so — a failed Re-download used to vanish silently.
    fn poll_downloads(&mut self) {
        self.poll_installs();
        self.poll_failed_downloads();
    }

    /// The downloads this editor asked for that ended without installing.
    fn poll_failed_downloads(&mut self) {
        if self.my_downloads.is_empty() {
            return;
        }
        let ended: Vec<(String, String, bool)> = {
            let s = self.library.download().state.lock();
            // Not ended while running or queued — nor while an install
            // the counter has not shown this editor yet is pending.
            if s.installs != self.seen_installs {
                return;
            }
            self.my_downloads
                .iter()
                .filter(|n| !s.is_working_on(n))
                .map(|n| {
                    let (why, error) = match (&s.status, &s.last_error) {
                        (Status::Cancelled(k), _) if k == n => {
                            (format!("the download of \"{n}\" was cancelled"), false)
                        }
                        (Status::Error(e), _) | (_, Some(e)) => {
                            (format!("could not download \"{n}\": {e}"), true)
                        }
                        _ => (format!("the download of \"{n}\" stopped"), true),
                    };
                    (n.clone(), why, error)
                })
                .collect()
        };
        for (name, why, error) in ended {
            self.my_downloads.remove(&name);
            if error {
                self.browser.set_error(why);
            } else {
                self.browser.set_info(why);
            }
        }
    }

    fn poll_installs(&mut self) {
        let new = {
            let s = self.library.download().state.lock();
            if s.installs == self.seen_installs {
                return;
            }
            // Two installs can finish between two frames (ba review): walk
            // every one this editor has not seen yet, not just the latest.
            let new = s.installs_since(self.seen_installs);
            self.seen_installs = s.installs;
            new
        };
        // The worker rescanned the library itself.
        self.rebaseline_library_poll();
        for installed in new {
            if !self.my_downloads.remove(&installed.name) {
                continue;
            }
            // The missing-kit banner's download: play it.
            if missing_kit::installed(self, &installed.name, &installed.id) {
                continue;
            }
            self.refresh_rows();
            self.browser
                .select(drumkit_library::mark_key(&installed.id));
            self.library_panel.tab = library_panel::Tab::Installed;
            self.browser.set_info(format!(
                "downloaded \"{}\" — Load plays it in this instance",
                installed.name
            ));
        }
    }

    /// Poll for other processes' changes: every 500 ms while the Library is
    /// open, every 2 s otherwise, one `stat` per path each time.
    fn poll_freshness(&mut self) {
        let now = std::time::Instant::now();
        let interval = if self.library_panel.open {
            BROWSER_POLL_INTERVAL
        } else {
            BAR_POLL_INTERVAL
        };
        if self.marks_poll.targets().is_empty() {
            let marks = self.library.marks().path();
            self.marks_poll = FreshnessPoll::new(vec![marks], interval);
        }
        self.marks_poll.set_interval(interval);
        if self.marks_poll.check(now) {
            self.library.refresh_marks();
        }

        if self.library_poll.targets().is_empty() {
            let paths = self.library.read().watch_paths();
            self.library_poll = FreshnessPoll::new(paths, interval);
            self.library_poll.mark_seen(now);
        }
        self.library_poll.set_interval(interval);
        if !self.jobs.busy() && self.queued.is_empty() && self.library_poll.check(now) {
            // Off the UI thread. A rescan only re-hashes manifests whose
            // size or mtime changed and writes nothing when nothing did,
            // so the poll settles.
            self.start_rescan(false);
        }
    }

    /// Toggle the favourite of kit `id`, reporting a failed write.
    pub(crate) fn toggle_favorite(&mut self, id: &str) {
        if let Err(e) = self.library.toggle_favorite(id) {
            self.browser
                .set_error(format!("could not save the favourite: {e}"));
        }
    }

    /// The library entry of the kit this instance plays (or is loading),
    /// worked out once per library revision and kit.
    pub(crate) fn loaded_entry(&mut self) -> Option<Arc<Entry>> {
        let path = kit_browser::kit_path_for_stepping(&self.bridge, self.requested_kit.as_ref());
        let key = (self.library.revision(), path);
        if let Some((cached, entry)) = &self.loaded_cache {
            if *cached == key {
                return entry.clone();
            }
        }
        let entry = key
            .1
            .as_deref()
            .and_then(|p| self.library.entry_for_manifest(p))
            .map(Arc::new);
        self.loaded_cache = Some((key, entry.clone()));
        entry
    }

    /// Whether `path` (the playing kit's manifest) is on disk, `stat`ed at
    /// most once a second.
    pub(crate) fn kit_file_exists(&mut self, path: &std::path::Path) -> bool {
        let now = std::time::Instant::now();
        match &self.kit_exists {
            Some((p, at, exists)) if p == path && now.duration_since(*at) < KIT_EXISTS_RECHECK => *exists,
            _ => {
                let exists = path.exists();
                self.kit_exists = Some((path.to_path_buf(), now, exists));
                exists
            }
        }
    }

    /// Load `entry` into this instance through the one load entry point.
    pub(crate) fn load_entry(&mut self, entry: &Entry, kind: LoadKind) {
        match kit_browser::load_library_kit(&self.bridge, &self.library, entry, kind) {
            Ok(req) => self.requested_kit = Some(req),
            Err(e) => self.browser.set_error(e),
        }
    }
}

/// A library action waiting for the running job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Queued {
    /// A confirmed delete of the row with this key.
    Delete(String),
    /// A kit folder or `.zip` picked for import.
    Import(PathBuf),
    /// The Rescan button.
    Rescan,
}

/// What `loaded_entry` is cached by: the library revision and the kit.
type LoadedKey = (u64, Option<PathBuf>);

/// Seconds since `*last` (then `now` becomes it), capped at half a second
/// so a meter not drawn for a while (another tab) falls at once rather
/// than by a huge step; zero on the first tick.
fn elapsed(last: &mut Option<f64>, now: f64) -> f32 {
    let dt = last.map_or(0.0, |t| (now - t).clamp(0.0, 0.5));
    *last = Some(now);
    dt as f32
}

/// Fold a published block peak (`f32::to_bits`) into a displayed level,
/// `dt` seconds after the last fold: an instant rise and a fall of 0.75×
/// per 100 ms — a real reading with readable ballistics, never a value
/// we made up, falling at the same speed whatever the repaint rate.
pub(crate) fn fold_peak(level: &mut f32, published_bits: u32, dt: f32) {
    const DECAY_PER_TENTH: f32 = 0.75;
    let published = f32::from_bits(published_bits);
    let published = if published.is_finite() && published > 0.0 {
        published
    } else {
        0.0
    };
    *level = if published >= *level {
        published
    } else {
        (*level * DECAY_PER_TENTH.powf(dt * 10.0)).max(published)
    };
    if *level < 1.0e-5 {
        *level = 0.0;
    }
}

/// Height of the header (brand, kit, Library…).
pub(crate) const HEADER_H: f32 = 40.0;
/// Height of the tab bar (tabs, preset bar).
pub(crate) const TAB_BAR_H: f32 = 38.0;
/// Height of the status bar.
pub(crate) const STATUS_H: f32 = 28.0;

impl EditorApp for DrumsEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        controls::lend_gestures(ui.ctx(), &self.gestures);
        self.poll_jobs();
        self.poll_downloads();
        self.poll_freshness();
        // Once per frame: the header's dropdown and ◀/▶ read the rows
        // this built.
        self.refresh_rows();

        let now = ui.ctx().input(|i| i.time);
        // A lit cell fades over a quarter second at ~30 Hz; meters falling
        // and work under way repaint at ~10 Hz; at rest only a slow poll
        // for the next hit.
        let lit = self.tick_hits(now);
        ui.ctx().request_repaint_after(if lit {
            LIT_REPAINT
        } else if self.something_moves() {
            LIVE_REPAINT
        } else {
            IDLE_REPAINT
        });

        let bar = |fill| {
            egui::Frame::default()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(14, 6))
                .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        };
        egui::Panel::top("drums_header")
            .exact_size(HEADER_H)
            .frame(bar(theme::BG_1))
            .show_inside(ui, |ui| chrome::draw_header(ui, self));
        egui::Panel::top("drums_tabs")
            .exact_size(TAB_BAR_H)
            .frame(bar(theme::BG_1))
            .show_inside(ui, |ui| chrome::draw_tab_bar(ui, self));
        egui::Panel::bottom("drums_status")
            .exact_size(STATUS_H)
            .frame(bar(theme::BG_1))
            .show_inside(ui, |ui| chrome::draw_status_bar(ui, self));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_0)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show_inside(ui, |ui| {
                // A missing kit says so over every tab, with what to do
                // about it (§5.3).
                if super::missing_kit::draw(ui, self) {
                    ui.add_space(10.0);
                }
                match self.tab {
                    Tab::Pads => pads_tab::draw(ui, self),
                    Tab::Mix => mix_tab::draw(ui, self),
                    Tab::Setup => setup_tab::draw(ui, self),
                }
            });

        if self.library_panel.open {
            library_panel::draw(ui, self);
        }
        self.close_stale_gestures(ui.ctx());
    }
}

impl DrumsEditorApp {
    /// Whether anything on screen is changing on its own: a meter still
    /// falling, a kit loading, a library job or download running, the
    /// Library open (its rows show other instances' downloads), a
    /// gesture open.
    fn something_moves(&self) -> bool {
        self.meters_moving()
            || !self.bridge.load_progress.snapshot().complete
            || self.jobs.busy()
            || !self.my_downloads.is_empty()
            || self.library_panel.open
            || self.gestures.any_open()
    }

    /// Announce the gestures no widget will end: a drag whose control
    /// was not drawn when the pointer let go (its table row scrolled out
    /// of view under a wheel), a key run whose control went away.
    fn close_stale_gestures(&mut self, ctx: &egui::Context) {
        let (now, down) = ctx.input(|i| (i.time, i.pointer.any_down()));
        let dragging = down || ctx.dragged_id().is_some();
        let closed = self.gestures.stale(now, dragging);
        controls::announce_closed(&self.bridge, &self.params, closed);
        if self.gestures.any_open() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl Drop for DrumsEditorApp {
    /// An editor closed mid-gesture never draws the frame that ends it:
    /// what moved is announced now, so it is still one undoable edit.
    fn drop(&mut self) {
        controls::announce_closed(&self.bridge, &self.params, self.gestures.drain());
    }
}

/// A fixed-size column inside a horizontal row, laid out top to bottom.
///
/// `ui.allocate_ui(size, ..)` would be the obvious call, and it is wrong
/// here: it reuses the *parent's* layout (egui's `allocate_ui` is
/// `allocate_ui_with_layout(size, *self.layout(), ..)`), and every column
/// in these bodies sits in a horizontal row — their contents ran left to
/// right once, the inspector laid out 1878 px wide in a 960 px window.
/// Every column states its own direction instead.
pub(crate) fn column<R>(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), add)
        .inner
}
