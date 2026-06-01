use anyhow::Result;
use gitnova_atlas::{
    investigate_many, CodexCliAgent, InvestigationBudget, InvestigationDiagram,
    InvestigationResult, InvestigationScope, InvestigationSource, InvestigationTask,
};
use gitnova_core::{build_graph_from_entries, query, scan_repository, CodeGraph};
use gitnova_enrich::embeddings::{self, MODEL2VEC_PROVIDER};
use gitnova_enrich::git::apply_git_churn;
use gitnova_enrich::llm;
use gitnova_enrich::lsp::apply_lsp_metadata;
use gitnova_rank::{diff, rank_graph_with_embeddings};
use gitnova_storage::{FileManifestEntry, SurrealStore};
use gitnova_wiki::{
    ContentFormat, Evidence, PageKind, PatchMode, TaskStatus, WikiOutline, WikiPage, WikiStore,
};
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
            tool("wiki_read_page", "Read a Gitnova wiki page and optional private note"),
            tool("wiki_deepen_page", "Run an Atlas child Codex investigation and merge the result into a wiki page"),
            tool("wiki_apply_outline", "Apply a Codex-authored wiki outline and deep task queue"),
            tool("wiki_read_outline", "Read the current Codex-authored wiki outline"),
            tool("wiki_expand_tree", "Run pending outline tasks with parallel Atlas child Codex agents")
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
    if wiki_tool_is_disabled_inside_atlas_child_agent(
        name,
        std::env::var_os("GITNOVA_ATLAS_CHILD").is_some(),
        std::env::var_os("GITNOVA_DISABLE_MCP_RECURSION").is_some(),
    ) {
        anyhow::bail!("wiki MCP tools are disabled inside Atlas child agent");
    }

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
        "wiki_deepen_page" => wiki_deepen_page(repo_state, arguments)?,
        "wiki_apply_outline" => wiki_apply_outline(repo_state, arguments)?,
        "wiki_read_outline" => wiki_read_outline(repo_state, arguments)?,
        "wiki_expand_tree" => wiki_expand_tree(repo_state, arguments)?,
        other => json!({ "error": format!("unknown tool {other}") }),
    };
    Ok(json!({
        "content": [
            { "type": "text", "text": serde_json::to_string(&output)? }
        ],
        "isError": false
    }))
}

fn wiki_tool_is_disabled_inside_atlas_child_agent(
    name: &str,
    is_atlas_child: bool,
    recursion_disabled: bool,
) -> bool {
    name.starts_with("wiki_") && is_atlas_child && recursion_disabled
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

fn wiki_deepen_page(repo: &Path, arguments: &Value) -> Result<Value> {
    let id = required_str(arguments, "id")?;
    let question = required_str(arguments, "question")?;
    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    let page = store.read_page(id)?;
    if page.content_format != ContentFormat::Markdown {
        anyhow::bail!("wiki_deepen_page currently supports markdown pages only");
    }

    let repo_path = arguments
        .get("repo_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.to_path_buf());
    let task_id = arguments
        .get("task_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("deepen-{}", id.replace('/', "-")));
    let task = InvestigationTask {
        task_id: task_id.clone(),
        question: question.to_string(),
        repo_path,
        scope: InvestigationScope {
            paths: string_array(arguments, "scope_paths"),
            symbols: string_array(arguments, "scope_symbols"),
        },
        expected_outputs: string_array(arguments, "expected_outputs"),
        budget: InvestigationBudget {
            max_nodes: arguments
                .get("max_nodes")
                .and_then(Value::as_u64)
                .unwrap_or(80) as usize,
            max_depth: arguments
                .get("max_depth")
                .and_then(Value::as_u64)
                .unwrap_or(4) as usize,
            timeout_secs: arguments
                .get("timeout_secs")
                .and_then(Value::as_u64)
                .unwrap_or(120),
        },
    };

    let mut agent = CodexCliAgent::new(
        arguments
            .get("codex_bin")
            .and_then(Value::as_str)
            .unwrap_or("codex"),
    );
    if let Some(model) = arguments.get("model").and_then(Value::as_str) {
        agent = agent.with_model(model);
    }
    if let Some(codex_home) = arguments.get("codex_home").and_then(Value::as_str) {
        agent = agent.with_codex_home(codex_home);
    }

    let result = agent.investigate(&task)?;
    merge_investigation_result(&store, id, question, &result)?;

    Ok(json!({
        "status": "deepened",
        "id": id,
        "task_id": result.task_id,
        "wiki_root": wiki_root,
        "sources_appended": result.sources.len(),
        "followups_appended": result.followups.len(),
        "confidence": result.confidence
    }))
}

fn wiki_apply_outline(repo: &Path, arguments: &Value) -> Result<Value> {
    let outline_value = arguments
        .get("outline")
        .ok_or_else(|| anyhow::anyhow!("missing required object argument: outline"))?;
    let outline: WikiOutline =
        serde_json::from_value(outline_value.clone()).map_err(anyhow::Error::from)?;
    let pages = outline.pages.len();
    let tasks = outline
        .pages
        .iter()
        .map(|page| page.deep_tasks.len())
        .sum::<usize>();
    let root = outline.root.clone();
    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    store.apply_outline(outline)?;
    Ok(json!({
        "status": "outline_applied",
        "wiki_root": wiki_root,
        "root": root,
        "pages": pages,
        "tasks": tasks
    }))
}

fn wiki_read_outline(repo: &Path, arguments: &Value) -> Result<Value> {
    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    let outline = store.read_outline()?;
    Ok(json!({
        "status": "outline_read",
        "wiki_root": wiki_root,
        "outline": outline
    }))
}

fn wiki_expand_tree(repo: &Path, arguments: &Value) -> Result<Value> {
    let root_page_id = arguments
        .get("root_page_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if root_page_id.trim().is_empty() {
        anyhow::bail!("missing required string argument: root_page_id");
    }
    let task_limit = arguments
        .get("task_limit")
        .and_then(Value::as_u64)
        .unwrap_or(3) as usize;
    let parallelism = arguments
        .get("parallelism")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    let wiki_root = wiki_root(repo, arguments);
    let store = WikiStore::open(&wiki_root)?;
    let pending = store.pending_tasks(root_page_id, task_limit)?;
    if pending.is_empty() {
        return Ok(json!({
            "status": "expanded",
            "wiki_root": wiki_root,
            "root_page_id": root_page_id,
            "tasks_completed": 0,
            "tasks_failed": 0,
            "results": []
        }));
    }

    let repo_path = arguments
        .get("repo_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.to_path_buf());
    let timeout_secs = arguments
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(120);
    let tasks = pending
        .iter()
        .map(|(_, task)| InvestigationTask {
            task_id: task.id.clone(),
            question: task.question.clone(),
            repo_path: repo_path.clone(),
            scope: InvestigationScope {
                paths: task.scope_paths.clone(),
                symbols: task.scope_symbols.clone(),
            },
            expected_outputs: task.expected_outputs.clone(),
            budget: InvestigationBudget {
                max_nodes: arguments
                    .get("max_nodes")
                    .and_then(Value::as_u64)
                    .unwrap_or(80) as usize,
                max_depth: arguments
                    .get("max_depth")
                    .and_then(Value::as_u64)
                    .unwrap_or(4) as usize,
                timeout_secs,
            },
        })
        .collect::<Vec<_>>();
    let mut agent = CodexCliAgent::new(
        arguments
            .get("codex_bin")
            .and_then(Value::as_str)
            .unwrap_or("codex"),
    );
    if let Some(model) = arguments.get("model").and_then(Value::as_str) {
        agent = agent.with_model(model);
    }
    if let Some(codex_home) = arguments.get("codex_home").and_then(Value::as_str) {
        agent = agent.with_codex_home(codex_home);
    }

    for (page_id, task) in &pending {
        store.update_task_status(page_id, &task.id, TaskStatus::Running, None, None)?;
    }

    let results = match investigate_many(agent, tasks, parallelism) {
        Ok(results) => results,
        Err(error) => {
            let error_text = error.to_string();
            for (page_id, task) in &pending {
                store.update_task_status(
                    page_id,
                    &task.id,
                    TaskStatus::Failed,
                    None,
                    Some(error_text.clone()),
                )?;
            }
            return Ok(json!({
                "status": "expanded",
                "wiki_root": wiki_root,
                "root_page_id": root_page_id,
                "tasks_completed": 0,
                "tasks_failed": pending.len(),
                "error": error_text,
                "results": []
            }));
        }
    };
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut summaries = Vec::new();
    for result in results {
        let Some((page_id, task)) = pending.iter().find(|(_, task)| task.id == result.task_id)
        else {
            failed += 1;
            summaries.push(json!({
                "task_id": result.task_id,
                "status": "failed",
                "error": "result task id was not pending"
            }));
            continue;
        };
        match merge_investigation_result(&store, page_id, &task.question, &result) {
            Ok(()) => {
                store.update_task_status(
                    page_id,
                    &task.id,
                    TaskStatus::Done,
                    Some(result.confidence),
                    None,
                )?;
                completed += 1;
                summaries.push(json!({
                    "task_id": result.task_id,
                    "page_id": page_id,
                    "status": "done",
                    "sources_appended": result.sources.len(),
                    "followups_appended": result.followups.len(),
                    "confidence": result.confidence
                }));
            }
            Err(error) => {
                store.update_task_status(
                    page_id,
                    &task.id,
                    TaskStatus::Failed,
                    None,
                    Some(error.to_string()),
                )?;
                failed += 1;
                summaries.push(json!({
                    "task_id": result.task_id,
                    "page_id": page_id,
                    "status": "failed",
                    "error": error.to_string()
                }));
            }
        }
    }

    Ok(json!({
        "status": "expanded",
        "wiki_root": wiki_root,
        "root_page_id": root_page_id,
        "tasks_completed": completed,
        "tasks_failed": failed,
        "results": summaries
    }))
}

fn merge_investigation_result(
    store: &WikiStore,
    id: &str,
    question: &str,
    result: &InvestigationResult,
) -> Result<()> {
    let mut public_patch = String::new();
    public_patch.push_str("\n\n## Atlas Investigation\n");
    public_patch.push_str(&format!("Question: {question}\n\n"));
    public_patch.push_str(result.summary_markdown.trim());
    public_patch.push('\n');
    if let Some(diagram) = &result.diagram {
        public_patch.push('\n');
        public_patch.push_str(&diagram_markdown(diagram));
    }
    store.patch_page(
        id,
        ContentFormat::Markdown,
        &public_patch,
        PatchMode::Append,
    )?;

    for source in &result.sources {
        store.append_evidence(id, evidence_from_investigation_source(source))?;
    }

    if !result.followups.is_empty() {
        let mut private_patch = String::from("Atlas Follow-ups\n");
        for followup in &result.followups {
            private_patch.push_str("- ");
            private_patch.push_str(followup);
            private_patch.push('\n');
        }
        store.patch_private_note(id, private_patch.trim_end(), PatchMode::Append)?;
    }
    Ok(())
}

fn diagram_markdown(diagram: &InvestigationDiagram) -> String {
    match diagram.format.as_str() {
        "svg" => format!("```svg\n{}\n```\n", diagram.content.trim()),
        "mermaid" => format!("```mermaid\n{}\n```\n", diagram.content.trim()),
        _ => String::new(),
    }
}

fn evidence_from_investigation_source(source: &InvestigationSource) -> Evidence {
    Evidence {
        file: source.file.clone(),
        start_line: source.start_line,
        end_line: source.end_line,
        note: source.note.clone(),
    }
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

fn string_array(arguments: &Value, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
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

    #[test]
    fn wiki_tools_are_disabled_inside_atlas_child_agent() {
        assert!(wiki_tool_is_disabled_inside_atlas_child_agent(
            "wiki_read_page",
            true,
            true
        ));
        assert!(!wiki_tool_is_disabled_inside_atlas_child_agent(
            "wiki_read_page",
            true,
            false
        ));
        assert!(!wiki_tool_is_disabled_inside_atlas_child_agent(
            "rank_context",
            true,
            true
        ));
    }

    #[test]
    fn wiki_deepen_page_runs_atlas_child_and_merges_result() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let codex_bin = temp.path().join("fake-codex");
        std::fs::write(
            &codex_bin,
            r#"#!/bin/sh
set -eu
out=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "-o" ] || [ "$prev" = "--output-last-message" ]; then
    out="$arg"
  fi
  prev="$arg"
done
cat > /dev/null
cat > "$out" <<'JSON'
{"task_id":"trace-query-executor-creator","summary_markdown":"QueryExecutorCreator builds executor families from query semantics.","sources":[{"file":"aios/ha3/search/query_executor/QueryExecutorCreator.cpp","start_line":10,"end_line":42,"note":"creator dispatch"}],"diagram":{"format":"svg","content":"<svg viewBox=\"0 0 100 40\"><text x=\"4\" y=\"20\">Creator</text></svg>"},"followups":["Trace bitmap executor index reader dependencies."],"confidence":0.82}
JSON
"#,
        )
        .unwrap();
        make_executable(&codex_bin);

        let mut repo_state = repo.clone();
        call_tool(
            "wiki_upsert_page",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/search/query-executors",
                "title": "Query Executors",
                "kind": "article",
                "content": "Executor Families\nInitial skeleton."
            }),
            &mut repo_state,
        )
        .unwrap();

        let deepened = call_tool(
            "wiki_deepen_page",
            &json!({
                "wiki_root": wiki_root,
                "repo_path": repo,
                "id": "ha3/search/query-executors",
                "task_id": "trace-query-executor-creator",
                "question": "Trace QueryExecutorCreator into each executor family.",
                "scope_paths": ["aios/ha3/search"],
                "scope_symbols": ["QueryExecutorCreator"],
                "expected_outputs": ["summary", "sources", "diagram", "followups"],
                "codex_bin": codex_bin,
                "timeout_secs": 5
            }),
            &mut repo_state,
        )
        .unwrap();
        let deepened = unwrap_tool_payload(deepened);
        assert_eq!(deepened["status"], "deepened");
        assert_eq!(deepened["task_id"], "trace-query-executor-creator");
        assert_eq!(deepened["sources_appended"], 1);
        assert_eq!(deepened["followups_appended"], 1);

        let html = std::fs::read_to_string(
            temp.path()
                .join(".gitnova/wiki/pages/ha3/search/query-executors.html"),
        )
        .unwrap();
        assert!(html.contains("QueryExecutorCreator builds executor families"));
        assert!(html.contains("<svg viewBox"));
        assert!(html.contains("data-gitnova-evidence"));
        assert!(!html.contains("Trace bitmap executor index reader dependencies.</p>"));

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
        assert!(private_read["private_note"]
            .as_str()
            .unwrap()
            .contains("Trace bitmap executor index reader dependencies."));
    }

    #[test]
    fn wiki_apply_outline_writes_pages_and_task_queue() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let mut repo_state = temp.path().to_path_buf();

        let applied = call_tool(
            "wiki_apply_outline",
            &json!({
                "wiki_root": wiki_root,
                "outline": {
                    "root": "ha3",
                    "pages": [
                        {
                            "id": "ha3",
                            "title": "HA3",
                            "kind": "index",
                            "summary": "Search layer overview.",
                            "purpose": "Give readers the top-level HA3 map.",
                            "content": "## Reading Path\nStart with search runtime.",
                            "deep_tasks": []
                        },
                        {
                            "id": "ha3/search/query-executors",
                            "title": "Query Executors",
                            "kind": "article",
                            "parent": "ha3",
                            "summary": "Executor families and dispatch.",
                            "purpose": "Explain executor creation and index reader usage.",
                            "deep_tasks": [{
                                "id": "trace-query-executor-creator",
                                "question": "Trace QueryExecutorCreator into each executor family.",
                                "scope_paths": ["aios/ha3/search"],
                                "scope_symbols": ["QueryExecutorCreator"],
                                "expected_outputs": ["summary", "sources", "diagram"]
                            }]
                        }
                    ]
                }
            }),
            &mut repo_state,
        )
        .unwrap();
        let applied = unwrap_tool_payload(applied);
        assert_eq!(applied["status"], "outline_applied");
        assert_eq!(applied["pages"], 2);
        assert_eq!(applied["tasks"], 1);

        let outline = call_tool(
            "wiki_read_outline",
            &json!({
                "wiki_root": wiki_root
            }),
            &mut repo_state,
        )
        .unwrap();
        let outline = unwrap_tool_payload(outline);
        assert_eq!(outline["outline"]["root"], "ha3");
        assert_eq!(
            outline["outline"]["pages"][1]["deep_tasks"][0]["status"],
            "pending"
        );

        let html = std::fs::read_to_string(temp.path().join(".gitnova/wiki/pages/ha3/index.html"))
            .unwrap();
        assert!(html.contains("Reading Path"));

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
        assert!(private_read["private_note"]
            .as_str()
            .unwrap()
            .contains("Trace QueryExecutorCreator"));
    }

    #[test]
    fn wiki_expand_tree_runs_pending_outline_tasks_and_marks_done() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let codex_bin = temp.path().join("fake-codex");
        std::fs::write(
            &codex_bin,
            r#"#!/bin/sh
set -eu
out=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "-o" ] || [ "$prev" = "--output-last-message" ]; then
    out="$arg"
  fi
  prev="$arg"
done
stdin="$(cat)"
task_id="$(printf '%s\n' "$stdin" | awk -F': ' '/^task_id:/{print $2; exit}')"
printf '{"task_id":"%s","summary_markdown":"summary for %s","sources":[{"file":"src/lib.rs","start_line":1,"end_line":2,"note":"test source"}],"diagram":null,"followups":["follow %s"],"confidence":0.77}\n' "$task_id" "$task_id" "$task_id" > "$out"
"#,
        )
        .unwrap();
        make_executable(&codex_bin);

        let mut repo_state = repo.clone();
        call_tool(
            "wiki_apply_outline",
            &json!({
                "wiki_root": wiki_root,
                "outline": {
                    "root": "ha3",
                    "pages": [{
                        "id": "ha3/search/query-executors",
                        "title": "Query Executors",
                        "kind": "article",
                        "summary": "Executor families and dispatch.",
                        "deep_tasks": [
                            {"id": "task-a", "question": "Investigate A.", "expected_outputs": ["summary"]},
                            {"id": "task-b", "question": "Investigate B.", "expected_outputs": ["summary"]}
                        ]
                    }]
                }
            }),
            &mut repo_state,
        )
        .unwrap();

        let expanded = call_tool(
            "wiki_expand_tree",
            &json!({
                "wiki_root": wiki_root,
                "repo_path": repo,
                "root_page_id": "ha3",
                "task_limit": 2,
                "parallelism": 2,
                "codex_bin": codex_bin,
                "timeout_secs": 5
            }),
            &mut repo_state,
        )
        .unwrap();
        let expanded = unwrap_tool_payload(expanded);
        assert_eq!(expanded["status"], "expanded");
        assert_eq!(expanded["tasks_completed"], 2);
        assert_eq!(expanded["tasks_failed"], 0);

        let outline = call_tool(
            "wiki_read_outline",
            &json!({
                "wiki_root": wiki_root
            }),
            &mut repo_state,
        )
        .unwrap();
        let outline = unwrap_tool_payload(outline);
        assert_eq!(
            outline["outline"]["pages"][0]["deep_tasks"][0]["status"],
            "done"
        );
        assert_eq!(
            outline["outline"]["pages"][0]["deep_tasks"][1]["status"],
            "done"
        );

        let html = std::fs::read_to_string(
            temp.path()
                .join(".gitnova/wiki/pages/ha3/search/query-executors.html"),
        )
        .unwrap();
        assert!(html.contains("summary for task-a"));
        assert!(html.contains("summary for task-b"));
    }

    fn unwrap_tool_payload(value: Value) -> Value {
        let text = value["content"][0]["text"].as_str().expect("tool text");
        serde_json::from_str(text).expect("inner payload")
    }

    #[cfg(unix)]
    fn make_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &std::path::Path) {}
}
