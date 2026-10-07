# Autosave and crash recovery

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: ba doc #171 "Autosave triggering", todos #462–#471 (not in
> the repo); `docs/history/reviews/code-review-todo.md` (UPD-07, STATE-11,
> FU-M12a, FU-M12b, FU-R1a, UX-13).

While a project is open and has unsaved changes, Resonance periodically
writes a recovery snapshot beside the real save. If the app dies, the next
open of that project (or, for a never-saved project, the next app start)
offers to bring the snapshot back.

## Behaviour

### Autosave

- **On by default, every 30 s at most** (`AutosaveSettings { enabled: true,
  interval_secs: 30, backup_retention: 10 }` in `settings.json`). Toggled and
  tuned in the Settings view; persisted immediately.
- **Change-gated.** The tick fires an autosave only when it is enabled, the
  project is dirty, it changed since the last autosave (revision differs), a
  project is open, nothing owns the project (load, offline render,
  recording), no save is in flight, and the interval has passed since the
  later of "became dirty" and "last autosave". A clock jumping backwards only
  delays it.
- **Separate files.** It writes `project.autosave.json` and its own MIDI and
  plugin files under `autosave/`, never `project.json` or the files that
  points at; audio WAVs are shared. So "Don't save" on quit really gets you
  the last manual save.
- **Not a commit.** An autosave leaves the project dirty, skips the recents
  list, writes no backup, runs no clip GC, and never satisfies a control
  `project.save` job.
- **Untitled projects** autosave into a per-session scratch dir,
  `<data>/resonance/autosave/<pid>-<nanos>/`.
- **Failures** are logged; after `AUTOSAVE_FAILURES_BEFORE_INDICATOR` (3) in a
  row a persistent indicator shows. Any later successful write clears it.

### Crash detection

- A session keeps a `.session.lock` marker (`{pid, session_id}`) in the open
  project's dir, or in its scratch dir once an untitled project has
  autosaved. It moves on load, Save As and new; a clean close removes it.
- A marker is "live" only if it is this session's or its pid is still
  running. The pid check reads `/proc/<pid>` and so works on Linux only;
  elsewhere any other session's marker counts as a crash. A dead marker
  plus an autosave newer than `project.json` (or no `project.json` at all)
  is a recoverable offer.

### Recovery

- **Saved project, GUI open:** a prompt offers *Recover autosave*, *Open last
  saved* or *Cancel*. A recovered project opens dirty, so the next Save
  writes it to the canonical files; the autosave file is left alone until
  then.
- **Untitled session:** at startup the newest crashed scratch dir is offered
  over the startup screen (*Recover* / *Discard* / *Cancel*). Recovered work
  lands untitled.
- **Control `project.open`:** never prompts. `recover_autosave: true` loads
  the autosave; the job result says `recovered_autosave` and/or
  `autosave_available`.
- **Clean-up:** a manual save deletes the project's autosave when it is not
  newer than the save, and deletes the superseded scratch dir. Marker-less
  scratch dirs untouched for 7 days are deleted (`gc_scratch_root`,
  `SCRATCH_GC_AGE`).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Trigger | `resonance-app/src/update/project_io/autosave.rs` | `AutosaveGate`, `should_autosave`, `tick_autosave` |
| Write path | `resonance-app/src/update/project_io/mod.rs`, `project/io.rs` | `start_autosave`, `autosave_scratch_dir`, `save_autosave`, `AUTOSAVE_SIDECAR_DIR` |
| Marker + probe | `resonance-app/src/project/session.rs` | `SESSION_MARKER`, `write_marker`, `marker_is_live`, `probe`, `retire_stale_autosave`, `gc_scratch_root`, `scan_scratch_root` |
| Prompt + answers | `resonance-app/src/update/project_io/recovery.rs` | `prompt_before_open`, `offer_orphaned_session`, `handle_choice`, `open_resolved`, `finish_load`, `sync_session_marker`, `close_session`, `after_save` |
| Settings | `resonance-app/src/settings.rs` | `AutosaveSettings` |
| Failure indicator | `resonance-app/src/state/banners.rs` | `note_autosave_failure`, `clear_autosave_failures` |
| Messages | `ProjectIoMessage::{Autosave, RecoveryChoice, OpenResolved, ToggleAutosave, SetAutosaveInterval}` | `message.rs` `RecoveryChoice` |

Every marker and scratch-dir write is best effort: an I/O error costs at
most a recovery prompt, never the session.

## Surfaces

- **GUI:** Settings view (autosave toggle, interval); recovery prompt modal;
  the autosave-failing indicator.
- **Control API / MCP:** `project.open {recover_autosave}` →
  `mcp__resonance__project_open`. No method changes autosave settings.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Gate predicate + tick | `resonance-app/tests/io/autosave_trigger.rs` | `./scripts/run-tests.py -p resonance-app` |
| Write path, side files | `resonance-app/tests/io/autosave_write.rs` | same |
| Settings + UI | `resonance-app/tests/io/autosave_settings.rs`, `autosave_settings_ui.rs` | same |
| Marker rules | `resonance-app/tests/io/crash_detection.rs` | same |
| Prompt, answers, control path | `resonance-app/tests/io/autosave_recovery.rs` | same |
| Failure indicator | `resonance-app/tests/io/save_failure_recovery.rs` | same |
| Goldens | `tests/snapshots/recovery_prompt_saved_project-wgpu.png`, `recovery_prompt_untitled_at_startup-wgpu.png` | `RESONANCE_BLESS=1 ./scripts/run-tests.py -p resonance-app` |

Not covered: an actual process crash (tests fake a dead pid / foreign
marker), and two live instances racing on one project dir.

## Decisions and deviations

- Autosave lives in app data, not the cache dir: for an untitled project it
  is the only copy of the work.
- A recovered project is dirty rather than auto-saved: the user decides when
  the recovered state replaces the last manual save.

## Known gaps

- Autosave settings are GUI-only; there is no control method for them.
- Liveness is a pid check, not a lock (FU-R1a): on macOS and Windows a
  second running instance looks crashed, so its autosave is offered for
  recovery and the startup scan can offer (and *Discard* can delete) its
  untitled scratch dir. `session.rs` names an advisory lock on the marker as
  the fix.
