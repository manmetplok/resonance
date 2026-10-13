# Control API

The machine control surface: a running Resonance serves JSON-RPC on a
per-user unix socket, `resonance-control` defines that wire contract,
the app's control layer (`control_socket.rs`, `control_jobs.rs`,
`state/control.rs`, `update/control/**`) executes each call through the
normal update and undo path, `resonance-mcp` republishes every method as
one MCP tool, and `resonance-agent-plugin` ships the skills that teach an
agent to use those tools. The docs here cover that plumbing and the
catalogue; each method's behaviour is documented by the feature area
that owns it.

| Feature | Doc | One-line summary |
|---|---|---|
| Control API architecture | [overview.md](overview.md) | Socket, threads, dispatch order, mutation gate, one-undo-per-call, revision counter, remote indicator. |
| Protocol and conventions | [protocol.md](protocol.md) | JSON-RPC envelope, framing, error kinds, `PROTOCOL_VERSION` and capabilities, strict params, ids and positions, the vertical-slice rule. |
| Method catalogue | [method-catalogue.md](method-catalogue.md) | All 187 methods with their MCP tool, kind and owning area. |
| Jobs | [jobs.md](jobs.md) | `job.status` / `job.wait`, which calls are jobs, retention, disconnects, MCP-side waiting. |
| MCP server | [mcp-server.md](mcp-server.md) | `resonance-mcp`: registration, tool naming, schema normalisation, error mapping, connection handling. |
| Agent plugin | [agent-plugin.md](agent-plugin.md) | The seven skills, their rules, and the lockstep test that keeps them in step with the tools. |

Rules for adding a method: [docs/guides/control-api-conventions.md](../../guides/control-api-conventions.md).
Open work: [docs/intents/control-api/](../../intents/control-api/).
