use anyhow::Result;
use gitnova_core::query;
use serde_json::{json, Value};
use std::path::Path;

pub fn list_resources() -> Value {
    json!({
        "resources": [
            resource("gitnova://graph/summary", "Graph summary"),
            resource("gitnova://graph/nodes", "Graph nodes"),
            resource("gitnova://graph/edges", "Graph edges"),
            resource("gitnova://graph/hubs", "High-degree graph hubs"),
            resource("gitnova://graph/recent-churn", "Recently changed nodes"),
            resource("gitnova://graph/dashboard-url", "Default local dashboard URL")
        ]
    })
}

fn resource(uri: &str, name: &str) -> Value {
    json!({
        "uri": uri,
        "name": name,
        "mimeType": "application/json"
    })
}

pub fn read_resource(repo: impl AsRef<Path>, uri: &str) -> Result<Value> {
    let repo = repo.as_ref();
    let graph = crate::tools::with_store(repo, |store| store.load_graph())?;
    let value = match uri {
        "gitnova://graph/summary" => json!(query::summarize(&graph)),
        "gitnova://graph/nodes" => json!(graph.nodes),
        "gitnova://graph/edges" => json!(graph.edges),
        "gitnova://graph/hubs" => json!(query::summarize(&graph).hubs),
        "gitnova://graph/recent-churn" => json!(query::summarize(&graph).recent_churn),
        "gitnova://graph/dashboard-url" => json!({"url": "http://127.0.0.1:4567"}),
        _ => json!({"error": "unknown resource"}),
    };
    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": serde_json::to_string(&value)?
        }],
        "structuredContent": value
    }))
}
