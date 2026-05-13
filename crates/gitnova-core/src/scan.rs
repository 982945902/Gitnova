use crate::error::Result;
use crate::language::detect_language;
use crate::model::Language;
use ignore::WalkBuilder;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub absolute_path: PathBuf,
    pub relative_path: String,
    pub language: Language,
    pub content_hash: String,
}

pub fn scan_repository(root: impl AsRef<Path>) -> Result<Vec<SourceFile>> {
    let root = root.as_ref();
    let mut files = Vec::new();
    let simple_ignores = load_simple_gitignore(root);
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true);
    builder.filter_entry(|entry| {
        let name = entry.file_name().to_string_lossy();
        !matches!(
            name.as_ref(),
            ".git" | ".gitnova" | "target" | "node_modules" | ".venv" | "dist" | "build"
        )
    });

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.into_path();
        let Some(language) = detect_language(&path) else {
            continue;
        };
        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if simple_ignores
            .iter()
            .any(|pattern| matches_simple_ignore(&relative_path, pattern))
        {
            continue;
        }
        let bytes = fs::read(&path)?;
        let content_hash = format!("{:x}", Sha256::digest(&bytes));
        files.push(SourceFile {
            absolute_path: path,
            relative_path,
            language,
            content_hash,
        });
    }
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

fn load_simple_gitignore(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join(".gitignore"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('!'))
        .map(|line| line.trim_end_matches('/').to_string())
        .collect()
}

fn matches_simple_ignore(path: &str, pattern: &str) -> bool {
    path == pattern
        || path.starts_with(&format!("{pattern}/"))
        || path.ends_with(&format!("/{pattern}"))
        || path.ends_with(pattern.trim_start_matches('*'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn scanner_respects_gitignore_and_detects_languages() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join(".gitignore"), "ignored.py\n").unwrap();
        fs::write(temp.path().join("lib.rs"), "fn main() {}\n").unwrap();
        fs::write(temp.path().join("app.ts"), "export const x = 1;\n").unwrap();
        fs::write(temp.path().join("ignored.py"), "print('no')\n").unwrap();
        fs::create_dir(temp.path().join("node_modules")).unwrap();
        let mut ignored = fs::File::create(temp.path().join("node_modules/pkg.js")).unwrap();
        writeln!(ignored, "console.log('no')").unwrap();

        let files = scan_repository(temp.path()).unwrap();
        let paths: Vec<_> = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect();
        assert_eq!(paths, vec!["app.ts", "lib.rs"]);
    }
}
