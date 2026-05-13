use crate::schema::{RpcRequest, RpcResponse};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub async fn serve_stdio() -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();
    let mut repo_state = std::env::var("GITNOVA_REPO")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(request) => handle_request(request, &mut repo_state).await,
            Err(err) => RpcResponse::error(None, -32700, err.to_string()),
        };
        stdout
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

async fn handle_request(request: RpcRequest, repo_state: &mut PathBuf) -> RpcResponse {
    let id = request.id.clone();
    let result = match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                "resources": {}
            },
            "serverInfo": {
                "name": "gitnova",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "notifications/initialized" => Ok(Value::Null),
        "tools/list" => Ok(crate::tools::list_tools()),
        "tools/call" => {
            let name = request
                .params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let args = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            crate::tools::call_tool(&name, &args, repo_state)
        }
        "resources/list" => Ok(crate::resources::list_resources()),
        "resources/read" => {
            let uri = request
                .params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default();
            crate::resources::read_resource(repo_state, uri)
        }
        other => Ok(json!({"error": format!("unknown method {other}")})),
    };

    match result {
        Ok(value) => RpcResponse::result(id, value),
        Err(err) => RpcResponse::error(id, -32000, err.to_string()),
    }
}
