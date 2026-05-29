use super::helpers::*;
use assert_cmd::Command;
use serde_json::Value;
use std::fs;

#[test]
fn cli_indexes_persists_and_ranks_rust_auth_fixture() {
    let (_temp, repo) = temp_fixture("rust_sample");

    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    assert!(repo.join(".gitnova/surrealdb").is_dir());
    assert!(repo.join(".gitnova/index.json").exists());

    let stats = Command::cargo_bin("gitnova")
        .unwrap()
        .args(["stats", "--repo", repo.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stats: Value = serde_json::from_slice(&stats).unwrap();
    assert!(stats["nodes"].as_u64().unwrap() >= 8);
    assert!(stats["edges"].as_u64().unwrap() >= 8);

    let ranked = Command::cargo_bin("gitnova")
        .unwrap()
        .args([
            "rank-context",
            "change auth validation",
            "--repo",
            repo.to_str().unwrap(),
            "--limit",
            "5",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let ranked: Value = serde_json::from_slice(&ranked).unwrap();
    let first = &ranked["results"][0]["node"];
    assert!(
        first["qualified_name"]
            .as_str()
            .unwrap()
            .contains("AuthService")
            || first["name"].as_str().unwrap().contains("validate")
    );
}

#[test]
fn salience_downranks_generic_utility_unless_query_targets_it() {
    let (_temp, repo) = temp_fixture("ts_sample");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    let auth = Command::cargo_bin("gitnova")
        .unwrap()
        .args([
            "rank-context",
            "change auth session validation",
            "--repo",
            repo.to_str().unwrap(),
            "--limit",
            "3",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let auth: Value = serde_json::from_slice(&auth).unwrap();
    let auth_names = serde_json::to_string(&auth["results"]).unwrap();
    assert!(auth_names.contains("validateSession"));
    assert!(!auth["results"][0]["node"]["qualified_name"]
        .as_str()
        .unwrap()
        .contains("formatDate"));

    let util = Command::cargo_bin("gitnova")
        .unwrap()
        .args([
            "rank-context",
            "formatDate utility",
            "--repo",
            repo.to_str().unwrap(),
            "--limit",
            "3",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let util: Value = serde_json::from_slice(&util).unwrap();
    assert!(util["results"][0]["node"]["qualified_name"]
        .as_str()
        .unwrap()
        .contains("formatDate"));
}

#[test]
fn graph_context_returns_web_ready_node_neighborhood() {
    let (_temp, repo) = temp_fixture("ts_sample");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    let output = Command::cargo_bin("gitnova")
        .unwrap()
        .args([
            "graph-context",
            "validateSession",
            "--repo",
            repo.to_str().unwrap(),
            "--depth",
            "1",
            "--limit",
            "20",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let context: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(context["schema_version"].as_u64().unwrap(), 1);
    assert!(context["target"]["qualified_name"]
        .as_str()
        .unwrap()
        .contains("validateSession"));
    assert!(context["summary"]
        .as_str()
        .unwrap()
        .contains("graph relationships"));
    assert!(context["nodes"].as_array().unwrap().len() >= 3);
    assert!(!context["edges"].as_array().unwrap().is_empty());

    let impact = Command::cargo_bin("gitnova")
        .unwrap()
        .args([
            "impact-analysis",
            "formatDate",
            "--repo",
            repo.to_str().unwrap(),
            "--limit",
            "10",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let impact: Value = serde_json::from_slice(&impact).unwrap();
    assert_eq!(
        impact["symbol"]["qualified_name"].as_str().unwrap(),
        "src/utils.ts::formatDate"
    );
}

#[test]
fn cli_exposes_core_workflows_and_embeddings() {
    let (_temp, repo) = temp_fixture("ts_sample");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    for args in [
        vec![
            "explain-symbol",
            "formatDate",
            "--repo",
            repo.to_str().unwrap(),
        ],
        vec![
            "graph-context",
            "formatDate",
            "--repo",
            repo.to_str().unwrap(),
            "--depth",
            "1",
        ],
        vec![
            "impact-analysis",
            "formatDate",
            "--repo",
            repo.to_str().unwrap(),
            "--limit",
            "10",
        ],
        vec![
            "architecture-map",
            "--repo",
            repo.to_str().unwrap(),
            "--focus",
            "auth",
        ],
        vec!["diff-context", "--repo", repo.to_str().unwrap()],
        vec![
            "embeddings",
            "build",
            "--repo",
            repo.to_str().unwrap(),
            "--provider",
            "local-semantic",
        ],
    ] {
        Command::cargo_bin("gitnova")
            .unwrap()
            .args(args)
            .assert()
            .success();
    }
}

#[test]
fn atlas_generates_single_file_html_report() {
    let (temp, repo) = temp_fixture("ts_sample");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    let html_path = temp.path().join("atlas.html");
    let json_path = temp.path().join("atlas.json");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args([
            "atlas",
            "--repo",
            repo.to_str().unwrap(),
            "--entry",
            "auth",
            "--output",
            html_path.to_str().unwrap(),
            "--emit-json",
            json_path.to_str().unwrap(),
            "--max-flows",
            "3",
            "--max-depth",
            "3",
        ])
        .assert()
        .success();

    let html = fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains("atlas-data"));
    assert!(html.contains("Execution Flows"));

    let json = fs::read_to_string(&json_path).unwrap();
    let report: Value = serde_json::from_str(&json).unwrap();
    assert!(report["flows"].as_array().unwrap().len() <= 3);
    let rendered = serde_json::to_string(&report["flows"]).unwrap();
    assert!(
        rendered.contains("auth") || rendered.contains("validate") || rendered.contains("session"),
        "atlas should preserve the requested auth-oriented flow, got {rendered}"
    );
}

#[test]
fn incremental_update_reports_skips_changes_and_deletes() {
    let (_temp, repo) = temp_fixture("rust_sample");
    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    let unchanged = Command::cargo_bin("gitnova")
        .unwrap()
        .args(["update", "--repo", repo.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unchanged: Value = serde_json::from_slice(&unchanged).unwrap();
    assert!(unchanged["skipped"].as_u64().unwrap() >= 4);
    assert_eq!(unchanged["changed"].as_u64().unwrap(), 0);

    fs::write(
        repo.join("src/extra.rs"),
        "pub fn new_domain_entrypoint() -> bool { true }\n",
    )
    .unwrap();
    let changed = Command::cargo_bin("gitnova")
        .unwrap()
        .args(["update", "--repo", repo.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let changed: Value = serde_json::from_slice(&changed).unwrap();
    assert!(changed["changed"].as_u64().unwrap() >= 1);

    fs::remove_file(repo.join("src/extra.rs")).unwrap();
    let deleted = Command::cargo_bin("gitnova")
        .unwrap()
        .args(["update", "--repo", repo.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let deleted: Value = serde_json::from_slice(&deleted).unwrap();
    assert!(deleted["deleted"].as_u64().unwrap() >= 1);
}

#[test]
fn cpp_fixture_indexes_and_ranks_with_utility_downranking() {
    let (_temp, repo) = temp_fixture("cpp_sample");

    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    let stats = Command::cargo_bin("gitnova")
        .unwrap()
        .args(["stats", "--repo", repo.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stats: Value = serde_json::from_slice(&stats).unwrap();
    assert_eq!(stats["files"].as_u64().unwrap(), 4);
    assert!(stats["languages"]["cpp"].as_u64().unwrap() >= 3);

    // Auth query should rank AuthService above utils
    let auth = Command::cargo_bin("gitnova")
        .unwrap()
        .args([
            "rank-context",
            "change auth session validation",
            "--repo",
            repo.to_str().unwrap(),
            "--limit",
            "3",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let auth: Value = serde_json::from_slice(&auth).unwrap();
    let auth_names = serde_json::to_string(&auth["results"]).unwrap();
    assert!(auth_names.contains("AuthService") || auth_names.contains("validateSession"));
    // formatDate (utility) should not be top result for auth query
    assert!(!auth["results"][0]["node"]["qualified_name"]
        .as_str()
        .unwrap()
        .contains("formatDate"));

    // Utility query should promote formatDate
    let util = Command::cargo_bin("gitnova")
        .unwrap()
        .args([
            "rank-context",
            "formatDate utility",
            "--repo",
            repo.to_str().unwrap(),
            "--limit",
            "3",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let util: Value = serde_json::from_slice(&util).unwrap();
    assert!(util["results"][0]["node"]["qualified_name"]
        .as_str()
        .unwrap()
        .contains("formatDate"));
}
