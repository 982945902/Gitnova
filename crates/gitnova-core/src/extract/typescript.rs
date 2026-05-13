use super::{fill_symbol_text_and_calls, import_from_line, ExtractedSymbol, FileExtraction};
use crate::model::NodeKind;
use crate::parser::{line_span, ParsedFile};
use regex::Regex;

pub fn extract(parsed: &ParsedFile) -> FileExtraction {
    let path = &parsed.source.relative_path;
    let mut extraction = FileExtraction::default();
    let import_re = Regex::new(r#"^\s*import\s+\{?([^}"']+)\}?\s+from"#).unwrap();
    let fn_re =
        Regex::new(r"\b(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let arrow_re =
        Regex::new(r"\b(?:export\s+)?(?:const|let)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=").unwrap();
    let class_re = Regex::new(r"\b(?:export\s+)?class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let interface_re = Regex::new(r"\b(?:export\s+)?interface\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let method_re = Regex::new(
        r"^\s*(?:public\s+|private\s+|protected\s+)?(?:async\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    )
    .unwrap();
    let mut current_class: Option<String> = None;
    let mut class_depth = 0isize;
    let mut symbols = Vec::new();

    for (index, line) in parsed.text.lines().enumerate() {
        if let Some(cap) = import_re.captures(line) {
            for name in cap[1].split(',') {
                let name = name.split_whitespace().next().unwrap_or("").to_string();
                if !name.is_empty() {
                    extraction
                        .imports
                        .push(import_from_line(path, index, line, name));
                }
            }
        }
        if let Some(cap) = class_re.captures(line) {
            let name = cap[1].to_string();
            push_symbol(
                &mut symbols,
                NodeKind::Class,
                path,
                index,
                line,
                &name,
                &name,
            );
            current_class = Some(name);
            class_depth = brace_delta(line).max(1);
            continue;
        }
        if let Some(cap) = interface_re.captures(line) {
            push_symbol(
                &mut symbols,
                NodeKind::Interface,
                path,
                index,
                line,
                &cap[1],
                &cap[1],
            );
            continue;
        }
        if let Some(cap) = fn_re.captures(line) {
            push_symbol(
                &mut symbols,
                NodeKind::Function,
                path,
                index,
                line,
                &cap[1],
                &cap[1],
            );
        } else if let Some(cap) = arrow_re.captures(line) {
            push_symbol(
                &mut symbols,
                NodeKind::Function,
                path,
                index,
                line,
                &cap[1],
                &cap[1],
            );
        } else if let (Some(parent), Some(cap)) = (&current_class, method_re.captures(line)) {
            let name = cap[1].to_string();
            let qualified = format!("{parent}::{name}");
            push_symbol(
                &mut symbols,
                NodeKind::Method,
                path,
                index,
                line,
                &name,
                &qualified,
            );
        }
        if current_class.is_some() {
            class_depth += brace_delta(line);
            if class_depth <= 0 {
                current_class = None;
                class_depth = 0;
            }
        }
    }

    fill_symbol_text_and_calls(&mut symbols, &parsed.text);
    extraction.symbols = symbols;
    extraction
}

fn push_symbol(
    symbols: &mut Vec<ExtractedSymbol>,
    kind: NodeKind,
    path: &str,
    index: usize,
    line: &str,
    name: &str,
    qualified_name: &str,
) {
    symbols.push(ExtractedSymbol {
        kind,
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        path: path.to_string(),
        span: line_span(index, line),
        text: String::new(),
        calls: Vec::new(),
        tags: Vec::new(),
    });
}

fn brace_delta(line: &str) -> isize {
    line.chars().filter(|ch| *ch == '{').count() as isize
        - line.chars().filter(|ch| *ch == '}').count() as isize
}
