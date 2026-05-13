use super::{fill_symbol_text_and_calls, import_from_line, ExtractedSymbol, FileExtraction};
use crate::model::NodeKind;
use crate::parser::{line_span, ParsedFile};
use regex::Regex;

pub fn extract(parsed: &ParsedFile) -> FileExtraction {
    let path = &parsed.source.relative_path;
    let import_re =
        Regex::new(r"^\s*(?:from\s+([A-Za-z0-9_\.]+)\s+import\s+(.+)|import\s+(.+))").unwrap();
    let class_re = Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let def_re = Regex::new(r"^\s*def\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let mut extraction = FileExtraction::default();
    let mut current_class: Option<(String, usize)> = None;
    let mut symbols = Vec::new();

    for (index, line) in parsed.text.lines().enumerate() {
        if let Some(cap) = import_re.captures(line) {
            let names = cap
                .get(2)
                .or_else(|| cap.get(3))
                .map(|m| m.as_str())
                .unwrap_or("");
            for name in names.split(',') {
                let name = name.split_whitespace().next().unwrap_or("").to_string();
                if !name.is_empty() {
                    extraction
                        .imports
                        .push(import_from_line(path, index, line, name));
                }
            }
        }
        let indent = line.chars().take_while(|ch| ch.is_whitespace()).count();
        if let Some((_, class_indent)) = &current_class {
            if !line.trim().is_empty() && indent <= *class_indent {
                current_class = None;
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
            current_class = Some((name, indent));
        } else if let Some(cap) = def_re.captures(line) {
            let name = cap[1].to_string();
            let qualified = current_class
                .as_ref()
                .map(|(class_name, _)| format!("{class_name}::{name}"))
                .unwrap_or_else(|| name.clone());
            let kind = if current_class.is_some() {
                NodeKind::Method
            } else {
                NodeKind::Function
            };
            push_symbol(&mut symbols, kind, path, index, line, &name, &qualified);
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
