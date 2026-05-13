use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
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

#[test]
fn cli_indexes_persists_and_ranks_rust_auth_fixture() {
    let (_temp, repo) = temp_fixture("rust_sample");

    Command::cargo_bin("gitnova")
        .unwrap()
        .args(["index", repo.to_str().unwrap(), "--force"])
        .assert()
        .success();

    assert!(repo.join(".gitnova/gitnova.db").exists());
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
            "local-hash",
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
