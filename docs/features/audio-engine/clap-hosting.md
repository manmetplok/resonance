# CLAP plugin hosting

> Verified against master @ `5d6a2993` on 2026-10-15.
> Spec history: `docs/history/reviews/plugin-audit.md`,
> `docs/history/reviews/plugin-audit-plan.md` (doc #275),
> `docs/history/specs/realtime-multithreading.md` §4.6 / §11.2,
> `docs/intents/prototypes/third-party-plugin-hosting/`.

The engine loads CLAP plugins (first-party and third-party), runs them in
track, sub-track, bus and master insert chains, and opens their editors. This
page covers finding, loading, running and removing plugins, the threading
contract, and bypass. Parameters and presets are in
[clap-params-and-presets.md](clap-params-and-presets.md); latency in
[latency-compensation.md](latency-compensation.md). The first-party plugins
themselves are in [../plugins-effects](../plugins-effects/) and
[../plugins-instruments](../plugins-instruments/).

## Behaviour

### Scanning

- Directories: `~/.clap` and `/usr/lib/clap` (Linux),
  `~/Library/Audio/Plug-Ins/CLAP` and `/Library/Audio/Plug-Ins/CLAP` (macOS),
  every `$CLAP_PATH` entry, and `target/bundled/` next to the executable and
  in the working directory. `.clap` files or bundle directories, searched
  recursively.
- **Startup scan** (`ScanPlugins`, sent once by the app): drops every
  instance and loads every bundle. **Live rescan** (`RescanPlugins`, Settings
  button and `plugins.rescan`): additive only; bundles already loaded stay
  loaded and only new files are opened. An uninstalled plugin stays in the
  catalog until restart.
- Answers: `PluginScanFailed { failures }` (bundles that would not load,
  plus duplicate-id warnings, e.g. a dev `target/bundled` build shadowing an
  installed one) then `PluginsScanned { plugins }`.
- After a scan, one discovery worker indexes presets
  (see [clap-params-and-presets.md](clap-params-and-presets.md)).

### Loading an instance (`AddPlugin { owner, clap_file_path, clap_plugin_id, id }`)

`chain::handle_add_plugin`, shared by every `ChainOwner` (track, bus,
master):

1. Refuse a live `id` (`Internal` error, never renumber).
2. `ensure_bundle` (load the `.clap` if not yet resident) and
   `resolve_plugin_id`. Failure → `PluginLoadFailed { instance_id, reason }`.
3. `ClapBundle::create_instance` at the engine rate: init, query extensions,
   activate, read latency, start processing.
4. Insert an `Arc<PluginSlot>` into the plugin map, append to the owner's
   chain, publish, and send `PluginAdded` with params, `has_gui`,
   `has_sidechain_input`, output port count and names (the app creates one
   sub-track per extra output port on a track chain).

`RemovePlugin` always answers `PluginRemoved` (also for unknown ids) and
drops the instance's sidechain route. `MovePlugin` answers `PluginMoved` with
the clamped index. "Replace" is a control-layer composition of remove + add.

### Running

- `process.rs` builds the input event list (param changes and notes, sorted
  by time), binds one buffer per **declared** audio port with its declared
  channel count (`PortBuffers`; unconnected inputs read silence, unwanted
  outputs write scratch), latches transport, calls `process()`, and zeroes
  any non-finite output sample.
- Note dialect is chosen once at activation from the first note input port
  (`note_ports.rs`, `NoteDialect`): CLAP note events, or MIDI 1.0 when the
  port prefers or only takes MIDI. Controllers, pitch bend and aftertouch
  are forwarded only to ports that accept MIDI. A plugin without
  `clap.note-ports` gets CLAP notes only.
- The sidechain key, when routed, is the extra input port.
- Every instance sits behind its own mutex. The live render `try_lock`s it
  and skips the plugin for a block on a miss; offline renders lock with
  back-off; engine-thread handlers `try_lock` and re-enqueue.

### Threading contract

- **Main thread:** the engine thread for life (`mark_main_thread`), the
  thread that creates an instance, and an offline render's worker between
  chunks.
- **Audio thread:** render-pool workers for life; the live callback and each
  offline chunk for the render (`AudioThreadScope`); and the host's own
  audio-thread calls (`start_processing`, `stop_processing`, `reset`,
  `params.flush` on an active plugin, `process`). u-he Hive aborts the
  process if `start_processing` is reported on the main thread.
- `is_main_thread()` is false on threads the host never marked.

### Host extensions served

`clap.latency` (`changed()` → engine restarts the instance at a safe point),
`clap.gui` (`closed()` is honoured; resize, show and hide requests are
refused or ignored because every editor is floating), `clap.thread-check`,
`clap.thread-pool` ([render-pool.md](render-pool.md)), `clap.preset-load`
(both ids), `clap.params` (`rescan`, `request_flush`). `request_restart()` is
honoured the same way as a latency change. The engine polls these flags at
most every 12 ms (`poll_plugin_host_requests`).

### Offline mode

Every offline render calls `render.set(OFFLINE)` on each plugin first and
back to realtime on every exit path (`OfflineRenderMode`). After an offline
render, a plugin that fails to restart is reported once as dead
(`plugins_dead_after_reset`).

### Bypass

Whole-chain (`SetFxBypass`) and per-slot (`SetPluginBypass`) bypass are
`BypassFade`s: target and fade position in one atomic, crossfading wet
against the chain's own input over `BYPASS_FADE_MS`. A plugin with its own
bypass parameter is bypassed through that parameter (it keeps running and
keeps its latency); otherwise the host stops calling it ("host bypass") and
its latency drops out of PDC. The host pushes its bypass into the plugin's
own parameter only when the host's bypass *changes*
(`PluginSlot::sync_own_bypass`), so a plugin whose loaded state is
self-bypassed (Mastering bypassed from its own header) stays bypassed. While nothing renders, the change lands
immediately (`apply_bypass_request`), which is what makes project load
restore bypass states exactly.

### Editors

`OpenPluginEditor` / `ClosePluginEditor` drive `is_api_supported → create →
show` (and `hide → destroy`) as a **floating** window, Wayland on Linux and
Cocoa on macOS; embedding is not implemented. There is no size negotiation:
`get_size`/`set_size` are `[main-thread & !floating]` in `gui.h`, so the
plugin sizes its own window (`clap_host/gui.rs`, code review HOST-15). The
host talks to plugins through raw `clap-sys`, not clack-host, so a clack
upgrade changes only the first-party plugins' side of the ABI. `PluginEditorState { open, failure }` is
the only truth about whether an editor is open, including a window the user
closed from its titlebar. The window runtimes are in
[plugin-editor-runtimes.md](../plugins-instruments/plugin-editor-runtimes.md) / [plugin-windows.md](../ui-shell/plugin-windows.md).

### Unloading

A `ClapBundle` never calls `deinit()` and never `dlclose`s: a loaded binary
stays resident for the process. Instances are destroyed on the engine thread
by the retire sweep; on shutdown the engine releases all plugins with a 2 s
budget.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Scan | `resonance-audio/src/engine/scan.rs` | `scan_plugins`, `rescan_plugins`, `scan_dirs`, `duplicate_id_warnings`, `spawn_discovery` |
| Bundles | `resonance-audio/src/clap_host/bundle.rs` | `ClapBundle`, `create_instance`, `build_instance`, `bundle_binary_path` |
| Host vtable, slots | `resonance-audio/src/clap_host/mod.rs` | `PluginSlot`, `SyncClapInstance`, host `get_extension`, `serial_only_plugin` |
| Instance | `resonance-audio/src/clap_host/instance.rs`, `instance/{ports,note_ports}.rs` | `ClapInstance`, `PortBuffers` |
| Process | `resonance-audio/src/clap_host/process.rs` | `process_multi_with_key`, `scrub_non_finite` |
| Thread roles | `resonance-audio/src/clap_host/thread_check.rs`, `thread_pool.rs` | `mark_main_thread`, `mark_audio_thread`, `AudioThreadScope` |
| State / restart | `resonance-audio/src/clap_host/state.rs` | `restart`, `reset_processing` |
| Editor | `resonance-audio/src/clap_host/gui.rs` | `PluginEditorFailure` mapping |
| Chain handlers | `resonance-audio/src/engine/chain.rs` | `handle_add_plugin`, `handle_remove_plugin`, `handle_move_plugin`, `handle_set_fx_bypass` |
| Instance handlers | `resonance-audio/src/engine/plugins.rs` | `ensure_bundle`, `poll_plugin_host_requests`, `service_host_restart_request`, `handle_open_plugin_editor`, `apply_bypass_request`, `release_all_plugins` |
| Bypass | `resonance-audio/src/bypass.rs` | `BypassFade`, `FxDryScratch`, `BYPASS_FADE_MS` |
| Offline mode | `resonance-audio/src/engine/bounce/render.rs` | `OfflineRenderMode` |

## Surfaces

- **GUI:** plugin picker and chain rows in the mixer inspector; "Open
  editor"; Settings → rescan button. See [plugin-chains.md](../mixer/plugin-chains.md) and [third-party-plugin-hosting.md](../plugins-instruments/third-party-plugin-hosting.md).
- **Control API / MCP:** `plugins.catalog` → `mcp__resonance__plugins_catalog`;
  `plugins.rescan` → `mcp__resonance__plugins_rescan`;
  `track.add_instrument`, `track|bus|master.add_effect`, `.remove_effect`,
  `.move_effect`, `.replace_effect`, `.set_fx_bypass`, `.set_plugin_bypass`
  → `mcp__resonance__track_add_instrument`, `…_add_effect`,
  `…_remove_effect`, `…_move_effect`, `…_replace_effect`,
  `…_set_fx_bypass`, `…_set_plugin_bypass` (track_, bus_, master_ prefixes).
- **Agent skills:** `arranging` (catalog), `mixing`, `mastering`, `spatial`,
  `drumming` (chains and params).

## How it's tested

| What | Where | Run |
|---|---|---|
| Real-ABI host behaviour (32 modules) | `resonance-audio/tests/clap_host/*.rs`, e.g. `clap_thread_roles.rs`, `clap_port_layout.rs`, `clap_note_dialect.rs`, `clap_note_event_order.rs`, `clap_all_notes_off.rs`, `clap_ffi_hardening.rs`, `clap_plugin_drop_order.rs`, `plugin_output_scrub.rs`, `plugin_bypass.rs`, `own_bypass_latency.rs`, `plugin_load_failure.rs`, `plugin_rescan.rs`, `plugin_editor_state.rs`, `plugin_id_duplicate_rejected.rs`, `sub_track_plugin_removal.rs`, `clap_bundle_path.rs`, `plugin_binaries.rs` | `./scripts/run-tests.py -p resonance-audio` (builds the plugin cdylibs) |
| Chain handlers | `tests/engine/track_plugin_chain.rs`, `track_plugin_move.rs`, `bus_plugin_move.rs`, `master_plugin_move.rs` | same |
| Offline render mode | `tests/bounce/offline_render_mode.rs` | same |
| App side | `resonance-app/tests/plugins/missing_plugin_slot.rs`, `settings_plugin_rescan_button.rs`; `tests/control/control_plugins_catalog.rs`, `control_plugins_rescan.rs`, `control_replace_effect.rs`, `control_plugin_bypass.rs` | `./scripts/run-tests.py -p resonance-app` |
| Editor windows (ignored; need a Wayland or macOS session) | `cargo test -p resonance-gate --test editor_open -- --ignored` and the others in CLAUDE.md | by hand |

Under plain `cargo test` the `clap_host` tests need the plugin binaries
built, or `RESONANCE_ALLOW_MISSING_PLUGIN_BINARIES=1`.

Not covered: third-party plugins (tests use first-party bundles and
hand-rolled fake plugins); the macOS bundle loading path beyond the path
resolution unit test.

## Decisions and deviations

- Binaries are never unloaded (a DPF plugin's `deinit` corrupted the heap;
  unloading is unsound in general).
- Editors are floating only, so `gui.set_parent` is never called.
- The host's own audio-thread calls take the audio role (found when Hive
  aborted).
- Stale doc comment: `AudioEvent::PluginLoadFailed` still says
  `instance_id` is `None` for an engine-allocated add; since ARCH-04 D-1
  every add carries an id, so the handlers always send `Some`.

## Known gaps

- Host extensions not served: `clap.log`, `clap.timer-support`,
  `clap.posix-fd-support`, `clap.state` (`mark_dirty`), `clap.audio-ports`
  host side, `clap.note-name`, `clap.voice-info`, `clap.tail`. Plugins that
  rely on timer or fd support for their GUI (common for X11 editors) cannot
  run their editor here; there is no X11 or embedded editor path either.
- A live rescan cannot remove a plugin.
