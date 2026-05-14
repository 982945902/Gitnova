use assert_cmd::cargo::cargo_bin;
use assert_cmd::Command;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn get_json(port: u16, path: &str) -> Option<Value> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let body = response.split("\r\n\r\n").nth(1)?;
    serde_json::from_str(body).ok()
}

fn get_text(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    response.split("\r\n\r\n").nth(1).map(str::to_string)
}

#[test]
fn dashboard_serves_summary_api() {
    let repo = fixture("rust_sample");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    let port = free_port();
    let mut child = ProcessCommand::new(cargo_bin("gitnova"))
        .args([
            "dashboard",
            "--repo",
            repo.to_str().unwrap(),
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut summary = None;
    while Instant::now() < deadline {
        if let Some(json) = get_json(port, "/api/summary") {
            summary = Some(json);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    child.kill().ok();
    child.wait().ok();

    let summary = summary.expect("dashboard /api/summary should respond with JSON");
    assert!(summary["nodes"].as_u64().unwrap() >= 8);
}

#[test]
fn dashboard_serves_graph_visualization_shell() {
    let repo = fixture("rust_sample");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    let port = free_port();
    let mut child = ProcessCommand::new(cargo_bin("gitnova"))
        .args([
            "dashboard",
            "--repo",
            repo.to_str().unwrap(),
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut html = None;
    while Instant::now() < deadline {
        if let Some(text) = get_text(port, "/") {
            html = Some(text);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    child.kill().ok();
    child.wait().ok();

    let html = html.expect("dashboard index should respond");
    assert!(html.contains("graph-canvas"));
    assert!(html.contains("Graph"));
    assert!(html.contains("graph-filter"));
    assert!(html.contains("graph-kind"));
    assert!(html.contains("graph-zoom-in"));
}
