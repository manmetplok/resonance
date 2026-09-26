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

Usage:
    scripts/run-tests.py [-jN] [-p CRATE]... [--no-build] [--] [libtest args]

    -jN            concurrent binaries (default: half the cores, since each
                   binary parallelises its own tests across threads too)
    -p CRATE       restrict to a crate; repeatable. Default: whole workspace.
    --no-build     skip the cargo build step and reuse what is on disk
    trailing args  passed through to every test binary (e.g. --nocapture)

Exit status is non-zero if any binary failed.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import subprocess
import sys
import time


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


def run_one(exe: str, cwd: str, extra: list[str]) -> tuple[str, int, str]:
    proc = subprocess.run(
        [exe, *extra],
        cwd=cwd,
        capture_output=True,
        text=True,
    )
    return exe, proc.returncode, proc.stdout + proc.stderr


def main() -> int:
    default_jobs = max(1, (os.cpu_count() or 4) // 2)

    parser = argparse.ArgumentParser(add_help=True)
    parser.add_argument("-j", type=int, default=default_jobs, dest="jobs")
    parser.add_argument("-p", action="append", default=[], dest="crates")
    parser.add_argument("--no-build", action="store_true")
    args, extra = parser.parse_known_args()
    if extra and extra[0] == "--":
        extra = extra[1:]

    binaries = build_and_enumerate(args.crates, build=not args.no_build)
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
