# Template picker, Save-as-Template dialog and template management

> Stage: **intent**
> Status: **partly built**
> Area: ui-shell
> Touches: startup screen, dialog / project-core
> ([project-templates.md](../../features/project-core/project-templates.md))
> Source: `docs/intents/prototypes/project-templates-starter-sessions/index.html`; `view/startup.rs` "Start from Template… (coming soon)"; code check at `8eac71b3`.

## Why

The template engine is built:

- four built-in templates (Vocal songwriting, Band recording, Beatmaking,
  Empty);
- user templates saved to `<config>/resonance/templates/`;
- stale-template detection;
- instantiation as untitled that never touches the source.

A GUI user can reach almost none of it:

- The startup screen's "Start from Template…" is disabled. The only GUI way
  to use a template is `Ctrl+N`, which always gives the Empty one.
- "Save as Template" exists only as a palette command, with fixed defaults:
  the template is named after the project folder, no description, both
  capture toggles forced on.
- Saving gives no feedback when it succeeds.
- A user template cannot be renamed or deleted from the app.
- Only an agent can start a project from a template, through `project.new
  { template }` — and only if it already knows the exact name; there is no
  way to list templates.
- A captured template does not carry its audio clip WAVs, so a template
  with audio opens with those clips missing.

## What it would do

- **Startup screen.** It gets Recent and Templates tabs, as in the
  prototype. The Templates tab shows the built-ins as cards, with a summary
  of track, bus and plugin counts, BPM and meter. "Your templates" follow,
  with stale ones greyed, disabled and showing the reason. Picking a card
  starts an untitled project from it. Empty, loading and error states as in
  the prototype.
- **New from template.** The same picker is available from File > New with
  a project open, through the project-switch confirm.
- **Save as Template… dialog.** It has a name, a description, a summary of
  what will be captured, and the two include toggles (markers & tempo,
  master chain). On success a short confirmation shows.
- **Per-template actions.** Rename (rewrites `template.json`), Reveal in
  file manager, and Delete.
- **Control list.** A control method lists templates (slug or name, kind,
  description, summary, stale reason) so `project.new {template}` can be
  used without guessing.
- **Audio.** Capture copies the audio clip WAVs the template names.
- **Recent rows** show "2h ago" from `last_opened_secs`.

## What exists to build on

- `resonance-app/src/update/project_io/templates.rs`:
  - `scan_user_templates`, `scan_templates_in`, `TemplateEntry::{Valid,
    Stale}`, `StaleReason`;
  - `compute_summary`, `TemplateSummary`, `TemplateCaptureOptions`,
    `write_template`;
  - `BuiltinTemplateId`, `builtin_templates`.
- `update/project_io/instantiate.rs`: `instantiate_builtin`,
  `load_user_template_task`, `begin_instantiate` (lands untitled).
- `ProjectIoMessage::SaveAsTemplate { name, description,
  include_markers_and_tempo, include_master_chain }` already carries
  everything a dialog would collect.
- The control side: `update/control/project.rs::resolve_template` behind
  `project.new`.
- `view/startup.rs` and `view/confirm_quit.rs` (the switch confirm).
- Tests: `resonance-app/tests/compose/{builtin_templates,templates_save,templates_scan,template_instantiate}.rs`.

## Open decisions

1. Should templates capture MIDI notes? The prototype says "structure — not
   its audio or notes", but the code captures notes. **Recommend keeping
   notes, with a third toggle "include notes"** (default off for the
   dialog).
2. Delete: permanent, the 30-day trash the preset library uses, or the OS
   trash? The two source notes disagreed (preset trash for consistency vs
   OS trash because it is user content). **Recommend a trash, not a hard
   delete**; which one is open.
3. Control surface. Options: `templates.list` + `templates.save`, or a
   read-only `project.templates` list (no remote capture or delete).
   **Recommend the list first, read-only**; add `templates.save` only if an
   agent workflow needs it.
4. Audio in templates. **Recommend** copying referenced `clip_<id>.wav` and
   `asset_<id>.wav` into the template folder at capture time.

## Slices

1. Startup Templates tab with the built-ins: pick → instantiate; also from
   File > New. Add goldens of the startup screen (populated, empty, stale),
   which has none today.
2. User templates and stale cards in the same tab, plus the control list
   method → MCP tool + lockstep test update; test that a listed name
   round-trips through `project.new`.
3. The Save-as-Template dialog (replacing the palette command's fixed
   defaults), with a success notice; test that the toggles reach
   `write_template`.
4. Rename, reveal and delete; tests on a temp templates root
   (`scan_templates_in`).
5. Audio: capture copies clip/asset WAVs; test that a template with an
   audio clip instantiates with the clip playable (no relink prompt).
6. Relative times on recent rows.

## How we'd know it works

- Goldens of the startup screen in both tabs and each picker state, and of
  the dialog, under `resonance-app/tests/snapshots/`.
- Reducer tests: picking a card produces an untitled project with the
  template's tracks.
- A dialog round trip writes `template.json` with the chosen options.
- `tests/compose/templates_*.rs` extended for rename, delete and audio.
- A control test listing templates and opening one by the listed name.
- A live check: capture a project with recorded audio as a template, start
  a new project from it, and hear the audio.
