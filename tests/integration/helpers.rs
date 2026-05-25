use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

pub fn copy_dir(src: &Path, dst: &Path) {
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

pub fn temp_fixture(name: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join(name);
    copy_dir(&fixture(name), &repo);
    (temp, repo)
}
