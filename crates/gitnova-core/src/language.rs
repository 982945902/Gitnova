use crate::model::Language;
use std::path::Path;

pub fn detect_language(path: impl AsRef<Path>) -> Option<Language> {
    let path = path.as_ref();
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some(Language::Rust),
        "ts" | "tsx" => Some(Language::TypeScript),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
        "py" => Some(Language::Python),
        "cpp" | "cxx" | "cc" | "c++" | "hpp" | "hxx" | "hh" | "h" | "c" => Some(Language::Cpp),
        _ => None,
    }
}

pub fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/test")
        || lower.contains("_test.")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.ends_with("tests.rs")
}
