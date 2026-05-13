use anyhow::Result;
use gitnova_core::{build_graph_from_entries, query, scan_repository};
use gitnova_enrich::embeddings::{self, LOCAL_HASH_PROVIDER};
use gitnova_enrich::git::apply_git_churn;
use gitnova_enrich::lsp::apply_lsp_metadata;
use gitnova_rank::{diff, rank_graph_with_embeddings};
use gitnova_storage::{FileManifestEntry, GitnovaStore};
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;

pub fn list_tools() -> Value {
    json!({
        "tools": [
            tool("index_project", "Index a repository into Gitnova storage"),
            tool("rank_context", "Rank code context by salience"),
            tool("explain_symbol", "Explain a symbol and its graph neighborhood"),
            tool("impact_analysis", "Find reverse dependencies for a symbol"),
            tool("architecture_map", "Summarize repository architecture"),
            tool("watch_project", "Report watch mode availability for a repository"),
            tool("diff_context", "Rank context from git diff paths"),
            tool("search_embeddings", "Search local-hash embeddings")
        ]
    })
}

fn tool(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "additionalProperties": true
        }
    })
}

pub fn call_tool(name: &str, arguments: &Value, repo_state: &mut PathBuf) -> Result<Value> {
    let output = match name {
        "index_project" => {
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| repo_state.clone());
            *repo_state = path.clone();
            json!(index_repo(&path)?)
        }
        "rank_context" => {
            let query_text = arguments
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("architecture");
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            let store = GitnovaStore::open(repo_state.as_path())?;
            let vectors = store
                .load_embeddings(LOCAL_HASH_PROVIDER)
                .unwrap_or_default();
            let similarities = if vectors.is_empty() {
                None
            } else {
                Some(embeddings::similarity_map(&graph, &vectors, query_text))
            };
            json!(rank_graph_with_embeddings(
                &graph,
                query_text,
                limit,
                similarities.as_ref()
            ))
        }
        "explain_symbol" => {
            let symbol = arguments
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("");
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            json!(query::explain_symbol(&graph, symbol))
        }
        "impact_analysis" => {
            let symbol = arguments
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("");
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            json!(query::impact_analysis(&graph, symbol, limit))
        }
        "architecture_map" => {
            let focus = arguments.get("focus").and_then(Value::as_str);
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            json!(query::architecture_map(&graph, focus))
        }
        "watch_project" => {
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| repo_state.clone());
            *repo_state = path.clone();
            start_watch_project(path)?
        }
        "diff_context" => {
            let base = arguments
                .get("base")
                .and_then(Value::as_str)
                .unwrap_or("main");
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            json!(diff::diff_context(
                &graph,
                repo_state.as_path(),
                base,
                limit
            ))
        }
        "search_embeddings" => {
            let query_text = arguments.get("query").and_then(Value::as_str).unwrap_or("");
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            let vectors =
                GitnovaStore::open(repo_state.as_path())?.load_embeddings(LOCAL_HASH_PROVIDER)?;
            json!(embeddings::search_embeddings(
                &graph, &vectors, query_text, limit
            ))
        }
        other => json!({ "error": format!("unknown tool {other}") }),
    };
    Ok(json!({
        "content": [
            { "type": "text", "text": serde_json::to_string(&output)? }
        ],
        "isError": false
    }))
}

fn start_watch_project(repo: PathBuf) -> Result<Value> {
    if !repo.exists() {
        anyhow::bail!("repo does not exist: {}", repo.display());
    }
    let watched = repo.clone();
    std::thread::spawn(move || {
        let (tx, rx) = channel();
        let Ok(mut watcher) = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) else {
            return;
        };
        if watcher.watch(&watched, RecursiveMode::Recursive).is_err() {
            return;
        }
        let _ = index_repo(&watched);
        for event in rx {
            if event.is_ok() {
                let _ = index_repo(&watched);
            }
        }
    });
    Ok(json!({
        "status": "started",
        "repo": repo
    }))
}

fn index_repo(repo: &Path) -> Result<Value> {
    let files = scan_repository(repo)?;
    let mut graph = build_graph_from_entries(repo, &files)?;
    apply_git_churn(repo, &mut graph)?;
    apply_lsp_metadata(&mut graph);
    let mut store = GitnovaStore::open(repo)?;
    store.save_graph(&graph)?;
    store.export_json(&graph)?;
    let now = gitnova_core::model::current_unix();
    let manifest = files
        .into_iter()
        .map(|file| FileManifestEntry {
            path: file.relative_path,
            content_hash: file.content_hash,
            language: file.language,
            indexed_at_unix: now,
        })
        .collect::<Vec<_>>();
    store.save_manifest(&manifest)?;
    Ok(json!({
        "status": "indexed",
        "summary": query::summarize(&graph)
    }))
}
