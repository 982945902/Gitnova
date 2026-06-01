use std::path::PathBuf;

use gitnova_mcp::tools::call_tool;
use serde_json::{json, Value};

fn main() -> anyhow::Result<()> {
    let repo = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/lishuo121/workspace/havenask"));
    let wiki_root = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join(".gitnova/wiki/ha3-atlas-exp"));
    let codex_bin = std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".to_string());

    if wiki_root.exists() {
        std::fs::remove_dir_all(&wiki_root)?;
    }

    let mut repo_state = repo.clone();
    let applied = call_tool(
        "wiki_apply_outline",
        &json!({
            "wiki_root": wiki_root,
            "outline": {
                "root": "ha3",
                "pages": [
                    {
                        "id": "ha3",
                        "title": "HA3 Search Layer",
                        "kind": "index",
                        "summary": "Query-facing search layer for parsing, execution, filtering, and ranking.",
                        "purpose": "Give readers a code-derived map of HA3 before deep pages are filled in.",
                        "content": "## Reading Path\nStart with Query Executors to understand how query semantics become doc-id seeking and matching.",
                        "deep_tasks": []
                    },
                    {
                        "id": "ha3/search/query-executors",
                        "title": "Query Executors",
                        "kind": "article",
                        "parent": "ha3",
                        "summary": "Executor families and dispatch from QueryExecutorCreator.",
                        "purpose": "Explain how QueryExecutorCreator creates executor families and what index reader APIs they depend on.",
                        "content": "## Skeleton\nThis page is intentionally sparse before Atlas investigates the code.",
                        "deep_tasks": [
                            {
                                "id": "trace-query-executor-creator",
                                "question": "Read only QueryExecutorCreator.h and QueryExecutorCreator.cpp. Summarize the main visitor methods and factory helpers that create query executors. Explain the high-level dispatch from query type to executor type, with source spans and one small diagram if useful. Do not chase every executor implementation; put deeper executor-family details in followups.",
                                "scope_paths": [
                                    "aios/ha3/ha3/search/query_executor/QueryExecutorCreator.h",
                                    "aios/ha3/ha3/search/query_executor/QueryExecutorCreator.cpp"
                                ],
                                "scope_symbols": [
                                    "QueryExecutorCreator",
                                    "QueryExecutor",
                                    "createTermQueryExecutor",
                                    "visitTermQuery",
                                    "visitPhraseQuery",
                                    "visitAndQuery",
                                    "visitOrQuery"
                                ],
                                "expected_outputs": ["summary", "sources", "diagram", "followups"]
                            }
                        ]
                    }
                ]
            }
        }),
        &mut repo_state,
    )?;
    println!("apply_outline: {}", payload(applied)?);

    let expanded = call_tool(
        "wiki_expand_tree",
        &json!({
            "wiki_root": wiki_root,
            "repo_path": repo,
            "root_page_id": "ha3",
            "task_limit": 1,
            "parallelism": 1,
            "codex_bin": codex_bin,
            "timeout_secs": 240,
            "max_nodes": 24,
            "max_depth": 2
        }),
        &mut repo_state,
    )?;
    println!("expand_tree: {}", payload(expanded)?);
    println!(
        "index: {}",
        repo_state
            .join(".gitnova/wiki/ha3-atlas-exp/pages/ha3/index.html")
            .display()
    );
    println!(
        "query_executors: {}",
        repo_state
            .join(".gitnova/wiki/ha3-atlas-exp/pages/ha3/search/query-executors.html")
            .display()
    );
    Ok(())
}

fn payload(value: Value) -> anyhow::Result<Value> {
    let text = value["content"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("tool returned no text payload"))?;
    Ok(serde_json::from_str(text)?)
}
