pub mod cpp;
pub mod javascript;
pub mod python;
pub mod rust;
pub mod typescript;

use crate::model::{Language, NodeKind, Span};
use crate::parser::{line_span, ParsedFile};
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedSymbol {
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub path: String,
    pub span: Span,
    pub text: String,
    pub calls: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedImport {
    pub name: String,
    pub qualified_name: String,
    pub path: String,
    pub span: Span,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub imports: Vec<ExtractedImport>,
}

pub fn extract_file(parsed: &ParsedFile) -> FileExtraction {
    match parsed.source.language {
        Language::Rust => rust::extract(parsed),
        Language::TypeScript => typescript::extract(parsed),
        Language::JavaScript => javascript::extract(parsed),
        Language::Python => python::extract(parsed),
        Language::Cpp => cpp::extract(parsed),
    }
}

fn symbol_text(lines: &[&str], start_line: usize, next_start_line: Option<usize>) -> String {
    let start = start_line.saturating_sub(1);
    let end = next_start_line
        .map(|line| line.saturating_sub(1))
        .unwrap_or(lines.len());
    lines[start..end.min(lines.len())].join("\n")
}

fn fill_symbol_text_and_calls(symbols: &mut [ExtractedSymbol], source: &str) {
    let lines: Vec<&str> = source.lines().collect();
    let starts: Vec<usize> = symbols
        .iter()
        .map(|symbol| symbol.span.start_line)
        .collect();
    for (index, symbol) in symbols.iter_mut().enumerate() {
        let next_start = starts.get(index + 1).copied();
        let text = symbol_text(&lines, symbol.span.start_line, next_start);
        let calls = calls_in_text(&text, &symbol.name);
        symbol.text = text;
        symbol.calls = calls;
    }
}

fn calls_in_text(text: &str, own_name: &str) -> Vec<String> {
    let re = Regex::new(r"([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap();
    let mut calls = Vec::new();
    for cap in re.captures_iter(text) {
        let name = cap[1].to_string();
        if name == own_name || is_keyword(&name) {
            continue;
        }
        if !calls.contains(&name) {
            calls.push(name);
        }
    }
    calls
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "for"
            | "while"
            | "match"
            | "return"
            | "new"
            | "Self"
            | "Some"
            | "Ok"
            | "Err"
            | "print"
            | "println"
            | "console"
            | "function"
            | "def"
            | "class"
    )
}

fn import_from_line(path: &str, line_index: usize, line: &str, name: String) -> ExtractedImport {
    ExtractedImport {
        qualified_name: format!("{path}::{name}"),
        name,
        path: path.to_string(),
        span: line_span(line_index, line),
        text: line.trim().to_string(),
    }
}
