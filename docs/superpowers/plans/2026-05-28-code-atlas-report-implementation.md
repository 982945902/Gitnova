# Code Atlas Report Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a first-pass `gitnova atlas` command that emits a deterministic flow-first code atlas JSON file and single-file HTML report.

**Architecture:** Add a focused `gitnova-atlas` crate that consumes `gitnova_core::CodeGraph`, identifies useful entrypoints, expands bounded execution flows, and renders a standalone HTML artifact. The CLI loads the graph through the existing SurrealDB/index.json fallback and delegates all report generation to the new crate.

**Tech Stack:** Rust 2021, serde/serde_json, clap, existing Gitnova graph/query model, assert_cmd integration tests.

---

### Task 1: Add Atlas Data Model And Flow Builder

**Files:**
- Create: `crates/gitnova-atlas/Cargo.toml`
- Create: `crates/gitnova-atlas/src/lib.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Write unit tests for entry-first atlas generation**

Create tests in `crates/gitnova-atlas/src/lib.rs` that build a small graph with `main -> create_app -> handle_request -> read_index` calls and assert that `build_atlas` returns one flow rooted at `main`, with bounded children and role labels.

- [ ] **Step 2: Run unit tests and verify failure**

Run: `cargo test -p gitnova-atlas`
Expected: FAIL before the crate exists.

- [ ] **Step 3: Implement minimal atlas structs and builder**

Implement `AtlasReport`, `AtlasFlow`, `AtlasNode`, `AtlasOptions`, and `build_atlas`. The builder should prefer entrypoint-like names (`main`, `run`, `start`, `serve`, `init`, `create`, `open`, `handler`, `executor`, `reader`, `writer`, `builder`) and expand only call/reference/contains/defines/extends edges with depth and node budgets.

- [ ] **Step 4: Run unit tests and verify pass**

Run: `cargo test -p gitnova-atlas`
Expected: PASS.

### Task 2: Render A Standalone HTML Report

**Files:**
- Modify: `crates/gitnova-atlas/src/lib.rs`

- [ ] **Step 1: Write unit test for HTML self-containment**

Add a test that calls `render_html(&report)` and asserts the output contains `<!doctype html>`, `Havenask Code Atlas` or `Code Atlas`, `application/json`, `atlas-data`, and a flow name.

- [ ] **Step 2: Run unit tests and verify failure**

Run: `cargo test -p gitnova-atlas`
Expected: FAIL until `render_html` exists.

- [ ] **Step 3: Implement `render_html`**

Render one HTML string with inline CSS, inline JavaScript, and embedded JSON in `<script id="atlas-data" type="application/json">`. The first screen should prioritize flow cards and summary metrics; the detail area should show a collapsible tree and source metadata.

- [ ] **Step 4: Run unit tests and verify pass**

Run: `cargo test -p gitnova-atlas`
Expected: PASS.

### Task 3: Add CLI Command

**Files:**
- Modify: `crates/gitnova-cli/Cargo.toml`
- Modify: `crates/gitnova-cli/src/main.rs`
- Modify: `tests/integration/cli_tests.rs`

- [ ] **Step 1: Write integration test**

Add a test that indexes `tests/fixtures/ts_sample`, runs `gitnova atlas --repo <repo> --entry auth --output <tmp>/atlas.html --emit-json <tmp>/atlas.json --max-flows 3 --max-depth 3`, and asserts both files exist. Assert JSON has `flows`, HTML has `atlas-data`, and at least one flow references auth/session/validate.

- [ ] **Step 2: Run focused integration test and verify failure**

Run: `cargo test -p gitnova --test full_challenge atlas_generates_single_file_html_report`
Expected: FAIL until CLI support exists.

- [ ] **Step 3: Add `atlas` subcommand**

Add `AtlasArgs` with `--repo`, `--entry`, `--output`, `--emit-json`, `--max-flows`, `--max-depth`, and `--max-nodes-per-flow`. Load the graph with `load_graph_with_fallback`, call `gitnova_atlas::build_atlas`, write JSON when requested, and write HTML when `--output` is set.

- [ ] **Step 4: Run focused integration test and verify pass**

Run: `cargo test -p gitnova --test full_challenge atlas_generates_single_file_html_report`
Expected: PASS.

### Task 4: Verify Existing Workflows

**Files:**
- No new files.

- [ ] **Step 1: Format**

Run: `cargo fmt --check`
Expected: PASS.

- [ ] **Step 2: Test atlas crate**

Run: `cargo test -p gitnova-atlas`
Expected: PASS.

- [ ] **Step 3: Test focused CLI flows**

Run: `cargo test -p gitnova --test full_challenge cli_indexes_persists_and_ranks_rust_auth_fixture atlas_generates_single_file_html_report`
Expected: PASS.

### Self-Review

- Spec coverage: Covers Gitnova-owned atlas generation, flow-first/deterministic output, JSON and single-file HTML artifacts.
- Placeholder scan: No TBD/TODO placeholders remain.
- Type consistency: `AtlasOptions`, `AtlasReport`, `build_atlas`, and `render_html` are consistently named across tasks.
