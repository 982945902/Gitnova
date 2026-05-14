# Architecture

Gitnova indexes a repository into a durable local graph:

```text
repo -> scan -> tree-sitter parse -> extraction -> graph -> enrichment -> ranking -> CLI/MCP/dashboard
```

The workspace is split into focused crates:

- `gitnova-core`: scanning, tree-sitter parse validation, AST-first symbol extraction, graph model, and graph queries.
- `gitnova-storage`: SQLite migrations, graph save/load, JSON export, file manifest, and embedding storage.
- `gitnova-rank`: salience feature extraction, scoring, explanations, and diff-aware ranking.
- `gitnova-enrich`: best-effort git churn, opt-in LSP definition/reference probing with timeouts, local embeddings, and command-backed neural embeddings.
- `gitnova-mcp`: rmcp-backed MCP stdio server, legacy line-mode compatibility, tools, and resources.
- `gitnova-dashboard`: Axum server with plain HTML/CSS/JS dashboard assets, ranked search, and an interactive graph canvas.
- `gitnova-cli`: command-line product surface.

The ranker intentionally penalizes generic high-degree utility nodes unless the
query explicitly overlaps the utility. If embeddings or LSP signals are absent,
their weights are redistributed into local lexical and graph features.
