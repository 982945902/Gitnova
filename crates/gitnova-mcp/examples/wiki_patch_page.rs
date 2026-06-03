use std::io::Read;
use std::path::PathBuf;

use gitnova_mcp::tools::call_tool;
use serde_json::{json, Value};

fn main() -> anyhow::Result<()> {
    let repo = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let wiki_root = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("usage: wiki_patch_page <repo> <wiki_root> <page_id>"))?;
    let page_id = std::env::args()
        .nth(3)
        .ok_or_else(|| anyhow::anyhow!("usage: wiki_patch_page <repo> <wiki_root> <page_id>"))?;
    let mut content = String::new();
    std::io::stdin().read_to_string(&mut content)?;

    let mut repo_state = repo.clone();
    let patched = call_tool(
        "wiki_patch_page",
        &json!({
            "wiki_root": wiki_root,
            "id": page_id,
            "content_format": "markdown",
            "mode": "replace",
            "content": content
        }),
        &mut repo_state,
    )?;
    println!("patch_page: {}", payload(patched)?);
    Ok(())
}

fn payload(value: Value) -> anyhow::Result<Value> {
    let text = value["content"][0]["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("tool returned no text payload"))?;
    Ok(serde_json::from_str(text)?)
}
