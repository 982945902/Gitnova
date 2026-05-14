use gitnova_core::CodeGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffContext {
    pub base: String,
    pub changed_paths: Vec<String>,
    pub files: Vec<DiffFile>,
    pub ranked: crate::score::RankResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub added_lines: Vec<usize>,
    pub deleted_lines: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: DiffStatus,
    pub added_lines: Vec<usize>,
    pub deleted_lines: Vec<usize>,
    pub hunks: Vec<DiffHunk>,
}

pub fn changed_paths(repo: impl AsRef<Path>, base: &str) -> Vec<String> {
    diff_files(repo, base)
        .into_iter()
        .map(|file| file.path)
        .collect()
}

pub fn diff_files(repo: impl AsRef<Path>, base: &str) -> Vec<DiffFile> {
    let repo = repo.as_ref();
    let mut files = parse_name_status(repo, base);
    let hunks = parse_diff_hunks(repo, base);
    for file in &mut files {
        if let Some(parsed_hunks) = hunks.get(&file.path) {
            file.hunks = parsed_hunks.clone();
            file.added_lines = sorted_unique(
                file.hunks
                    .iter()
                    .flat_map(|hunk| hunk.added_lines.iter().copied()),
            );
            file.deleted_lines = sorted_unique(
                file.hunks
                    .iter()
                    .flat_map(|hunk| hunk.deleted_lines.iter().copied()),
            );
        }
    }
    files
}

pub fn diff_context(
    graph: &CodeGraph,
    repo: impl AsRef<Path>,
    base: &str,
    limit: usize,
) -> DiffContext {
    let files = diff_files(repo, base);
    let changed_paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let changed: HashMap<_, _> = files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let query = if changed_paths.is_empty() {
        "recent changed code".to_string()
    } else {
        changed_paths.join(" ")
    };
    let mut ranked = crate::score::rank_graph(graph, &query, graph.nodes.len());
    for result in &mut ranked.results {
        if let Some(file) = changed.get(result.node.path.as_str()) {
            result.score += 0.35;
            result
                .explanation
                .strong_signals
                .push("changed in diff".into());
            if let Some(span) = result.node.span {
                let overlaps_added = file
                    .added_lines
                    .iter()
                    .any(|line| *line >= span.start_line && *line <= span.end_line);
                let overlaps_deleted = file
                    .deleted_lines
                    .iter()
                    .any(|line| *line >= span.start_line && *line <= span.end_line);
                if overlaps_added || overlaps_deleted {
                    result.score += 0.75;
                    result
                        .explanation
                        .strong_signals
                        .push("changed lines overlap symbol span".into());
                }
            }
        }
    }
    ranked.results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.results.truncate(limit);
    DiffContext {
        base: base.to_string(),
        changed_paths,
        files,
        ranked,
    }
}

fn parse_name_status(repo: &Path, base: &str) -> Vec<DiffFile> {
    let output = Command::new("git")
        .args([
            "-C",
            repo.to_string_lossy().as_ref(),
            "diff",
            "--name-status",
            "--find-renames",
            base,
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(parse_name_status_line)
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_name_status_line(line: &str) -> Option<DiffFile> {
    let parts = line.split('\t').collect::<Vec<_>>();
    let raw_status = parts.first()?.trim();
    let status_code = raw_status.chars().next()?;
    let status = match status_code {
        'A' => DiffStatus::Added,
        'M' => DiffStatus::Modified,
        'D' => DiffStatus::Deleted,
        'R' => DiffStatus::Renamed,
        'C' => DiffStatus::Copied,
        'T' => DiffStatus::TypeChanged,
        'U' => DiffStatus::Unmerged,
        _ => DiffStatus::Unknown(raw_status.to_string()),
    };
    let (old_path, path) = if matches!(status, DiffStatus::Renamed | DiffStatus::Copied) {
        (
            parts.get(1).map(|value| (*value).to_string()),
            parts.get(2)?,
        )
    } else {
        (None, parts.get(1)?)
    };
    Some(DiffFile {
        path: (*path).to_string(),
        old_path,
        status,
        added_lines: Vec::new(),
        deleted_lines: Vec::new(),
        hunks: Vec::new(),
    })
}

fn parse_diff_hunks(repo: &Path, base: &str) -> HashMap<String, Vec<DiffHunk>> {
    let output = Command::new("git")
        .args([
            "-C",
            repo.to_string_lossy().as_ref(),
            "diff",
            "--unified=0",
            "--find-renames",
            base,
        ])
        .output();
    let Ok(output) = output else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }

    let mut hunks_by_path: HashMap<String, Vec<DiffHunk>> = HashMap::new();
    let mut current_path: Option<String> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let hunk_re = regex::Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap();

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.starts_with("diff --git ") {
            flush_hunk(&mut hunks_by_path, &current_path, &mut current_hunk);
            current_path = parse_git_diff_path(line);
            continue;
        }
        if let Some(cap) = hunk_re.captures(line) {
            flush_hunk(&mut hunks_by_path, &current_path, &mut current_hunk);
            let old_start = cap[1].parse().unwrap_or(0);
            let old_count = cap
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let new_start = cap[3].parse().unwrap_or(0);
            let new_count = cap
                .get(4)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            old_line = old_start;
            new_line = new_start;
            current_hunk = Some(DiffHunk {
                old_start,
                old_count,
                new_start,
                new_count,
                added_lines: Vec::new(),
                deleted_lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = current_hunk.as_mut() else {
            continue;
        };
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            hunk.added_lines.push(new_line);
            new_line += 1;
        } else if line.starts_with('-') {
            hunk.deleted_lines.push(old_line);
            old_line += 1;
        } else {
            old_line += 1;
            new_line += 1;
        }
    }
    flush_hunk(&mut hunks_by_path, &current_path, &mut current_hunk);
    hunks_by_path
}

fn parse_git_diff_path(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let _diff = parts.next()?;
    let _git = parts.next()?;
    let _old = parts.next()?;
    let new = parts.next()?;
    Some(new.trim_start_matches("b/").to_string())
}

fn flush_hunk(
    hunks_by_path: &mut HashMap<String, Vec<DiffHunk>>,
    current_path: &Option<String>,
    current_hunk: &mut Option<DiffHunk>,
) {
    if let (Some(path), Some(hunk)) = (current_path, current_hunk.take()) {
        hunks_by_path.entry(path.clone()).or_default().push(hunk);
    }
}

fn sorted_unique(values: impl Iterator<Item = usize>) -> Vec<usize> {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::build_graph;
    use std::fs;
    use std::process::Command;

    #[test]
    fn diff_context_parses_status_hunks_and_boosts_changed_symbol_lines() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = temp.path();
        Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo)
            .output()
            .unwrap();

        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("src/auth.ts"),
            "export function validateSession(session: Session) {\n  return true;\n}\n\nexport function formatDate(value: Date) {\n  return value.toISOString();\n}\n",
        )
        .unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo)
            .output()
            .unwrap();

        fs::write(
            repo.join("src/auth.ts"),
            "export function validateSession(session: Session) {\n  return Boolean(session.token);\n}\n\nexport function formatDate(value: Date) {\n  return value.toISOString();\n}\n",
        )
        .unwrap();

        let graph = build_graph(repo).unwrap();
        let context = diff_context(&graph, repo, "HEAD", 5);
        let file = context
            .files
            .iter()
            .find(|file| file.path == "src/auth.ts")
            .expect("changed file metadata");

        assert_eq!(file.status, DiffStatus::Modified);
        assert!(file.added_lines.contains(&2));
        assert!(!file.hunks.is_empty());
        assert_eq!(context.ranked.results[0].node.name, "validateSession");
        assert!(context.ranked.results[0]
            .explanation
            .strong_signals
            .iter()
            .any(|signal| signal.contains("changed lines")));
    }
}
