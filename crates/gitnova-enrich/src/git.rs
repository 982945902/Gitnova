use anyhow::Result;
use git2::{DiffOptions, Repository};
use gitnova_core::{graph::recompute_degrees, CodeGraph, NodeKind};
use std::collections::HashMap;
use std::path::Path;

pub fn apply_git_churn(repo_root: impl AsRef<Path>, graph: &mut CodeGraph) -> Result<()> {
    let repo = match Repository::discover(repo_root.as_ref()) {
        Ok(repo) => repo,
        Err(_) => return Ok(()),
    };
    let workdir = repo.workdir().unwrap_or(repo_root.as_ref());
    let mut churn_by_path: HashMap<String, u32> = HashMap::new();
    let mut last_changed: HashMap<String, u64> = HashMap::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let cutoff = now - 90 * 24 * 60 * 60;
    let mut revwalk = match repo.revwalk() {
        Ok(revwalk) => revwalk,
        Err(_) => return Ok(()),
    };
    if revwalk.push_head().is_err() {
        return Ok(());
    }
    for oid in revwalk.take(500).flatten() {
        let commit = match repo.find_commit(oid) {
            Ok(commit) => commit,
            Err(_) => continue,
        };
        let commit_time = commit.time().seconds();
        let tree = match commit.tree() {
            Ok(tree) => tree,
            Err(_) => continue,
        };
        let parent_tree = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|parent| parent.tree().ok())
        } else {
            None
        };
        let mut diff_opts = DiffOptions::new();
        let diff =
            match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut diff_opts)) {
                Ok(diff) => diff,
                Err(_) => continue,
            };
        for delta in diff.deltas() {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|path| path.to_string_lossy().replace('\\', "/"));
            if let Some(path) = path {
                if commit_time >= cutoff {
                    *churn_by_path.entry(path.clone()).or_default() += 1;
                }
                last_changed
                    .entry(path)
                    .and_modify(|existing| *existing = (*existing).max(commit_time as u64))
                    .or_insert(commit_time as u64);
            }
        }
    }
    for node in &mut graph.nodes {
        if matches!(node.kind, NodeKind::Repository | NodeKind::Import) {
            continue;
        }
        let path = normalize_to_repo_path(workdir, &node.path);
        node.metrics.churn_90d = *churn_by_path.get(&path).unwrap_or(&0);
        node.metrics.last_changed_unix = last_changed.get(&path).copied();
    }
    recompute_degrees(graph);
    Ok(())
}

fn normalize_to_repo_path(_workdir: &Path, path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::build_graph;
    use std::fs;
    use std::process::Command;

    #[test]
    fn non_git_repo_falls_back_to_zero_churn() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("lib.rs"), "pub fn a() {}\n").unwrap();
        let mut graph = build_graph(temp.path()).unwrap();
        apply_git_churn(temp.path(), &mut graph).unwrap();
        assert!(graph.nodes.iter().all(|node| node.metrics.churn_90d == 0));
    }

    #[test]
    fn git_repo_records_churn() {
        let temp = tempfile::TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "gitnova@example.test"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Gitnova"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        fs::write(temp.path().join("lib.rs"), "pub fn a() {}\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "one"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        fs::write(temp.path().join("lib.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "two"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        let mut graph = build_graph(temp.path()).unwrap();
        apply_git_churn(temp.path(), &mut graph).unwrap();
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.path == "lib.rs" && node.metrics.churn_90d > 0));
    }
}
