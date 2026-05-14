use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspStatus {
    pub tool: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LspRelationKind {
    Definition,
    Reference,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LspEnrichment {
    pub symbol_id: String,
    pub provider: String,
    pub kind: LspRelationKind,
    pub target_uri: String,
    pub target_line: u32,
    pub target_character: u32,
}

pub trait LspTransport {
    fn send(&mut self, message: &Value) -> anyhow::Result<()>;
    fn receive(&mut self) -> anyhow::Result<Option<Value>>;
}

pub fn detect_language_servers() -> Vec<LspStatus> {
    [
        "rust-analyzer",
        "typescript-language-server",
        "pyright-langserver",
    ]
    .into_iter()
    .map(|tool| LspStatus {
        tool: tool.to_string(),
        available: Command::new(tool).arg("--version").output().is_ok(),
    })
    .collect()
}

pub fn enrich_best_effort() -> Vec<LspStatus> {
    detect_language_servers()
}

pub fn run_lsp_probe_with_command(
    command: impl AsRef<Path>,
    args: &[&str],
    root: impl AsRef<Path>,
    graph: &gitnova_core::CodeGraph,
) -> anyhow::Result<Vec<LspEnrichment>> {
    let mut transport = ProcessLspTransport::spawn(command, args)?;
    let result = probe_lsp_graph(&mut transport, root.as_ref(), graph);
    let _ = transport.send(&json!({
        "jsonrpc": "2.0",
        "id": 10_000,
        "method": "shutdown",
        "params": null
    }));
    let _ = transport.send(&json!({
        "jsonrpc": "2.0",
        "method": "exit"
    }));
    result
}

pub fn probe_lsp_graph(
    transport: &mut impl LspTransport,
    root: &Path,
    graph: &gitnova_core::CodeGraph,
) -> anyhow::Result<Vec<LspEnrichment>> {
    let mut id = 1u64;
    transport.send(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": file_uri(root),
            "capabilities": {}
        }
    }))?;
    let _ = transport.receive()?;
    id += 1;

    let file_texts = graph
        .nodes
        .iter()
        .filter(|node| node.kind == gitnova_core::NodeKind::File)
        .map(|node| (node.path.clone(), node.text.clone()))
        .collect::<HashMap<_, _>>();
    let mut opened = HashSet::new();
    let mut enrichments = Vec::new();

    for node in graph.nodes.iter().filter(|node| {
        matches!(
            node.kind,
            gitnova_core::NodeKind::Function
                | gitnova_core::NodeKind::Method
                | gitnova_core::NodeKind::Class
                | gitnova_core::NodeKind::Struct
                | gitnova_core::NodeKind::Trait
                | gitnova_core::NodeKind::Interface
        ) && node.span.is_some()
    }) {
        let Some(span) = node.span else {
            continue;
        };
        let uri = file_uri(&root.join(&node.path));
        if opened.insert(node.path.clone()) {
            transport.send(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": node.language.map(|language| language.as_str()).unwrap_or("text"),
                        "version": 1,
                        "text": file_texts.get(&node.path).cloned().unwrap_or_default()
                    }
                }
            }))?;
        }

        let position = json!({
            "line": span.start_line.saturating_sub(1),
            "character": span.start_col.saturating_sub(1)
        });
        transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": uri },
                "position": position
            }
        }))?;
        if let Some(response) = transport.receive()? {
            collect_locations(
                &mut enrichments,
                &node.id,
                "lsp",
                LspRelationKind::Definition,
                response.get("result").unwrap_or(&Value::Null),
            );
        }
        id += 1;

        transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/references",
            "params": {
                "textDocument": { "uri": uri },
                "position": position,
                "context": { "includeDeclaration": false }
            }
        }))?;
        if let Some(response) = transport.receive()? {
            collect_locations(
                &mut enrichments,
                &node.id,
                "lsp",
                LspRelationKind::Reference,
                response.get("result").unwrap_or(&Value::Null),
            );
        }
        id += 1;
    }

    Ok(enrichments)
}

pub fn apply_lsp_enrichments(
    graph: &mut gitnova_core::CodeGraph,
    enrichments: &[LspEnrichment],
) -> usize {
    let mut applied = 0usize;
    let mut existing_edges = graph
        .edges
        .iter()
        .map(|edge| (edge.from.clone(), edge.to.clone(), edge.kind.clone()))
        .collect::<HashSet<_>>();
    let targets = graph
        .nodes
        .iter()
        .filter_map(|node| {
            node.span.map(|span| {
                (
                    node.id.clone(),
                    node.path.clone(),
                    span.start_line,
                    span.end_line,
                )
            })
        })
        .collect::<Vec<_>>();

    for enrichment in enrichments {
        if let Some(source) = graph
            .nodes
            .iter_mut()
            .find(|node| node.id == enrichment.symbol_id)
        {
            let prefix = match enrichment.kind {
                LspRelationKind::Definition => "lsp_definition",
                LspRelationKind::Reference => "lsp_reference",
            };
            let tag = format!(
                "{prefix}:{}:{}:{}",
                enrichment.provider,
                enrichment.target_uri,
                enrichment.target_line + 1
            );
            if !source.tags.contains(&tag) {
                source.tags.push(tag);
                applied += 1;
            }
        }
        let Some(target_path) = path_from_file_uri(&enrichment.target_uri) else {
            continue;
        };
        let target_line = enrichment.target_line as usize + 1;
        if let Some((target_id, _, _, _)) = targets.iter().find(|(_, path, start, end)| {
            (target_path == *path || target_path.ends_with(&format!("/{path}")))
                && target_line >= *start
                && target_line <= *end
        }) {
            gitnova_core::graph::add_edge(
                &mut graph.edges,
                &mut existing_edges,
                &enrichment.symbol_id,
                target_id,
                gitnova_core::EdgeKind::References,
                9_000,
            );
        }
    }
    gitnova_core::graph::recompute_degrees(graph);
    applied
}

pub fn apply_lsp_metadata(graph: &mut gitnova_core::CodeGraph) -> Vec<LspStatus> {
    let statuses = detect_language_servers();
    if let Some(repo) = graph
        .nodes
        .iter_mut()
        .find(|node| node.kind == gitnova_core::NodeKind::Repository)
    {
        for status in &statuses {
            let state = if status.available {
                "available"
            } else {
                "missing"
            };
            repo.tags.push(format!("lsp:{}:{state}", status.tool));
        }
    }
    if std::env::var("GITNOVA_LSP_PROBE").as_deref() == Ok("1") {
        let repo_root = PathBuf::from(&graph.repo_root);
        let mut probe_tags = Vec::new();
        for status in statuses.iter().filter(|status| status.available) {
            let Some(args) = lsp_stdio_args(&status.tool) else {
                continue;
            };
            match run_lsp_probe_with_command(&status.tool, args, &repo_root, graph) {
                Ok(enrichments) => {
                    let count = apply_lsp_enrichments(graph, &enrichments);
                    probe_tags.push(format!("lsp:{}:probed:{count}", status.tool));
                }
                Err(err) => probe_tags.push(format!("lsp:{}:probe-error:{err}", status.tool)),
            }
        }
        if let Some(repo) = graph
            .nodes
            .iter_mut()
            .find(|node| node.kind == gitnova_core::NodeKind::Repository)
        {
            repo.tags.extend(probe_tags);
        }
    }
    statuses
}

fn lsp_stdio_args(tool: &str) -> Option<&'static [&'static str]> {
    match tool {
        "rust-analyzer" => Some(&[]),
        "typescript-language-server" | "pyright-langserver" => Some(&["--stdio"]),
        _ => None,
    }
}

struct ProcessLspTransport {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ProcessLspTransport {
    fn spawn(command: impl AsRef<Path>, args: &[&str]) -> anyhow::Result<Self> {
        let mut child = Command::new(command.as_ref())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("lsp process missing stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("lsp process missing stdout"))?;
        Ok(Self {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }
}

impl LspTransport for ProcessLspTransport {
    fn send(&mut self, message: &Value) -> anyhow::Result<()> {
        write_lsp_message(&mut self.stdin, message)
    }

    fn receive(&mut self) -> anyhow::Result<Option<Value>> {
        read_lsp_message(&mut self.stdout)
    }
}

fn write_lsp_message(writer: &mut impl Write, message: &Value) -> anyhow::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

fn read_lsp_message(reader: &mut impl BufRead) -> anyhow::Result<Option<Value>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse::<usize>()?);
        }
    }
    let Some(length) = content_length else {
        return Ok(None);
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

fn collect_locations(
    enrichments: &mut Vec<LspEnrichment>,
    symbol_id: &str,
    provider: &str,
    kind: LspRelationKind,
    result: &Value,
) {
    match result {
        Value::Array(items) => {
            for item in items {
                collect_location(enrichments, symbol_id, provider, kind.clone(), item);
            }
        }
        Value::Object(_) => collect_location(enrichments, symbol_id, provider, kind, result),
        _ => {}
    }
}

fn collect_location(
    enrichments: &mut Vec<LspEnrichment>,
    symbol_id: &str,
    provider: &str,
    kind: LspRelationKind,
    item: &Value,
) {
    let uri = item
        .get("uri")
        .or_else(|| item.get("targetUri"))
        .and_then(Value::as_str);
    let range = item
        .get("range")
        .or_else(|| item.get("targetRange"))
        .or_else(|| item.get("targetSelectionRange"));
    let Some((uri, range)) = uri.zip(range) else {
        return;
    };
    let start = range.get("start").unwrap_or(&Value::Null);
    let Some(line) = start.get("line").and_then(Value::as_u64) else {
        return;
    };
    let character = start.get("character").and_then(Value::as_u64).unwrap_or(0);
    enrichments.push(LspEnrichment {
        symbol_id: symbol_id.to_string(),
        provider: provider.to_string(),
        kind,
        target_uri: uri.to_string(),
        target_line: line as u32,
        target_character: character as u32,
    });
}

fn file_uri(path: &Path) -> String {
    let path = path.to_string_lossy().replace(' ', "%20");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

fn path_from_file_uri(uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file://")?.replace("%20", " ");
    Some(PathBuf::from(path).to_string_lossy().to_string())
}

#[cfg(test)]
#[derive(Debug)]
pub struct MockLspTransport {
    responses: std::collections::VecDeque<Value>,
    sent: Vec<Value>,
}

#[cfg(test)]
impl MockLspTransport {
    pub fn new(responses: Vec<Value>) -> Self {
        Self {
            responses: responses.into(),
            sent: Vec::new(),
        }
    }

    pub fn sent_methods(&self) -> Vec<String> {
        self.sent
            .iter()
            .filter_map(|message| message.get("method").and_then(Value::as_str))
            .map(ToOwned::to_owned)
            .collect()
    }
}

#[cfg(test)]
impl LspTransport for MockLspTransport {
    fn send(&mut self, message: &Value) -> anyhow::Result<()> {
        self.sent.push(message.clone());
        Ok(())
    }

    fn receive(&mut self) -> anyhow::Result<Option<Value>> {
        Ok(self.responses.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_lsp_tools_are_status_not_errors() {
        let statuses = detect_language_servers();
        assert_eq!(statuses.len(), 3);
    }

    #[test]
    fn lsp_metadata_is_attached_without_requiring_tools() {
        let mut graph = gitnova_core::CodeGraph::empty("repo".into());
        graph.nodes.push(gitnova_core::Node {
            id: "repo".into(),
            kind: gitnova_core::NodeKind::Repository,
            name: "repo".into(),
            qualified_name: "repo".into(),
            path: String::new(),
            span: None,
            language: None,
            text: String::new(),
            tags: Vec::new(),
            metrics: gitnova_core::NodeMetrics::default(),
        });
        let statuses = apply_lsp_metadata(&mut graph);
        assert_eq!(statuses.len(), 3);
        assert!(graph.nodes[0]
            .tags
            .iter()
            .any(|tag| tag.starts_with("lsp:")));
    }

    #[test]
    fn lsp_probe_records_definition_and_reference_enrichments() {
        let mut graph = gitnova_core::CodeGraph::empty("/repo".into());
        graph.nodes.push(gitnova_core::Node {
            id: "repo".into(),
            kind: gitnova_core::NodeKind::Repository,
            name: "repo".into(),
            qualified_name: "/repo".into(),
            path: String::new(),
            span: None,
            language: None,
            text: String::new(),
            tags: Vec::new(),
            metrics: gitnova_core::NodeMetrics::default(),
        });
        graph.nodes.push(gitnova_core::Node {
            id: "symbol".into(),
            kind: gitnova_core::NodeKind::Function,
            name: "validateSession".into(),
            qualified_name: "src/auth.ts::validateSession".into(),
            path: "src/auth.ts".into(),
            span: Some(gitnova_core::Span {
                start_line: 10,
                start_col: 8,
                end_line: 12,
                end_col: 2,
            }),
            language: Some(gitnova_core::Language::TypeScript),
            text: "function validateSession() { return checkToken(); }".into(),
            tags: Vec::new(),
            metrics: gitnova_core::NodeMetrics::default(),
        });

        let mut transport = MockLspTransport::new(vec![
            serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}),
            serde_json::json!({"jsonrpc":"2.0","id":2,"result":{"uri":"file:///repo/src/token.ts","range":{"start":{"line":4,"character":0},"end":{"line":4,"character":10}}}}),
            serde_json::json!({"jsonrpc":"2.0","id":3,"result":[{"uri":"file:///repo/src/auth.ts","range":{"start":{"line":15,"character":2},"end":{"line":15,"character":17}}}]}),
        ]);

        let enrichments =
            probe_lsp_graph(&mut transport, std::path::Path::new("/repo"), &graph).unwrap();
        let methods = transport.sent_methods();

        assert!(methods.contains(&"initialize".to_string()));
        assert!(methods.contains(&"textDocument/definition".to_string()));
        assert!(methods.contains(&"textDocument/references".to_string()));
        assert!(enrichments
            .iter()
            .any(|item| item.kind == LspRelationKind::Definition));
        assert!(enrichments
            .iter()
            .any(|item| item.kind == LspRelationKind::Reference));

        let applied = apply_lsp_enrichments(&mut graph, &enrichments);
        assert!(applied >= 2);
        assert!(graph.nodes[1]
            .tags
            .iter()
            .any(|tag| tag.starts_with("lsp_definition:")));
    }
}
