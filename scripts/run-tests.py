#!/usr/bin/env python3
"""Run the workspace test suite in parallel.

`cargo test` walks its test targets **serially** — it parallelises the tests
inside one binary, but runs the binaries one after another. This workspace has
~370 of them, so that walk is most of the wall clock: at ~620 it was 237 s
serially against 67 s at `-j8` for the identical set (ba doc #285 §3).

So this script does what cargo won't: build the test binaries with cargo, then
run them concurrently. Two details matter and are the reason this is a script
rather than a shell one-liner:

* **Working directory.** A test binary must run with its own crate root as the
  cwd, because the golden-image tests name their PNGs crate-relatively
  (`tests/snapshots/...`). Launch them from the workspace root instead and 52
  app tests fail on missing goldens — a failure that looks like a regression and
  is not one.
* **Failure reporting.** A parallel run interleaves output, so each binary's
  output is captured and only failures are printed, whole, at the end.
* **Non-test build.** Test builds turn on dev-dependency features (e.g.
  resonance-audio's `test-internals`), so code that only compiles with them
  passes the suite while a plain `cargo build` is broken. A `cargo check` of
  the same crates without test targets runs first to catch that.
* **Plugin binaries.** Some `resonance-audio` tests load a real first-party
  plugin (`plugin_binary("resonance-eq")` in `tests/clap_host/`). A test
  build compiles the plugin crates but never leaves their cdylibs where a
  test can find them (`target/debug/lib<crate>.so` is only written for a
  crate cargo *builds*), so those plugins are `cargo build`-ed too. The
  list is read from the tests themselves, and a test whose binary is
  missing fails rather than skipping (unless
  RESONANCE_ALLOW_MISSING_PLUGIN_BINARIES=1).

Usage:
    scripts/run-tests.py [-jN] [-p CRATE]... [--no-build] [--no-check] [--] [libtest args]

    -jN            concurrent binaries (default: half the cores, since each
                   binary parallelises its own tests across threads too)
    -p CRATE       restrict to a crate; repeatable. Default: whole workspace.
    --no-build     skip the cargo build step and reuse what is on disk
    --no-check     skip the plain (non-test) `cargo check`
    trailing args  passed through to every test binary (e.g. --nocapture)

Exit status is non-zero if any binary failed.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import glob
import json
import os
import re
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def build_and_enumerate(crates: list[str], build: bool) -> list[tuple[str, str]]:
    """Return [(executable, crate_root)] for every test binary in scope.

    `cargo test --no-run --message-format=json` reports one `compiler-artifact`
    per built target; the ones with `profile.test` set and an `executable` are
    the test binaries. `manifest_path` gives us the crate root to run them from.
    """
    cmd = ["cargo", "test", "--no-run", "--message-format=json"]
    if crates:
        for c in crates:
            cmd += ["-p", c]
    else:
        cmd.append("--workspace")

    started = time.monotonic()
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        # cargo puts diagnostics on stderr; the json stream is on stdout.
        sys.stderr.write(proc.stderr)
        sys.exit(f"build failed ({proc.returncode})")
    if build:
        print(f"built in {time.monotonic() - started:.0f}s")

    found: list[tuple[str, str]] = []
    for line in proc.stdout.splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-artifact":
            continue
        exe = msg.get("executable")
        if not exe or not msg.get("profile", {}).get("test"):
            continue
        crate_root = os.path.dirname(msg["target"]["src_path"])
        # tests/foo.rs -> crate root is the parent of tests/
        manifest = msg.get("manifest_path")
        if manifest:
            crate_root = os.path.dirname(manifest)
        found.append((exe, crate_root))
    return found


def check_non_test(crates: list[str]) -> None:
    """`cargo check` the crates in scope without test targets or dev features."""
    cmd = ["cargo", "check"]
    if crates:
        for c in crates:
            cmd += ["-p", c]
    else:
        cmd.append("--workspace")
    started = time.monotonic()
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        sys.exit(f"non-test build (cargo check) failed ({proc.returncode})")
    print(f"non-test check passed in {time.monotonic() - started:.0f}s")


def plugin_cdylibs() -> list[str]:
    """The plugin crates the resonance-audio tests load by binary.

    Derived from the tests rather than kept as a list here, so a new test
    cannot name a plugin this script forgets to build: every
    `plugin_binary("<crate>")` call, plus — in a file that calls
    `plugin_binary` at all — any string literal naming a crate under
    `plugins/`, since a helper may take the crate name and pass it on
    (`harness_with("resonance-drums", ...)`).
    """
    found: set[str] = set()
    direct = re.compile(r'plugin_binary\("([a-z0-9_-]+)"\)')
    literal = re.compile(r'"([a-z0-9_-]+)"')
    plugin_crates = {
        os.path.basename(os.path.dirname(c))
        for c in glob.glob(os.path.join(ROOT, "plugins/*/Cargo.toml"))
    }
    for path in glob.glob(os.path.join(ROOT, "resonance-audio/tests/**/*.rs"), recursive=True):
        with open(path, encoding="utf-8") as f:
            text = f.read()
        found.update(direct.findall(text))
        if "plugin_binary(" in text:
            found.update(n for n in literal.findall(text) if n in plugin_crates)
    return sorted(found)


def build_plugin_cdylibs(crates: list[str]) -> None:
    """`cargo build` the plugins the tests in scope load (see module docs)."""
    if crates and "resonance-audio" not in crates:
        return
    plugins = plugin_cdylibs()
    if not plugins:
        return
    cmd = ["cargo", "build"]
    for p in plugins:
        cmd += ["-p", p]
    started = time.monotonic()
    proc = subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        sys.exit(f"plugin build failed ({proc.returncode})")
    print(f"built {len(plugins)} plugin cdylibs in {time.monotonic() - started:.0f}s")


# Plugin presets resolve a loaded identity against the user preset
# directory, and the first look at a directory converts legacy files in
# it. Point every test at a private, empty root so a run never reads or
# rewrites the real ~/.local/share/resonance/plugin-presets. Tests that
# need presets set their own root; an explicit override is respected.
PRESET_ROOT = os.path.join(
    tempfile.gettempdir(), f"resonance-test-plugin-presets-{os.getpid()}"
)

# The same for the NAM model library and the shared library marks store
# (favourites/tags): a test that goes through a plugin binary it cannot hand
# a library to (the clap_host tests, the app's plugin instances) must never
# index or mark the user's real ~/.local/share/resonance/amp-models.
AMP_MODEL_ROOT = os.path.join(
    tempfile.gettempdir(), f"resonance-test-amp-models-{os.getpid()}"
)
LIBRARY_ROOT = os.path.join(
    tempfile.gettempdir(), f"resonance-test-library-{os.getpid()}"
)
# And the cache (the preset-discovery index), which lives under
# $XDG_CACHE_HOME in the app.
CACHE_ROOT = os.path.join(
    tempfile.gettempdir(), f"resonance-test-cache-{os.getpid()}"
)


def run_one(exe: str, cwd: str, extra: list[str]) -> tuple[str, int, str]:
    env = dict(os.environ)
    env.setdefault("RESONANCE_PLUGIN_PRESET_DIR", PRESET_ROOT)
    env.setdefault("RESONANCE_AMP_MODEL_DIR", AMP_MODEL_ROOT)
    env.setdefault("RESONANCE_LIBRARY_DIR", LIBRARY_ROOT)
    env.setdefault("RESONANCE_CACHE_DIR", CACHE_ROOT)
    proc = subprocess.run(
        [exe, *extra],
        cwd=cwd,
        capture_output=True,
        text=True,
        env=env,
    )
    return exe, proc.returncode, proc.stdout + proc.stderr


def main() -> int:
    default_jobs = max(1, (os.cpu_count() or 4) // 2)

    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument("-j", type=int, default=default_jobs, dest="jobs")
    parser.add_argument("-p", action="append", default=[], dest="crates")
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--no-check", action="store_true")
    args, extra = parser.parse_known_args()
    if extra and extra[0] == "--":
        extra = extra[1:]

    if not args.no_check and not args.no_build:
        check_non_test(args.crates)

    binaries = build_and_enumerate(args.crates, build=not args.no_build)
    if not args.no_build:
        build_plugin_cdylibs(args.crates)
    if not binaries:
        return print("no test binaries found") or 1

    print(f"running {len(binaries)} test binaries, {args.jobs} at a time")
    started = time.monotonic()
    failures: list[tuple[str, str]] = []

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(run_one, exe, cwd, extra) for exe, cwd in binaries]
        for done in concurrent.futures.as_completed(futures):
            exe, code, output = done.result()
            if code != 0:
                failures.append((exe, output))

    elapsed = time.monotonic() - started
    if failures:
        for exe, output in failures:
            print(f"\n{'=' * 70}\nFAILED {os.path.basename(exe)}\n{'=' * 70}")
            print(output)
        print(f"\n{len(failures)} of {len(binaries)} binaries failed in {elapsed:.0f}s")
        return 1

    print(f"all {len(binaries)} binaries passed in {elapsed:.0f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
