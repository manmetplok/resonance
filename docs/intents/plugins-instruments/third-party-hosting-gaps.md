# Third-party plugin hosting: catalog UI, scan control and isolation

> Stage: **intent**
> Status: **partly built**
> Area: plugins-instruments
> Touches: UI with audio-engine (scan, isolation)
> Source: `docs/intents/prototypes/third-party-plugin-hosting/` (epic #45);
> checked against `resonance-audio/src/engine/scan.rs`,
> `resonance-app/src/view/settings.rs`.
> Register items covered: plugin audit X5 (GUI half; the MCP half is in control-api/agent-readouts.md).

## Why

Hosting works, and missing plugins are handled well, but users cannot see
or steer what was scanned: Settings shows counts and failures, nothing else.
Users cannot add a plugin folder without setting `$CLAP_PATH` before
launch, cannot see which plugins are third-party, cannot cancel or follow
a slow scan, and do not see a plugin's latency. A crashing or hanging
third-party plugin takes the whole app down, at scan time or while
playing. On Linux, plugins with only an X11 editor show no editor.

## What it would do

- Settings → Plugins lists scan folders (built-in ones locked) with
  "Add folder…" and remove, persisted in settings.
- A catalog table (name, vendor, type, source, status) with search, type
  filter and a "failed only" filter.
- Scan progress with the current file and a cancel button.
- The add picker shows source and vendor facets and a "3P" badge.
- The plugin window shows reported latency ("512 smp · compensated").
- Isolation: scan each new bundle in a child process with a timeout before
  loading it in-process, and keep a blocklist of bundles that crashed or
  timed out.
- Optional: X11 editor support through XWayland.

## What exists to build on

- `ScannedPlugin` already carries vendor and path
  (`resonance-audio/src/engine/scan.rs`); `plugins.catalog` returns id,
  name, kind and `scan_failures`.
- Additive rescan (`rescan_plugins`), failure reporting, missing-plugin
  slots (`docs/features/plugins-instruments/third-party-plugin-hosting.md`).
- Latency is tracked per instance (`resonance-audio/tests/clap_host/clap_latency_tracking.rs`).

## Open decisions

1. In-process vs out-of-process scanning. Recommended: out-of-process probe
   for new or changed bundles only, cached by path + mtime, so start-up
   stays fast and a bad bundle is skipped next time.
2. Where user folders are stored. Recommended: app settings, merged with
   `$CLAP_PATH`.
3. Whether to add a catalog cache at all. Recommended: only together with
   the out-of-process probe.

## Slices

1. Catalog table + vendor/source in `plugins.catalog`.
2. User scan folders (setting + UI + control method).
3. Scan progress and cancel.
4. Latency readout in the plugin window.
5. Out-of-process probe, timeout and blocklist.

## How we'd know it works

App tests next to `resonance-app/tests/plugins/settings_plugin_rescan_button.rs`
and `control/control_plugins_rescan.rs`; a fixture bundle that hangs or
aborts in `clap_entry.init` is skipped and listed as failed while the app
keeps running.
