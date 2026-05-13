# Gitnova Full Challenge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Gitnova as a local-first GitNexus-class MCP code intelligence system with tree-sitter indexing, SQLite persistence, salience ranking, optional embeddings, LSP enrichment, git churn signals, watch mode, diff-aware ranking, MCP resources, and a lightweight web dashboard.

**Architecture:** Gitnova is a Rust workspace with separate crates for core graph/indexing, storage, ranking, LSP/git enrichment, embeddings, MCP, CLI, and dashboard serving. The system must remain useful without network access: tree-sitter graph indexing, SQLite storage, salience ranking, CLI, MCP tools, and dashboard are mandatory; embeddings and LSP are optional enrichments that degrade gracefully when local providers or language servers are unavailable.

**Tech Stack:** Rust 2021, `tokio`, `clap`, `serde`, `serde_json`, `thiserror`, `anyhow`, `ignore`, `walkdir`, `tree-sitter`, `rusqlite`, `notify`, `git2`, `rmcp`, `schemars`, `tower`, `axum`, `tower-http`, optional `lsp-types`, optional local embedding provider abstraction, plain HTML/CSS/JS dashboard served by Axum.

---

## 0. Codex Goal Prompt

Paste this into Codex Goal from the root of the new `Gitnova` repository:

```text
Implement Gitnova Full Challenge end to end from docs/GITNOVA_FULL_CHALLENGE_CODEX_GOAL.md.

Build a Rust workspace that provides:
- local repository scanning with .gitignore support
- tree-sitter indexing for Rust, TypeScript/JavaScript, and Python
- SQLite-backed graph storage plus JSON export/import
- salience ranking that downranks generic high-degree utilities unless the query explicitly asks for them
- optional local embedding abstraction blended with salience ranking
- optional LSP enrichment for definitions/references when language servers are available
- git history/churn features through git2
- incremental indexing and watch mode
- PR/diff-aware ranking from git diff
- MCP stdio server with tools and resources
- CLI for all core workflows
- lightweight local web dashboard for graph inspection

Constraints:
- Follow this plan task by task.
- Use TDD for each behavior.
- Keep each crate focused.
- Must work offline with no external database and no mandatory LLM/API calls.
- Embeddings and LSP must be optional and degrade gracefully.
- Do not block core indexing if optional enrichers fail.
- Commit after each completed task.
- Final verification must run cargo fmt --check, cargo test, cargo clippy -- -D warnings, CLI smoke tests, MCP smoke tests, and dashboard smoke test.
```

## 1. Product Definition

Gitnova is a local-first MCP code intelligence system. It gives coding agents a durable, queryable model of a repository:

```text
repo -> scan -> tree-sitter AST -> code graph -> enrichment -> salience ranker -> MCP/CLI/dashboard
```

The core thesis:

```text
Code retrieval should rank by architectural salience, not raw graph degree.
```

High-degree utilities such as `utils`, `logger`, `types`, `constants`, and `config` should not dominate context retrieval unless the query explicitly asks for them.

## 2. Full Challenge Scope

### Mandatory

- Rust workspace.
- CLI binary named `gitnova`.
- MCP stdio server.
- Local SQLite storage at `.gitnova/gitnova.db`.
- JSON export at `.gitnova/index.json`.
- Tree-sitter parsing for Rust, TypeScript, JavaScript, Python.
- Graph nodes: repository, file, module, function, method, class, struct, trait/interface, import.
- Graph edges: contains, defines, imports, calls, references.
- Salience ranker.
- Git churn/recency features.
- Incremental indexing.
- Watch mode.
- Diff-aware ranking.
- MCP tools.
- MCP resources.
- Lightweight dashboard.
- Tests and fixtures.

### Optional But Implemented

These must be implemented as best-effort features:

- LSP enrichment.
- Embedding search.

They must not be required for normal operation.

### Explicitly Not Required

- Hosted SaaS.
- Authentication.
- Multi-user collaboration.
- Remote GitHub app.
- Cloud embeddings.
- Full semantic type resolution equal to an IDE.

## 3. Workspace Layout

Create:

```text
Gitnova/
  Cargo.toml
  README.md
  docs/
    GITNOVA_FULL_CHALLENGE_CODEX_GOAL.md
    ARCHITECTURE.md
    MCP_CLIENT_CONFIG.md
    DASHBOARD.md
  crates/
    gitnova-core/
      src/
        lib.rs
        error.rs
        model.rs
        language.rs
        scan.rs
        parser.rs
        extract/
          mod.rs
          rust.rs
          typescript.rs
          javascript.rs
          python.rs
        graph.rs
        query.rs
    gitnova-storage/
      src/
        lib.rs
        sqlite.rs
        json_export.rs
        migrations.rs
    gitnova-rank/
      src/
        lib.rs
        features.rs
        score.rs
        explain.rs
        diff.rs
    gitnova-enrich/
      src/
        lib.rs
        git.rs
        lsp.rs
        embeddings.rs
    gitnova-mcp/
      src/
        lib.rs
        server.rs
        tools.rs
        resources.rs
        schema.rs
    gitnova-dashboard/
      src/
        lib.rs
        server.rs
      assets/
        index.html
        app.js
        styles.css
    gitnova-cli/
      src/
        main.rs
  tests/
    fixtures/
      rust_sample/
      ts_sample/
      py_sample/
      mixed_repo/
    integration/
      cli_tests.rs
      mcp_contract_tests.rs
      dashboard_tests.rs
```

Top-level `Cargo.toml`:

```toml
[workspace]
members = [
    "crates/gitnova-core",
    "crates/gitnova-storage",
    "crates/gitnova-rank",
    "crates/gitnova-enrich",
    "crates/gitnova-mcp",
    "crates/gitnova-dashboard",
    "crates/gitnova-cli"
]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
anyhow = "1.0"
thiserror = "2.0"
tokio = { version = "1.39", features = ["full"] }
clap = { version = "4.5", features = ["derive"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
schemars = "1.0"
rmcp = { version = "0.12.0", features = ["server", "transport-io"] }
walkdir = "2.5"
ignore = "0.4"
regex = "1.12"
sha2 = "0.10"
uuid = { version = "1.6", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
rusqlite = { version = "0.32", features = ["bundled"] }
notify = "8.2"
git2 = "0.20"
tree-sitter = "0.25"
tree-sitter-rust = "0.24"
tree-sitter-javascript = "0.25"
tree-sitter-typescript = "0.23"
tree-sitter-python = "0.25"
lsp-types = "0.97"
axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["fs", "cors"] }
tempfile = "3.23"
assert_cmd = "2.0"
predicates = "3.1"
```

## 4. Milestones

### Milestone 1: Offline Core

Outcome:

- scan
- parse
- extract
- build graph
- write SQLite and JSON
- rank context
- CLI

This milestone must pass before MCP, LSP, embeddings, watch, or dashboard work begins.

### Milestone 2: MCP Product Surface

Outcome:

- MCP stdio server
- MCP tools
- MCP resources
- client config docs

### Milestone 3: Enrichment

Outcome:

- git churn/recency
- diff-aware ranking
- optional LSP enrichment
- optional embeddings

### Milestone 4: Continuous Local Experience

Outcome:

- incremental indexing
- watch mode
- dashboard

## 5. Data Model

Core model in `gitnova-core/src/model.rs`:

```rust
pub type NodeId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeGraph {
    pub schema_version: u32,
    pub repo_root: String,
    pub indexed_at_unix: u64,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub path: String,
    pub span: Option<Span>,
    pub language: Option<Language>,
    pub text: String,
    pub tags: Vec<String>,
    pub metrics: NodeMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeMetrics {
    pub in_degree: usize,
    pub out_degree: usize,
    pub churn_90d: u32,
    pub last_changed_unix: Option<u64>,
    pub is_test: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Repository,
    File,
    Module,
    Function,
    Method,
    Class,
    Struct,
    Trait,
    Interface,
    Import,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub confidence_basis_points: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Defines,
    Imports,
    Calls,
    References,
}
```

## 6. SQLite Schema

Implement migrations in `gitnova-storage/src/migrations.rs`.

Tables:

```sql
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS nodes (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  qualified_name TEXT NOT NULL,
  path TEXT NOT NULL,
  language TEXT,
  text TEXT NOT NULL,
  start_line INTEGER,
  start_col INTEGER,
  end_line INTEGER,
  end_col INTEGER,
  tags_json TEXT NOT NULL,
  metrics_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS edges (
  from_id TEXT NOT NULL,
  to_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  confidence_basis_points INTEGER NOT NULL,
  PRIMARY KEY (from_id, to_id, kind)
);

CREATE INDEX IF NOT EXISTS idx_nodes_path ON nodes(path);
CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);
CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id);

CREATE TABLE IF NOT EXISTS file_manifest (
  path TEXT PRIMARY KEY,
  content_hash TEXT NOT NULL,
  language TEXT NOT NULL,
  indexed_at_unix INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS embeddings (
  node_id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  dimension INTEGER NOT NULL,
  vector_json TEXT NOT NULL
);
```

## 7. Salience Ranking

Formula:

```text
score =
  lexical_weight          * lexical_match
+ query_overlap_weight    * query_overlap
+ relation_weight         * relation_importance
+ proximity_weight        * graph_proximity
+ domain_weight           * domain_specificity
+ churn_weight            * churn
+ embedding_weight        * embedding_similarity
+ lsp_weight              * lsp_confidence
- hub_penalty_weight      * hub_penalty
- utility_penalty_weight  * utility_penalty
- test_penalty_weight     * test_penalty
```

Default:

```rust
pub struct RankConfig {
    pub lexical_weight: f64,          // 0.25
    pub query_overlap_weight: f64,    // 0.18
    pub relation_weight: f64,         // 0.12
    pub proximity_weight: f64,        // 0.10
    pub domain_weight: f64,           // 0.10
    pub churn_weight: f64,            // 0.08
    pub embedding_weight: f64,        // 0.12
    pub lsp_weight: f64,              // 0.05
    pub hub_penalty_weight: f64,      // 0.20
    pub utility_penalty_weight: f64,  // 0.20
    pub test_penalty_weight: f64,     // 0.08
}
```

If embeddings are unavailable, redistribute `embedding_weight` across lexical and query overlap. If LSP is unavailable, redistribute `lsp_weight` across relation and proximity.

Utility penalty is soft:

```text
if query_overlap >= 0.50:
  utility_penalty *= 0.25
```

## 8. MCP Surface

Tools:

- `index_project`
- `rank_context`
- `explain_symbol`
- `impact_analysis`
- `architecture_map`
- `watch_project`
- `diff_context`
- `search_embeddings`

Resources:

- `gitnova://graph/summary`
- `gitnova://graph/nodes`
- `gitnova://graph/edges`
- `gitnova://graph/hubs`
- `gitnova://graph/recent-churn`
- `gitnova://graph/dashboard-url`

All MCP tool outputs must be JSON text. Do not print logs to stdout in MCP mode.

## 9. CLI Surface

Commands:

```bash
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

Use `local-hash` embeddings for V0.2 if no real local embedding model is available. It should be deterministic lexical hashing, clearly documented as a baseline, and replaceable later.

## 10. Task Plan

### Task 1: Scaffold Workspace

Create the full workspace and compiling crates.

Acceptance:

- `cargo test` passes.
- `cargo run -p gitnova -- --help` works.
- Commit: `feat: scaffold gitnova workspace`

### Task 2: Core Model

Implement graph model, language detection, stable IDs, and typed errors.

Acceptance:

- model serialization tests pass.
- IDs are deterministic for fixture symbols.
- Commit: `feat: define gitnova graph model`

### Task 3: Scanner

Implement repo scanning with `ignore`.

Acceptance:

- respects `.gitignore`.
- skips `.git`, `target`, `node_modules`, `.venv`, `dist`, `build`.
- detects Rust, TS, JS, Python.
- Commit: `feat: scan repositories`

### Task 4: Tree-sitter Parsing

Implement parser abstraction for Rust, TS, JS, Python.

Acceptance:

- parses all fixture files.
- invalid files produce typed non-fatal parse diagnostics.
- Commit: `feat: parse source with tree-sitter`

### Task 5: Symbol Extraction

Extract functions, methods, classes, structs, traits/interfaces, imports, and simple calls.

Acceptance:

- fixtures produce expected symbols and spans.
- call/reference extraction is heuristic but tested.
- Commit: `feat: extract symbols from source`

### Task 6: Graph Builder

Build repository/file/symbol graph and deduplicate edges.

Acceptance:

- contains/defines/imports/calls/references edges exist in fixtures.
- duplicate edges are removed.
- Commit: `feat: build code graph`

### Task 7: SQLite Storage

Implement SQLite migrations, graph save/load, and JSON export.

Acceptance:

- `.gitnova/gitnova.db` created.
- `.gitnova/index.json` exported.
- save/load round-trip passes.
- Commit: `feat: persist graph to sqlite`

### Task 8: Git Churn Enrichment

Use `git2` to compute churn and last changed timestamp per file.

Acceptance:

- works inside git fixture repo.
- non-git repo falls back to zero churn without failure.
- Commit: `feat: enrich graph with git churn`

### Task 9: Salience Ranking

Implement ranking features, scoring, explanations, and utility override.

Acceptance:

- domain auth node outranks high-degree utility for auth query.
- utility wins when query targets it.
- explanation mentions strong and weak signals.
- Commit: `feat: rank context by salience`

### Task 10: Core Query Workflows

Implement:

- rank context
- explain symbol
- impact analysis
- architecture map
- diff context

Acceptance:

- all workflows return typed JSON-ready structs.
- reverse dependency impact works for utility fixture.
- Commit: `feat: add graph query workflows`

### Task 11: CLI

Implement all mandatory CLI commands except dashboard/watch/embeddings.

Acceptance:

- `index`, `stats`, `rank-context`, `explain-symbol`, `impact-analysis`, `architecture-map`, `diff-context`, `serve` exist.
- CLI integration tests pass.
- Commit: `feat: add gitnova cli`

### Task 12: MCP Tools

Implement MCP stdio server and tools:

- `index_project`
- `rank_context`
- `explain_symbol`
- `impact_analysis`
- `architecture_map`
- `diff_context`

Acceptance:

- contract tests validate request/response schema.
- server emits no non-protocol stdout logs.
- Commit: `feat: expose gitnova mcp tools`

### Task 13: MCP Resources

Implement resources:

- graph summary
- nodes
- edges
- hubs
- recent churn
- dashboard URL

Acceptance:

- resources list and read in contract tests.
- Commit: `feat: expose gitnova mcp resources`

### Task 14: Incremental Indexing

Implement file manifest hash tracking and `gitnova update`.

Acceptance:

- unchanged files are skipped.
- changed files are reindexed.
- deleted files remove nodes/edges.
- Commit: `feat: add incremental indexing`

### Task 15: Watch Mode

Use `notify` to watch source files and run incremental updates.

Acceptance:

- watch loop responds to file create/modify/delete in tests or manual smoke.
- MCP `watch_project` starts a watch task and reports status.
- Commit: `feat: watch projects for graph updates`

### Task 16: LSP Enrichment

Implement best-effort LSP enrichment.

Supported tools:

- Rust: `rust-analyzer`
- TS/JS: `typescript-language-server`
- Python: `pyright-langserver`

Rules:

- Detect missing tools.
- Do not fail indexing if unavailable.
- Add enrichment metadata and higher confidence references when available.

Acceptance:

- unit tests cover missing-tool graceful fallback.
- integration is feature-gated or best-effort.
- Commit: `feat: add optional lsp enrichment`

### Task 17: Embedding Abstraction

Implement deterministic local-hash embeddings first.

Rules:

- No network.
- Store vectors in SQLite as JSON.
- Blend embedding similarity into ranker.
- CLI command: `gitnova embeddings build --repo <path> --provider local-hash`.
- MCP tool: `search_embeddings`.

Acceptance:

- embeddings build for fixtures.
- search returns stable results.
- ranker works with and without embeddings.
- Commit: `feat: add local embedding search`

### Task 18: Dashboard

Implement local dashboard served by Axum.

Routes:

- `/`
- `/api/summary`
- `/api/nodes`
- `/api/edges`
- `/api/rank?query=...`
- `/api/hubs`
- `/api/churn`

UI requirements:

- search box
- ranked results list
- node detail pane
- hub list
- architecture summary
- no build step; plain HTML/CSS/JS

Acceptance:

- `gitnova dashboard --repo <path> --port 4567` starts.
- `/api/summary` returns JSON.
- dashboard smoke test passes.
- Commit: `feat: add local dashboard`

### Task 19: Documentation

Write:

- `README.md`
- `docs/ARCHITECTURE.md`
- `docs/MCP_CLIENT_CONFIG.md`
- `docs/DASHBOARD.md`

Acceptance:

- docs match implemented CLI and MCP tools.
- Commit: `docs: document gitnova full challenge`

### Task 20: Final Verification

Run:

```bash
cargo fmt --check
cargo test
cargo clippy -- -D warnings
cargo run -p gitnova -- index tests/fixtures/rust_sample --force
cargo run -p gitnova -- stats --repo tests/fixtures/rust_sample
cargo run -p gitnova -- rank-context "change auth validation" --repo tests/fixtures/rust_sample --limit 5
cargo run -p gitnova -- explain-symbol "AuthService::validate" --repo tests/fixtures/rust_sample
cargo run -p gitnova -- impact-analysis "formatDate" --repo tests/fixtures/ts_sample --limit 10
cargo run -p gitnova -- diff-context --repo tests/fixtures/rust_sample
cargo run -p gitnova -- embeddings build --repo tests/fixtures/rust_sample --provider local-hash
cargo run -p gitnova -- dashboard --repo tests/fixtures/rust_sample --port 4567
```

For dashboard smoke, start the server, fetch `http://127.0.0.1:4567/api/summary`, verify JSON, then stop the server.

Acceptance:

- all commands pass.
- MCP server starts.
- dashboard starts.
- high-degree utility behavior is proven by tests.
- final commit: `chore: verify gitnova full challenge`

## 11. Fixture Requirements

Create fixtures proving:

- Rust auth service and controller.
- TS utility hub with `formatDate` imported by many files.
- Python payment reconciliation domain service.
- Mixed repo architecture map.
- Git fixture with multiple commits for churn tests.

The TS utility fixture is mandatory because it proves Gitnova's core ranking thesis.

## 12. Risk Controls

If Codex Goal hits time or complexity pressure, preserve this priority order:

1. Tree-sitter graph.
2. SQLite persistence.
3. Salience ranker.
4. CLI.
5. MCP tools.
6. Git churn and diff context.
7. MCP resources.
8. Incremental indexing.
9. Watch mode.
10. Embeddings.
11. LSP.
12. Dashboard.

Never sacrifice the MCP + salience ranking core for optional enrichment.

## 13. Acceptance Criteria

Gitnova Full Challenge is complete when:

- A local repo can be indexed into SQLite.
- JSON export works.
- CLI workflows work.
- MCP tools work.
- MCP resources work.
- Tree-sitter supports Rust, TS/JS, Python fixtures.
- Salience ranking proves utility downranking and query-conditioned promotion.
- Git churn and diff-aware ranking influence results.
- Incremental indexing updates changed files.
- Watch mode updates the graph after file changes.
- Embedding search works with local-hash provider.
- LSP gracefully enriches when available and gracefully skips when unavailable.
- Dashboard shows summary, nodes, edges, hubs, and ranked search results.
- `cargo fmt --check`, `cargo test`, and `cargo clippy -- -D warnings` pass.
