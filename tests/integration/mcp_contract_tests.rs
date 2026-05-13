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
            "resources/read",
            json!({"uri":"gitnova://graph/summary"}),
        ),
    ] {
        writeln!(stdin, "{line}").unwrap();
    }

    let mut responses = Vec::new();
    for _ in 0..5 {
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
    assert!(serde_json::to_string(&responses[4])
        .unwrap()
        .contains("\"nodes\""));
}
