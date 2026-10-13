#!/usr/bin/env python3
# Re-exec under python3 when started as `sh scripts/vocal-intelligibility.py`.
''':'
exec python3 "$0" "$@"
'''
# Assigned, not a bare docstring: the sh trampoline above is the module's
# first string, so it would be `__doc__` and the --help text.
__doc__ = """Score vocal renders for intelligibility: transcribe each WAV with
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

The scoring, the silence trim and the Whisper call are not defined here:
they are imported from the singing skill's own checker,
`resonance-agent-plugin/skills/singing/scripts/intelligibility.py`, so the
audition and an agent scoring a vocal stem get the same number for the same
file. That script's `--self-test` covers them.

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
import importlib.util
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
FIXTURE = REPO_ROOT / "resonance-app" / "tests" / "vocal" / "fixtures" / "bank_audition_song.json"
SHARED = REPO_ROOT / "resonance-agent-plugin" / "skills" / "singing" / "scripts" / "intelligibility.py"


def _load_shared():
    """The singing skill's checker, imported by path (it ships in the
    plugin, not as a package)."""
    spec = importlib.util.spec_from_file_location("resonance_intelligibility", SHARED)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


shared = _load_shared()


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


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("dir", type=Path, help="audition output directory")
    parser.add_argument("--model", default=shared.DEFAULT_MODEL,
                        help=f"Whisper model (default {shared.DEFAULT_MODEL})")
    parser.add_argument("--json", type=Path, help="write per-file results here")
    args = parser.parse_args()

    entries = load_entries(args.dir)
    if not entries:
        print(f"no WAVs to score in {args.dir}", file=sys.stderr)
        return 1

    if not shared.whisper_available():
        shared.reexec_under_uv(Path(__file__), sys.argv[1:])
        return 1

    model = shared.load_model(args.model)
    results = []
    for e in sorted(entries, key=lambda e: (e.get("bank", ""), e.get("speaker") or "")):
        heard = shared.transcribe(model, args.dir / e["file"]) or ""
        acc = shared.word_accuracy(e["text"], heard)
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
        print(f"{e.get('bank', '?'):<15} {speaker:<11} {shift:>4} {acc:6.0%}  | {heard}", flush=True)

    by_bank: dict[str, list[float]] = {}
    for r in results:
        by_bank.setdefault(r["bank"] or "?", []).append(r["accuracy"])
    print()
    for bank, accs in sorted(by_bank.items()):
        print(f"{bank:<15} best {max(accs):6.0%}  mean {sum(accs) / len(accs):6.0%}  ({len(accs)} takes)")
    if args.json:
        args.json.write_text(json.dumps(results, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
