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
    let max_pages = std::env::args()
        .nth(3)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(8);

    if wiki_root.exists() {
        std::fs::remove_dir_all(&wiki_root)?;
    }

    let mut repo_state = repo.clone();
    let planned = call_tool(
        "wiki_plan_repo",
        &json!({
            "wiki_root": wiki_root,
            "repo_path": repo,
            "root_id": "havenask",
            "title": "Havenask",
            "summary": "Distributed C++ search engine: query, indexing, SQL, serving, build, scheduling, and supporting runtime subsystems.",
            "max_pages": max_pages
        }),
        &mut repo_state,
    )?;
    let planned = payload(planned)?;
    println!("plan_repo: {}", planned);
    println!(
        "index: {}",
        planned["wiki_root"]
            .as_str()
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string()
            + "/pages/havenask/index.html"
    );
    Ok(())
}

fn payload(value: Value) -> anyhow::Result<Value> {
    let text = value["content"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("tool returned no text payload"))?;
    Ok(serde_json::from_str(text)?)
}
