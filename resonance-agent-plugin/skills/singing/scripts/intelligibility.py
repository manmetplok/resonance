#!/usr/bin/env python3
"""Score sung vocal takes for intelligibility: transcribe each WAV with
Whisper and compare what it heard with the lyric it was meant to sing.

    intelligibility.py --text "walk with me along the river" take.wav [more.wav ...]
    intelligibility.py --text-file lyric.txt vocal-stem.wav --json

The lyric may be pasted in the vocal synth's lyric syntax: `·` and `-`
syllable breaks inside a word are joined, standalone `_` / `+` holds and
`AP` breaths are dropped, and `[..]` phoneme blocks are dropped (they are
sounds, not words a transcript can match). Several lines are one lyric.

Per file it prints the word accuracy (1 - WER: substitutions, deletions and
insertions over the lyric's word count, on lowercased words with
punctuation stripped, floored at 0), what Whisper heard, and the lyric words
it missed. `--json` prints the same as JSON instead: `{"reference": <the
lyric as scored>, "results": [{"file", "silent", "accuracy", "heard",
"missed"}, ...]}`. Leading and
trailing silence is trimmed before transcribing (a stem of a vocal that
enters at bar 9 starts with silence); a file with nothing above -60 dBFS is
reported as silent rather than transcribed. Long takes are fine: Whisper
works through them in 30 s windows.

Singing is hard for a speech recogniser, so read the number comparatively —
before and after a fix, one voice against another — not as a grade.

Whisper (`openai-whisper`, model `small.en`) is optional. When it is not
importable the script re-runs itself through `uv` with a CPU-only PyTorch;
the first run downloads PyTorch (about 0.5 GB) into uv's cache and the model
into ~/.cache/whisper. Without `uv`, install it first:

    curl -LsSf https://astral.sh/uv/install.sh | sh

`--self-test` checks the lyric stripping, normalising, scoring and trimming
code without Whisper.

The repo's `scripts/vocal-intelligibility.py` imports this file, so the bank
audition and this check score identically.
"""

import argparse
import json
import math
import os
import re
import subprocess
import sys
from pathlib import Path

DEFAULT_MODEL = "small.en"
# Whisper resamples every file to 16 kHz mono.
WHISPER_RATE = 16_000
REEXEC_MARK = "RESONANCE_INTELLIGIBILITY_REEXEC"
UV_INSTALL_HINT = (
    "install uv (curl -LsSf https://astral.sh/uv/install.sh | sh) and run this "
    "again, or pip install openai-whisper"
)

# ---------------------------------------------------------------------------
# Text: lyric syntax, normalising, scoring
# ---------------------------------------------------------------------------

_PHONEME_BLOCK = re.compile(r"\[[^\]]*\]")


def strip_lyric_syntax(text: str) -> str:
    """The words a listener should hear in a lyric written in the vocal
    synth's syntax (see the singing skill's references/lyrics.md)."""
    text = _PHONEME_BLOCK.sub(" ", text)
    out = []
    for token in text.split():
        # Standalone holds and breaths take no word. A lowercase "ap" is a
        # real word and stays.
        if token in ("_", "+", "AP"):
            continue
        # `a·_·ma·zing`: a hold piece inside a broken word goes, then the
        # breaks join the pieces. A spaced `-` is a dash, not a break; it
        # never reaches here as part of a word.
        if "·" in token or ("-" in token and token.strip("-")):
            pieces = [p for p in re.split(r"[·\-]", token) if p not in ("", "_")]
            token = "".join(pieces)
        out.append(token)
    return " ".join(out)


def words(text: str) -> list[str]:
    """Lowercased words with punctuation stripped; curly apostrophes
    count as straight ones."""
    return re.findall(r"[a-z']+", text.lower().replace("’", "'"))


def _alignment(ref: list[str], hyp: list[str]) -> list[list[int]]:
    """Levenshtein table over words: `d[i][j]` is the distance between
    `ref[:i]` and `hyp[:j]`."""
    d = [[0] * (len(hyp) + 1) for _ in range(len(ref) + 1)]
    for i in range(len(ref) + 1):
        d[i][0] = i
    for j in range(len(hyp) + 1):
        d[0][j] = j
    for i, r in enumerate(ref, 1):
        for j, h in enumerate(hyp, 1):
            d[i][j] = min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + (r != h))
    return d


def edit_distance(ref: list[str], hyp: list[str]) -> int:
    return _alignment(ref, hyp)[len(ref)][len(hyp)]


def word_accuracy(ref_text: str, hyp_text: str) -> float:
    """1 - WER of `hyp_text` against `ref_text`, floored at 0 (0 for an
    empty reference)."""
    ref, hyp = words(ref_text), words(hyp_text)
    if not ref:
        return 0.0
    return max(0.0, 1.0 - edit_distance(ref, hyp) / len(ref))


def missed_words(ref_text: str, hyp_text: str) -> list[str]:
    """The lyric words a minimal alignment does not match: deleted or
    heard as something else, in lyric order."""
    ref, hyp = words(ref_text), words(hyp_text)
    d = _alignment(ref, hyp)
    i, j, missed = len(ref), len(hyp), []
    while i > 0 or j > 0:
        if i > 0 and j > 0 and ref[i - 1] == hyp[j - 1] and d[i][j] == d[i - 1][j - 1]:
            i, j = i - 1, j - 1
        elif i > 0 and j > 0 and d[i][j] == d[i - 1][j - 1] + 1:
            missed.append(ref[i - 1])
            i, j = i - 1, j - 1
        elif i > 0 and d[i][j] == d[i - 1][j] + 1:
            missed.append(ref[i - 1])
            i -= 1
        else:
            j -= 1
    missed.reverse()
    return missed


def score(ref_text: str, heard: str) -> dict:
    """Accuracy, what was heard and the missed words, for one take."""
    return {
        "accuracy": round(word_accuracy(ref_text, heard), 3),
        "heard": heard,
        "missed": missed_words(ref_text, heard),
    }


# ---------------------------------------------------------------------------
# Audio: silence trim, Whisper
# ---------------------------------------------------------------------------

FRAME_S = 0.02
SILENCE_DBFS = -60.0
# Below the loudest frame by this much still counts as the take.
RELATIVE_DB = 45.0
PAD_S = 0.25


def _frame_rms(samples, frame: int) -> list[float]:
    try:
        import numpy as np  # noqa: PLC0415 — present wherever whisper is
    except ImportError:
        np = None
    if np is not None and isinstance(samples, np.ndarray):
        n = len(samples) // frame
        if n == 0:
            return []
        blocks = samples[: n * frame].astype("float64").reshape(n, frame)
        return list(np.sqrt((blocks * blocks).mean(axis=1)))
    return [
        math.sqrt(sum(x * x for x in samples[k : k + frame]) / frame)
        for k in range(0, len(samples) - frame + 1, frame)
    ]


def sounding_bounds(samples, rate: int = WHISPER_RATE) -> tuple[int, int] | None:
    """`(start, end)` sample bounds of the take — from the first to the
    last frame within `RELATIVE_DB` of the loudest and above
    `SILENCE_DBFS`, padded by `PAD_S` — or `None` when nothing sounds."""
    frame = max(1, int(rate * FRAME_S))
    rms = _frame_rms(samples, frame)
    if not rms:
        return None
    peak = max(rms)
    floor = max(10 ** (SILENCE_DBFS / 20), peak * 10 ** (-RELATIVE_DB / 20))
    if peak < 10 ** (SILENCE_DBFS / 20):
        return None
    loud = [k for k, v in enumerate(rms) if v >= floor]
    pad = int(rate * PAD_S)
    start = max(0, loud[0] * frame - pad)
    end = min(len(samples), (loud[-1] + 1) * frame + pad)
    return start, end


def load_model(name: str = DEFAULT_MODEL):
    """The Whisper model, downloading it to ~/.cache/whisper on first use."""
    import whisper  # noqa: PLC0415 — optional dependency

    cached = Path.home() / ".cache" / "whisper" / f"{name}.pt"
    if not cached.exists():
        print(f"downloading the Whisper {name} model to {cached.parent} (first use)",
              file=sys.stderr, flush=True)
    return whisper.load_model(name)


def transcribe(model, path: Path) -> str | None:
    """What Whisper hears in `path` after trimming its leading and
    trailing silence, or `None` when the file is silent."""
    import whisper  # noqa: PLC0415 — optional dependency

    audio = whisper.load_audio(str(path))
    bounds = sounding_bounds(audio)
    if bounds is None:
        return None
    start, end = bounds
    out = model.transcribe(
        audio[start:end], language="en", fp16=False, temperature=0
    )
    return out["text"].strip()


def whisper_available() -> bool:
    try:
        import whisper  # noqa: F401, PLC0415
    except ImportError:
        return False
    return True


def reexec_under_uv(script: Path, argv: list[str]) -> None:
    """Re-run `script` with `argv` under `uv` with openai-whisper and a
    CPU-only PyTorch. Exits with the child's status; never returns."""
    if os.environ.get(REEXEC_MARK):
        sys.exit("whisper is still not importable under uv; " + UV_INSTALL_HINT)
    cmd = [
        "uv", "run", "--quiet", "--python", "3.12", "--with", "openai-whisper",
        "--index-url", "https://download.pytorch.org/whl/cpu",
        "--extra-index-url", "https://pypi.org/simple",
        "python", str(script.resolve()), *argv,
    ]
    env = dict(os.environ, **{REEXEC_MARK: "1"})
    try:
        raise SystemExit(subprocess.run(cmd, env=env).returncode)
    except FileNotFoundError:
        sys.exit("whisper is not installed and `uv` is not on PATH to fetch it: " + UV_INSTALL_HINT)


# ---------------------------------------------------------------------------
# Self-test (no Whisper)
# ---------------------------------------------------------------------------


def self_test() -> int:
    checks = 0

    def eq(got, want, what):
        nonlocal checks
        checks += 1
        if got != want:
            raise AssertionError(f"{what}: got {got!r}, want {want!r}")

    # Lyric syntax.
    eq(strip_lyric_syntax("re·mem·ber the night AP we will sing"),
       "remember the night we will sing", "breaks join, AP drops")
    eq(strip_lyric_syntax("love _ _ and + more"), "love and more", "holds drop")
    eq(strip_lyric_syntax("a·_·ma·zing grace"), "amazing grace", "hold piece drops")
    eq(strip_lyric_syntax("ri-ver wait - no"), "river wait - no", "inner - joins, spaced - stays")
    eq(strip_lyric_syntax("[l ih · l iy] lily ap"), "lily ap", "phoneme block drops, lowercase ap stays")
    eq(words(strip_lyric_syntax("wait - no")), ["wait", "no"], "a dash is not a word")
    eq(strip_lyric_syntax("line one\nline·two"), "line one linetwo", "lines join")

    # Normalising.
    eq(words("Walk, with ME!"), ["walk", "with", "me"], "case and punctuation")
    eq(words("Don’t stop"), ["don't", "stop"], "curly apostrophe")

    # Scoring.
    eq(word_accuracy("walk with me", "walk with me"), 1.0, "identical")
    eq(word_accuracy("walk with me now", "walk with he now"), 0.75, "one substitution")
    eq(word_accuracy("walk", "walk walk walk walk"), 0.0, "insertions floor at 0")
    eq(word_accuracy("", "anything"), 0.0, "empty lyric")
    eq(word_accuracy("Walk with me.", "walk, with me"), 1.0, "punctuation ignored")
    eq(missed_words("walk with me along the river", "walk with along a river"),
       ["me", "the"], "missed: a deletion and a substitution")
    eq(missed_words("walk with me", "walk with me"), [], "nothing missed")
    eq(missed_words("walk with me", ""), ["walk", "with", "me"], "all missed")
    eq(score("walk with me", "walk with"),
       {"accuracy": 0.667, "heard": "walk with", "missed": ["me"]}, "score shape")

    # Silence trim (pure-Python path).
    rate = 1_000
    tone = [0.5 * math.sin(2 * math.pi * 50 * n / rate) for n in range(rate)]
    take = [0.0] * rate + tone + [0.0] * (2 * rate)
    bounds = sounding_bounds(take, rate)
    pad = int(rate * PAD_S)
    eq(bounds, (rate - pad, 2 * rate + pad), "trim to the tone plus padding")
    eq(sounding_bounds([0.0] * rate, rate), None, "digital silence")
    eq(sounding_bounds([1e-4] * rate, rate), None, "below -60 dBFS is silence")
    try:
        import numpy as np  # noqa: PLC0415
    except ImportError:
        pass
    else:
        eq(sounding_bounds(np.array(take, dtype="float32"), rate), bounds, "numpy path agrees")

    print(f"self-test passed ({checks} checks)")
    return 0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    lyric = parser.add_mutually_exclusive_group()
    lyric.add_argument("--text", help="the lyric as it should be heard (lyric syntax is fine)")
    lyric.add_argument("--text-file", type=Path, help="read the lyric from this file")
    parser.add_argument("files", nargs="*", type=Path, help="WAV files to score")
    parser.add_argument("--json", action="store_true", help="print JSON instead of a table")
    parser.add_argument("--model", default=DEFAULT_MODEL, help=f"Whisper model (default {DEFAULT_MODEL})")
    parser.add_argument("--self-test", action="store_true", help="test the scoring code and exit")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()
    if args.text is None and args.text_file is None:
        parser.error("pass the lyric with --text or --text-file")
    if not args.files:
        parser.error("name at least one WAV file")
    text = args.text if args.text is not None else args.text_file.read_text()
    reference = strip_lyric_syntax(text)
    if not words(reference):
        parser.error("the lyric has no words left to score against")
    missing = [str(f) for f in args.files if not f.is_file()]
    if missing:
        parser.error(f"no such file: {', '.join(missing)}")

    if not whisper_available():
        reexec_under_uv(Path(__file__), argv)

    model = load_model(args.model)
    results = []
    for path in args.files:
        heard = transcribe(model, path)
        if heard is None:
            result = {"file": str(path), "silent": True, **score(reference, "")}
        else:
            result = {"file": str(path), "silent": False, **score(reference, heard)}
        results.append(result)
        if not args.json:
            if result["silent"]:
                print(f"{path}: SILENT (nothing above {SILENCE_DBFS:.0f} dBFS)", flush=True)
            else:
                missed = " ".join(result["missed"]) or "-"
                print(f"{path}: {result['accuracy']:.0%}", flush=True)
                print(f"  heard:  {result['heard']}", flush=True)
                print(f"  missed: {missed}", flush=True)
    if args.json:
        print(json.dumps({"reference": reference, "results": results}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
