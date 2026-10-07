# Project core

The app's data spine in `resonance-app`: the `ProjectFile` model and its
`.rproj` bundle, save / open / new and the autosave, crash-recovery and
backup machinery around them, templates, undo/redo, the `Reconcile` driver
that drives app and engine to a project after a load or an undo, the rules
for who allocates entity ids, the `Resonance` struct and its iced
update loop with engine-event dispatch, the hermetic test app, and the
`arch-invariants` test that keeps the workspace's structure honest. The
layering rules themselves live in [`ARCHITECTURE.md`](../../../ARCHITECTURE.md).
Open work for this area is in [`docs/intents/project-core/`](../../intents/project-core/).

| Feature | Doc | One-line summary |
|---|---|---|
| Project file format | [project-file-format.md](project-file-format.md) | `.rproj` bundle layout, `ProjectFile` contents, version 2 and additive growth |
| Save, open, new and quit | [save-open-new.md](save-open-new.md) | async engine-collected save, Save As, versioned backups, open/new over unsaved changes, quit, `project.*` jobs |
| Autosave and crash recovery | [autosave-and-recovery.md](autosave-and-recovery.md) | change-gated 30 s autosave to a side file, session marker, recovery prompt and `recover_autosave` |
| Clip storage and GC | [clip-storage-and-gc.md](clip-storage-and-gc.md) | one write-once `clip_<id>.wav` per clip, persisted before undo, reaped after a manual save |
| Project templates | [project-templates.md](project-templates.md) | four built-in starters in code, user templates on disk, instantiate untitled; no picker UI yet |
| Undo and redo | [undo-redo.md](undo-redo.md) | whole-project snapshots, one entry per action / gesture / control call, shared with agents via `edit.*` |
| Reconcile | [reconcile.md](reconcile.md) | one 36-domain table restores a project after a disk load (from empty) or an undo (by diff) |
| Entity ids | [entity-ids.md](entity-ids.md) | app-side allocators per id space, never-reset clip counter, clip-id grant for engine-created clips |
| App state | [app-state.md](app-state.md) | the 40-field `Resonance` struct and its domain sub-states |
| Update loop | [update-loop.md](update-loop.md) | `Message` domains, gates → undo → dispatch, the tick, exhaustive engine-event routing, control requests |
| Hermetic test app | [hermetic-test-app.md](hermetic-test-app.md) | `new_for_test*` constructors with a capturing engine and temp-rooted user dirs |
| Architecture invariants | [architecture-invariants.md](architecture-invariants.md) | 31 grep/metadata tests for crate layering, test layout, logging, locking and app boundaries |
