# Project templates

> Verified against master @ `f82be9b0` + the TIGER Mystic default on 2026-10-13.
> Spec history: ba impl-plan doc #197 / design #192, todos #665/#666 (not in
> the repo); prototype `docs/intents/prototypes/project-templates-starter-sessions/index.html`.

A template is a project used as a starting point. Four built-in starters are
defined in code, and the user can capture the open project as their own
template. Starting from either gives an **untitled** project, so the
template itself is never overwritten.

## Behaviour

### Built-in starters

| Slug | Name | Content |
|---|---|---|
| `vocal-songwriting` | Vocal songwriting | chord progression, a Compose vocal line on the TIGER voicebank (Mystic, a tenor line in TIGER's comfortable C3–A4), pad/bass bed |
| `band-recording` | Band recording | six mic/DI audio tracks routed to drum and instrument busses, master chain |
| `beatmaking` | Beatmaking | drum sampler and wavetable synth with reverb and delay return busses |
| `empty` | Empty | blank project at the default tempo and meter |

- Built as `ProjectFile` values in memory (`BuiltinTemplateId::build`), so
  they always match the current format and never go stale.
- They name first-party plugins by CLAP id only (from
  `resonance_plugin::first_party`); the `.clap` path is filled in from the
  runtime plugin scan, and plugins open at their own defaults.
- `empty` is what `Ctrl+N` / New Project instantiates.

### User templates

- Stored under `<config>/resonance/templates/<slug>/` in the normal project
  shape (`project.json`, `midi/`, `plugins/`) plus a `template.json` sidecar
  (`name`, `description`, `built_in`, `schema_version`, summary chips,
  `created_secs`). The folder name is a slug of the name; a clash appends
  `-2`, `-3`, …
- **Capture** ("Save as Template" command) serializes the current state
  synchronously with `build_project_file` and the app's cached plugin blobs
  (no engine round-trip, so a missing plugin's blob is kept). Two toggles
  exist in the model: carry the tempo/signature map, carry the master chain.
  The command currently always passes the project's folder name (or
  "Untitled"), an empty description, and both toggles on.
- **Scan** classifies each folder: valid, or stale (template.json
  unparsable, `schema_version` newer than this build, project.json
  unparsable). A folder without `template.json` is skipped. Stale entries are
  left on disk.
- Capture writes only `project.json`, MIDI and plugin files; audio clip WAVs
  are **not** copied into the template folder.

### Instantiation

`begin_instantiate` stops the transport, clears undo history and plugin
mirrors, sets the path to `None`, sends `ClearAll` and replays the template
through the normal load path (`all_cleared` → `replay_loaded_project`). The
result is untitled, gets the session scratch dir as its anchor, is not added
to recent projects, and its first Save becomes Save As.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model, scan, capture, built-ins | `resonance-app/src/update/project_io/templates.rs` | `BuiltinTemplateId`, `builtin_templates`, `Template`, `TemplateMetadata`, `TemplateEntry`, `StaleReason`, `scan_user_templates`, `scan_templates_in`, `write_template`, `TemplateCaptureOptions`, `templates_dir` |
| Instantiate | `resonance-app/src/update/project_io/instantiate.rs` | `instantiate_builtin`, `load_user_template_task`, `begin_instantiate` |
| Capture entry point | `resonance-app/src/update/project_io/mod.rs` | `ProjectIoMessage::SaveAsTemplate`, `save_current_as_template_in`, `ProjectIoMessage::TemplateLoaded` |
| Command | `resonance-app/src/commands/{mod.rs,resolve.rs}` | `CommandId::SaveAsTemplate` |
| Control | `resonance-app/src/update/control/project.rs` | `resolve_template` |

## Surfaces

- **GUI:** command palette "Save as Template"; `Ctrl+N` (Empty only). The
  startup screen shows "Start from Template… (coming soon)", disabled.
- **Control API / MCP:** `project.new {template}` →
  `mcp__resonance__project_new`. `template` is a built-in slug, else a valid
  user template's exact name; omitted means `empty`. `template_id` is an
  alias. There is no method to list templates or to save one.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Built-in content (plugin ids, routing) | `resonance-app/tests/compose/builtin_templates.rs` | `./scripts/run-tests.py -p resonance-app` |
| Instantiate lands untitled | `resonance-app/tests/compose/template_instantiate.rs` | same |
| Capture + toggles + slugs | `resonance-app/tests/compose/templates_save.rs` | same |
| Scan / stale classification | `resonance-app/tests/compose/templates_scan.rs` | same |
| `project.new {template}` | `resonance-app/tests/control/control_project.rs` | same |

Not covered: a template with audio clips (the WAV gap above), and any
template-picker UI (none exists).

## Decisions and deviations

- Built-ins are code, not shipped files, so they cannot drift from the
  format and need no install step.
- The prototype's template picker (tabs, cards with summary chips, per-card
  rename/delete menu, empty/loading/error states) and its Save-as-Template
  dialog were not built; only the data layer and the control path were.

## Known gaps

- No GUI picker, no rename/delete of user templates, no capture dialog, and
  no control method to list templates. See
  [template-picker](../../intents/ui-shell/template-picker-and-save-dialog.md).
- A captured template that holds audio clips opens with those clips missing.
