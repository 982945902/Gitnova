use anyhow::Result;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use gitnova_core::query;
use gitnova_enrich::llm;
use gitnova_rank::rank_graph;
use gitnova_storage::SurrealStore;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

const DIST_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/dist");

#[derive(Clone)]
struct DashboardState {
    repo_root: Arc<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct RankParams {
    query: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ContextParams {
    query: Option<String>,
    selector: Option<String>,
    node_id: Option<String>,
    symbol: Option<String>,
    depth: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AnswerParams {
    query: Option<String>,
    depth: Option<usize>,
    limit: Option<usize>,
}

pub async fn run_dashboard(repo_root: PathBuf, port: u16) -> Result<()> {
    let state = DashboardState {
        repo_root: Arc::new(repo_root),
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/assets/{*path}", get(asset))
        .route("/api/summary", get(summary))
        .route("/api/nodes", get(nodes))
        .route("/api/edges", get(edges))
        .route("/api/rank", get(rank))
        .route("/api/answer", get(answer))
        .route("/api/graph-context", get(graph_context))
        .route("/api/explain", get(explain))
        .route("/api/impact", get(impact))
        .route("/api/hubs", get(hubs))
        .route("/api/churn", get(churn))
        .with_state(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Response {
    if let Some(html) = read_dist_file("index.html") {
        return Html(html).into_response();
    }
    Html(include_str!("../assets/index.html").to_string()).into_response()
}

async fn asset(AxumPath(path): AxumPath<String>) -> Response {
    if let Some(bytes) = read_dist_bytes(&format!("assets/{path}")) {
        return with_content_type(bytes, content_type_for(&path));
    }
    match path.as_str() {
        "app.js" => (
            [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
            include_str!("../assets/app.js").as_bytes().to_vec(),
        )
            .into_response(),
        "styles.css" => (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            include_str!("../assets/styles.css").as_bytes().to_vec(),
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn read_dist_file(path: &str) -> Option<String> {
    fs::read_to_string(format!("{DIST_DIR}/{path}")).ok()
}

fn read_dist_bytes(path: &str) -> Option<Vec<u8>> {
    fs::read(format!("{DIST_DIR}/{path}")).ok()
}

fn with_content_type(bytes: Vec<u8>, content_type: &'static str) -> Response {
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type),
    );
    response
}

fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

async fn summary(State(state): State<DashboardState>) -> Json<Value> {
    Json(load_graph_value(&state, |graph| {
        json!(query::summarize(&graph))
    }))
}

async fn nodes(State(state): State<DashboardState>) -> Json<Value> {
    Json(load_graph_value(&state, |graph| json!(graph.nodes)))
}

async fn edges(State(state): State<DashboardState>) -> Json<Value> {
    Json(load_graph_value(&state, |graph| json!(graph.edges)))
}

async fn rank(
    State(state): State<DashboardState>,
    Query(params): Query<RankParams>,
) -> Json<Value> {
    Json(load_graph_value(&state, |graph| {
        let query = params.query.unwrap_or_else(|| "architecture".into());
        json!(rank_graph(&graph, &query, params.limit.unwrap_or(10)))
    }))
}

async fn answer(
    State(state): State<DashboardState>,
    Query(params): Query<AnswerParams>,
) -> Json<Value> {
    Json(load_graph_value(&state, |graph| {
        let query = params
            .query
            .unwrap_or_else(|| "Where is auth session validated?".into());
        json!(llm::answer_with_context(
            &graph,
            &query,
            params.depth.unwrap_or(1),
            params.limit.unwrap_or(40)
        ))
    }))
}

async fn graph_context(
    State(state): State<DashboardState>,
    Query(params): Query<ContextParams>,
) -> Json<Value> {
    Json(load_graph_value(&state, |graph| {
        let selector = selector_from_params(&params, &graph);
        json!(query::graph_context(
            &graph,
            &selector,
            params.depth.unwrap_or(1),
            params.limit.unwrap_or(40)
        ))
    }))
}

async fn explain(
    State(state): State<DashboardState>,
    Query(params): Query<ContextParams>,
) -> Json<Value> {
    Json(load_graph_value(&state, |graph| {
        let selector = selector_from_params(&params, &graph);
        json!(query::explain_node(&graph, &selector))
    }))
}

async fn impact(
    State(state): State<DashboardState>,
    Query(params): Query<ContextParams>,
) -> Json<Value> {
    Json(load_graph_value(&state, |graph| {
        let selector = selector_from_params(&params, &graph);
        json!(query::impact_analysis(
            &graph,
            &selector,
            params.limit.unwrap_or(20)
        ))
    }))
}

async fn hubs(State(state): State<DashboardState>) -> Json<Value> {
    Json(load_graph_value(&state, |graph| {
        json!(query::summarize(&graph).hubs)
    }))
}

async fn churn(State(state): State<DashboardState>) -> Json<Value> {
    Json(load_graph_value(&state, |graph| {
        json!(query::summarize(&graph).recent_churn)
    }))
}

fn load_graph_value<F>(state: &DashboardState, f: F) -> Value
where
    F: FnOnce(gitnova_core::CodeGraph) -> Value,
{
    match load_dashboard_graph(state) {
        Ok(graph) => f(graph),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

fn load_dashboard_graph(state: &DashboardState) -> Result<gitnova_core::CodeGraph> {
    SurrealStore::open(state.repo_root.as_ref())
        .and_then(|store| store.load_graph())
        .or_else(|_| {
            gitnova_storage::json_export::import_graph(
                &state.repo_root.join(".gitnova/index.json"),
            )
        })
}

fn selector_from_params(params: &ContextParams, graph: &gitnova_core::CodeGraph) -> String {
    for value in [&params.node_id, &params.selector, &params.symbol] {
        if let Some(value) = value.as_deref() {
            if !value.trim().is_empty() {
                return value.to_string();
            }
        }
    }
    if let Some(query_text) = params.query.as_deref() {
        if let Some(result) = rank_graph(graph, query_text, 1).results.into_iter().next() {
            return result.node.id;
        }
        return query_text.to_string();
    }
    String::new()
}
