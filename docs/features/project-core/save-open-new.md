# Save, open, new and quit

> Verified against master @ `acac5ea3` on 2026-10-15.
> Spec history: `docs/history/reviews/code-review-todo.md` (STATE-01, STATE-09,
> STATE-11, STATE-12, STATE2-02, UX-01, UX-04, FU-A1a); control surface from
> ba doc #265 (todos #1149/#1151).

The user (or an agent) saves the open project to a `.rproj` bundle, saves it
somewhere else, opens another one, starts a fresh one, and quits — without
losing unsaved work and without one project's files leaking into another.
Every manual save also writes a timestamped backup.

## Behaviour

### Save and Save As

- **Save** (`Ctrl+S`) on a titled project starts an async save; on an
  untitled one it becomes **Save As** (`Ctrl+Shift+S`), which opens a native
  dialog (default dir `~/Documents/resonance`). A chosen name without
  `.rproj` gets it appended (case-insensitive check).
- A save is a two-part engine round-trip followed by an async write:
  1. `SetProjectDir(path)`, then `SaveClipsToProjectDir` (the engine writes,
     copies or transcodes every audio clip WAV into `path/audio/`) and
     `SaveAllPluginStates`, in parallel. Both carry the save's `request` id
     (`ProjectIoState::next_engine_request`), which the engine echoes in
     `ClipsSavedToProjectDir` / `ClipsSaveFailed` / `AllPluginStatesSaved`.
     The `SaveCollector` waits for both replies carrying its id.
  2. `try_finish_save` builds the `ProjectFile`, fills in the cached blob for
     any plugin the engine could not save (a missing `.clap`), and writes
     `project.json`, MIDI files and plugin blobs off the update loop.
  3. On success: a versioned backup, then the clip WAV GC (see
     [clip-storage-and-gc.md](clip-storage-and-gc.md)), then
     `ProjectSaved(Ok)`.
- Only one collector runs at a time (the engine has one project dir). A
  manual save during another save is queued and starts when the first
  finishes; an autosave backs off.
- A collector ignores replies with another id (BUG-61): a late reply to a
  save the watchdog abandoned, or the `AllPluginStatesSaved` of a
  "save track as preset" capture, which sends its own id and is the only
  thing its reply completes. Plugin-state replies refresh the undo blob
  cache whoever asked.
- A save cleans only what it captured: an edit made while the files were
  being written leaves the project dirty (`save_capture_revision`).
- A collector the engine never answers is failed after 180 s by the tick
  watchdog (`SAVE_COLLECT_TIMEOUT`), so one lost reply cannot wedge later
  saves, opens and news.
- A clip whose WAV is not in the bundle after the save is reported in a
  banner ("will be silent when reopened"); the save itself still succeeds.
- Save As into an existing bundle first raises the clip-id and asset-id
  counters past every `clip_<id>.wav` / `asset_<id>.wav` already there, so
  no file is overwritten.
- **Save As onto another project** — a folder with a `project.json` that is
  not the open project's (compared through symlinks) — asks first:
  "Replace project X? Its history and media will be deleted."
  (`ConfirmReplaceProject`). Cancel saves nothing (and cancels a pending
  switch, like cancelling the Save As dialog). Replace moves the target's
  `project.json`, `project.autosave.json`, `autosave/`, `backups/`,
  `audio/`, `midi/` and `plugins/` into a sibling
  `.<name>.replaced-<pid>-<ns>` dir (one rename each, on the update loop),
  deletes that dir in a background task, then saves (BUG-62). Other files
  in the folder stay. It refuses — moving nothing and saving nothing, with
  a banner — when an entry is, holds or sits inside a dir the session
  still reads (the open project, the untitled scratch dir, a pending Save
  As carry source), or another running Resonance holds the target's
  session marker; a rename that fails puts back what already moved
  (`update/project_io/replace.rs`). Without the clear, the old backups
  kept the old clips alive through the GC and the old autosave was
  offered for recovery.
- Save As copies the clip WAVs only the undo history names from the old
  bundle into the new one, in the save's write task (BUG-60; see
  [clip-storage-and-gc.md](clip-storage-and-gc.md)).

### Versioned backups

After every successful manual save `write_backup` copies `project.json` to
`backups/project-<RFC3339>.json`, hard-links the MIDI and plugin files it
names into `backups/project-<ts>.files/` (later saves rewrite those in
place), and prunes to `backup_retention` (default 10, in settings). A failed
backup is logged and does not fail the save. Autosaves never back up.

### Open

- **Open** (`Ctrl+O`) picks a `.rproj` folder; the startup screen and its
  recent list (`recent.json` in the config dir, max 10, dead entries removed
  when clicked) open directly.
- The load runs async under an `open_token`; a later open makes an earlier
  result stale. The path and the engine's project dir are only adopted when
  the load succeeds, so a failed open leaves the current project intact.
- On success: stop transport, clear undo history and the plugin mirrors,
  send `ClearAll`; on `AllCleared` the project is replayed through the
  [reconcile driver](reconcile.md). Afterwards the relink modal opens if any
  pool file or clip WAV is missing, external instruments get their patches
  re-sent, and the view scrolls to the start.
- A GUI open of a project with a crash-left autosave shows the recovery
  prompt first ([autosave-and-recovery.md](autosave-and-recovery.md)).

### New

- `Ctrl+N` / "New Project" command replaces the project with the built-in
  **Empty** template, untitled. It is refused while a load or save runs.
- The startup screen's "New Project" button is different: it opens the Save
  As dialog, and the startup session is saved there and becomes the project.
- An untitled project gets an anchor dir (the session's autosave scratch dir)
  as the engine's project dir, so recordings, imports and undo work before
  the first save. The first Save As copies the clip WAVs the history still
  names from the scratch dir into the bundle.

### Switching over unsaved changes, and quitting

- A GUI Open / New over unsaved changes asks **Save / Don't save / Cancel**.
  "Save" on an untitled project goes through Save As; cancelling that cancels
  the switch. If the save missed a late edit, the dialog asks again.
- Closing the window while dirty asks **Save & Quit / Discard & Quit /
  Cancel**. A clean close removes the session marker and scratch dir and
  shuts the engine down (150 ms).
- Every project switch is refused while an offline render owns the engine
  (bounce, stem export, freeze, measurement).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Message enum + handler | `resonance-app/src/update/project_io/mod.rs` | `ProjectIoMessage`, `handle`, `begin_save`, `start_save`, `request_switch`, `perform_switch`, `start_open`, `check_save_watchdog`, `anchor_untitled_project` |
| Save As, replacing another project | `resonance-app/src/update/project_io/save_as.rs`, `replace.rs` | `with_rproj_extension`, `save_as_to`; `replaces_another_project`, `save_as_confirmed`, `REPLACED_ENTRIES` |
| Engine replies | `resonance-app/src/engine_events/project_io.rs` | `clips_saved`, `all_plugin_states_saved`, `try_finish_save`, `report_clips_without_audio`, `all_cleared` |
| Disk I/O | `resonance-app/src/project/io.rs` | `save_project`, `load_project`, `write_backup`, `prune_backups`, `list_backups` |
| Dialogs | `resonance-app/src/update/project_io/dialogs.rs` | `save_project_as_dialog`, `open_project_dialog`, `load_project_task` |
| State | `resonance-app/src/state/project_io.rs`, `state/modal.rs` | `ProjectIoState` (`project_path`, `save_state`, `pending_load`, `open_token`, `untitled_anchor`, `carry_clip_wavs_from`, …), `ProjectSwitch`, `confirm_replace_project` |
| Dialog | `resonance-app/src/view/confirm_quit.rs` | `view_confirm_replace_overlay` |
| Recent list | `resonance-app/src/recent.rs` | `add`, `remove`, `MAX_RECENT` |
| Control handlers | `resonance-app/src/update/control/project.rs` | `new`, `open`, `save`, `save_as`, `busy_guard`, `dirty_guard` |
| View | `resonance-app/src/view/startup.rs` | startup modal |

Invariant: anything that replaces the project must clear the undo history,
`plugin_mirror` caches and `pending_plugin_param_overrides` together, or a
blob from the old project can land on a colliding instance id in the new one.

## Surfaces

- **GUI:** startup screen (New / Open / recent list); command palette and
  shortcuts `Ctrl+N`, `Ctrl+O`, `Ctrl+S`, `Ctrl+Shift+S`; window close.
- **Control API / MCP** (all run as jobs; poll `job.status` / `job.wait`):
  `project.new {template?, confirm}` → `mcp__resonance__project_new`,
  `project.open {path, confirm, recover_autosave}` → `mcp__resonance__project_open`,
  `project.save {path?, confirm}` → `mcp__resonance__project_save`,
  `project.save_as {path, confirm}` → `mcp__resonance__project_save_as`.
  Unsaved changes on new/open, or overwriting another existing path, answer
  `needs_confirmation` until `confirm: true`. For a save over another
  project the refusal says its history and media would be deleted, and
  `confirm: true` is the answer to the GUI's "Replace project?" dialog
  (`ProjectIoMessage::SaveAsConfirmed`): the target is cleared the same
  way, then saved. No new parameter; the protocol version is unchanged. Paths must be absolute. The
  job completes only after the replay ran, so a read right after
  `job_wait` sees the new project.
- **Agent skills:** none in `resonance-agent-plugin/skills/`. The
  user-level `resonance-studio` skill (outside the repo) calls
  `project_new` / `project_save`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Switch / quit dialogs, untitled anchor | `resonance-app/tests/io/project_lifecycle_ux.rs` | `./scripts/run-tests.py -p resonance-app` |
| Failed open keeps path | `resonance-app/tests/io/open_failure_keeps_path.rs` | same |
| Late edit keeps dirty | `resonance-app/tests/io/save_keeps_dirty_for_late_edit.rs` | same |
| Engine never answers, clip save fails | `resonance-app/tests/io/save_failure_recovery.rs` | same |
| Replace-project dialog, cancel, clear, refusals, control confirm | `resonance-app/tests/io/save_as_existing_project.rs` | same |
| Replies reach only their own request | `resonance-app/tests/io/save_reply_tagging.rs`, `tests/control/control_track_presets.rs` (`a_preset_capture_reply_does_not_complete_a_collecting_save`), `resonance-audio/tests/engine/save_clips_always_answers.rs` | same, `-p resonance-audio` |
| Backups | `resonance-app/tests/io/project_backups.rs` | same |
| Control `project.*` | `resonance-app/tests/control/control_project.rs`, `control_jobs.rs`, `control_mutation_gate_loading.rs` | same |

Not covered: the rfd file dialogs themselves (no headless dialog), a real
engine's `SaveClipsToProjectDir` copy/transcode during Save As (tests use the
command-capturing engine), and opening a backup JSON end to end.

## Decisions and deviations

- The control API never shows a dialog: `project.open` decides recovery up
  front with `recover_autosave`, and dirty or overwrite cases are refused
  with `needs_confirmation` instead of a modal.
- Replacing a project deletes its files right away (in the background),
  not after the save succeeds: the user confirmed the deletion, and the
  open project is still in memory to save again if the write fails.
- Backups hard-link side files rather than copy them; audio WAVs are shared
  by name because they never change.

## Known gaps

- Backups are written and pruned, but there is no restore UI: `list_backups`
  has no caller. See [restore-from-backup](../../intents/project-core/restore-from-backup.md).
- The startup screen's "New Project" asks for a save location first, while
  `Ctrl+N` creates an untitled project; the two paths behave differently.
