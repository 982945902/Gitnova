use assert_cmd::cargo::cargo_bin;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

fn temp_fixture(name: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join(name);
    copy_dir(&fixture(name), &repo);
    (temp, repo)
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

fn module_script_src(html: &str) -> Option<String> {
    let marker = "type=\"module\"";
    let script_index = html.find(marker)?;
    let script_start = html[..script_index].rfind("<script")?;
    let script_end = html[script_index..].find('>')? + script_index;
    let script = &html[script_start..script_end];
    let src_marker = "src=\"";
    let src_start = script.find(src_marker)? + src_marker.len();
    let src_end = script[src_start..].find('"')? + src_start;
    Some(script[src_start..src_end].to_string())
}

#[test]
fn dashboard_serves_summary_api() {
    let (_temp, repo) = temp_fixture("rust_sample");
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
fn dashboard_serves_ranked_graph_context_api() {
    let (_temp, repo) = temp_fixture("ts_sample");
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
    let mut context = None;
    while Instant::now() < deadline {
        if let Some(json) = get_json(
            port,
            "/api/graph-context?query=auth%20session&depth=1&limit=20",
        ) {
            context = Some(json);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    child.kill().ok();
    child.wait().ok();

    let context = context.expect("dashboard /api/graph-context should respond with JSON");
    assert!(context["target"]["qualified_name"]
        .as_str()
        .unwrap()
        .contains("validateSession"));
    assert!(!context["edges"].as_array().unwrap().is_empty());
}

#[test]
fn dashboard_serves_answer_api_with_deterministic_fallback() {
    let (_temp, repo) = temp_fixture("ts_sample");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    let port = free_port();
    let mut command = ProcessCommand::new(cargo_bin("gitnova"));
    command
        .args([
            "dashboard",
            "--repo",
            repo.to_str().unwrap(),
            "--port",
            &port.to_string(),
        ])
        .env_remove("GITNOVA_LLM_API_KEY")
        .env_remove("GITNOVA_LLM_MODEL")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut answer = None;
    while Instant::now() < deadline {
        if let Some(json) = get_json(
            port,
            "/api/answer?query=Where%20is%20auth%20session%20validated%3F",
        ) {
            answer = Some(json);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    child.kill().ok();
    child.wait().ok();

    let answer = answer.expect("dashboard /api/answer should respond with JSON");
    assert_eq!(answer["llm_used"], false);
    assert!(answer["fallback_reason"]
        .as_str()
        .unwrap()
        .contains("GITNOVA"));
    assert!(answer["answer"]
        .as_str()
        .unwrap()
        .contains("validateSession"));
    assert!(answer["evidence"][0]["qualified_name"]
        .as_str()
        .unwrap()
        .contains("validateSession"));
    assert!(answer["context"]["target"]["qualified_name"]
        .as_str()
        .unwrap()
        .contains("validateSession"));
}

#[test]
fn dashboard_serves_graph_visualization_shell() {
    let (_temp, repo) = temp_fixture("rust_sample");
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
    let mut app_js = None;
    while Instant::now() < deadline {
        if let Some(text) = get_text(port, "/") {
            if let Some(src) = module_script_src(&text) {
                app_js = get_text(port, &src);
            }
            html = Some(text);
            if app_js.is_some() {
                break;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    child.kill().ok();
    child.wait().ok();

    let html = html.expect("dashboard index should respond");
    assert!(html.contains("Gitnova Dashboard"));
    assert!(html.contains("id=\"root\""));
    assert!(html.contains("type=\"module\""));
    assert!(html.contains("/assets/"));

    let app_js = app_js.expect("dashboard app js should respond");
    assert!(app_js.contains("Gitnova") || app_js.contains("three"));
}
