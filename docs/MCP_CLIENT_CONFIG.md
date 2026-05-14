# MCP Client Config

Run the Gitnova MCP server from the workspace. The default stdio server is
rmcp-backed:

```bash
cargo run -p gitnova -- serve
```

Set `GITNOVA_REPO` when a client should default to a specific indexed
repository:

```bash
GITNOVA_REPO=/path/to/repo cargo run -p gitnova -- serve
```

For older line-oriented test clients, set `GITNOVA_USE_LEGACY_STDIO=1`.

Tools:

- `index_project`
- `rank_context`
- `explain_symbol`
- `impact_analysis`
- `architecture_map`
- `watch_project`
- `watch_status`
- `stop_watch`
- `diff_context`
- `search_embeddings`

Resources:

- `gitnova://graph/summary`
- `gitnova://graph/nodes`
- `gitnova://graph/edges`
- `gitnova://graph/hubs`
- `gitnova://graph/recent-churn`
- `gitnova://graph/dashboard-url`

Tool outputs are JSON text content. The server does not write logs to stdout in
MCP mode.
