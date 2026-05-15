use assert_cmd::cargo::cargo_bin;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn rpc(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string()
}

#[test]
fn mcp_stdio_lists_tools_resources_and_calls_index() {
    let mut child = Command::new(cargo_bin("gitnova"))
        .arg("serve")
        .env("GITNOVA_REPO", fixture("rust_sample"))
        .env("GITNOVA_USE_LEGACY_STDIO", "1")
        .env_remove("GITNOVA_LLM_API_KEY")
        .env_remove("GITNOVA_LLM_MODEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    for line in [
        rpc(1, "initialize", json!({})),
        rpc(2, "tools/list", json!({})),
        rpc(3, "resources/list", json!({})),
        rpc(
            4,
            "tools/call",
            json!({
                "name":"index_project",
                "arguments":{"path": fixture("rust_sample"), "force": true}
            }),
        ),
        rpc(
            5,
            "tools/call",
            json!({
                "name":"graph_context",
                "arguments":{"query":"auth validation", "depth": 1, "limit": 20}
            }),
        ),
        rpc(
            6,
            "tools/call",
            json!({
                "name":"answer_with_context",
                "arguments":{"query":"Where is auth validation handled?", "depth": 1, "limit": 20}
            }),
        ),
        rpc(
            7,
            "resources/read",
            json!({"uri":"gitnova://graph/summary"}),
        ),
        rpc(
            8,
            "tools/call",
            json!({
                "name":"watch_project",
                "arguments":{"path": fixture("rust_sample")}
            }),
        ),
    ] {
        writeln!(stdin, "{line}").unwrap();
    }

    let mut responses = Vec::new();
    for _ in 0..8 {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        responses.push(serde_json::from_str::<Value>(&line).unwrap());
    }

    child.kill().ok();
    child.wait().ok();

    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "gitnova");
    assert!(serde_json::to_string(&responses[1])
        .unwrap()
        .contains("rank_context"));
    assert!(serde_json::to_string(&responses[2])
        .unwrap()
        .contains("gitnova://graph/nodes"));
    assert!(serde_json::to_string(&responses[3])
        .unwrap()
        .contains("indexed"));
    assert!(
        serde_json::to_string(&responses[4])
            .unwrap()
            .contains("graph_context")
            || serde_json::to_string(&responses[4])
                .unwrap()
                .contains("target")
    );
    assert!(serde_json::to_string(&responses[5])
        .unwrap()
        .contains("evidence"));
    assert!(serde_json::to_string(&responses[5])
        .unwrap()
        .contains("llm_used"));
    assert!(serde_json::to_string(&responses[6])
        .unwrap()
        .contains("\"nodes\""));
    assert!(serde_json::to_string(&responses[7])
        .unwrap()
        .contains("started"));
}

#[test]
fn mcp_default_stdio_uses_rmcp_server() {
    let mut child = Command::new(cargo_bin("gitnova"))
        .arg("serve")
        .env("GITNOVA_REPO", fixture("rust_sample"))
        .env_remove("GITNOVA_LLM_API_KEY")
        .env_remove("GITNOVA_LLM_MODEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    writeln!(
        stdin,
        "{}",
        rpc(
            1,
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "gitnova-test", "version": "0.0.0"}
            }),
        )
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let init: Value = serde_json::from_str(&line).unwrap();

    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}})
    )
    .unwrap();
    writeln!(stdin, "{}", rpc(2, "tools/list", json!({}))).unwrap();
    stdin.flush().unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let tools: Value = serde_json::from_str(&line).unwrap();

    child.kill().ok();
    child.wait().ok();

    assert_eq!(init["result"]["serverInfo"]["name"], "gitnova-mcp");
    assert!(serde_json::to_string(&tools)
        .unwrap()
        .contains("index_project"));
    assert!(serde_json::to_string(&tools)
        .unwrap()
        .contains("graph_context"));
    assert!(serde_json::to_string(&tools)
        .unwrap()
        .contains("answer_with_context"));
}
