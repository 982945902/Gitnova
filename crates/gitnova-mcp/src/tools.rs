use anyhow::Result;
use gitnova_core::{build_graph_from_entries, query, scan_repository, CodeGraph};
use gitnova_enrich::embeddings::{self, MODEL2VEC_PROVIDER};
use gitnova_enrich::git::apply_git_churn;
use gitnova_enrich::llm;
use gitnova_enrich::lsp::apply_lsp_metadata;
use gitnova_rank::{diff, rank_graph_with_embeddings};
use gitnova_storage::{FileManifestEntry, SurrealStore};
use gitnova_wiki::{ContentFormat, Evidence, PageKind, PatchMode, WikiPage, WikiStore};
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

/// Global singleton store to avoid RocksDB lock conflicts within the same process.
/// SurrealDB v2's RocksDB engine doesn't release file locks when the connection is dropped,
/// so we keep one connection alive for the entire server lifetime.
static GLOBAL_STORE: OnceLock<Mutex<Option<(PathBuf, SurrealStore)>>> = OnceLock::new();

static WATCHERS: OnceLock<Mutex<HashMap<String, WatchRecord>>> = OnceLock::new();
static NEXT_WATCH_ID: AtomicU64 = AtomicU64::new(1);

/// Access a SurrealStore for the given repo, using a global singleton connection.
/// RocksDB file locks are not properly released on connection drop within the same process,
/// so we keep one connection alive for the entire server lifetime.
pub(crate) fn with_store<R>(repo: &Path, f: impl FnOnce(&SurrealStore) -> Result<R>) -> Result<R> {
    let lock = GLOBAL_STORE.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().unwrap();
    if guard.as_ref().map_or(true, |(p, _)| p != repo) {
        // Different repo path — replace the store (old RocksDB connection is dropped,
        // which is safe because the new store uses a different database directory).
        *guard = Some((repo.to_path_buf(), SurrealStore::open(repo)?));
    }
    f(&guard.as_ref().unwrap().1)
}

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
            tool("search_embeddings", "Search persisted embeddings by provider"),
            tool("wiki_upsert_page", "Create or update a Gitnova wiki page"),
            tool("wiki_patch_page", "Patch public Gitnova wiki page content and render HTML"),
            tool("wiki_patch_private_note", "Patch private agent-only notes for a Gitnova wiki page"),
            tool("wiki_append_evidence", "Append machine-readable evidence to a Gitnova wiki page"),
            tool("wiki_read_page", "Read a Gitnova wiki page and optional private note")
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
            with_store(repo_state, |store| {
                let graph = store.load_graph()?;
                let vectors = store
                    .load_embeddings(MODEL2VEC_PROVIDER)
                    .unwrap_or_default();
                let similarities = if vectors.is_empty() {
                    None
                } else {
                    Some(embeddings::similarity_map(&graph, &vectors, query_text))
                };
                Ok(json!(rank_graph_with_embeddings(
                    &graph,
                    query_text,
                    limit,
                    similarities.as_ref()
                )))
            })?
        }
        "graph_context" => {
            let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(40) as usize;
            with_store(repo_state, |store| {
                let graph = store.load_graph()?;
                let selector = selector_from_arguments(arguments, &graph);
                Ok(json!(query::graph_context(&graph, &selector, depth, limit)))
            })?
        }
        "answer_with_context" => {
            let query_text = arguments.get("query").and_then(Value::as_str).unwrap_or("");
            let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(40) as usize;
            with_store(repo_state, |store| {
                let graph = store.load_graph()?;
                Ok(json!(llm::answer_with_context(
                    &graph, query_text, depth, limit
                )))
            })?
        }
        "llm_explain_node" => with_store(repo_state, |store| {
            let graph = store.load_graph()?;
            let selector = selector_from_arguments(arguments, &graph);
            let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(40) as usize;
            Ok(json!(llm::llm_explain_node(
                &graph, &selector, depth, limit
            )))
        })?,
        "llm_impact_summary" => with_store(repo_state, |store| {
            let graph = store.load_graph()?;
            let selector = selector_from_arguments(arguments, &graph);
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            Ok(json!(llm::llm_impact_summary(&graph, &selector, limit)))
        })?,
        "explain_node" => with_store(repo_state, |store| {
            let graph = store.load_graph()?;
            let selector = selector_from_arguments(arguments, &graph);
            Ok(json!(query::explain_node(&graph, &selector)))
        })?,
        "explain_symbol" => {
            let symbol = arguments
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("");
            with_store(repo_state, |store| {
                let graph = store.load_graph()?;
                Ok(json!(query::explain_symbol(&graph, symbol)))
            })?
        }
        "impact_analysis" | "impact" => {
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            with_store(repo_state, |store| {
                let graph = store.load_graph()?;
                let selector = selector_from_arguments(arguments, &graph);
                Ok(json!(query::impact_analysis(&graph, &selector, limit)))
            })?
        }
        "architecture_map" => {
            let focus = arguments.get("focus").and_then(Value::as_str);
            with_store(repo_state, |store| {
                let graph = store.load_graph()?;
                Ok(json!(query::architecture_map(&graph, focus)))
            })?
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
            with_store(repo_state, |store| {
                let graph = store.load_graph()?;
                Ok(json!(diff::diff_context(
                    &graph,
                    repo_state.as_path(),
                    base,
                    limit
                )))
            })?
        }
        "search_embeddings" => {
            let query_text = arguments.get("query").and_then(Value::as_str).unwrap_or("");
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
            let provider = arguments
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or(MODEL2VEC_PROVIDER);
            with_store(repo_state, |store| {
                let graph = store.load_graph()?;
                let vectors = store.load_embeddings(provider)?;
                Ok(json!(embeddings::search_embeddings_with_provider(
                    &graph, &vectors, query_text, provider, limit
                )?))
            })?
        }
        "wiki_upsert_page" => wiki_upsert_page(repo_state, arguments)?,
        "wiki_patch_page" => wiki_patch_page(repo_state, arguments)?,
        "wiki_patch_private_note" => wiki_patch_private_note(repo_state, arguments)?,
        "wiki_append_evidence" => wiki_append_evidence(repo_state, arguments)?,
        "wiki_read_page" => wiki_read_page(repo_state, arguments)?,
        other => json!({ "error": format!("unknown tool {other}") }),
    };
    Ok(json!({
        "content": [
            { "type": "text", "text": serde_json::to_string(&output)? }
        ],
        "isError": false
    }))
}

fn wiki_upsert_page(repo: &Path, arguments: &Value) -> Result<Value> {
    let id = required_str(arguments, "id")?;
    let title = required_str(arguments, "title")?;
    let kind = parse_page_kind(
        arguments
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("article"),
    )?;
    let content_format = parse_content_format(
        arguments
            .get("content_format")
            .and_then(Value::as_str)
            .unwrap_or("markdown"),
    )?;
    let mut page = WikiPage::new(id, title, kind);
    page.content_format = content_format;
    if let Some(summary) = arguments.get("summary").and_then(Value::as_str) {
        page = page.with_summary(summary);
    }
    if let Some(parent) = arguments.get("parent").and_then(Value::as_str) {
        page = page.with_parent(parent);
    }

    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    store.upsert_page(page)?;
    if let Some(content) = arguments.get("content").and_then(Value::as_str) {
        store.patch_page(id, content_format, content, PatchMode::Replace)?;
    }
    let page = store.read_page(id)?;
    Ok(json!({
        "status": "upserted",
        "id": page.id,
        "wiki_root": wiki_root,
        "page_ref": page_ref(&page.id, page.kind),
        "content_ref": content_ref(&page.id, page.content_format)
    }))
}

fn wiki_patch_page(repo: &Path, arguments: &Value) -> Result<Value> {
    let id = required_str(arguments, "id")?;
    let content = required_str(arguments, "content")?;
    let format = parse_content_format(
        arguments
            .get("content_format")
            .and_then(Value::as_str)
            .unwrap_or("markdown"),
    )?;
    let mode = parse_patch_mode(
        arguments
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("replace"),
    )?;
    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    store.patch_page(id, format, content, mode)?;
    let page = store.read_page(id)?;
    Ok(json!({
        "status": "patched",
        "id": page.id,
        "wiki_root": wiki_root,
        "page_ref": page_ref(&page.id, page.kind),
        "content_ref": content_ref(&page.id, page.content_format)
    }))
}

fn wiki_patch_private_note(repo: &Path, arguments: &Value) -> Result<Value> {
    let id = required_str(arguments, "id")?;
    let content = required_str(arguments, "content")?;
    let mode = parse_patch_mode(
        arguments
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("replace"),
    )?;
    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    store.patch_private_note(id, content, mode)?;
    Ok(json!({
        "status": "patched_private_note",
        "id": id,
        "wiki_root": wiki_root,
        "private_ref": format!("blocks_private/{id}.md")
    }))
}

fn wiki_append_evidence(repo: &Path, arguments: &Value) -> Result<Value> {
    let id = required_str(arguments, "id")?;
    let file = required_str(arguments, "file")?;
    let mut evidence = Evidence::new(file);
    if let (Some(start_line), Some(end_line)) = (
        arguments.get("start_line").and_then(Value::as_u64),
        arguments.get("end_line").and_then(Value::as_u64),
    ) {
        evidence = evidence.with_span(start_line as u32, end_line as u32);
    }
    if let Some(note) = arguments.get("note").and_then(Value::as_str) {
        evidence = evidence.with_note(note);
    }

    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    store.append_evidence(id, evidence)?;
    let page = store.read_page(id)?;
    Ok(json!({
        "status": "evidence_appended",
        "id": page.id,
        "wiki_root": wiki_root,
        "evidence_count": page.evidence.len()
    }))
}

fn wiki_read_page(repo: &Path, arguments: &Value) -> Result<Value> {
    let id = required_str(arguments, "id")?;
    let include_private = arguments
        .get("include_private")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    let page = store.read_page(id)?;
    let private_note = if include_private {
        Some(store.read_private_note(id)?)
    } else {
        None
    };
    Ok(json!({
        "status": "read",
        "wiki_root": wiki_root,
        "page": page,
        "private_note": private_note
    }))
}

fn wiki_root(repo: &Path, arguments: &Value) -> PathBuf {
    arguments
        .get("wiki_root")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join(".gitnova/wiki"))
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required string argument: {key}"))
}

fn parse_page_kind(value: &str) -> Result<PageKind> {
    match value {
        "index" => Ok(PageKind::Index),
        "article" => Ok(PageKind::Article),
        other => anyhow::bail!("unknown wiki page kind: {other}"),
    }
}

fn parse_content_format(value: &str) -> Result<ContentFormat> {
    match value {
        "markdown" => Ok(ContentFormat::Markdown),
        "html" => Ok(ContentFormat::Html),
        other => anyhow::bail!("unknown wiki content format: {other}"),
    }
}

fn parse_patch_mode(value: &str) -> Result<PatchMode> {
    match value {
        "replace" => Ok(PatchMode::Replace),
        "append" => Ok(PatchMode::Append),
        other => anyhow::bail!("unknown wiki patch mode: {other}"),
    }
}

fn page_ref(id: &str, kind: PageKind) -> String {
    match kind {
        PageKind::Index => format!("pages/{id}/index.html"),
        PageKind::Article => format!("pages/{id}.html"),
    }
}

fn content_ref(id: &str, format: ContentFormat) -> String {
    let extension = match format {
        ContentFormat::Markdown => "md",
        ContentFormat::Html => "html",
    };
    format!("blocks/{id}.{extension}")
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
    let now = gitnova_core::model::current_unix();
    let manifest = files
        .iter()
        .map(|file| FileManifestEntry {
            path: file.relative_path.clone(),
            content_hash: file.content_hash.clone(),
            language: file.language,
            indexed_at_unix: now,
        })
        .collect::<Vec<_>>();
    with_store(repo, |store| {
        store.save_graph(&graph)?;
        store.export_json(&graph)?;
        store.save_manifest(&manifest)?;
        Ok(())
    })?;
    Ok(json!({
        "status": "indexed",
        "summary": query::summarize(&graph)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn wiki_tools_write_public_pages_and_private_notes() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let mut repo_state = temp.path().to_path_buf();

        let created = call_tool(
            "wiki_upsert_page",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/search/query-executors",
                "title": "Query Executors",
                "kind": "article",
                "summary": "How HA3 dispatches query execution.",
                "parent": "ha3/search"
            }),
            &mut repo_state,
        )
        .unwrap();
        let created = unwrap_tool_payload(created);
        assert_eq!(created["status"], "upserted");
        assert_eq!(created["id"], "ha3/search/query-executors");

        let patched = call_tool(
            "wiki_patch_page",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/search/query-executors",
                "content_format": "markdown",
                "mode": "replace",
                "content": "Executor Families\nThe public reader path lives here."
            }),
            &mut repo_state,
        )
        .unwrap();
        let patched = unwrap_tool_payload(patched);
        assert_eq!(patched["status"], "patched");

        let private = call_tool(
            "wiki_patch_private_note",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/search/query-executors",
                "mode": "replace",
                "content": "Important Agent Follow-up\nTrace QueryExecutorCreator next."
            }),
            &mut repo_state,
        )
        .unwrap();
        let private = unwrap_tool_payload(private);
        assert_eq!(private["status"], "patched_private_note");

        let html = std::fs::read_to_string(
            temp.path()
                .join(".gitnova/wiki/pages/ha3/search/query-executors.html"),
        )
        .unwrap();
        assert!(html.contains("Executor Families"));
        assert!(!html.contains("Important Agent Follow-up"));
        assert!(!html.contains("QueryExecutorCreator"));

        let public_read = call_tool(
            "wiki_read_page",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/search/query-executors"
            }),
            &mut repo_state,
        )
        .unwrap();
        let public_read = unwrap_tool_payload(public_read);
        assert!(public_read["private_note"].is_null());

        let private_read = call_tool(
            "wiki_read_page",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/search/query-executors",
                "include_private": true
            }),
            &mut repo_state,
        )
        .unwrap();
        let private_read = unwrap_tool_payload(private_read);
        assert_eq!(
            private_read["private_note"],
            "Important Agent Follow-up\nTrace QueryExecutorCreator next."
        );
    }

    #[test]
    fn wiki_evidence_tool_stays_machine_readable() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let mut repo_state = temp.path().to_path_buf();

        call_tool(
            "wiki_upsert_page",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/query-pipeline",
                "title": "Query Pipeline",
                "kind": "article"
            }),
            &mut repo_state,
        )
        .unwrap();

        let evidence = call_tool(
            "wiki_append_evidence",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/query-pipeline",
                "file": "aios/ha3/search/query_executor/QueryExecutorCreator.cpp",
                "start_line": 10,
                "end_line": 42,
                "note": "executor creation entry"
            }),
            &mut repo_state,
        )
        .unwrap();
        let evidence = unwrap_tool_payload(evidence);
        assert_eq!(evidence["status"], "evidence_appended");
        assert_eq!(evidence["evidence_count"], 1);

        let html = std::fs::read_to_string(
            temp.path()
                .join(".gitnova/wiki/pages/ha3/query-pipeline.html"),
        )
        .unwrap();
        assert!(html.contains("data-gitnova-evidence"));
        assert!(!html.contains("executor creation entry</p>"));
    }

    fn unwrap_tool_payload(value: Value) -> Value {
        let text = value["content"][0]["text"].as_str().expect("tool text");
        serde_json::from_str(text).expect("inner payload")
    }
}
