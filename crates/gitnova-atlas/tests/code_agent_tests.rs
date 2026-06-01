use gitnova_atlas::{
    investigate_many, CodexCliAgent, InvestigationBudget, InvestigationScope, InvestigationTask,
};
use std::fs;

#[test]
fn codex_cli_agent_runs_child_codex_with_read_only_guardrails() {
    let temp = tempfile::TempDir::new().unwrap();
    let codex_bin = temp.path().join("fake-codex");
    let capture_dir = temp.path().join("capture");
    fs::create_dir_all(&capture_dir).unwrap();
    fs::write(
        &codex_bin,
        format!(
            r#"#!/bin/sh
set -eu
out=""
prev=""
for arg in "$@"; do
  printf '%s\n' "$arg" >> "{capture}/args.txt"
  if [ "$prev" = "-o" ] || [ "$prev" = "--output-last-message" ]; then
    out="$arg"
  fi
  prev="$arg"
done
cat > "{capture}/stdin.txt"
env | grep '^GITNOVA_' > "{capture}/env.txt"
cat > "$out" <<'JSON'
{{"task_id":"trace","summary_markdown":"Found the executor creator path.","sources":[{{"file":"aios/ha3/search/QueryExecutorCreator.cpp","start_line":10,"end_line":42,"note":"creator entry"}}],"diagram":{{"format":"mermaid","content":"flowchart TD\\nA-->B"}},"followups":["Trace each executor family."],"confidence":0.81}}
JSON
"#,
            capture = capture_dir.display()
        ),
    )
    .unwrap();
    make_executable(&codex_bin);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let codex_home = temp.path().join("codex-home");
    let task = InvestigationTask {
        task_id: "trace".to_string(),
        question: "Trace QueryExecutorCreator into each executor family.".to_string(),
        repo_path: repo.clone(),
        scope: InvestigationScope {
            paths: vec!["aios/ha3/search".to_string()],
            symbols: vec!["QueryExecutorCreator".to_string()],
        },
        expected_outputs: vec![
            "summary".to_string(),
            "sources".to_string(),
            "diagram".to_string(),
        ],
        budget: InvestigationBudget {
            timeout_secs: 5,
            ..InvestigationBudget::default()
        },
    };

    let result = CodexCliAgent::new(&codex_bin)
        .with_model("gpt-5-codex")
        .with_codex_home(&codex_home)
        .investigate(&task)
        .unwrap();

    assert_eq!(result.task_id, "trace");
    assert_eq!(result.sources.len(), 1);
    assert_eq!(result.diagram.unwrap().format, "mermaid");

    let args = fs::read_to_string(capture_dir.join("args.txt")).unwrap();
    assert!(args.contains("exec\n"));
    assert!(args.contains("--ignore-user-config\n"));
    assert!(args.contains("--ephemeral\n"));
    assert!(args.contains("-s\n"));
    assert!(args.contains("read-only\n"));
    assert!(args.contains("--output-schema\n"));
    assert!(args.contains("-o\n"));
    assert!(args.contains("-m\n"));
    assert!(args.contains("gpt-5-codex\n"));
    assert!(args.contains("-C\n"));
    assert!(args.contains(&format!("{}\n", repo.display())));

    let stdin = fs::read_to_string(capture_dir.join("stdin.txt")).unwrap();
    assert!(stdin.contains("task_id: trace"));
    assert!(stdin.contains("Trace QueryExecutorCreator"));
    assert!(stdin.contains("Return only JSON"));
    assert!(stdin.contains("Do not edit files"));
    assert!(stdin.contains("Do not call Gitnova MCP tools"));

    let env = fs::read_to_string(capture_dir.join("env.txt")).unwrap();
    assert!(env.contains("GITNOVA_ATLAS_CHILD=1"));
    assert!(env.contains("GITNOVA_DISABLE_MCP_RECURSION=1"));
}

#[test]
fn investigate_many_runs_tasks_in_parallel_and_preserves_order() {
    let temp = tempfile::TempDir::new().unwrap();
    let codex_bin = temp.path().join("fake-codex");
    let started_dir = temp.path().join("started");
    fs::create_dir_all(&started_dir).unwrap();
    fs::write(
        &codex_bin,
        format!(
            r#"#!/bin/sh
set -eu
out=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "-o" ] || [ "$prev" = "--output-last-message" ]; then
    out="$arg"
  fi
  prev="$arg"
done
stdin="$(cat)"
task_id="$(printf '%s\n' "$stdin" | awk -F': ' '/^task_id:/{{print $2; exit}}')"
touch "{started}/$task_id"
i=0
count=0
while [ "$i" -lt 50 ]; do
  count="$(find "{started}" -type f | wc -l | tr -d ' ')"
  if [ "$count" -ge 3 ]; then
    break
  fi
  i=$((i + 1))
  sleep 0.05
done
if [ "$count" -lt 3 ]; then
  echo "parallel barrier failed" >&2
  exit 7
fi
printf '{{"task_id":"%s","summary_markdown":"summary for %s","sources":[],"diagram":null,"followups":[],"confidence":0.5}}\n' "$task_id" "$task_id" > "$out"
"#,
            started = started_dir.display()
        ),
    )
    .unwrap();
    make_executable(&codex_bin);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let tasks = ["a", "b", "c"]
        .into_iter()
        .map(|id| InvestigationTask {
            task_id: id.to_string(),
            question: format!("Investigate task {id}."),
            repo_path: repo.clone(),
            scope: InvestigationScope::default(),
            expected_outputs: vec!["summary".to_string()],
            budget: InvestigationBudget {
                timeout_secs: 5,
                ..InvestigationBudget::default()
            },
        })
        .collect::<Vec<_>>();

    let results = investigate_many(CodexCliAgent::new(&codex_bin), tasks, 3).unwrap();

    assert_eq!(
        results
            .iter()
            .map(|result| result.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
}

#[test]
fn codex_cli_agent_reports_child_failures() {
    let temp = tempfile::TempDir::new().unwrap();
    let codex_bin = temp.path().join("fake-codex");
    fs::write(
        &codex_bin,
        r#"#!/bin/sh
echo "boom" >&2
exit 2
"#,
    )
    .unwrap();
    make_executable(&codex_bin);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let task = InvestigationTask {
        task_id: "bad".to_string(),
        question: "Investigate failure.".to_string(),
        repo_path: repo,
        scope: InvestigationScope::default(),
        expected_outputs: vec!["summary".to_string()],
        budget: InvestigationBudget::default(),
    };

    let error = CodexCliAgent::new(&codex_bin)
        .investigate(&task)
        .unwrap_err()
        .to_string();
    assert!(error.contains("codex exec failed"));
    assert!(error.contains("boom"));
}

#[test]
fn codex_cli_agent_times_out_child_processes() {
    let temp = tempfile::TempDir::new().unwrap();
    let codex_bin = temp.path().join("fake-codex");
    fs::write(
        &codex_bin,
        r#"#!/bin/sh
sleep 5
"#,
    )
    .unwrap();
    make_executable(&codex_bin);

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let task = InvestigationTask {
        task_id: "slow".to_string(),
        question: "Investigate timeout.".to_string(),
        repo_path: repo,
        scope: InvestigationScope::default(),
        expected_outputs: vec!["summary".to_string()],
        budget: InvestigationBudget {
            timeout_secs: 1,
            ..InvestigationBudget::default()
        },
    };

    let error = CodexCliAgent::new(&codex_bin)
        .investigate(&task)
        .unwrap_err()
        .to_string();
    assert!(error.contains("timed out"));
}

#[test]
fn codex_cli_agent_truncates_preloaded_source_on_utf8_boundaries() {
    let temp = tempfile::TempDir::new().unwrap();
    let codex_bin = temp.path().join("fake-codex");
    let capture_dir = temp.path().join("capture");
    fs::create_dir_all(&capture_dir).unwrap();
    fs::write(
        &codex_bin,
        format!(
            r#"#!/bin/sh
set -eu
out=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "-o" ] || [ "$prev" = "--output-last-message" ]; then
    out="$arg"
  fi
  prev="$arg"
done
cat > "{capture}/stdin.txt"
cat > "$out" <<'JSON'
{{"task_id":"utf8","summary_markdown":"ok","sources":[],"diagram":null,"followups":[],"confidence":0.5}}
JSON
"#,
            capture = capture_dir.display()
        ),
    )
    .unwrap();
    make_executable(&codex_bin);

    let repo = temp.path().join("repo");
    fs::create_dir_all(repo.join("aios/ha3/search")).unwrap();
    fs::write(
        repo.join("aios/ha3/search/Unicode.cpp"),
        format!("{}你", "a".repeat(47_999)),
    )
    .unwrap();
    let task = InvestigationTask {
        task_id: "utf8".to_string(),
        question: "Read a large file with unicode near the truncation boundary.".to_string(),
        repo_path: repo,
        scope: InvestigationScope {
            paths: vec!["aios/ha3/search/Unicode.cpp".to_string()],
            symbols: vec![],
        },
        expected_outputs: vec!["summary".to_string()],
        budget: InvestigationBudget {
            timeout_secs: 5,
            ..InvestigationBudget::default()
        },
    };

    CodexCliAgent::new(&codex_bin).investigate(&task).unwrap();

    let stdin = fs::read_to_string(capture_dir.join("stdin.txt")).unwrap();
    assert!(stdin.contains("--- truncated ---"));
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &std::path::Path) {}
