#!/usr/bin/env python3
"""Run the blind voicebank audition (vocal-mcp-control-and-quality.md §12
slice 6, D30) and hide which file is which bank/speaker.

The render itself lives in `resonance-app/tests/vocal/bank_audition.rs`, an
`#[ignore]`d test: it renders the committed fixture
(`tests/vocal/fixtures/bank_audition_song.json`) through the real
registry → resolver → segment → render path on every installed voicebank ×
speaker, and writes plainly-named WAVs (`<bank>__<speaker>.wav`) plus a
`manifest.json` into the directory `RESONANCE_AUDITION_OUT` names. That test
scans banks the same way the app does (`VoicebankRoots::for_machine`), so
either `RESONANCE_SVS_MODELS_DIR` or a bank dropped into the user voicebank
folder makes it find something; with neither it no-ops and this script
reports that plainly.

This script is the part that makes it *blind*: it runs that test, then
shuffles `manifest.json`'s entries behind random 8-character tokens, renames
each WAV to `<token>.wav`, and writes `key.json` (token -> bank/speaker) —
which only `--reveal` prints. Every token file gets the same modification
time, so a directory listing sorted by time does not give the render order
(and with it the bank order) away. Listen to the files named by token; once
you've picked, run with `--reveal` to see what they were.

The output directory is only ever cleared of what this script writes
(`*.wav`, `manifest.json`, `key.json`); a non-empty directory holding
neither a `key.json` nor a `manifest.json` is refused rather than touched.
If the render fails part-way but wrote a `manifest.json`, the files it did
write are still shuffled (so no plainly-named WAV is left behind) before
the failure is reported.

Usage:
    scripts/vocal-audition.py              # render, shuffle, list files to listen to
    scripts/vocal-audition.py --out DIR    # use a different output dir (default target/audition)
    scripts/vocal-audition.py --reveal     # print the token -> bank/speaker key
"""

from __future__ import annotations

import argparse
import json
import os
import secrets
import string
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO_ROOT / "target" / "audition"


# What this script writes into the output directory, and all it removes.
OWN_NAMES = ("manifest.json", "key.json")


def random_token(length: int = 8) -> str:
    alphabet = string.ascii_lowercase + string.digits
    return "".join(secrets.choice(alphabet) for _ in range(length))


def is_ours(path: Path) -> bool:
    return path.is_file() and (path.suffix == ".wav" or path.name in OWN_NAMES)


def clear_out_dir(out_dir: Path) -> bool:
    """Remove a previous run's files from `out_dir`. Refuses (returns False)
    a non-empty directory that no run of this script wrote."""
    out_dir.mkdir(parents=True, exist_ok=True)
    entries = list(out_dir.iterdir())
    if entries and not any((out_dir / n).exists() for n in OWN_NAMES):
        print(
            f"{out_dir} is not empty and holds no key.json or manifest.json from an "
            "earlier audition; pick an empty --out (nothing was deleted)",
            file=sys.stderr,
        )
        return False
    for stale in entries:
        if is_ours(stale):
            stale.unlink()
    return True


def reveal(out_dir: Path) -> int:
    key_path = out_dir / "key.json"
    if not key_path.exists():
        print(f"no key.json in {out_dir}; run without --reveal first", file=sys.stderr)
        return 1
    key = json.loads(key_path.read_text())
    for token in sorted(key):
        entry = key[token]
        speaker = entry.get("speaker") or "(default)"
        duration = entry.get("duration_sec")
        suffix = f", {duration:.2f}s" if duration is not None else ""
        print(f"{token}.wav -> {entry['bank']} / {speaker}{suffix}")
    return 0


def run_render(out_dir: Path) -> int:
    env = dict(os.environ)
    env["RESONANCE_AUDITION_OUT"] = str(out_dir)
    cmd = ["cargo", "test", "-p", "resonance-app", "--test", "vocal", "--",
           "--ignored", "audition", "--nocapture"]
    print(f"running: {' '.join(cmd)}  (RESONANCE_AUDITION_OUT={out_dir})")
    proc = subprocess.run(cmd, cwd=REPO_ROOT, env=env)
    return proc.returncode


def shuffle_into_key(out_dir: Path) -> int:
    manifest_path = out_dir / "manifest.json"
    if not manifest_path.exists():
        print(
            "no manifest.json was produced; the audition test found no "
            "installed voicebanks (set RESONANCE_SVS_MODELS_DIR, or drop a "
            "bank into the user voicebank folder) or the render failed "
            "above",
            file=sys.stderr,
        )
        return 1
    manifest = json.loads(manifest_path.read_text())
    entries = manifest.get("entries", [])
    if not entries:
        print("manifest.json lists no bank x speaker combinations", file=sys.stderr)
        return 1

    secrets.SystemRandom().shuffle(entries)
    # One timestamp for every file: the listing order by time says nothing.
    stamp = time.time()
    used: set[str] = set()
    key: dict[str, dict] = {}
    for entry in entries:
        token = random_token()
        while token in used:
            token = random_token()
        used.add(token)
        src = out_dir / entry["file"]
        if not src.exists():
            # A render that failed part-way listed a file it never wrote.
            continue
        dst = out_dir / f"{token}.wav"
        src.rename(dst)
        os.utime(dst, (stamp, stamp))
        key[token] = {
            "bank": entry["bank"],
            "speaker": entry.get("speaker"),
            "duration_sec": entry.get("duration_sec"),
        }
    manifest_path.unlink()
    (out_dir / "key.json").write_text(json.dumps(key, indent=2, sort_keys=True) + "\n")

    print(f"\nwrote {len(key)} files to {out_dir}, shuffled behind random tokens.")
    print("listen blind, then run `scripts/vocal-audition.py --reveal` to see the mapping.\n")
    print("files:")
    for wav in sorted(out_dir.glob("*.wav")):
        print(f"  {wav}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--out", type=Path, default=DEFAULT_OUT,
        help="output directory (default: target/audition)",
    )
    parser.add_argument(
        "--reveal", action="store_true",
        help="print the token -> bank/speaker key for an already-rendered --out and exit",
    )
    args = parser.parse_args()

    if args.reveal:
        return reveal(args.out)

    if not clear_out_dir(args.out):
        return 2
    rc = run_render(args.out)
    # Shuffle whatever was written, even after a failure, so no file is
    # left under its bank's name; then report the failure.
    if (args.out / "manifest.json").exists():
        shuffled = shuffle_into_key(args.out)
        if rc == 0:
            return shuffled
    if rc != 0:
        # A render that died before its manifest: drop the plainly-named
        # WAVs it wrote (`<bank>__<speaker>.wav`) rather than reveal them.
        for plain in args.out.glob("*__*.wav"):
            plain.unlink()
        print(f"the audition render failed (exit {rc}); see the output above", file=sys.stderr)
        return rc
    return shuffle_into_key(args.out)


if __name__ == "__main__":
    raise SystemExit(main())
