# resonance-agent-plugin

A [Claude Code plugin](https://code.claude.com/docs/en/plugins) of **skills** for
driving a running resonance DAW through the [`resonance-mcp`](../resonance-mcp)
server.

It ships procedures, not capability. The MCP server already exposes the verbs —
`meter_stems`, `mixer_set_volume_db`, `master_add_effect` and the rest. What no
tool description can hold is the *procedure* that ties them together: the order
to diagnose in, what a measurement justifies, and when to stop. That is what
lives here, and it loads only when a mixing or mastering task actually starts,
instead of costing context on every session the way a tool description does.

## What's in it

| Skill | Invoke | Covers |
|---|---|---|
| `song-structure` | `/resonance-agent-plugin:song-structure` | Choosing a form and laying it out. Definitions vs. placements, the `place: false` trap, bar math. |
| `arranging` | `/resonance-agent-plugin:arranging` | Chord grids, `generate_part`, hand-written MIDI, density and register across sections. |
| `drumming` | `/resonance-agent-plugin:drumming` | The pattern library, and hand-written grooves the library cannot reach. |
| `mixing` | `/resonance-agent-plugin:mixing` | Measure → diagnose → balance → verify. Faults, balance, tone, dynamics, in that order. |
| `mastering` | `/resonance-agent-plugin:mastering` | Master-bus chain on `com.resonance.mastering`, stage by stage, to delivery targets. |

All five are model-invocable, so Claude reaches for them when you just say "mix
this" or "write a drum part" — you do not have to type the command.

They run roughly in that order — structure, then parts, then balance — and each
one hands off to the next by pointing at it. Claude Code has no notion of a
nested "sub-skill", so `drumming` and `song-structure` are siblings of
`arranging` rather than children of it. That is the better shape anyway: "write
me a jazz drum part" is a complete request on its own and should not have to load
the whole arranging procedure to get answered.

Reference files are shared across skills via `${CLAUDE_PLUGIN_ROOT}`:

| File | Shared by |
|---|---|
| `skills/mixing/references/reading-meters.md` | `mixing`, `mastering` |
| `skills/song-structure/references/forms.md` | `song-structure`, `arranging` |
| `skills/arranging/references/generators.md` | `arranging` |
| `skills/drumming/references/drum-map.md` | `drumming` |
| `skills/drumming/references/grooves.md` | `drumming` |

`drum-map.md` is worth calling out: the kit is General MIDI in the middle and
**not** GM at the edges (note 39 is a sidestick, not a clap; 21–29 are extended
articulations in no GM chart), so a part written from GM memory triggers the
wrong pads. Its ground truth is `resonance-common/src/drum_map.rs`.

## Install

This plugin contains **no `.mcp.json`**. Register the server separately, once:

```sh
cargo build --release -p resonance-mcp
claude mcp add --transport stdio resonance -- /absolute/path/to/target/release/resonance-mcp
```

That keeps the tools named `mcp__resonance__<tool>`. Bundling the server config
into the plugin would rename every tool to
`mcp__plugin_resonance-agent-plugin_resonance__<tool>`, breaking existing
permission allowlists and hook matchers for no benefit here, where the plugin
and the server ship from the same checkout anyway.

Then load the plugin. For local use, point Claude Code at this directory:

```sh
claude --plugin-dir /absolute/path/to/resonance-agent-plugin
```

To distribute it, publish this directory through a
[marketplace](https://code.claude.com/docs/en/plugin-marketplaces) — a git repo
with a `.claude-plugin/marketplace.json` is enough, and it can be private.

## Versioning

Three things version independently, and all three have to agree:

| | Versioned by | Skew shows up as |
|---|---|---|
| These skills | `version` in `.claude-plugin/plugin.json` | a skill naming a tool that no longer exists |
| `resonance-mcp` | this checkout | — |
| **The running app** | whatever the user has open | a tool answering `unsupported` |

Living in this repo pins the first two together. `resonance-mcp/tests/agent_plugin_lockstep.rs`
enforces it:

- `control_protocol_version` in `lockstep.json` must equal
  `resonance_control::PROTOCOL_VERSION`, so a protocol bump fails the suite until
  someone re-reads the skills. (It lives in its own file because the plugin
  manifest schema has no field for it — `claude plugin validate` warns on
  unknown manifest keys.)
- every `mcp__resonance__<tool>` named anywhere in this directory must exist in
  the router, so a renamed or removed tool fails the suite instead of failing
  mid-mix;
- every `SKILL.md` must call `control_hello`.

That last one covers what versioning cannot. The app is a separate binary from
the MCP server, so the plugin's version says nothing about the build the user
actually has open. `control_hello` returns that build's `capabilities`, and both
skills open by checking the methods they need against it and stopping if one is
missing. Do not remove that step.

When you bump `PROTOCOL_VERSION`, bump `version` here too — installed copies
only pick up changes when that field moves.

## Writing more skills

Skills here are per-**craft**, never per-**project**. A skill that names a track,
a track count or a genre is a bug: the way to stay project-agnostic is to open
by reading (`song_summary`, `song_tracks`, `master_summary`) and branch on what
comes back. Song-specific conventions belong in that song's directory — a
`CLAUDE.md`, or a nested `.claude/skills/` if it genuinely needs a procedural
override — not in this plugin.
