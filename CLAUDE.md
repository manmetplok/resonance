# Resonance — collaboration notes

## UI work routes through the `ux-design` agent

Any task that adds, modifies, restyles, or repositions a view, layout,
panel, control, button, knob, meter, dialog, menu, or other visual
element MUST be delegated to the `ux-design` agent (via the Agent tool,
`subagent_type: ux-design`) before edits land. This is non-negotiable —
it's how we keep theme usage, layout constants, performance rules, and
`iced_test` snapshot discipline consistent across the codebase.

Triggering file paths (non-exhaustive):

- `resonance-app/src/view/**`
- `resonance-app/src/theme.rs`
- `resonance-app/src/timeline*.rs`, `resonance-app/src/timeline_draw.rs`
- `plugins/*/src/editor/**`
- `wayland-plugin-gui/src/**`

Triggering vocabulary: "layout", "spacing", "color", "alignment",
"font", "hover/pressed/active state", "redesign", "restyle", "mockup".

The agent's instructions live in `.claude/agents/ux-design.md`.

## Visual verification routes through the `e2e-tester` agent

Once a UI change has landed, verification (running and/or creating
`iced_test` headless snapshot tests) is delegated to the `e2e-tester`
agent (`.claude/agents/e2e-tester.md`). The `programmer` agent invokes
it as the final step of every task; if you're driving UI work outside
the `programmer` flow, invoke `e2e-tester` yourself. Triggering
vocabulary: "screenshot", "snapshot test", "iced_test", "golden image".
