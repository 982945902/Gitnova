use anyhow::Result;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use gitnova_core::query;
use gitnova_rank::rank_graph;
use gitnova_storage::GitnovaStore;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

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

pub async fn run_dashboard(repo_root: PathBuf, port: u16) -> Result<()> {
    let state = DashboardState {
        repo_root: Arc::new(repo_root),
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/assets/app.js", get(app_js))
        .route("/assets/styles.css", get(styles_css))
        .route("/api/summary", get(summary))
        .route("/api/nodes", get(nodes))
        .route("/api/edges", get(edges))
        .route("/api/rank", get(rank))
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

async fn index() -> impl IntoResponse {
    Html(include_str!("../assets/index.html"))
}

async fn app_js() -> impl IntoResponse {
    (
        [("content-type", "application/javascript; charset=utf-8")],
        include_str!("../assets/app.js"),
    )
}

async fn styles_css() -> impl IntoResponse {
    (
        [("content-type", "text/css; charset=utf-8")],
        include_str!("../assets/styles.css"),
    )
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
    match GitnovaStore::open(state.repo_root.as_ref()).and_then(|store| store.load_graph()) {
        Ok(graph) => f(graph),
        Err(err) => json!({ "error": err.to_string() }),
    }
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
