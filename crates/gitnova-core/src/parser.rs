use crate::error::{GitnovaError, Result};
use crate::model::{Language, Span};
use crate::scan::SourceFile;
use std::fs;
use std::path::Path;
use tree_sitter::Parser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub path: String,
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub source: SourceFile,
    pub text: String,
    pub diagnostics: Vec<ParseDiagnostic>,
}

pub fn parse_source_file(file: &SourceFile) -> Result<ParsedFile> {
    let text = fs::read_to_string(&file.absolute_path)?;
    let mut parser = Parser::new();
    let language = tree_sitter_language(file.language);
    parser
        .set_language(&language)
        .map_err(|err| GitnovaError::TreeSitter(err.to_string()))?;
    let tree = parser.parse(&text, None).ok_or_else(|| {
        GitnovaError::TreeSitter(format!("could not parse {}", file.relative_path))
    })?;
    let mut diagnostics = Vec::new();
    if tree.root_node().has_error() {
        diagnostics.push(ParseDiagnostic {
            path: file.relative_path.clone(),
            line: 1,
            message: "tree-sitter reported syntax errors; extraction continued heuristically"
                .into(),
        });
    }
    Ok(ParsedFile {
        source: file.clone(),
        text,
        diagnostics,
    })
}

pub fn parse_repository_files(files: &[SourceFile]) -> Vec<ParsedFile> {
    files
        .iter()
        .filter_map(|file| parse_source_file(file).ok())
        .collect()
}

pub fn line_span(line_index: usize, line: &str) -> Span {
    Span {
        start_line: line_index + 1,
        start_col: 1,
        end_line: line_index + 1,
        end_col: line.chars().count() + 1,
    }
}

fn tree_sitter_language(language: Language) -> tree_sitter::Language {
    match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
    }
}

pub fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::scan_repository;
    use std::fs;

    #[test]
    fn parser_reports_invalid_files_non_fatally() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("broken.rs"), "fn broken( {\n").unwrap();
        let files = scan_repository(temp.path()).unwrap();
        let parsed = parse_source_file(&files[0]).unwrap();
        assert!(!parsed.diagnostics.is_empty());
    }
}
