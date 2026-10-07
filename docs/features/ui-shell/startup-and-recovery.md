# Startup screen, recent projects and the recovery prompt

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: code review FU-M12a (recovery), UX-01/UX-03 (switch confirm,
> "not saved"); prototype
> `docs/intents/prototypes/project-templates-starter-sessions/index.html`.

When no project is open, the app shows a modal **New session** screen. It
offers New Project, Open Project and the five most recent projects. If the
last session crashed with unsaved work, or a project being opened has an
autosave newer than its last save, a **Recover unsaved work?** prompt asks
whether to recover it first. This doc covers the shell side. The autosave
writer, the session marker and templates belong to project-core
([autosave-and-recovery.md](../project-core/autosave-and-recovery.md), [project-templates.md](../project-core/project-templates.md)).

## Behaviour

**Startup screen** (`view/startup.rs::view_startup_overlay`)

- It shows while `!io.has_active_project` and is `Overlay::Startup`, second
  only to Recovery.
- It is a 560 px dialog titled "New session" / "Start a project to begin".
- It has no close button. The backdrop ignores clicks, Esc does nothing,
  and the palette won't open over it.
- **New Project** (`StartNewProject`) opens a Save As file dialog. Saving the
  boot session, which already has a default "Track 1", into the chosen
  folder makes it the active project.
- **Open Project…** opens the OS file dialog.
- **Start from Template… (coming soon)** is a disabled button.
- **RECENT PROJECTS** lists up to 5 entries (`MAX_RECENT_SHOWN`). Each row
  shows the name and parent folder and opens with `OpenRecent(path)`. With
  no entries the list reads "No recent projects".
- The screen goes away once `has_active_project` becomes true. That happens
  after a completed manual save, a project load, or a template instantiate.
- While it is up, `update/gates.rs` swallows project-mutating messages, and
  control mutations fail with "no active project — open or create one
  first".

**Recent projects** (`recent.rs`)

- The list is `<config dir>/resonance/recent.json`: `RecentEntry { path,
  display_name, last_opened_secs }`, newest first, at most 10
  (`MAX_RECENT`), de-duplicated.
- It is updated on a manual save and on a load. Autosave and template
  instantiation don't touch it.
- Missing files are not pruned on load. Clicking an entry whose file is
  gone shows "Project not found: … — removed from recent projects." and
  removes it.
- A corrupt file is quarantined. Writes are atomic.
- Test apps start with an empty list, and their writes go to the hermetic
  temp root.

**Recovery prompt** (`view/recovery_prompt.rs`)

- Title: "Recover unsaved work?".
- The body differs for an untitled session and a saved project. For a saved
  project it says how many minutes the autosave is newer than the last
  save.
- A note says a recovered project opens with unsaved changes.
- Buttons:
  - Cancel. A backdrop click or Esc also cancels.
  - Discard (untitled) or Open last saved (titled).
  - **Recover autosave**.
- It appears in two cases:
  - **On a GUI open**, through `recovery::prompt_before_open`, when the
    project's `.session.lock` marker belongs to a dead process and
    `project.autosave.json` is newer than `project.json`.
  - **At boot**, through `offer_orphaned_session`, for the newest crashed
    *untitled* session found under the autosave scratch root. In this case
    it is drawn on top of the startup screen.
- Liveness of the session marker is checked through `/proc/<pid>` on Linux
  only. On other platforms, a marker from another process counts as dead.

**Unsaved-work signals in the shell**

- The chrome label reads "· not saved", "· unsaved" or "· saved"
  ([window-chrome-and-views.md](window-chrome-and-views.md)).
- After three autosave failures in a row, the status area shows an AUTOSAVE
  line ([overlays-and-dialogs.md](overlays-and-dialogs.md)).
- Open, Open Recent and New over unsaved changes ask Save / Don't Save /
  Cancel first. Quitting asks the same question.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Startup view | `resonance-app/src/view/startup.rs` | `view_startup_overlay`, `MAX_RECENT_SHOWN`, `DIALOG_WIDTH` |
| Recents | `resonance-app/src/recent.rs` | `RecentEntry`, `load`, `add`, `remove`, `insert_pure`, `MAX_RECENT` |
| Recovery view | `resonance-app/src/view/recovery_prompt.rs` | `view_recovery_prompt_overlay` |
| Recovery flow | `resonance-app/src/update/project_io/recovery.rs` | `prompt_before_open`, `offer_orphaned_session`, `handle_choice`, `close_session`, `sync_session_marker` |
| Crash detection | `resonance-app/src/project/session.rs` | `probe`, `RecoveryOffer`, `marker_is_live`, `scan_scratch_root`, `gc_scratch_root` |
| Project I/O messages | `resonance-app/src/update/project_io/mod.rs` | `StartNewProject`, `OpenProject`, `OpenRecent`, `RecoveryChoice`, `request_switch` |
| Gate | `resonance-app/src/update/gates.rs` | `is_gated_message` |

## Surfaces

- **GUI:** the startup screen, recovery prompt and switch/quit confirms.
- **Control API / MCP:**
  - `project.new` (optional `template`), `project.open` (with
    `recover_autosave`), `project.save` and `project.save_as` →
    `mcp__resonance__project_new`, `mcp__resonance__project_open`,
    `mcp__resonance__project_save`, `mcp__resonance__project_save_as`.
  - There is no method for recents or listing templates.
- **Agent skills:** none of the `resonance-agent-plugin/skills/*` skills
  use these. The user-level `resonance-studio` skill, which is outside the
  repo, starts sessions with `project_new`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Recovery prompt, its answers, control `recover_autosave`, the startup offer | `resonance-app/tests/io/autosave_recovery.rs` | `./scripts/run-tests.py -p resonance-app` |
| Session marker rules and the quit flow | `resonance-app/tests/io/crash_detection.rs` | same |
| Switch confirm, untitled anchor | `resonance-app/tests/io/project_lifecycle_ux.rs` | same |
| Recent-entry write location; startup default track | `resonance-app/tests/io/hermetic_user_state.rs`, `io/startup_default_track.rs` | same |
| Recents dedupe and truncate | inline `#[cfg(test)]` in `resonance-app/src/recent.rs` (the documented exception in ARCHITECTURE.md) | same |
| Startup is a modal Esc can't close | `resonance-app/tests/performance/shortcut_dispatch.rs` | same |
| Goldens | `recovery_prompt_saved_project`, `recovery_prompt_untitled_at_startup` (prompt over the startup screen), `project_switch_confirm`, `untitled_chrome_not_saved` | `RESONANCE_BLESS=1 …` |

Not covered:

- There is no golden of the startup screen on its own, and no test of its
  rows.
- There is no test for loading recents, quarantining a corrupt file, or
  removing a missing entry on click.

## Decisions and deviations

- New Project from the startup screen asks for a folder first (Save As).
  New Project with a project already open makes an untitled project
  instead. The control API's `project.new` also makes an untitled project.
- `recent.rs`'s comment says `resonance-app` is "a binary crate with no
  `lib.rs`". It has had a `lib.rs` for a while. The inline tests remain
  because they exercise private helpers.

## Known gaps

- Template picking and a Save-as-Template dialog have no GUI. Only the
  disabled "coming soon" button and the palette's fixed-default "Save as
  Template" exist
  ([intent](../../intents/ui-shell/template-picker-and-save-dialog.md)).
- Recent rows don't show when a project was last opened, although
  `last_opened_secs` is stored.
- Liveness of the session marker is checked on Linux only (FU-R1a).
