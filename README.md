# Gitnova

Gitnova is a local-first MCP code intelligence system for Rust, TypeScript,
JavaScript, and Python repositories. It scans source files with `.gitignore`
support, parses with tree-sitter, stores a graph in SQLite, ranks context by
architectural salience, and exposes the graph through a CLI, MCP stdio server,
and local dashboard.

## Quick Start

```bash
cargo run -p gitnova -- index /path/to/repo --force
cargo run -p gitnova -- stats --repo /path/to/repo
cargo run -p gitnova -- rank-context "change auth validation" --repo /path/to/repo --limit 10
cargo run -p gitnova -- serve
cargo run -p gitnova -- dashboard --repo /path/to/repo --port 4567
```

Data is written to `.gitnova/gitnova.db` and `.gitnova/index.json` inside the
indexed repository. Embeddings and LSP enrichment are best-effort local features;
core indexing, ranking, storage, CLI, MCP, and dashboard workflows work offline.

## Commands

```text
gitnova index <path> [--force]
gitnova update --repo <path>
gitnova watch --repo <path>
gitnova stats --repo <path>
gitnova rank-context "query" --repo <path> --limit 10
gitnova explain-symbol "symbol" --repo <path>
gitnova impact-analysis "symbol" --repo <path> --limit 20
gitnova architecture-map --repo <path> --focus auth
gitnova diff-context --repo <path> [--base main]
gitnova embeddings build --repo <path> --provider local-hash
gitnova dashboard --repo <path> --port 4567
gitnova serve
```

