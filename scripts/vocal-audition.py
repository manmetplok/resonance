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
which only `--reveal` prints. Listen to the files named by token; once
you've picked, run with `--reveal` to see what they were.

Usage:
    scripts/vocal-audition.py              # render, shuffle, list files to listen to
    scripts/vocal-audition.py --out DIR    # use a different output dir (default target/audition)
    scripts/vocal-audition.py --reveal     # print the token -> bank/speaker key
"""

from __future__ import annotations

import argparse
import json
import os
import random
import string
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO_ROOT / "target" / "audition"


def random_token(length: int = 8) -> str:
    alphabet = string.ascii_lowercase + string.digits
    return "".join(random.choices(alphabet, k=length))


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
    out_dir.mkdir(parents=True, exist_ok=True)
    for stale in out_dir.iterdir():
        if stale.is_file():
            stale.unlink()
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

    random.shuffle(entries)
    used: set[str] = set()
    key: dict[str, dict] = {}
    for entry in entries:
        token = random_token()
        while token in used:
            token = random_token()
        used.add(token)
        src = out_dir / entry["file"]
        dst = out_dir / f"{token}.wav"
        src.rename(dst)
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

    rc = run_render(args.out)
    if rc != 0:
        return rc
    return shuffle_into_key(args.out)


if __name__ == "__main__":
    raise SystemExit(main())
