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
        .unwrap_or_else(|| repo.join(".gitnova/wiki/repo-plan-exp"));
    let root_page_id = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "havenask".to_string());
    let budget_pages = std::env::args()
        .nth(4)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(3);
    let parallelism = std::env::args()
        .nth(5)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2);
    let execute = std::env::args()
        .nth(6)
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let retry_failed = std::env::args()
        .nth(7)
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let dry_run_children_only = std::env::args()
        .nth(8)
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let codex_bin = std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".to_string());

    let mut repo_state = repo.clone();
    let run = call_tool(
        "wiki_run_repo",
        &json!({
            "wiki_root": wiki_root,
            "repo_path": repo,
            "root_page_id": root_page_id,
            "budget_pages": budget_pages,
            "parallelism": parallelism,
            "execute": execute,
            "retry_failed": retry_failed,
            "dry_run_children_only": dry_run_children_only,
            "codex_bin": codex_bin,
            "timeout_secs": 300,
            "max_nodes": 32,
            "max_depth": 2
        }),
        &mut repo_state,
    )?;
    println!("run_repo: {}", payload(run)?);
    Ok(())
}

fn payload(value: Value) -> anyhow::Result<Value> {
    let text = value["content"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("tool returned no text payload"))?;
    Ok(serde_json::from_str(text)?)
}
