# Restore from a versioned backup

> Status: **partly built**
> Area: project-core (`docs/features/project-core/save-open-new.md`)
> Source: backup subsystem (ba todo #462, code review STATE-12);
> `list_backups` exists "for the restore UI" but has no caller. Checked open
> in code on 2026-10-07.

## Why

Every manual save writes `backups/project-<timestamp>.json` and keeps the
last 10, with hard-linked MIDI and plugin files so each backup holds its own
versions. But nothing in the app lets a user (or an agent) go back to one:

- `project::list_backups` is exported and tested, and nothing calls it.
- The Open dialog picks `.rproj` folders only, so a backup JSON cannot be
  chosen.
- `load_project` can read a backup JSON (it resolves paths against the
  bundle root), and `project.open` accepts any existing absolute path. But
  the open adopts the path it was given as the project path, so opening
  `…/backups/project-<ts>.json` makes the JSON file the "project folder" and
  the next save tries to write inside it. This path is untested.

## What it would do

- A "Restore backup…" action on an open, saved project lists its backups
  newest first (timestamp, maybe track count) and opens the chosen one.
- The restored state opens **into the same bundle**, dirty, with the bundle
  as project path — like a recovered autosave — so a normal Save makes it
  the current version and a further backup is written of what it replaced.
- An agent can list and restore backups over the control API.

## What exists to build on

- `resonance-app/src/project/io.rs`: `write_backup`, `list_backups`,
  `scan_backups`, `BackupEntry`, `load_project` (already backup-aware).
- `resonance-app/src/update/project_io/recovery.rs`: the recovered-autosave
  flow (`start_load` with a separate target file and open dir, `finish_load`
  marking the project dirty) is the same shape a backup restore needs.
- `AutosaveSettings::backup_retention` and its Settings UI.
- `resonance-app/tests/io/project_backups.rs` for the helpers.

## Open decisions

1. Over unsaved changes. Recommended: the same Save / Don't save / Cancel
   dialog an Open raises.
2. Undo across a restore. Recommended: a restore is a load — it clears the
   history, like Open.
3. Control shape. Recommended: `project.backups` (list) and
   `project.open {path: <bundle>, backup: <timestamp>}` rather than passing
   the backup file path, so the open dir is never the JSON file.

## Slices

1. **Fix the open target:** opening a backup (by bundle + timestamp) loads
   the backup file but adopts the bundle dir; test that a following save
   writes `project.json` in the bundle.
2. **GUI:** "Restore backup…" command and list modal; golden snapshot.
3. **Control:** list + restore methods and MCP tools; lockstep test.

## How we'd know it works

- An io test: save three times with different content, restore the oldest,
  check the replayed state matches it, save, and find the newest backup is
  the pre-restore version.
- The backup's own MIDI/plugin files (not the bundle's current ones) are what
  load.
