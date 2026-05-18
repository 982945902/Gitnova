# Gitnova

Gitnova is a local-first MCP code intelligence system for Rust, TypeScript,
JavaScript, Python, and C++ repositories. It scans source files with `.gitignore`
support, parses with tree-sitter, stores a graph in SQLite, ranks context by
architectural salience, and exposes the graph through a CLI, rmcp-backed MCP
stdio server, and local dashboard.

## Quick Start

```bash
cargo run -p gitnova -- index /path/to/repo --force
cargo run -p gitnova -- stats --repo /path/to/repo
cargo run -p gitnova -- rank-context "change auth validation" --repo /path/to/repo --limit 10
cargo run -p gitnova -- serve
cargo run -p gitnova -- dashboard --repo /path/to/repo --port 4567
```

Data is written to `.gitnova/gitnova.db` and `.gitnova/index.json` inside the
indexed repository. Core indexing, ranking, storage, CLI, MCP, and dashboard
workflows work offline. LSP probing is opt-in with `GITNOVA_LSP_PROBE=1` and
uses project config plus timeouts before attempting installed language servers.
Embedding providers include deterministic local options and a command-backed
model provider via `GITNOVA_EMBEDDING_COMMAND`.

## Commands

```text
gitnova index <path> [--force]
gitnova update --repo <path>
gitnova watch --repo <path>
gitnova stats --repo <path>
gitnova rank-context "query" --repo <path> --limit 10
gitnova explain-symbol "symbol" --repo <path>
gitnova graph-context "symbol-or-node-id" --repo <path> --depth 1 --limit 40
gitnova impact-analysis "symbol" --repo <path> --limit 20
gitnova architecture-map --repo <path> --focus auth
gitnova diff-context --repo <path> [--base main]
gitnova embeddings build --repo <path> --provider local-hash
gitnova embeddings build --repo <path> --provider local-semantic
GITNOVA_EMBEDDING_COMMAND=/path/to/embedder gitnova embeddings build --repo <path> --provider neural-command
gitnova dashboard --repo <path> --port 4567
gitnova serve
```

For the product demo loop, use `rank-context` to find the most relevant code,
then `graph-context` to return the focused node, source snippet, immediate
relationships, and graph edges that the Web showcase can highlight.

## Optional LLM Layer

Gitnova's LLM support is an explanation layer over deterministic evidence.
Indexing, ranking, graph context, impact analysis, MCP, and the dashboard still
work without an API key. Configure an OpenAI-compatible chat completions
provider only when you want natural-language wording:

```bash
GITNOVA_LLM_API_KEY=...
GITNOVA_LLM_BASE_URL=https://api.openai.com/v1
GITNOVA_LLM_MODEL=...
```

MCP tools such as `answer_with_context`, `llm_explain_node`, and
`llm_impact_summary` always return structured `evidence` with node ids, paths,
spans, qualified names, and source snippets. Without LLM config they return a
deterministic fallback answer using the same evidence.
