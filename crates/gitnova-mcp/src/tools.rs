use anyhow::Result;
use gitnova_atlas::{
    investigate_many_outcomes, CodexCliAgent, InvestigationBudget, InvestigationDiagram,
    InvestigationOutcome, InvestigationResult, InvestigationScope, InvestigationSource,
    InvestigationTask,
};
use gitnova_core::{build_graph_from_entries, query, scan_repository, CodeGraph};
use gitnova_enrich::embeddings::{self, MODEL2VEC_PROVIDER};
use gitnova_enrich::git::apply_git_churn;
use gitnova_enrich::llm;
use gitnova_enrich::lsp::apply_lsp_metadata;
use gitnova_rank::{diff, rank_graph_with_embeddings};
use gitnova_storage::{json_export, FileManifestEntry, SurrealStore};
use gitnova_wiki::{
    ContentFormat, DeepTask, Evidence, OutlinePage, PageKind, PatchMode, TaskStatus, WikiOutline,
    WikiPage, WikiStore,
};
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
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
    if guard.as_ref().is_none_or(|(p, _)| p != repo) {
        // Different repo path — replace the store (old RocksDB connection is dropped,
        // which is safe because the new store uses a different database directory).
        *guard = Some((repo.to_path_buf(), SurrealStore::open(repo)?));
    }
    f(&guard.as_ref().unwrap().1)
}

pub(crate) fn load_graph(repo: &Path) -> Result<CodeGraph> {
    if tokio::runtime::Handle::try_current().is_err() {
        if let Ok(graph) = with_store(repo, |store| store.load_graph()) {
            return Ok(graph);
        }
    }
    json_export::import_graph(repo.join(".gitnova/index.json"))
}

fn load_embeddings(repo: &Path, provider: &str) -> HashMap<String, Vec<f32>> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return HashMap::new();
    }
    with_store(repo, |store| store.load_embeddings(provider)).unwrap_or_default()
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
            tool("wiki_plan_repo", "Create a repo-level Gitnova wiki outline and deep task queue"),
            tool("wiki_run_repo", "Score a repo wiki and expand the lowest-completion pages"),
            tool("wiki_read_page", "Read a Gitnova wiki page and optional private note"),
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
            let graph = load_graph(repo_state)?;
            let vectors = load_embeddings(repo_state, MODEL2VEC_PROVIDER);
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
            let graph = load_graph(repo_state)?;
            let selector = selector_from_arguments(arguments, &graph);
            json!(query::graph_context(&graph, &selector, depth, limit))
        }
        "answer_with_context" => {
            let query_text = arguments.get("query").and_then(Value::as_str).unwrap_or("");
            let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(40) as usize;
            let graph = load_graph(repo_state)?;
            json!(llm::answer_with_context(&graph, query_text, depth, limit))
        }
        "llm_explain_node" => {
            let graph = load_graph(repo_state)?;
            let selector = selector_from_arguments(arguments, &graph);
            let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(1) as usize;
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(40) as usize;
            json!(llm::llm_explain_node(&graph, &selector, depth, limit))
        }
        "llm_impact_summary" => {
            let graph = load_graph(repo_state)?;
            let selector = selector_from_arguments(arguments, &graph);
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            json!(llm::llm_impact_summary(&graph, &selector, limit))
        }
        "explain_node" => {
            let graph = load_graph(repo_state)?;
            let selector = selector_from_arguments(arguments, &graph);
            json!(query::explain_node(&graph, &selector))
        }
        "explain_symbol" => {
            let symbol = arguments
                .get("symbol")
                .and_then(Value::as_str)
                .unwrap_or("");
            let graph = load_graph(repo_state)?;
            json!(query::explain_symbol(&graph, symbol))
        }
        "impact_analysis" | "impact" => {
            let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            let graph = load_graph(repo_state)?;
            let selector = selector_from_arguments(arguments, &graph);
            json!(query::impact_analysis(&graph, &selector, limit))
        }
        "architecture_map" => {
            let focus = arguments.get("focus").and_then(Value::as_str);
            let graph = load_graph(repo_state)?;
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
            let graph = load_graph(repo_state)?;
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
                .unwrap_or(MODEL2VEC_PROVIDER);
            let graph = load_graph(repo_state)?;
            let vectors = load_embeddings(repo_state, provider);
            json!(embeddings::search_embeddings_with_provider(
                &graph, &vectors, query_text, provider, limit
            )?)
        }
        "wiki_plan_repo" => wiki_plan_repo(repo_state, arguments)?,
        "wiki_run_repo" => wiki_run_repo(repo_state, arguments)?,
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
    store.append_evidence(id, evidence.clone())?;
    let source_published = store.publish_source_file(repo, &evidence.file)?;
    let page = store.read_page(id)?;
    Ok(json!({
        "status": "evidence_appended",
        "id": page.id,
        "wiki_root": wiki_root,
        "evidence_count": page.evidence.len(),
        "source_published": source_published
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

fn wiki_plan_repo(repo: &Path, arguments: &Value) -> Result<Value> {
    let repo_path = arguments
        .get("repo_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.to_path_buf());
    let wiki_root = wiki_root(&repo_path, arguments);
    let outline = if let Some(outline_value) = arguments.get("outline") {
        serde_json::from_value::<WikiOutline>(outline_value.clone())?
    } else {
        build_repo_wiki_outline(&repo_path, arguments)?
    };
    let root = outline.root.clone();
    let page_count = outline.pages.len();
    let task_count = outline
        .pages
        .iter()
        .map(|page| page.deep_tasks.len())
        .sum::<usize>();
    let store = WikiStore::open(&wiki_root)?;
    store.apply_outline(outline)?;
    Ok(json!({
        "status": "repo_wiki_planned",
        "root": root,
        "pages": page_count,
        "tasks": task_count,
        "wiki_root": wiki_root,
        "outline_ref": "outline.json",
        "planner": if arguments.get("outline").is_some() { "codex_outline" } else { "repo_heuristic" }
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
    let merge = merge_investigation_result(&store, id, question, &result, &task.repo_path)?;

    Ok(json!({
        "status": "deepened",
        "id": id,
        "task_id": result.task_id,
        "wiki_root": wiki_root,
        "sources_appended": merge.sources_appended,
        "source_warnings": merge.source_warnings,
        "followups_appended": merge.followups_appended,
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

fn wiki_run_repo(repo: &Path, arguments: &Value) -> Result<Value> {
    let repo_path = arguments
        .get("repo_path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.to_path_buf());
    let wiki_root = wiki_root(&repo_path, arguments);
    let store = WikiStore::open(&wiki_root)?;
    let outline = store.read_outline()?;
    let root_page_id = arguments
        .get("root_page_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| outline.root.clone());
    if root_page_id.trim().is_empty() {
        anyhow::bail!("missing required string argument: root_page_id");
    }
    let budget_pages = arguments
        .get("budget_pages")
        .or_else(|| arguments.get("task_limit"))
        .and_then(Value::as_u64)
        .unwrap_or(3) as usize;
    let parallelism = arguments
        .get("parallelism")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    let retry_failed = arguments
        .get("retry_failed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let execute = arguments
        .get("execute")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let plan_deeper = arguments
        .get("plan_deeper")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let dry_run_children_only = arguments
        .get("dry_run_children_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut quality = score_wiki_pages(&store, &outline, &root_page_id)?;
    let planned_child_pages = if plan_deeper {
        plan_deeper_wiki_pages(&repo_path, &outline, &root_page_id, &quality)
    } else {
        Vec::new()
    };
    let mut active_outline = outline;
    if execute && !planned_child_pages.is_empty() {
        append_planned_child_pages(&store, &mut active_outline, &planned_child_pages)?;
        quality = score_wiki_pages(&store, &active_outline, &root_page_id)?;
    }
    let selected = select_wiki_tasks(
        &active_outline,
        &root_page_id,
        retry_failed,
        budget_pages,
        &quality,
    );
    let selected_tasks = selected
        .iter()
        .map(|(page_id, task)| {
            let budget = task_budget_for_page(&repo_path, page_id, task, arguments);
            json!({
                "page_id": page_id,
                "task_id": task.id,
                "status": task.status,
                "question": task.question,
                "budget": budget.to_json()
            })
        })
        .collect::<Vec<_>>();
    let (completed, failed, results) = if execute && !dry_run_children_only && !selected.is_empty()
    {
        expand_selected_wiki_tasks(&store, &selected, &repo_path, arguments, parallelism)?
    } else {
        (0, 0, Vec::new())
    };

    Ok(json!({
        "status": if execute { "repo_wiki_run" } else { "repo_wiki_run_planned" },
        "wiki_root": wiki_root,
        "root_page_id": root_page_id,
        "execute": execute,
        "retry_failed": retry_failed,
        "plan_deeper": plan_deeper,
        "budget_pages": budget_pages,
        "parallelism": parallelism,
        "planned_child_pages": planned_child_pages.iter().map(outline_page_to_json).collect::<Vec<_>>(),
        "selected_tasks": selected_tasks,
        "quality": quality.iter().map(WikiPageQuality::to_json).collect::<Vec<_>>(),
        "expansion": {
            "tasks_completed": completed,
            "tasks_failed": failed,
            "results": results
        }
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
    let retry_failed = arguments
        .get("retry_failed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let pending = if retry_failed {
        store.retryable_tasks(root_page_id, task_limit)?
    } else {
        store.pending_tasks(root_page_id, task_limit)?
    };
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
    let (completed, failed, summaries) =
        expand_selected_wiki_tasks(&store, &pending, &repo_path, arguments, parallelism)?;

    Ok(json!({
        "status": "expanded",
        "wiki_root": wiki_root,
        "root_page_id": root_page_id,
        "tasks_completed": completed,
        "tasks_failed": failed,
        "results": summaries
    }))
}

fn expand_selected_wiki_tasks(
    store: &WikiStore,
    pending: &[(String, DeepTask)],
    repo_path: &Path,
    arguments: &Value,
    parallelism: usize,
) -> Result<(usize, usize, Vec<Value>)> {
    let mut task_budgets = HashMap::new();
    let tasks = pending
        .iter()
        .map(|(page_id, task)| {
            let budget = task_budget_for_page(repo_path, page_id, task, arguments);
            task_budgets.insert(task.id.clone(), budget.clone());
            InvestigationTask {
                task_id: task.id.clone(),
                question: task.question.clone(),
                repo_path: repo_path.to_path_buf(),
                scope: InvestigationScope {
                    paths: task.scope_paths.clone(),
                    symbols: task.scope_symbols.clone(),
                },
                expected_outputs: task.expected_outputs.clone(),
                budget: budget.into_investigation_budget(),
            }
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

    for (page_id, task) in pending {
        store.update_task_status(page_id, &task.id, TaskStatus::Running, None, None)?;
    }

    let results = investigate_many_outcomes(agent, tasks, parallelism);
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut summaries = Vec::new();
    for outcome in results {
        match outcome {
            InvestigationOutcome::Done(result) => {
                let Some((page_id, task)) =
                    pending.iter().find(|(_, task)| task.id == result.task_id)
                else {
                    failed += 1;
                    summaries.push(json!({
                        "task_id": result.task_id,
                        "status": "failed",
                        "error": "result task id was not pending"
                    }));
                    continue;
                };
                match merge_investigation_result(store, page_id, &task.question, &result, repo_path)
                {
                    Ok(merge) => {
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
                            "sources_appended": merge.sources_appended,
                            "source_warnings": merge.source_warnings,
                            "followups_appended": merge.followups_appended,
                            "confidence": result.confidence,
                            "budget": task_budgets.get(&result.task_id).map(TaskBudget::to_json)
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
                            "error": error.to_string(),
                            "budget": task_budgets.get(&result.task_id).map(TaskBudget::to_json)
                        }));
                    }
                }
            }
            InvestigationOutcome::Failed { task_id, error } => {
                let Some((page_id, task)) = pending.iter().find(|(_, task)| task.id == task_id)
                else {
                    failed += 1;
                    summaries.push(json!({
                        "task_id": task_id,
                        "status": "failed",
                        "error": error
                    }));
                    continue;
                };
                store.update_task_status(
                    page_id,
                    &task.id,
                    TaskStatus::Failed,
                    None,
                    Some(error.clone()),
                )?;
                failed += 1;
                summaries.push(json!({
                    "task_id": task_id,
                    "page_id": page_id,
                    "status": "failed",
                    "error": error,
                    "budget": task_budgets.get(&task_id).map(TaskBudget::to_json)
                }));
            }
        }
    }

    Ok((completed, failed, summaries))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskBudget {
    max_nodes: usize,
    max_depth: usize,
    timeout_secs: u64,
    reason: &'static str,
    scope_files: usize,
}

impl TaskBudget {
    fn into_investigation_budget(self) -> InvestigationBudget {
        InvestigationBudget {
            max_nodes: self.max_nodes,
            max_depth: self.max_depth,
            timeout_secs: self.timeout_secs,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "max_nodes": self.max_nodes,
            "max_depth": self.max_depth,
            "timeout_secs": self.timeout_secs,
            "reason": self.reason,
            "scope_files": self.scope_files
        })
    }
}

fn task_budget_for_page(
    repo_path: &Path,
    page_id: &str,
    task: &DeepTask,
    arguments: &Value,
) -> TaskBudget {
    let default_nodes = arguments
        .get("max_nodes")
        .and_then(Value::as_u64)
        .unwrap_or(80) as usize;
    let default_depth = arguments
        .get("max_depth")
        .and_then(Value::as_u64)
        .unwrap_or(4) as usize;
    let default_timeout = arguments
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(120);
    let scope_files = count_scope_files(repo_path, &task.scope_paths, 5000);
    let wide_scope = scope_files >= 300
        || task.scope_paths.iter().any(|path| {
            matches!(
                path.as_str(),
                "aios/storage/indexlib/file_system"
                    | "aios/storage/indexlib/index"
                    | "aios/storage/indexlib/table"
            )
        });
    let medium_scope = scope_files >= 80
        || page_id.ends_with("/tablet")
        || page_id.ends_with("/kv-kkv")
        || page_id.ends_with("/normal-table");

    let (nodes, depth, timeout, reason) = if wide_scope {
        (160, 5, 600, "wide_scope")
    } else if medium_scope {
        (112, 4, 420, "medium_scope")
    } else {
        (80, 4, 300, "default_scope")
    };

    TaskBudget {
        max_nodes: task.max_nodes.unwrap_or(default_nodes.max(nodes)),
        max_depth: task.max_depth.unwrap_or(default_depth.max(depth)),
        timeout_secs: task.timeout_secs.unwrap_or(default_timeout.max(timeout)),
        reason,
        scope_files,
    }
}

fn count_scope_files(repo_path: &Path, scope_paths: &[String], cap: usize) -> usize {
    let mut total = 0usize;
    for scope_path in scope_paths {
        total += count_files_under(&repo_path.join(scope_path), cap.saturating_sub(total));
        if total >= cap {
            break;
        }
    }
    total
}

fn count_files_under(path: &Path, cap: usize) -> usize {
    if cap == 0 || !path.exists() {
        return 0;
    }
    if path.is_file() {
        return 1;
    }
    let mut count = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if count >= cap {
                return count;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                count += 1;
            }
        }
    }
    count
}

#[derive(Debug, Clone)]
struct WikiPageQuality {
    page_id: String,
    title: String,
    score: u64,
    content_chars: usize,
    citations: usize,
    evidence: usize,
    diagrams: usize,
    pending_tasks: usize,
    failed_tasks: usize,
    done_tasks: usize,
    reasons: Vec<&'static str>,
}

impl WikiPageQuality {
    fn to_json(&self) -> Value {
        json!({
            "page_id": self.page_id,
            "title": self.title,
            "score": self.score,
            "content_chars": self.content_chars,
            "citations": self.citations,
            "evidence": self.evidence,
            "diagrams": self.diagrams,
            "pending_tasks": self.pending_tasks,
            "failed_tasks": self.failed_tasks,
            "done_tasks": self.done_tasks,
            "reasons": self.reasons
        })
    }
}

fn score_wiki_pages(
    store: &WikiStore,
    outline: &WikiOutline,
    root_page_id: &str,
) -> Result<Vec<WikiPageQuality>> {
    let mut quality = Vec::new();
    for outline_page in &outline.pages {
        if !is_page_under_root_id(&outline_page.id, root_page_id) {
            continue;
        }
        let page = store.read_page(&outline_page.id)?;
        quality.push(score_wiki_page(&page, outline_page));
    }
    quality.sort_by(|a, b| {
        a.score
            .cmp(&b.score)
            .then_with(|| a.page_id.cmp(&b.page_id))
    });
    Ok(quality)
}

fn score_wiki_page(page: &WikiPage, outline_page: &OutlinePage) -> WikiPageQuality {
    let content = page.content.trim();
    let content_chars = content.chars().count();
    let citations = content.matches("{{source:").count() + content.matches("{{src:").count();
    let diagrams = content.matches("```mermaid").count() + content.matches("```svg").count();
    let pending_tasks = outline_page
        .deep_tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Pending)
        .count();
    let failed_tasks = outline_page
        .deep_tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Failed)
        .count();
    let done_tasks = outline_page
        .deep_tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Done)
        .count();

    let mut score = 0u64;
    let mut reasons = Vec::new();
    if page
        .summary
        .as_ref()
        .is_some_and(|summary| !summary.trim().is_empty())
    {
        score += 10;
    } else {
        reasons.push("missing_summary");
    }
    score += if content_chars >= 700 {
        25
    } else if content_chars >= 240 {
        15
    } else if content_chars >= 80 {
        5
    } else {
        reasons.push("thin_content");
        0
    };
    if citations > 0 {
        score += (citations.min(3) as u64) * 10;
    } else {
        reasons.push("no_inline_citations");
    }
    if page.evidence.is_empty() {
        reasons.push("no_source_spans");
    } else {
        score += (page.evidence.len().min(5) as u64) * 4;
    }
    if diagrams > 0 {
        score += 15;
    } else {
        reasons.push("no_diagram");
    }
    if done_tasks > 0 {
        score += 10;
    }
    if done_tasks > 0 && pending_tasks == 0 && failed_tasks == 0 {
        score += 10;
    }
    if pending_tasks > 0 {
        reasons.push("pending_tasks");
    }
    if failed_tasks > 0 {
        reasons.push("failed_tasks");
        score = score.saturating_sub(10);
    }

    WikiPageQuality {
        page_id: page.id.clone(),
        title: page.title.clone(),
        score: score.min(100),
        content_chars,
        citations,
        evidence: page.evidence.len(),
        diagrams,
        pending_tasks,
        failed_tasks,
        done_tasks,
        reasons,
    }
}

fn select_wiki_tasks(
    outline: &WikiOutline,
    root_page_id: &str,
    retry_failed: bool,
    budget_pages: usize,
    quality: &[WikiPageQuality],
) -> Vec<(String, DeepTask)> {
    let mut selected = Vec::new();
    for page_quality in quality {
        if selected.len() >= budget_pages {
            break;
        }
        let Some(page) = outline
            .pages
            .iter()
            .find(|page| page.id == page_quality.page_id)
        else {
            continue;
        };
        if !is_page_under_root_id(&page.id, root_page_id) {
            continue;
        }
        let Some(task) = page.deep_tasks.iter().find(|task| {
            task.status == TaskStatus::Pending
                || (retry_failed && task.status == TaskStatus::Failed)
        }) else {
            continue;
        };
        selected.push((page.id.clone(), task.clone()));
    }
    selected
}

fn plan_deeper_wiki_pages(
    repo_path: &Path,
    outline: &WikiOutline,
    root_page_id: &str,
    quality: &[WikiPageQuality],
) -> Vec<OutlinePage> {
    let existing = outline
        .pages
        .iter()
        .map(|page| page.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut planned = Vec::new();
    for page_quality in quality {
        if (page_quality.done_tasks == 0 && page_quality.failed_tasks == 0)
            || page_quality.pending_tasks > 0
            || !is_page_under_root_id(&page_quality.page_id, root_page_id)
        {
            continue;
        }
        for spec in deeper_page_specs(&page_quality.page_id) {
            if existing.contains(spec.id.as_str()) {
                continue;
            }
            if !spec.paths.iter().any(|path| repo_path.join(path).exists()) {
                continue;
            }
            planned.push(spec.into_outline_page());
        }
    }
    planned
}

fn append_planned_child_pages(
    store: &WikiStore,
    outline: &mut WikiOutline,
    pages: &[OutlinePage],
) -> Result<()> {
    for page in pages {
        let mut wiki_page = WikiPage::new(&page.id, &page.title, page.kind);
        wiki_page.summary = page.summary.clone();
        wiki_page.parent = page.parent.clone();
        store.upsert_page(wiki_page)?;
        if let Some(content) = &page.content {
            store.patch_page(
                &page.id,
                ContentFormat::Markdown,
                content,
                PatchMode::Replace,
            )?;
        }
        let private_note = planned_child_private_note(page);
        if !private_note.is_empty() {
            store.patch_private_note(&page.id, &private_note, PatchMode::Replace)?;
        }
        outline.pages.push(page.clone());
    }
    store.write_outline(outline)
}

fn planned_child_private_note(page: &OutlinePage) -> String {
    let mut note = String::new();
    if let Some(purpose) = &page.purpose {
        note.push_str("Purpose\n");
        note.push_str(purpose.trim());
    }
    if !page.deep_tasks.is_empty() {
        if !note.is_empty() {
            note.push_str("\n\n");
        }
        note.push_str("Deep Tasks\n");
        for task in &page.deep_tasks {
            note.push_str("- [");
            note.push_str(task_status_str(task.status));
            note.push_str("] ");
            note.push_str(&task.id);
            note.push_str(": ");
            note.push_str(task.question.trim());
            note.push('\n');
        }
    }
    note.trim_end().to_string()
}

fn task_status_str(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::Running => "running",
        TaskStatus::Done => "done",
        TaskStatus::Failed => "failed",
    }
}

fn outline_page_to_json(page: &OutlinePage) -> Value {
    json!({
        "id": page.id,
        "title": page.title,
        "kind": page.kind,
        "summary": page.summary,
        "parent": page.parent,
        "tasks": page.deep_tasks.iter().map(|task| {
            json!({
                "id": task.id,
                "question": task.question,
                "status": task.status
            })
        }).collect::<Vec<_>>()
    })
}

struct DeeperPageSpec {
    id: String,
    title: String,
    summary: String,
    parent: String,
    paths: Vec<String>,
    symbols: Vec<String>,
    question: String,
    diagram_hint: &'static str,
}

impl DeeperPageSpec {
    fn into_outline_page(self) -> OutlinePage {
        let task_id = format!("map-{}", self.id.replace('/', "-"));
        let scope = self.summary.clone();
        OutlinePage {
            id: self.id,
            title: self.title,
            kind: PageKind::Index,
            summary: Some(self.summary),
            parent: Some(self.parent),
            purpose: Some(
                "Expand a high-complexity subsystem into a deeper code-wiki page.".to_string(),
            ),
            content: Some(format!("## Scope\n{scope}\n")),
            deep_tasks: vec![DeepTask {
                id: task_id,
                question: format!(
                    "{} Use wiki-style sections, inline citations, and a compact {} diagram.",
                    self.question, self.diagram_hint
                ),
                scope_paths: self.paths,
                scope_symbols: self.symbols,
                expected_outputs: vec![
                    "wiki_style_summary".to_string(),
                    "inline_citations".to_string(),
                    "sources".to_string(),
                    "diagram".to_string(),
                    "followups".to_string(),
                ],
                max_nodes: None,
                max_depth: None,
                timeout_secs: None,
                status: TaskStatus::Pending,
                confidence: None,
                error: None,
            }],
        }
    }
}

fn deeper_page_specs(page_id: &str) -> Vec<DeeperPageSpec> {
    match page_id {
        "havenask/storage/indexlib" => indexlib_deeper_page_specs(page_id),
        "havenask/storage/indexlib/file-system" => indexlib_file_system_deeper_page_specs(page_id),
        "havenask/ha3" => ha3_deeper_page_specs(page_id),
        _ => Vec::new(),
    }
}

fn indexlib_deeper_page_specs(parent: &str) -> Vec<DeeperPageSpec> {
    vec![
        DeeperPageSpec {
            id: format!("{parent}/base-structures"),
            title: "Base Structures".to_string(),
            summary: "Core status/result types, framework primitives, tablet data structures, schema/config boundaries, and lifecycle support.".to_string(),
            parent: parent.to_string(),
            paths: vec![
                "aios/storage/indexlib/base".to_string(),
                "aios/storage/indexlib/framework".to_string(),
                "aios/storage/indexlib/config".to_string(),
            ],
            symbols: vec!["Status".to_string(), "TabletData".to_string(), "ITabletFactory".to_string()],
            question: "Map indexlib base structures and framework primitives: status/result flow, tablet data, schema/config boundaries, lifecycle hooks, and how higher layers depend on them.".to_string(),
            diagram_hint: "dependency",
        },
        DeeperPageSpec {
            id: format!("{parent}/file-system"),
            title: "File System".to_string(),
            summary: "Indexlib file system, directory abstraction, package files, load config, flush, archive, WAL, and fslib integration.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/file_system".to_string()],
            symbols: vec!["IFileSystem".to_string(), "Directory".to_string(), "FileReader".to_string()],
            question: "Map the indexlib file-system layer: filesystem/directory abstractions, readers/writers, package files, load config, flush/archive/WAL, and how table/index code reaches storage.".to_string(),
            diagram_hint: "layering",
        },
        DeeperPageSpec {
            id: format!("{parent}/index"),
            title: "Index Layer".to_string(),
            summary: "Index plugin boundary, inverted index, attribute, summary, source, primary key, deletion map, and per-index reader/writer families.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/index".to_string()],
            symbols: vec!["InvertedIndexReader".to_string(), "IIndexReader".to_string(), "IIndexFactory".to_string()],
            question: "Map the index layer: plugin/factory boundary, reader/writer families, inverted index, attribute, summary, source, primary key, deletion map, and how table readers bind indexes.".to_string(),
            diagram_hint: "module",
        },
        DeeperPageSpec {
            id: format!("{parent}/kv-kkv"),
            title: "KV / KKV".to_string(),
            summary: "KV and KKV table readers/writers, pkey/skey lookup, value formatting, cache reader, segment readers, and merge/index-task flow.".to_string(),
            parent: parent.to_string(),
            paths: vec![
                "aios/storage/indexlib/table/kv_table".to_string(),
                "aios/storage/indexlib/table/kkv_table".to_string(),
                "aios/storage/indexlib/index/kv".to_string(),
                "aios/storage/indexlib/index/kkv".to_string(),
            ],
            symbols: vec!["KVTabletReader".to_string(), "KKVTabletReader".to_string(), "KVIndexReader".to_string()],
            question: "Map KV and KKV: table reader/writer flow, pkey/skey lookup, value formatting, cache readers, segment readers, and merge/index-task boundaries.".to_string(),
            diagram_hint: "lookup",
        },
        DeeperPageSpec {
            id: format!("{parent}/normal-table"),
            title: "Normal Table".to_string(),
            summary: "Normal-table tablet reader/writer, index reader wiring, deletion map, summary/source/attribute integration, and index task lifecycle.".to_string(),
            parent: parent.to_string(),
            paths: vec![
                "aios/storage/indexlib/table/normal_table".to_string(),
                "aios/storage/indexlib/indexlib/partition".to_string(),
            ],
            symbols: vec!["NormalTabletReader".to_string(), "NormalTabletWriter".to_string(), "OnlinePartitionReader".to_string()],
            question: "Map normal-table serving and build flow: tablet reader/writer, index reader wiring, deletion map, summary/source/attribute integration, and legacy partition handoff.".to_string(),
            diagram_hint: "lifecycle",
        },
        DeeperPageSpec {
            id: format!("{parent}/tablet"),
            title: "Tablet Framework".to_string(),
            summary: "Tablet abstraction, tablet reader/writer lifecycle, tablet data, read/write resources, schema/config, and index task orchestration.".to_string(),
            parent: parent.to_string(),
            paths: vec![
                "aios/storage/indexlib/table".to_string(),
                "aios/storage/indexlib/framework".to_string(),
            ],
            symbols: vec!["Tablet".to_string(), "ITablet".to_string(), "TabletWriter".to_string(), "TabletReader".to_string()],
            question: "Map the tablet framework: ITablet/Tablet lifecycle, reader/writer responsibilities, TabletData, read/write resources, schema/config loading, and index task orchestration.".to_string(),
            diagram_hint: "lifecycle",
        },
    ]
}

fn indexlib_file_system_deeper_page_specs(parent: &str) -> Vec<DeeperPageSpec> {
    vec![
        DeeperPageSpec {
            id: format!("{parent}/file"),
            title: "Files And Directories".to_string(),
            summary: "Directory, file node, file reader/writer abstractions, storage metrics, and how higher layers open index files.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/file_system/file".to_string()],
            symbols: vec![
                "Directory".to_string(),
                "FileReader".to_string(),
                "FileWriter".to_string(),
            ],
            question: "Map file-system file and directory abstractions: Directory APIs, file nodes, readers/writers, storage metrics, and how index/table code opens files.".to_string(),
            diagram_hint: "class",
        },
        DeeperPageSpec {
            id: format!("{parent}/fslib"),
            title: "Fslib Integration".to_string(),
            summary: "Fslib wrapper, storage backend handoff, path utilities, and local/remote file access boundary.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/file_system/fslib".to_string()],
            symbols: vec!["FslibWrapper".to_string(), "FileSystemWrapper".to_string()],
            question: "Map fslib integration: wrapper responsibilities, backend handoff, path utilities, local/remote file access, and error propagation.".to_string(),
            diagram_hint: "boundary",
        },
        DeeperPageSpec {
            id: format!("{parent}/load-config"),
            title: "Load Config".to_string(),
            summary: "LoadConfig, lifecycle strategy, cache decisions, mmap/direct-io behavior, and reader selection rules.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/file_system/load_config".to_string()],
            symbols: vec!["LoadConfig".to_string(), "LoadConfigList".to_string()],
            question: "Map load-config behavior: lifecycle strategy, cache decisions, mmap/direct-io settings, reader selection rules, and how options reach file opening.".to_string(),
            diagram_hint: "decision",
        },
        DeeperPageSpec {
            id: format!("{parent}/package"),
            title: "Package Files".to_string(),
            summary: "Package file metadata, package reader/writer flow, directory packaging, and segment file aggregation.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/file_system/package".to_string()],
            symbols: vec![
                "PackageFileWriter".to_string(),
                "PackageFileReader".to_string(),
            ],
            question: "Map package files: package metadata, reader/writer flow, directory packaging, segment aggregation, and reopen/read behavior.".to_string(),
            diagram_hint: "flow",
        },
        DeeperPageSpec {
            id: format!("{parent}/flush"),
            title: "Flush".to_string(),
            summary: "Flush operation queue, file flushing, storage sync, lifecycle handoff, and error handling.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/file_system/flush".to_string()],
            symbols: vec![
                "FlushOperation".to_string(),
                "FlushFileOperation".to_string(),
            ],
            question: "Map file-system flush: operation queue, file flushing, storage sync, lifecycle handoff, and failure handling.".to_string(),
            diagram_hint: "sequence",
        },
        DeeperPageSpec {
            id: format!("{parent}/wal"),
            title: "WAL".to_string(),
            summary: "Write-ahead-log file handling, writer/reader lifecycle, recovery boundary, and consistency guarantees.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/file_system/wal".to_string()],
            symbols: vec!["Wal".to_string(), "WalWriter".to_string()],
            question: "Map WAL support: writer/reader lifecycle, recovery boundary, consistency guarantees, and how WAL integrates with filesystem operations.".to_string(),
            diagram_hint: "lifecycle",
        },
        DeeperPageSpec {
            id: format!("{parent}/archive"),
            title: "Archive".to_string(),
            summary: "Archive folder behavior, archive files, package/archive relationship, and read/write paths.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/storage/indexlib/file_system/archive".to_string()],
            symbols: vec!["ArchiveFolder".to_string(), "ArchiveFile".to_string()],
            question: "Map archive support: archive folders/files, package/archive relationship, read/write paths, and lifecycle behavior.".to_string(),
            diagram_hint: "flow",
        },
    ]
}

fn ha3_deeper_page_specs(parent: &str) -> Vec<DeeperPageSpec> {
    vec![
        DeeperPageSpec {
            id: format!("{parent}/queryparser"),
            title: "Query Parser".to_string(),
            summary: "Query syntax scanning/parsing, expression tree construction, default operators, labels, range handling, and evaluator handoff.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/ha3/ha3/queryparser".to_string()],
            symbols: vec!["QueryParser".to_string(), "DefaultQueryExprEvaluator".to_string()],
            question: "Map HA3 queryparser: scanner/parser flow, grammar features, expression node factories, default operators, labels, range handling, and evaluator handoff.".to_string(),
            diagram_hint: "flow",
        },
        DeeperPageSpec {
            id: format!("{parent}/common-query"),
            title: "Common Query Model".to_string(),
            summary: "Common Query hierarchy, terms, boolean/rank composition, visitor dispatch, labels, match-data level, and serialization behavior.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/ha3/ha3/common/query".to_string()],
            symbols: vec!["Query".to_string(), "QueryVisitor".to_string(), "TermQuery".to_string()],
            question: "Map the common Query model: query hierarchy, term/phrase/range/multi-term types, boolean and rank composition, visitor dispatch, labels, and match-data level.".to_string(),
            diagram_hint: "class",
        },
        DeeperPageSpec {
            id: format!("{parent}/search/query-executors"),
            title: "Query Executors".to_string(),
            summary: "QueryExecutorCreator dispatch, term/phrase/boolean/rank/number/doc-id executors, posting iterator selection, match-data wiring, and timeout handling.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/ha3/ha3/search/query_executor".to_string()],
            symbols: vec!["QueryExecutorCreator".to_string(), "QueryExecutor".to_string()],
            question: "Map query executors deeply: QueryExecutorCreator visitor dispatch, term/phrase/boolean/rank/number/doc-id executors, posting iterator selection, match-data wiring, and timeout handling.".to_string(),
            diagram_hint: "dispatch",
        },
        DeeperPageSpec {
            id: format!("{parent}/search/single-layer-searcher"),
            title: "Single Layer Searcher".to_string(),
            summary: "SingleLayerSearcher runtime handoff, layer metadata, query executor lifecycle, filter integration, match data, seek loop, and result collection.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/ha3/ha3/search".to_string()],
            symbols: vec!["SingleLayerSearcher".to_string(), "Filter".to_string(), "MatchDataManager".to_string()],
            question: "Map SingleLayerSearcher: runtime setup, layer metadata, query executor lifecycle, filters, match data, seek loop, timeout checks, and result collection.".to_string(),
            diagram_hint: "runtime",
        },
        DeeperPageSpec {
            id: format!("{parent}/rank"),
            title: "Ranking".to_string(),
            summary: "Ranking comparator stack, scorer integration, matchdoc references, sorter behavior, rank profile handoff, and priority queue path.".to_string(),
            parent: parent.to_string(),
            paths: vec!["aios/ha3/ha3/rank".to_string()],
            symbols: vec!["Comparator".to_string(), "Scorer".to_string(), "RankProfile".to_string()],
            question: "Map HA3 ranking: comparator stack, scorer integration, matchdoc references, sorter behavior, rank profile handoff, and priority queue path.".to_string(),
            diagram_hint: "flow",
        },
    ]
}

fn is_page_under_root_id(page_id: &str, root_page_id: &str) -> bool {
    page_id == root_page_id
        || page_id
            .strip_prefix(root_page_id)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn merge_investigation_result(
    store: &WikiStore,
    id: &str,
    question: &str,
    result: &InvestigationResult,
    repo_path: &Path,
) -> Result<MergeSummary> {
    let existing_evidence_count = store.read_page(id)?.evidence.len();
    let mut source_index_map = HashMap::new();
    let mut sources_appended = 0usize;
    let mut source_warnings = Vec::new();
    for (source_index, source) in result.sources.iter().enumerate() {
        match validate_investigation_source(repo_path, source) {
            Ok(()) => {
                store.append_evidence(id, evidence_from_investigation_source(source))?;
                store.publish_source_file(repo_path, &source.file)?;
                sources_appended += 1;
                source_index_map
                    .insert(source_index + 1, existing_evidence_count + sources_appended);
            }
            Err(warning) => source_warnings.push(warning.to_string()),
        }
    }
    if !result.sources.is_empty() && sources_appended == 0 {
        anyhow::bail!(
            "atlas returned no valid source spans ({} rejected)",
            source_warnings.len()
        );
    }

    let mut public_patch = String::new();
    public_patch.push_str("\n\n## Overview\n\n");
    let summary_markdown = strip_public_diagram_blocks(result.summary_markdown.trim());
    public_patch.push_str(&remap_source_markers(
        summary_markdown.trim(),
        &source_index_map,
    ));
    public_patch.push('\n');
    if let Some(diagram) = &result.diagram {
        public_patch.push_str("\n## Flow\n\n");
        public_patch.push_str(&diagram_markdown(diagram));
    }
    store.patch_page(
        id,
        ContentFormat::Markdown,
        &public_patch,
        PatchMode::Append,
    )?;

    let mut private_patch = String::new();
    private_patch.push_str("Atlas Question\n");
    private_patch.push_str(question.trim());
    if !source_warnings.is_empty() {
        private_patch.push_str("\n\nAtlas Source Warnings\n");
        for warning in &source_warnings {
            private_patch.push_str("- ");
            private_patch.push_str(warning);
            private_patch.push('\n');
        }
    }
    if !result.followups.is_empty() {
        private_patch.push_str("\n\nAtlas Follow-ups\n");
        for followup in &result.followups {
            private_patch.push_str("- ");
            private_patch.push_str(followup);
            private_patch.push('\n');
        }
    }
    if !private_patch.trim().is_empty() {
        store.patch_private_note(id, private_patch.trim_end(), PatchMode::Append)?;
    }
    Ok(MergeSummary {
        sources_appended,
        source_warnings: source_warnings.len(),
        followups_appended: result.followups.len(),
    })
}

fn remap_source_markers(content: &str, source_index_map: &HashMap<usize, usize>) -> String {
    let mut output = String::new();
    let mut remaining = content;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find("}}") else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let marker = &after_start[..end];
        if let Some(index) = parse_source_marker_index(marker) {
            if let Some(mapped_index) = source_index_map.get(&index) {
                output.push_str("{{source:");
                output.push_str(&mapped_index.to_string());
                output.push_str("}}");
            }
        } else {
            output.push_str("{{");
            output.push_str(marker);
            output.push_str("}}");
        }
        remaining = &after_start[end + 2..];
    }
    output.push_str(remaining);
    output
}

fn strip_public_diagram_blocks(content: &str) -> String {
    let mut output = Vec::new();
    let mut lines = content.lines().peekable();
    while let Some(line) = lines.next() {
        if is_diagram_fence(line) {
            remove_preceding_diagram_heading(&mut output);
            for next in lines.by_ref() {
                if next.trim_start().starts_with("```") {
                    break;
                }
            }
            continue;
        }
        output.push(line.to_string());
    }
    output.join("\n")
}

fn is_diagram_fence(line: &str) -> bool {
    let language = line
        .trim_start()
        .strip_prefix("```")
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    language.starts_with("mermaid") || language.starts_with("svg")
}

fn remove_preceding_diagram_heading(output: &mut Vec<String>) {
    while output.last().is_some_and(|line| line.trim().is_empty()) {
        output.pop();
    }
    if output
        .last()
        .is_some_and(|line| is_public_diagram_heading(line))
    {
        output.pop();
    }
    while output.last().is_some_and(|line| line.trim().is_empty()) {
        output.pop();
    }
}

fn is_public_diagram_heading(line: &str) -> bool {
    matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "flow"
            | "**flow**"
            | "## flow"
            | "### flow"
            | "diagram"
            | "**diagram**"
            | "## diagram"
            | "### diagram"
    )
}

fn parse_source_marker_index(marker: &str) -> Option<usize> {
    marker
        .strip_prefix("source:")
        .or_else(|| marker.strip_prefix("src:"))?
        .trim()
        .parse::<usize>()
        .ok()
}

struct MergeSummary {
    sources_appended: usize,
    source_warnings: usize,
    followups_appended: usize,
}

fn validate_investigation_source(repo_path: &Path, source: &InvestigationSource) -> Result<()> {
    let file = source.file.trim();
    if file.is_empty() {
        anyhow::bail!("empty source file path");
    }
    let relative_path = Path::new(file);
    if relative_path.is_absolute() {
        anyhow::bail!("{file}: absolute paths are not allowed");
    }
    if relative_path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        anyhow::bail!("{file}: parent directory traversal is not allowed");
    }

    let full_path = repo_path.join(relative_path);
    if !full_path.is_file() {
        anyhow::bail!("{file}: source file does not exist");
    }

    let line_count = std::fs::read_to_string(&full_path)
        .map(|content| content.lines().count() as u32)
        .unwrap_or(0);
    if let (Some(start), Some(end)) = (source.start_line, source.end_line) {
        if start > end {
            anyhow::bail!("{file}: start_line is after end_line ({start} > {end})");
        }
    }
    if let Some(start) = source.start_line {
        if start == 0 || start > line_count {
            anyhow::bail!("{file}: start_line {start} is line out of range 1..={line_count}");
        }
    }
    if let Some(end) = source.end_line {
        if end == 0 || end > line_count {
            anyhow::bail!("{file}: end_line {end} is line out of range 1..={line_count}");
        }
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

fn build_repo_wiki_outline(repo_path: &Path, arguments: &Value) -> Result<WikiOutline> {
    let root_id = arguments
        .get("root_id")
        .and_then(Value::as_str)
        .map(safe_page_segment)
        .unwrap_or_else(|| {
            repo_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(safe_page_segment)
                .unwrap_or_else(|| "repo".to_string())
        });
    let title = arguments
        .get("title")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| title_from_segment(&root_id));
    let summary = arguments
        .get("summary")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            "Repository-level code wiki generated from the source tree.".to_string()
        });
    let max_pages = arguments
        .get("max_pages")
        .and_then(Value::as_u64)
        .unwrap_or(8) as usize;
    let focus_paths = string_array(arguments, "focus_paths");

    let mut pages = vec![OutlinePage {
        id: root_id.clone(),
        title,
        kind: PageKind::Index,
        summary: Some(summary),
        parent: None,
        purpose: Some("Repo-level entry point. Keep this page navigational and link readers into subsystem pages.".to_string()),
        content: Some("## Reading Path\nStart with the subsystem pages below. Expand each page only after its scope and task are clear.".to_string()),
        deep_tasks: Vec::new(),
    }];

    for component in repo_components() {
        if pages.len().saturating_sub(1) >= max_pages {
            break;
        }
        if !focus_paths.is_empty()
            && !focus_paths.iter().any(|focus| {
                component
                    .paths
                    .iter()
                    .any(|path| path.starts_with(focus) || focus.starts_with(path))
            })
        {
            continue;
        }
        if !component
            .paths
            .iter()
            .any(|path| repo_path.join(path).exists())
        {
            continue;
        }
        let id = format!("{}/{}", root_id, component.slug);
        pages.push(OutlinePage {
            id: id.clone(),
            title: component.title.to_string(),
            kind: PageKind::Index,
            summary: Some(component.summary.to_string()),
            parent: Some(root_id.clone()),
            purpose: Some(format!(
                "Explain the {} subsystem as part of the repo wiki. Keep details grounded in source citations.",
                component.title
            )),
            content: Some(format!(
                "## Scope\n{}\n\n## Next Question\n{}",
                component.summary, component.question
            )),
            deep_tasks: vec![DeepTask {
                id: format!("map-{}", component.slug.replace('/', "-")),
                question: component.question.to_string(),
                scope_paths: component.paths.iter().map(|path| path.to_string()).collect(),
                scope_symbols: component
                    .symbols
                    .iter()
                    .map(|symbol| symbol.to_string())
                    .collect(),
                expected_outputs: vec![
                    "wiki_style_summary".to_string(),
                    "inline_citations".to_string(),
                    "sources".to_string(),
                    "diagram".to_string(),
                    "followups".to_string(),
                ],
                max_nodes: None,
                max_depth: None,
                timeout_secs: None,
                status: TaskStatus::Pending,
                confidence: None,
                error: None,
            }],
        });
    }

    Ok(WikiOutline {
        version: 1,
        root: root_id,
        pages,
    })
}

struct RepoComponent {
    slug: &'static str,
    title: &'static str,
    summary: &'static str,
    paths: &'static [&'static str],
    symbols: &'static [&'static str],
    question: &'static str,
}

fn repo_components() -> Vec<RepoComponent> {
    vec![
        RepoComponent {
            slug: "ha3",
            title: "HA3 Search Layer",
            summary: "Query parsing, common query model, executor creation, filtering, ranking, and search runtime.",
            paths: &["aios/ha3/ha3"],
            symbols: &["QueryExecutorCreator", "QueryParser", "SingleLayerSearcher"],
            question: "Map the HA3 search layer: query parsing, query model, executor creation, filtering, ranking, and the main runtime handoff. Use wiki-style sections, inline citations, and one compact flow diagram.",
        },
        RepoComponent {
            slug: "storage/indexlib",
            title: "Storage / indexlib",
            summary: "Index storage, table/partition lifecycle, inverted index, attributes, summary, KV/KKV, and tablet framework.",
            paths: &["aios/storage/indexlib"],
            symbols: &["Tablet", "IndexPartition", "InvertedIndexReader"],
            question: "Map indexlib at subsystem level: table lifecycle, read/write path, inverted index, attributes, summary, KV/KKV, and tablet boundaries. Use wiki-style sections, inline citations, and a lifecycle or module diagram.",
        },
        RepoComponent {
            slug: "sql",
            title: "SQL / Iquan",
            summary: "SQL planning, optimization, Navi execution kernels, catalog integration, and scan/join/agg operators.",
            paths: &["aios/sql"],
            symbols: &["Iquan", "ScanKernel", "SqlSearchInfoCollector"],
            question: "Map the SQL subsystem: optimizer boundary, plan execution, core Navi kernels, scan/join/agg path, and catalog/config dependencies. Use wiki-style sections and inline citations.",
        },
        RepoComponent {
            slug: "suez",
            title: "Suez Service Framework",
            summary: "Serving framework, table management, deployment orchestration, heartbeat, and role runtime.",
            paths: &["aios/suez"],
            symbols: &["SuezServer", "TableManager", "HeartbeatManager"],
            question: "Map Suez as the service/runtime layer: table management, deployment orchestration, heartbeat, and how search roles are wired. Use wiki-style sections, inline citations, and a compact relationship diagram.",
        },
        RepoComponent {
            slug: "build-service",
            title: "Build Service",
            summary: "Distributed index build administration, builder tasks, processors, mergers, and worker orchestration.",
            paths: &["aios/apps/facility/build_service", "aios/build_service"],
            symbols: &["BuilderController", "BuildTask", "Processor"],
            question: "Map Build Service: admin, processor/builder/merger roles, task lifecycle, and handoff to index storage. Use wiki-style sections, inline citations, and a lifecycle diagram.",
        },
        RepoComponent {
            slug: "navi",
            title: "Navi DAG Execution",
            summary: "Graph construction and dataflow execution engine used by SQL/search runtime.",
            paths: &["aios/navi"],
            symbols: &["Graph", "GraphBuilder", "Kernel"],
            question: "Map Navi: graph model, kernel execution, dataflow scheduling, and integration points with SQL/search. Use wiki-style sections, inline citations, and a flow diagram.",
        },
        RepoComponent {
            slug: "swift",
            title: "Swift Messaging",
            summary: "Messaging and streaming infrastructure used by build and service workflows.",
            paths: &["aios/swift"],
            symbols: &["SwiftAdmin", "SwiftReader", "SwiftWriter"],
            question: "Map Swift messaging: admin, topic/partition model, reader/writer flow, and integrations with build/service components. Use wiki-style sections and inline citations.",
        },
        RepoComponent {
            slug: "catalog",
            title: "Catalog Metadata",
            summary: "Metadata service and entity model for database, table, partition, and schema state.",
            paths: &["aios/catalog"],
            symbols: &["Catalog", "Database", "Table"],
            question: "Map Catalog: entity model, metadata operations, table/schema lifecycle, and consumers. Use wiki-style sections, inline citations, and an entity relationship diagram.",
        },
        RepoComponent {
            slug: "expression",
            title: "Expression Framework",
            summary: "Expression parsing/evaluation, function registry, attributes, and scoring support.",
            paths: &["aios/expression"],
            symbols: &["AttributeExpression", "FunctionInterface", "SyntaxExpr"],
            question: "Map the expression framework: syntax tree, attribute expression evaluation, function registry, and search/rank integration. Use wiki-style sections and inline citations.",
        },
    ]
}

fn safe_page_segment(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if (ch == '-' || ch == '_' || ch == '.') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "repo".to_string()
    } else {
        trimmed
    }
}

fn title_from_segment(value: &str) -> String {
    value
        .split(['-', '_', '/'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    json_export::export_graph(&graph, repo.join(".gitnova/index.json"))?;
    json_export::export_manifest(&manifest, repo.join(".gitnova/manifest.json"))?;
    if tokio::runtime::Handle::try_current().is_err() {
        let _ = with_store(repo, |store| {
            store.save_graph(&graph)?;
            store.export_json(&graph)?;
            store.save_manifest(&manifest)?;
            Ok(())
        });
    }
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
        assert!(html.contains("<h2>Source Spans</h2>"));
        assert!(html.contains("aios/ha3/search/query_executor/QueryExecutorCreator.cpp:10-42"));
        assert!(html.contains("executor creation entry"));
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
    fn wiki_plan_repo_builds_repo_level_outline_from_source_tree() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("aios/ha3/ha3/search")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/table")).unwrap();
        std::fs::create_dir_all(repo.join("aios/sql/ops")).unwrap();

        let mut repo_state = repo.clone();
        let planned = call_tool(
            "wiki_plan_repo",
            &json!({
                "wiki_root": wiki_root,
                "repo_path": repo,
                "root_id": "havenask",
                "title": "Havenask",
                "max_pages": 3
            }),
            &mut repo_state,
        )
        .unwrap();
        let planned = unwrap_tool_payload(planned);
        assert_eq!(planned["status"], "repo_wiki_planned");
        assert_eq!(planned["root"], "havenask");
        assert_eq!(planned["planner"], "repo_heuristic");
        assert_eq!(planned["pages"], 4);
        assert_eq!(planned["tasks"], 3);

        let outline = call_tool(
            "wiki_read_outline",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki")
            }),
            &mut repo_state,
        )
        .unwrap();
        let outline = unwrap_tool_payload(outline);
        let pages = outline["outline"]["pages"].as_array().unwrap();
        assert!(pages.iter().any(|page| page["id"] == "havenask"));
        assert!(pages.iter().any(|page| page["id"] == "havenask/ha3"));
        assert!(pages
            .iter()
            .any(|page| page["id"] == "havenask/storage/indexlib"));
        assert!(pages.iter().any(|page| page["id"] == "havenask/sql"));

        let html =
            std::fs::read_to_string(temp.path().join(".gitnova/wiki/pages/havenask/index.html"))
                .unwrap();
        assert!(html.contains("HA3 Search Layer"));
        assert!(html.contains("Storage / indexlib"));
        assert!(html.contains("SQL / Iquan"));
    }

    #[test]
    fn wiki_plan_repo_accepts_codex_authored_outline() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut repo_state = repo.clone();

        let planned = call_tool(
            "wiki_plan_repo",
            &json!({
                "wiki_root": wiki_root,
                "repo_path": repo,
                "outline": {
                    "root": "custom",
                    "pages": [{
                        "id": "custom",
                        "title": "Custom Wiki",
                        "kind": "index",
                        "summary": "Codex-authored schema.",
                        "content": "## Start\nCustom reading path.",
                        "deep_tasks": []
                    }]
                }
            }),
            &mut repo_state,
        )
        .unwrap();
        let planned = unwrap_tool_payload(planned);
        assert_eq!(planned["planner"], "codex_outline");
        assert_eq!(planned["pages"], 1);
        let html =
            std::fs::read_to_string(temp.path().join(".gitnova/wiki/pages/custom/index.html"))
                .unwrap();
        assert!(html.contains("Custom Wiki"));
        assert!(html.contains("Custom reading path."));
    }

    #[test]
    fn wiki_run_repo_dry_run_prioritizes_low_completion_pages() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut repo_state = repo.clone();

        call_tool(
            "wiki_apply_outline",
            &json!({
                "wiki_root": wiki_root,
                "outline": {
                    "root": "havenask",
                    "pages": [
                        {
                            "id": "havenask",
                            "title": "Havenask",
                            "kind": "index",
                            "content": "## Reading Path\nStart here.",
                            "deep_tasks": []
                        },
                        {
                            "id": "havenask/ha3",
                            "title": "HA3",
                            "kind": "index",
                            "parent": "havenask",
                            "content": "## Scope\nSkeleton only.",
                            "deep_tasks": [{
                                "id": "map-ha3",
                                "question": "Map HA3.",
                                "expected_outputs": ["summary"]
                            }]
                        },
                        {
                            "id": "havenask/storage/indexlib",
                            "title": "Storage",
                            "kind": "index",
                            "parent": "havenask",
                            "content": "## Overview\nStorage has a substantial cited map. {{source:1}}\n\n## Flow\n\n```mermaid\nflowchart TD\nA[\"Tablet\"] --> B[\"Reader\"]\n```",
                            "deep_tasks": [{
                                "id": "map-storage",
                                "question": "Map storage.",
                                "status": "done",
                                "confidence": 0.8
                            }]
                        }
                    ]
                }
            }),
            &mut repo_state,
        )
        .unwrap();
        call_tool(
            "wiki_append_evidence",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "id": "havenask/storage/indexlib",
                "file": "src/lib.rs",
                "start_line": 1,
                "end_line": 2,
                "note": "storage evidence"
            }),
            &mut repo_state,
        )
        .unwrap();

        let run = call_tool(
            "wiki_run_repo",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "repo_path": repo,
                "root_page_id": "havenask",
                "budget_pages": 1,
                "execute": false
            }),
            &mut repo_state,
        )
        .unwrap();
        let run = unwrap_tool_payload(run);
        assert_eq!(run["status"], "repo_wiki_run_planned");
        assert_eq!(run["execute"], false);
        assert_eq!(run["selected_tasks"].as_array().unwrap().len(), 1);
        assert_eq!(run["selected_tasks"][0]["page_id"], "havenask/ha3");
        assert_eq!(run["selected_tasks"][0]["task_id"], "map-ha3");
        assert!(
            run["quality"][0]["score"].as_u64().unwrap()
                <= run["quality"][1]["score"].as_u64().unwrap()
        );
        assert_eq!(run["expansion"]["tasks_completed"], 0);
        assert_eq!(run["expansion"]["tasks_failed"], 0);
    }

    #[test]
    fn wiki_run_repo_can_retry_failed_tasks_without_replanning() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut repo_state = repo.clone();

        call_tool(
            "wiki_apply_outline",
            &json!({
                "wiki_root": wiki_root,
                "outline": {
                    "root": "havenask",
                    "pages": [{
                        "id": "havenask/ha3",
                        "title": "HA3",
                        "kind": "index",
                        "parent": "havenask",
                        "content": "## Scope\nFailed before.",
                        "deep_tasks": [{
                            "id": "map-ha3",
                            "question": "Map HA3.",
                            "status": "failed",
                            "error": "temporary limit"
                        }]
                    }]
                }
            }),
            &mut repo_state,
        )
        .unwrap();

        let without_retry = call_tool(
            "wiki_run_repo",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "repo_path": repo,
                "root_page_id": "havenask",
                "budget_pages": 1,
                "execute": false
            }),
            &mut repo_state,
        )
        .unwrap();
        let without_retry = unwrap_tool_payload(without_retry);
        assert!(without_retry["selected_tasks"]
            .as_array()
            .unwrap()
            .is_empty());

        let with_retry = call_tool(
            "wiki_run_repo",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "repo_path": repo,
                "root_page_id": "havenask",
                "budget_pages": 1,
                "retry_failed": true,
                "execute": false
            }),
            &mut repo_state,
        )
        .unwrap();
        let with_retry = unwrap_tool_payload(with_retry);
        assert_eq!(with_retry["selected_tasks"].as_array().unwrap().len(), 1);
        assert_eq!(with_retry["selected_tasks"][0]["task_id"], "map-ha3");
    }

    #[test]
    fn wiki_run_repo_plans_deeper_indexlib_pages_after_overview_is_done() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/base")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/framework")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/table")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/table/kv_table")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/table/kkv_table")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/table/normal_table")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/index")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/index/kv")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/index/kkv")).unwrap();
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/file_system")).unwrap();
        let mut repo_state = repo.clone();

        call_tool(
            "wiki_apply_outline",
            &json!({
                "wiki_root": wiki_root,
                "outline": {
                    "root": "havenask",
                    "pages": [{
                        "id": "havenask/storage/indexlib",
                        "title": "Storage / indexlib",
                        "kind": "index",
                        "parent": "havenask",
                        "summary": "Index storage overview.",
                        "content": "## Overview\nDeep indexlib overview. {{source:1}}\n\n## Flow\n\n```mermaid\nflowchart TD\nA[\"Tablet\"] --> B[\"Index\"]\n```",
                        "deep_tasks": [{
                            "id": "map-storage-indexlib",
                            "question": "Map indexlib.",
                            "status": "done",
                            "confidence": 0.86
                        }]
                    }]
                }
            }),
            &mut repo_state,
        )
        .unwrap();
        call_tool(
            "wiki_append_evidence",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "id": "havenask/storage/indexlib",
                "file": "aios/storage/indexlib/table/Tablet.h",
                "start_line": 1,
                "end_line": 2,
                "note": "tablet evidence"
            }),
            &mut repo_state,
        )
        .unwrap();

        let dry_run = call_tool(
            "wiki_run_repo",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "repo_path": repo,
                "root_page_id": "havenask/storage/indexlib",
                "budget_pages": 4,
                "execute": false,
                "plan_deeper": true
            }),
            &mut repo_state,
        )
        .unwrap();
        let dry_run = unwrap_tool_payload(dry_run);
        assert_eq!(dry_run["status"], "repo_wiki_run_planned");
        assert_eq!(dry_run["planned_child_pages"].as_array().unwrap().len(), 6);
        assert!(dry_run["planned_child_pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|page| page["id"] == "havenask/storage/indexlib/tablet"));
        assert!(dry_run["selected_tasks"].as_array().unwrap().is_empty());

        let outline_after_dry_run = call_tool(
            "wiki_read_outline",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki")
            }),
            &mut repo_state,
        )
        .unwrap();
        let outline_after_dry_run = unwrap_tool_payload(outline_after_dry_run);
        assert_eq!(
            outline_after_dry_run["outline"]["pages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let planned = call_tool(
            "wiki_run_repo",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "repo_path": repo,
                "root_page_id": "havenask/storage/indexlib",
                "budget_pages": 2,
                "execute": true,
                "plan_deeper": true,
                "dry_run_children_only": true
            }),
            &mut repo_state,
        )
        .unwrap();
        let planned = unwrap_tool_payload(planned);
        assert_eq!(planned["planned_child_pages"].as_array().unwrap().len(), 6);
        assert_eq!(planned["selected_tasks"].as_array().unwrap().len(), 2);
        assert_eq!(
            planned["selected_tasks"][0]["page_id"],
            "havenask/storage/indexlib/base-structures"
        );

        let outline = call_tool(
            "wiki_read_outline",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki")
            }),
            &mut repo_state,
        )
        .unwrap();
        let outline = unwrap_tool_payload(outline);
        let pages = outline["outline"]["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 7);
        assert!(pages
            .iter()
            .any(|page| page["id"] == "havenask/storage/indexlib/file-system"));
        assert!(pages.iter().any(|page| {
            page["id"] == "havenask/storage/indexlib/kv-kkv"
                && page["deep_tasks"][0]["status"] == "pending"
        }));
    }

    #[test]
    fn wiki_run_repo_assigns_deeper_budget_for_complex_scopes() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("aios/storage/indexlib/file_system")).unwrap();
        let mut repo_state = repo.clone();

        call_tool(
            "wiki_apply_outline",
            &json!({
                "wiki_root": wiki_root,
                "outline": {
                    "root": "havenask",
                    "pages": [{
                        "id": "havenask/storage/indexlib/file-system",
                        "title": "File System",
                        "kind": "index",
                        "parent": "havenask/storage/indexlib",
                        "summary": "Indexlib file-system layer.",
                        "content": "## Scope\nMap filesystem.",
                        "deep_tasks": [{
                            "id": "map-file-system",
                            "question": "Map file system.",
                            "scope_paths": ["aios/storage/indexlib/file_system"]
                        }]
                    }]
                }
            }),
            &mut repo_state,
        )
        .unwrap();

        let run = call_tool(
            "wiki_run_repo",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "repo_path": repo,
                "root_page_id": "havenask/storage/indexlib/file-system",
                "budget_pages": 1,
                "execute": false,
                "plan_deeper": false,
                "max_nodes": 32,
                "max_depth": 2,
                "timeout_secs": 60
            }),
            &mut repo_state,
        )
        .unwrap();
        let run = unwrap_tool_payload(run);
        let budget = &run["selected_tasks"][0]["budget"];
        assert_eq!(budget["reason"], "wide_scope");
        assert_eq!(budget["max_nodes"], 160);
        assert_eq!(budget["max_depth"], 5);
        assert_eq!(budget["timeout_secs"], 600);
    }

    #[test]
    fn wiki_run_repo_plans_file_system_children_after_failed_parent_task() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        for dir in [
            "file",
            "fslib",
            "load_config",
            "package",
            "flush",
            "wal",
            "archive",
        ] {
            std::fs::create_dir_all(repo.join("aios/storage/indexlib/file_system").join(dir))
                .unwrap();
        }
        let mut repo_state = repo.clone();

        call_tool(
            "wiki_apply_outline",
            &json!({
                "wiki_root": wiki_root,
                "outline": {
                    "root": "havenask",
                    "pages": [{
                        "id": "havenask/storage/indexlib/file-system",
                        "title": "File System",
                        "kind": "index",
                        "parent": "havenask/storage/indexlib",
                        "summary": "Indexlib file-system layer.",
                        "content": "## Scope\nParent was too broad.",
                        "deep_tasks": [{
                            "id": "map-file-system",
                            "question": "Map file system.",
                            "scope_paths": ["aios/storage/indexlib/file_system"],
                            "status": "failed",
                            "error": "atlas returned no valid source spans"
                        }]
                    }]
                }
            }),
            &mut repo_state,
        )
        .unwrap();

        let run = call_tool(
            "wiki_run_repo",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "repo_path": repo,
                "root_page_id": "havenask/storage/indexlib/file-system",
                "budget_pages": 3,
                "execute": false,
                "plan_deeper": true
            }),
            &mut repo_state,
        )
        .unwrap();
        let run = unwrap_tool_payload(run);
        let planned = run["planned_child_pages"].as_array().unwrap();
        assert_eq!(planned.len(), 7);
        assert!(planned
            .iter()
            .any(|page| page["id"] == "havenask/storage/indexlib/file-system/load-config"));
        assert!(planned
            .iter()
            .any(|page| page["id"] == "havenask/storage/indexlib/file-system/package"));
    }

    #[test]
    fn wiki_deepen_page_runs_atlas_child_and_merges_result() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("aios/ha3/search/query_executor")).unwrap();
        std::fs::write(
            repo.join("aios/ha3/search/query_executor/QueryExecutorCreator.cpp"),
            (1..=50)
                .map(|line| format!("line {line}\n"))
                .collect::<String>(),
        )
        .unwrap();
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
{"task_id":"trace-query-executor-creator","summary_markdown":"QueryExecutorCreator builds executor families from query semantics.\n\n**Flow**\n\n```mermaid\nflowchart TD\nA-->B\n```","sources":[{"file":"aios/ha3/search/query_executor/QueryExecutorCreator.cpp","start_line":10,"end_line":42,"note":"creator dispatch"}],"diagram":{"format":"svg","content":"<svg viewBox=\"0 0 100 40\"><text x=\"4\" y=\"20\">Creator</text></svg>"},"followups":["Trace bitmap executor index reader dependencies."],"confidence":0.82}
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
        assert_eq!(deepened["source_warnings"], 0);
        assert_eq!(deepened["followups_appended"], 1);

        let html = std::fs::read_to_string(
            temp.path()
                .join(".gitnova/wiki/pages/ha3/search/query-executors.html"),
        )
        .unwrap();
        assert!(html.contains("<h2>Overview</h2>"));
        assert!(html.contains("QueryExecutorCreator builds executor families"));
        assert!(html.contains("<h2>Flow</h2>"));
        assert_eq!(html.matches("<h2>Flow</h2>").count(), 1);
        assert!(html.contains("<svg viewBox"));
        assert!(!html.contains("<strong>Flow</strong>"));
        assert!(!html.contains("A--&gt;B"));
        assert!(!html.contains("Question: Trace QueryExecutorCreator"));
        assert!(!html.contains("Atlas Investigation"));
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
    fn wiki_deepen_page_filters_invalid_source_spans_into_private_warnings() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/lib.rs"), "fn first() {}\nfn second() {}\n").unwrap();
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
{"task_id":"validate-sources","summary_markdown":"Validated source spans should be trustworthy.","sources":[{"file":"src/lib.rs","start_line":1,"end_line":2,"note":"valid span"},{"file":"src/missing.rs","start_line":1,"end_line":2,"note":"missing file"},{"file":"/tmp/outside.rs","start_line":1,"end_line":2,"note":"absolute path"},{"file":"src/lib.rs","start_line":5,"end_line":6,"note":"line out of range"},{"file":"src/lib.rs","start_line":2,"end_line":1,"note":"reversed span"}],"diagram":null,"followups":[],"confidence":0.7}
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
                "content": "Initial skeleton."
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
                "task_id": "validate-sources",
                "question": "Validate source spans.",
                "codex_bin": codex_bin,
                "timeout_secs": 5
            }),
            &mut repo_state,
        )
        .unwrap();
        let deepened = unwrap_tool_payload(deepened);
        assert_eq!(deepened["sources_appended"], 1);
        assert_eq!(deepened["source_warnings"], 4);

        let page = call_tool(
            "wiki_read_page",
            &json!({
                "wiki_root": wiki_root,
                "id": "ha3/search/query-executors",
                "include_private": true
            }),
            &mut repo_state,
        )
        .unwrap();
        let page = unwrap_tool_payload(page);
        assert_eq!(page["page"]["evidence"].as_array().unwrap().len(), 1);
        assert_eq!(page["page"]["evidence"][0]["file"], "src/lib.rs");
        let private_note = page["private_note"].as_str().unwrap();
        assert!(private_note.contains("Atlas Source Warnings"));
        assert!(private_note.contains("src/missing.rs"));
        assert!(private_note.contains("/tmp/outside.rs"));
        assert!(private_note.contains("line out of range"));
        assert!(private_note.contains("start_line is after end_line"));
    }

    #[test]
    fn wiki_expand_tree_fails_task_when_atlas_returns_no_valid_source_spans() {
        let temp = tempfile::TempDir::new().unwrap();
        let wiki_root = temp.path().join(".gitnova/wiki");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/lib.rs"), "fn first() {}\n").unwrap();
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
{"task_id":"bad-sources","summary_markdown":"This should not be trusted.","sources":[{"file":"src/missing.rs","start_line":1,"end_line":2,"note":"missing"}],"diagram":null,"followups":[],"confidence":0.5}
JSON
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
                    "root": "repo",
                    "pages": [{
                        "id": "repo/file-system",
                        "title": "File System",
                        "kind": "article",
                        "content": "## Scope\nFile-system skeleton.",
                        "deep_tasks": [{
                            "id": "bad-sources",
                            "question": "Map file system.",
                            "scope_paths": ["src"],
                            "expected_outputs": ["summary", "sources"]
                        }]
                    }]
                }
            }),
            &mut repo_state,
        )
        .unwrap();

        let expanded = call_tool(
            "wiki_expand_tree",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki"),
                "repo_path": repo,
                "root_page_id": "repo",
                "task_limit": 1,
                "parallelism": 1,
                "codex_bin": codex_bin,
                "timeout_secs": 5
            }),
            &mut repo_state,
        )
        .unwrap();
        let expanded = unwrap_tool_payload(expanded);
        assert_eq!(expanded["tasks_completed"], 0);
        assert_eq!(expanded["tasks_failed"], 1);
        assert!(expanded["results"][0]["error"]
            .as_str()
            .unwrap()
            .contains("no valid source spans"));

        let outline = call_tool(
            "wiki_read_outline",
            &json!({
                "wiki_root": temp.path().join(".gitnova/wiki")
            }),
            &mut repo_state,
        )
        .unwrap();
        let outline = unwrap_tool_payload(outline);
        assert_eq!(
            outline["outline"]["pages"][0]["deep_tasks"][0]["status"],
            "failed"
        );
        let html = std::fs::read_to_string(
            temp.path()
                .join(".gitnova/wiki/pages/repo/file-system.html"),
        )
        .unwrap();
        assert!(!html.contains("This should not be trusted."));
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
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/lib.rs"), "fn first() {}\nfn second() {}\n").unwrap();
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

    #[test]
    fn source_markers_are_remapped_to_page_evidence_indices() {
        let mut source_index_map = HashMap::new();
        source_index_map.insert(1, 4);
        source_index_map.insert(3, 5);

        assert_eq!(
            remap_source_markers(
                "Term path. {{source:1}} Missing path. {{source:2}} Alias. {{src:3}}",
                &source_index_map
            ),
            "Term path. {{source:4}} Missing path.  Alias. {{source:5}}"
        );
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
