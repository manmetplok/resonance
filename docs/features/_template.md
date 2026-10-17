# <Feature name>

> Verified against master @ `<short sha>` on <YYYY-MM-DD>.
> Also verified: <work on top of that sha, if any (drop the line if none)>
> Spec history: `docs/history/specs/<file>.md` (if any).

<One paragraph: what the user (or an agent) can do with this, in plain words.>

## Behaviour

What it does **today** — the rules, limits and defaults a user or agent runs
into. Describe the code, not the plan. Short subsections or bullets.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model / persistence | `crate/src/path.rs` | `TypeName`, `fn_name` |
| Engine / DSP | … | … |
| App state + update | … | … |
| View | … | … |

Note any invariants a change must keep (RT-safety, undo, id ownership, …).

## Surfaces

- **GUI:** where it is in the app (tab, panel, menu, shortcut).
- **Control API / MCP:** `method.name` → `mcp__resonance__tool_name`.
- **Agent skills:** which `resonance-agent-plugin/skills/*` use it.

## How it's tested

| What | Where | Run |
|---|---|---|
| Unit / integration | `crate/tests/<group>/<module>.rs` | `./scripts/run-tests.py -p crate` |
| Goldens | `tests/snapshots/<name>*.png` | `RESONANCE_BLESS=1 …` to re-bless |
| By hand | e.g. Wayland editor test, live session | command |

Say what is **not** covered.

## Decisions and deviations

Choices that are not obvious from the code, and where the build differs from
the original spec (with the reason).

## Known gaps

Bullets, each linking to an intent in `docs/intents/` if one exists.
