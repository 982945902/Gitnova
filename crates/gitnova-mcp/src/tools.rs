use anyhow::Result;
use gitnova_core::{build_graph_from_entries, query, scan_repository, CodeGraph};
use gitnova_enrich::embeddings::{self, LOCAL_HASH_PROVIDER};
use gitnova_enrich::git::apply_git_churn;
use gitnova_enrich::llm;
use gitnova_enrich::lsp::apply_lsp_metadata;
use gitnova_rank::{diff, rank_graph_with_embeddings};
use gitnova_storage::{FileManifestEntry, GitnovaStore};
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

static WATCHERS: OnceLock<Mutex<HashMap<String, WatchRecord>>> = OnceLock::new();
static NEXT_WATCH_ID: AtomicU64 = AtomicU64::new(1);

struct WatchRecord {
    repo: PathBuf,
    stop: Arc<AtomicBool>,
    status: WatchState,
    events_seen: u64,
    last_indexed_unix: Option<u64>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchState {
    Running,
    Stopped,
    Failed,
}

impl WatchState {
    fn as_str(self) -> &'static str {
        match self {
            WatchState::Running => "running",
            WatchState::Stopped => "stopped",
            WatchState::Failed => "failed",
        }
    }
}

pub fn list_tools() -> Value {
    json!({
        "tools": [
            tool("index_project", "Index a repository into Gitnova storage"),
            tool("rank_context", "Rank code context by salience"),
            tool("search_rank", "Alias for rank_context with Web-friendly schema"),
            tool("graph_context", "Return a focused node neighborhood and relationship edges"),
            tool("explain_node", "Explain a node selected by id, symbol, or query"),
            tool("answer_with_context", "Answer a question using ranked graph evidence and optional LLM explanation"),
            tool("llm_explain_node", "Explain a node using graph evidence and optional LLM wording"),
            tool("llm_impact_summary", "Summarize symbol impact using graph evidence and optional LLM wording"),
            tool("explain_symbol", "Explain a symbol and its graph neighborhood"),
            tool("impact_analysis", "Find reverse dependencies for a symbol"),
            tool("impact", "Alias for impact_analysis with node/query selectors"),
            tool("architecture_map", "Summarize repository architecture"),
            tool("watch_project", "Report watch mode availability for a repository"),
            tool("watch_status", "Report a managed watcher's current state"),
            tool("stop_watch", "Stop a managed watcher by watch_id"),
            tool("diff_context", "Rank context from git diff paths"),
            tool("search_embeddings", "Search persisted embeddings by provider")
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
        "rank_context" | "search_rank" => {
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
        "graph_context" => {
            let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(40) as usize;
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            let selector = selector_from_arguments(arguments, &graph);
            json!(query::graph_context(&graph, &selector, depth, limit))
        }
        "answer_with_context" => {
            let query_text = arguments.get("query").and_then(Value::as_str).unwrap_or("");
            let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(40) as usize;
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            json!(llm::answer_with_context(&graph, query_text, depth, limit))
        }
        "llm_explain_node" => {
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            let selector = selector_from_arguments(arguments, &graph);
            let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(40) as usize;
            json!(llm::llm_explain_node(&graph, &selector, depth, limit))
        }
        "llm_impact_summary" => {
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            let selector = selector_from_arguments(arguments, &graph);
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            json!(llm::llm_impact_summary(&graph, &selector, limit))
        }
        "explain_node" => {
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            let selector = selector_from_arguments(arguments, &graph);
            json!(query::explain_node(&graph, &selector))
        }
        "explain_symbol" => {
            let symbol = arguments
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("");
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            json!(query::explain_symbol(&graph, symbol))
        }
        "impact_analysis" | "impact" => {
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            let selector = selector_from_arguments(arguments, &graph);
            json!(query::impact_analysis(&graph, &selector, limit))
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
        "watch_status" => {
            let watch_id = arguments
                .get("watch_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            watch_status(watch_id)
        }
        "stop_watch" => {
            let watch_id = arguments
                .get("watch_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            stop_watch_project(watch_id)
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
            let provider = arguments
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or(LOCAL_HASH_PROVIDER);
            let graph = GitnovaStore::open(repo_state.as_path())?.load_graph()?;
            let vectors = GitnovaStore::open(repo_state.as_path())?.load_embeddings(provider)?;
            json!(embeddings::search_embeddings_with_provider(
                &graph, &vectors, query_text, provider, limit
            )?)
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

fn selector_from_arguments(arguments: &Value, graph: &CodeGraph) -> String {
    for key in ["node_id", "selector", "symbol"] {
        if let Some(value) = arguments.get(key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return value.to_string();
            }
        }
    }
    if let Some(query_text) = arguments.get("query").and_then(Value::as_str) {
        if let Some(result) = rank_graph_with_embeddings(graph, query_text, 1, None)
            .results
            .into_iter()
            .next()
        {
            return result.node.id;
        }
        return query_text.to_string();
    }
    String::new()
}

fn start_watch_project(repo: PathBuf) -> Result<Value> {
    if !repo.exists() {
        anyhow::bail!("repo does not exist: {}", repo.display());
    }
    let watch_id = format!(
        "watch-{}-{}",
        gitnova_core::model::current_unix(),
        NEXT_WATCH_ID.fetch_add(1, Ordering::Relaxed)
    );
    let stop = Arc::new(AtomicBool::new(false));
    watchers().lock().unwrap().insert(
        watch_id.clone(),
        WatchRecord {
            repo: repo.clone(),
            stop: stop.clone(),
            status: WatchState::Running,
            events_seen: 0,
            last_indexed_unix: None,
            handle: None,
        },
    );
    let watched = repo.clone();
    let thread_watch_id = watch_id.clone();
    let handle = std::thread::spawn(move || {
        let (tx, rx) = channel();
        let Ok(mut watcher) = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) else {
            update_watch_state(&thread_watch_id, WatchState::Failed, None, None);
            return;
        };
        if watcher.watch(&watched, RecursiveMode::Recursive).is_err() {
            update_watch_state(&thread_watch_id, WatchState::Failed, None, None);
            return;
        }
        if index_repo(&watched).is_ok() {
            update_watch_state(
                &thread_watch_id,
                WatchState::Running,
                None,
                Some(gitnova_core::model::current_unix()),
            );
        }
        while !stop.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(Ok(event)) => {
                    if event
                        .paths
                        .iter()
                        .all(|path| path.components().any(|part| part.as_os_str() == ".gitnova"))
                    {
                        continue;
                    }
                    if index_repo(&watched).is_ok() {
                        update_watch_state(
                            &thread_watch_id,
                            WatchState::Running,
                            Some(1),
                            Some(gitnova_core::model::current_unix()),
                        );
                    }
                }
                Ok(Err(_)) => update_watch_state(&thread_watch_id, WatchState::Failed, None, None),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        update_watch_state(&thread_watch_id, WatchState::Stopped, None, None);
    });
    if let Some(record) = watchers().lock().unwrap().get_mut(&watch_id) {
        record.handle = Some(handle);
    }
    Ok(json!({
        "status": "started",
        "watch_id": watch_id,
        "repo": repo
    }))
}

fn watch_status(watch_id: &str) -> Value {
    let watchers = watchers().lock().unwrap();
    let Some(record) = watchers.get(watch_id) else {
        return json!({
            "status": "unknown",
            "watch_id": watch_id
        });
    };
    json!({
        "status": record.status.as_str(),
        "watch_id": watch_id,
        "repo": record.repo.to_string_lossy().to_string(),
        "events_seen": record.events_seen,
        "last_indexed_unix": record.last_indexed_unix
    })
}

fn stop_watch_project(watch_id: &str) -> Value {
    let mut watchers = watchers().lock().unwrap();
    let Some(record) = watchers.get_mut(watch_id) else {
        return json!({
            "status": "unknown",
            "watch_id": watch_id
        });
    };
    record.stop.store(true, Ordering::Relaxed);
    record.status = WatchState::Stopped;
    json!({
        "status": "stopped",
        "watch_id": watch_id,
        "repo": record.repo.to_string_lossy().to_string()
    })
}

fn watchers() -> &'static Mutex<HashMap<String, WatchRecord>> {
    WATCHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn update_watch_state(
    watch_id: &str,
    status: WatchState,
    events_delta: Option<u64>,
    last_indexed_unix: Option<u64>,
) {
    if let Some(record) = watchers().lock().unwrap().get_mut(watch_id) {
        record.status = status;
        if let Some(delta) = events_delta {
            record.events_seen += delta;
        }
        if last_indexed_unix.is_some() {
            record.last_indexed_unix = last_indexed_unix;
        }
    }
}

fn index_repo(repo: &Path) -> Result<Value> {
    let files = scan_repository(repo)?;
    let mut graph = build_graph_from_entries(repo, &files)?;
    apply_git_churn(repo, &mut graph)?;
    apply_lsp_metadata(&mut graph);
    let store = GitnovaStore::open(repo)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_watchers_report_status_and_stop() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("lib.rs"), "pub fn entry() {}\n").unwrap();

        let started = start_watch_project(temp.path().to_path_buf()).unwrap();
        let watch_id = started["watch_id"].as_str().expect("watch id");
        let status = watch_status(watch_id);
        assert_eq!(status["status"], "running");
        assert_eq!(status["repo"], temp.path().to_string_lossy().to_string());

        let stopped = stop_watch_project(watch_id);
        assert_eq!(stopped["status"], "stopped");
        let status = watch_status(watch_id);
        assert_eq!(status["status"], "stopped");
    }
}
