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
- `search_rank`
- `graph_context`
- `explain_node`
- `answer_with_context`
- `llm_explain_node`
- `llm_impact_summary`
- `explain_symbol`
- `impact_analysis`
- `impact`
- `architecture_map`
- `watch_project`
- `watch_status`
- `stop_watch`
- `diff_context`
- `search_embeddings`

Optional LLM explanation tools use an OpenAI-compatible provider configured via
`GITNOVA_LLM_API_KEY`, `GITNOVA_LLM_BASE_URL`, and `GITNOVA_LLM_MODEL`. If the
provider is not configured or fails, these tools return deterministic fallback
answers with the same structured evidence.

Resources:

- `gitnova://graph/summary`
- `gitnova://graph/nodes`
- `gitnova://graph/edges`
- `gitnova://graph/hubs`
- `gitnova://graph/recent-churn`
- `gitnova://graph/dashboard-url`

Tool outputs are JSON text content. The server does not write logs to stdout in
MCP mode.
