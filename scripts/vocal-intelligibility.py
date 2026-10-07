#!/usr/bin/env python3
# Re-exec under python3 when started as `sh scripts/vocal-intelligibility.py`.
''':'
exec python3 "$0" "$@"
'''
"""Score vocal renders for intelligibility: transcribe each WAV with
Whisper and compare the words against the lyric it was meant to sing.

Input is a directory written by the bank audition
(`resonance-app/tests/vocal/bank_audition.rs`): either the plainly-named
WAVs plus `manifest.json` that the test itself writes, or the shuffled
`<token>.wav` files plus `key.json` that `scripts/vocal-audition.py` leaves.
Both carry the bank and speaker of every file; the expected text comes from
the manifest/key (`text`) or, failing that, the committed fixture
(`resonance-app/tests/vocal/fixtures/bank_audition_song.json`).

Word accuracy is `1 - WER` (word error rate: substitutions + deletions +
insertions over the lyric's word count, by edit distance on lowercased
words with punctuation stripped), floored at 0. Singing is hard for a
speech recogniser, so read the numbers comparatively — bank against bank,
before against after — not as an absolute grade.

Whisper is an optional dependency. When `import whisper` fails, the script
re-runs itself through `uv` with a CPU-only PyTorch:

    uv run --python 3.12 --with openai-whisper \\
        --index-url https://download.pytorch.org/whl/cpu \\
        --extra-index-url https://pypi.org/simple python <this script> ...

so `uv` on PATH is all it needs (the first run downloads PyTorch and the
model, ~0.5 GB, into uv's and Whisper's caches).

Usage:
    scripts/vocal-intelligibility.py DIR             # table to stdout
    scripts/vocal-intelligibility.py DIR --json F    # also write per-file results
    scripts/vocal-intelligibility.py DIR --model small.en
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
FIXTURE = REPO_ROOT / "resonance-app" / "tests" / "vocal" / "fixtures" / "bank_audition_song.json"
REEXEC_MARK = "RESONANCE_INTELLIGIBILITY_REEXEC"


def words(text: str) -> list[str]:
    return re.findall(r"[a-z']+", text.lower().replace("’", "'"))


def edit_distance(ref: list[str], hyp: list[str]) -> int:
    prev = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        cur = [i] + [0] * len(hyp)
        for j, h in enumerate(hyp, 1):
            cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (r != h))
        prev = cur
    return prev[-1]


def word_accuracy(ref_text: str, hyp_text: str) -> float:
    ref, hyp = words(ref_text), words(hyp_text)
    if not ref:
        return 0.0
    return max(0.0, 1.0 - edit_distance(ref, hyp) / len(ref))


def load_entries(directory: Path) -> list[dict]:
    """Every WAV in `directory` with its bank, speaker and expected text."""
    fallback_text = None
    if FIXTURE.exists():
        fallback_text = json.loads(FIXTURE.read_text()).get("text")
    manifest = directory / "manifest.json"
    key = directory / "key.json"
    entries: list[dict] = []
    if manifest.exists():
        data = json.loads(manifest.read_text())
        text = data.get("text", fallback_text)
        for e in data.get("entries", []):
            entries.append({**e, "text": e.get("text", text)})
    elif key.exists():
        for token, e in json.loads(key.read_text()).items():
            entries.append({**e, "file": f"{token}.wav", "text": e.get("text", fallback_text)})
    else:
        for wav in sorted(directory.glob("*.wav")):
            entries.append({"file": wav.name, "bank": "?", "speaker": None, "text": fallback_text})
    missing = [e["file"] for e in entries if not e.get("text")]
    if missing:
        sys.exit(f"no expected text for {missing}; the fixture is missing and the manifest has none")
    return [e for e in entries if (directory / e["file"]).exists()]


def reexec_under_uv() -> None:
    if os.environ.get(REEXEC_MARK):
        sys.exit("whisper is still not importable under uv; see the install hint in --help")
    cmd = [
        "uv", "run", "--quiet", "--python", "3.12", "--with", "openai-whisper",
        "--index-url", "https://download.pytorch.org/whl/cpu",
        "--extra-index-url", "https://pypi.org/simple",
        "python", str(Path(__file__).resolve()), *sys.argv[1:],
    ]
    env = dict(os.environ, **{REEXEC_MARK: "1"})
    try:
        raise SystemExit(subprocess.run(cmd, env=env).returncode)
    except FileNotFoundError:
        sys.exit("whisper is not installed and `uv` is not on PATH to fetch it "
                 "(pip install openai-whisper, or install uv)")


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("dir", type=Path, help="audition output directory")
    parser.add_argument("--model", default="small.en", help="Whisper model (default small.en)")
    parser.add_argument("--json", type=Path, help="write per-file results here")
    args = parser.parse_args()

    entries = load_entries(args.dir)
    if not entries:
        print(f"no WAVs to score in {args.dir}", file=sys.stderr)
        return 1

    try:
        import whisper  # noqa: PLC0415 — optional dependency
    except ImportError:
        reexec_under_uv()
        return 1

    model = whisper.load_model(args.model)
    results = []
    for e in sorted(entries, key=lambda e: (e.get("bank", ""), e.get("speaker") or "")):
        out = model.transcribe(
            str(args.dir / e["file"]), language="en", fp16=False, temperature=0
        )
        heard = out["text"].strip()
        acc = word_accuracy(e["text"], heard)
        results.append({
            "file": e["file"],
            "bank": e.get("bank"),
            "speaker": e.get("speaker"),
            "transpose": e.get("transpose"),
            "accuracy": round(acc, 3),
            "heard": heard,
        })
        speaker = e.get("speaker") or "-"
        shift = e.get("transpose")
        shift = f"{shift:+d}" if isinstance(shift, int) else ""
        print(f"{e.get('bank', '?'):<8} {speaker:<10} {shift:>4} {acc:6.0%}  | {heard}", flush=True)

    by_bank: dict[str, list[float]] = {}
    for r in results:
        by_bank.setdefault(r["bank"] or "?", []).append(r["accuracy"])
    print()
    for bank, accs in sorted(by_bank.items()):
        print(f"{bank:<8} best {max(accs):6.0%}  mean {sum(accs) / len(accs):6.0%}  ({len(accs)} takes)")
    if args.json:
        args.json.write_text(json.dumps(results, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
