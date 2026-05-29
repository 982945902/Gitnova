use gitnova_core::model::{CodeGraph, Edge, EdgeKind, Node, NodeKind, Span};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasOptions {
    pub entry: Option<String>,
    pub max_flows: usize,
    pub max_depth: usize,
    pub max_nodes_per_flow: usize,
}

impl Default for AtlasOptions {
    fn default() -> Self {
        Self {
            entry: None,
            max_flows: 6,
            max_depth: 4,
            max_nodes_per_flow: 80,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasReport {
    pub schema_version: u32,
    pub title: String,
    pub repo_root: String,
    pub indexed_at_unix: u64,
    pub summary: AtlasSummary,
    pub flows: Vec<AtlasFlow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasSummary {
    pub nodes: usize,
    pub edges: usize,
    pub files: usize,
    pub selected_flows: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasFlow {
    pub id: String,
    pub name: String,
    pub entry: AtlasNode,
    pub nodes: Vec<AtlasNode>,
    pub edges: Vec<AtlasEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasNode {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub role: String,
    pub path: String,
    pub span: Option<Span>,
    pub depth: usize,
    pub in_degree: usize,
    pub out_degree: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

pub fn build_atlas(graph: &CodeGraph, options: AtlasOptions) -> AtlasReport {
    let max_flows = options.max_flows.max(1);
    let max_depth = options.max_depth.min(8);
    let max_nodes_per_flow = options.max_nodes_per_flow.max(1);
    let entries = select_entrypoints(graph, options.entry.as_deref(), max_flows);
    let flows = entries
        .into_iter()
        .map(|entry| build_flow(graph, entry, max_depth, max_nodes_per_flow))
        .collect::<Vec<_>>();
    AtlasReport {
        schema_version: 1,
        title: report_title(graph),
        repo_root: graph.repo_root.clone(),
        indexed_at_unix: graph.indexed_at_unix,
        summary: AtlasSummary {
            nodes: graph.nodes.len(),
            edges: graph.edges.len(),
            files: graph
                .nodes
                .iter()
                .filter(|node| node.kind == NodeKind::File)
                .count(),
            selected_flows: flows.len(),
        },
        flows,
    }
}

pub fn render_html(report: &AtlasReport) -> anyhow::Result<String> {
    let data = serde_json::to_string(report)?.replace('<', "\\u003c");
    let title = escape_html(&report.title);
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    :root {{
      color-scheme: dark;
      --bg: #101412;
      --panel: #171d1a;
      --panel-2: #202721;
      --line: #344238;
      --text: #edf2ec;
      --muted: #a7b3aa;
      --accent: #7ddfbd;
      --accent-2: #f0c674;
      --danger: #f48f8f;
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      min-height: 100vh;
      background: var(--bg);
      color: var(--text);
      font: 14px/1.5 Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }}
    .app {{ display: grid; grid-template-columns: 360px minmax(0, 1fr); min-height: 100vh; }}
    aside {{
      border-right: 1px solid var(--line);
      background: #121713;
      padding: 24px;
      overflow: auto;
      max-height: 100vh;
    }}
    main {{ padding: 28px; overflow: auto; max-height: 100vh; }}
    h1 {{ margin: 0 0 8px; font-size: 26px; letter-spacing: 0; }}
    h2 {{ margin: 0 0 14px; font-size: 18px; letter-spacing: 0; }}
    h3 {{ margin: 0; font-size: 14px; letter-spacing: 0; }}
    .muted {{ color: var(--muted); }}
    .stats {{ display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin: 20px 0; }}
    .stat {{ border: 1px solid var(--line); background: var(--panel); border-radius: 8px; padding: 12px; }}
    .stat strong {{ display: block; font-size: 20px; color: var(--accent); }}
    .flow-list {{ display: grid; gap: 10px; }}
    button.flow-card {{
      width: 100%;
      border: 1px solid var(--line);
      background: var(--panel);
      color: var(--text);
      border-radius: 8px;
      padding: 12px;
      text-align: left;
      cursor: pointer;
    }}
    button.flow-card[aria-pressed="true"] {{ border-color: var(--accent); background: #18251f; }}
    .flow-card span {{ display: block; color: var(--muted); margin-top: 4px; overflow-wrap: anywhere; }}
    .hero {{
      border: 1px solid var(--line);
      background: var(--panel);
      border-radius: 8px;
      padding: 18px;
      margin-bottom: 16px;
    }}
    .tree {{ display: grid; gap: 8px; }}
    .node {{
      border: 1px solid var(--line);
      background: var(--panel);
      border-radius: 8px;
      padding: 10px 12px;
      margin-left: calc(var(--depth) * 20px);
    }}
    .node-header {{ display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }}
    .badge {{
      border: 1px solid var(--line);
      border-radius: 999px;
      padding: 2px 8px;
      color: var(--accent-2);
      background: var(--panel-2);
      font-size: 12px;
    }}
    .path {{ color: var(--muted); overflow-wrap: anywhere; }}
    .meta {{ color: var(--muted); font-size: 12px; margin-top: 4px; }}
    .empty {{ border: 1px dashed var(--line); border-radius: 8px; padding: 18px; color: var(--muted); }}
    @media (max-width: 860px) {{
      .app {{ grid-template-columns: 1fr; }}
      aside {{ max-height: none; border-right: 0; border-bottom: 1px solid var(--line); }}
      main {{ max-height: none; padding: 20px; }}
    }}
  </style>
</head>
<body>
  <div class="app">
    <aside>
      <h1>{title}</h1>
      <div class="muted" id="repo-root"></div>
      <div class="stats">
        <div class="stat"><strong id="stat-files">0</strong>files</div>
        <div class="stat"><strong id="stat-flows">0</strong>flows</div>
        <div class="stat"><strong id="stat-nodes">0</strong>nodes</div>
        <div class="stat"><strong id="stat-edges">0</strong>edges</div>
      </div>
      <h2>Execution Flows</h2>
      <div class="flow-list" id="flow-list"></div>
    </aside>
    <main>
      <section class="hero">
        <h2 id="flow-title">Code Atlas</h2>
        <div class="muted" id="flow-subtitle">Flow-first repository report.</div>
      </section>
      <section class="tree" id="flow-tree"></section>
    </main>
  </div>
  <script id="atlas-data" type="application/json">{data}</script>
  <script>
    const report = JSON.parse(document.getElementById("atlas-data").textContent);
    const list = document.getElementById("flow-list");
    const tree = document.getElementById("flow-tree");
    const title = document.getElementById("flow-title");
    const subtitle = document.getElementById("flow-subtitle");
    document.getElementById("repo-root").textContent = report.repo_root;
    document.getElementById("stat-files").textContent = report.summary.files.toLocaleString();
    document.getElementById("stat-flows").textContent = report.summary.selected_flows.toLocaleString();
    document.getElementById("stat-nodes").textContent = report.summary.nodes.toLocaleString();
    document.getElementById("stat-edges").textContent = report.summary.edges.toLocaleString();

    function escapeText(value) {{
      const span = document.createElement("span");
      span.textContent = value ?? "";
      return span.innerHTML;
    }}

    function renderList(activeIndex) {{
      list.innerHTML = "";
      report.flows.forEach((flow, index) => {{
        const button = document.createElement("button");
        button.type = "button";
        button.className = "flow-card";
        button.setAttribute("aria-pressed", String(index === activeIndex));
        button.innerHTML = `<h3>${{escapeText(flow.name)}}</h3><span>${{escapeText(flow.entry.qualified_name)}}</span>`;
        button.addEventListener("click", () => renderFlow(index));
        list.appendChild(button);
      }});
    }}

    function renderFlow(index) {{
      const flow = report.flows[index];
      renderList(index);
      if (!flow) {{
        title.textContent = "No flows found";
        subtitle.textContent = "Try a broader --entry selector or re-index the repository.";
        tree.innerHTML = '<div class="empty">No atlas nodes were selected.</div>';
        return;
      }}
      title.textContent = flow.name;
      subtitle.textContent = `${{flow.nodes.length.toLocaleString()}} selected nodes · ${{flow.edges.length.toLocaleString()}} local relationships`;
      tree.innerHTML = "";
      flow.nodes.forEach((node) => {{
        const div = document.createElement("article");
        div.className = "node";
        div.style.setProperty("--depth", node.depth);
        const line = node.span ? `:${{node.span.start_line}}` : "";
        div.innerHTML = `
          <div class="node-header">
            <span class="badge">${{escapeText(node.role)}}</span>
            <strong>${{escapeText(node.name)}}</strong>
            <span class="muted">${{escapeText(node.kind)}}</span>
          </div>
          <div class="path">${{escapeText(node.qualified_name)}}</div>
          <div class="meta">${{escapeText(node.path)}}${{line}} · in ${{node.in_degree}} · out ${{node.out_degree}}</div>
        `;
        tree.appendChild(div);
      }});
    }}

    renderFlow(0);
  </script>
</body>
</html>"#
    ))
}

fn select_entrypoints<'a>(
    graph: &'a CodeGraph,
    entry: Option<&str>,
    max_flows: usize,
) -> Vec<&'a Node> {
    let entry_lower = entry.map(|value| value.to_ascii_lowercase());
    let mut candidates = graph
        .nodes
        .iter()
        .filter(|node| is_entry_candidate(node))
        .filter(|node| {
            entry_lower.as_ref().map_or(true, |entry| {
                let haystack = format!("{} {} {}", node.name, node.qualified_name, node.path)
                    .to_ascii_lowercase();
                haystack.contains(entry)
            })
        })
        .map(|node| (entry_score(node), node))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.path.cmp(&right.1.path))
            .then_with(|| left.1.qualified_name.cmp(&right.1.qualified_name))
    });
    candidates
        .into_iter()
        .map(|(_, node)| node)
        .take(max_flows)
        .collect()
}

fn is_entry_candidate(node: &Node) -> bool {
    matches!(
        node.kind,
        NodeKind::Function
            | NodeKind::Method
            | NodeKind::Class
            | NodeKind::Struct
            | NodeKind::Trait
            | NodeKind::Interface
    )
}

fn build_flow(
    graph: &CodeGraph,
    entry: &Node,
    max_depth: usize,
    max_nodes_per_flow: usize,
) -> AtlasFlow {
    let by_id = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut selected = HashSet::from([entry.id.clone()]);
    let mut depths = HashMap::from([(entry.id.clone(), 0usize)]);
    let mut queue = VecDeque::from([(entry.id.clone(), 0usize)]);
    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth || selected.len() >= max_nodes_per_flow {
            continue;
        }
        let mut outgoing = graph
            .edges
            .iter()
            .filter(|edge| edge.from == current && follows_flow_edge(edge))
            .filter_map(|edge| by_id.get(edge.to.as_str()).map(|node| (edge, *node)))
            .filter(|(_, node)| is_flow_node(node))
            .collect::<Vec<_>>();
        outgoing.sort_by_key(|(_, node)| {
            std::cmp::Reverse(entry_score(node) + node.metrics.in_degree + node.metrics.out_degree)
        });
        for (_, node) in outgoing.into_iter().take(8) {
            if selected.insert(node.id.clone()) {
                depths.insert(node.id.clone(), depth + 1);
                queue.push_back((node.id.clone(), depth + 1));
            }
            if selected.len() >= max_nodes_per_flow {
                break;
            }
        }
    }
    let mut nodes = selected
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .map(|node| atlas_node(node, *depths.get(&node.id).unwrap_or(&0)))
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.depth
            .cmp(&right.depth)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
    });
    let edges = graph
        .edges
        .iter()
        .filter(|edge| selected.contains(&edge.from) && selected.contains(&edge.to))
        .filter(|edge| follows_flow_edge(edge))
        .map(|edge| AtlasEdge {
            from: edge.from.clone(),
            to: edge.to.clone(),
            kind: edge_kind_label(&edge.kind).to_string(),
        })
        .collect::<Vec<_>>();
    AtlasFlow {
        id: entry.id.clone(),
        name: flow_name(entry),
        entry: atlas_node(entry, 0),
        nodes,
        edges,
    }
}

fn is_flow_node(node: &Node) -> bool {
    !matches!(
        node.kind,
        NodeKind::Repository | NodeKind::Import | NodeKind::Variable | NodeKind::Macro
    )
}

fn follows_flow_edge(edge: &Edge) -> bool {
    matches!(
        edge.kind,
        EdgeKind::Calls
            | EdgeKind::References
            | EdgeKind::Contains
            | EdgeKind::Defines
            | EdgeKind::Extends
    )
}

fn entry_score(node: &Node) -> usize {
    let name = node.name.to_ascii_lowercase();
    let qualified = node.qualified_name.to_ascii_lowercase();
    let path = node.path.to_ascii_lowercase();
    let mut score = node.metrics.in_degree + node.metrics.out_degree;
    if name == "main" {
        score += 10_000;
    }
    for keyword in [
        "run", "start", "serve", "init", "create", "open", "handler", "executor", "reader",
        "writer", "builder", "service", "app",
    ] {
        if name.contains(keyword) || qualified.contains(keyword) || path.contains(keyword) {
            score += 1_000;
        }
    }
    score
}

fn atlas_node(node: &Node, depth: usize) -> AtlasNode {
    AtlasNode {
        id: node.id.clone(),
        name: node.name.clone(),
        qualified_name: node.qualified_name.clone(),
        kind: node_kind_label(&node.kind).to_string(),
        role: classify_role(node).to_string(),
        path: node.path.clone(),
        span: node.span,
        depth,
        in_degree: node.metrics.in_degree,
        out_degree: node.metrics.out_degree,
    }
}

fn classify_role(node: &Node) -> &'static str {
    let haystack =
        format!("{} {} {}", node.name, node.qualified_name, node.path).to_ascii_lowercase();
    if node.name == "main" || haystack.contains("bootstrap") {
        "entrypoint"
    } else if haystack.contains("app")
        || haystack.contains("service")
        || haystack.contains("server")
    {
        "app"
    } else if haystack.contains("handler") || haystack.contains("route") || haystack.contains("rpc")
    {
        "handler"
    } else if haystack.contains("executor")
        || haystack.contains("processor")
        || haystack.contains("workitem")
    {
        "domain flow"
    } else if haystack.contains("reader")
        || haystack.contains("writer")
        || haystack.contains("store")
        || haystack.contains("index")
        || haystack.contains("segment")
    {
        "boundary"
    } else if matches!(node.kind, NodeKind::File | NodeKind::Module) {
        "container"
    } else {
        "method"
    }
}

fn flow_name(node: &Node) -> String {
    format!("{} Flow", node.name)
}

fn report_title(graph: &CodeGraph) -> String {
    let name = std::path::Path::new(&graph.repo_root)
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "Repository".to_string());
    format!("{name} Code Atlas")
}

fn node_kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Repository => "repository",
        NodeKind::File => "file",
        NodeKind::Module => "module",
        NodeKind::Function => "function",
        NodeKind::Method => "method",
        NodeKind::Class => "class",
        NodeKind::Struct => "struct",
        NodeKind::Enum => "enum",
        NodeKind::Union => "union",
        NodeKind::Typedef => "typedef",
        NodeKind::Variable => "variable",
        NodeKind::Macro => "macro",
        NodeKind::Trait => "trait",
        NodeKind::Interface => "interface",
        NodeKind::Import => "import",
        NodeKind::Unknown => "unknown",
    }
}

fn edge_kind_label(kind: &EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Contains => "contains",
        EdgeKind::Defines => "defines",
        EdgeKind::Imports => "imports",
        EdgeKind::Calls => "calls",
        EdgeKind::References => "references",
        EdgeKind::Extends => "extends",
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::model::Language;
    use gitnova_core::model::{CodeGraph, Edge, NodeMetrics};

    fn node(id: &str, name: &str, kind: NodeKind, path: &str) -> Node {
        Node {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            qualified_name: format!("{path}::{name}"),
            path: path.to_string(),
            span: None,
            language: Some(Language::Rust),
            text: String::new(),
            tags: Vec::new(),
            metrics: NodeMetrics {
                in_degree: 1,
                out_degree: 1,
                ..NodeMetrics::default()
            },
        }
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            confidence_basis_points: 10_000,
        }
    }

    fn sample_graph() -> CodeGraph {
        CodeGraph {
            schema_version: 1,
            repo_root: "/tmp/havenask".to_string(),
            indexed_at_unix: 0,
            nodes: vec![
                node("main", "main", NodeKind::Function, "src/main.rs"),
                node("app", "create_app", NodeKind::Function, "src/app.rs"),
                node(
                    "handler",
                    "handle_request",
                    NodeKind::Function,
                    "src/handler.rs",
                ),
                node("reader", "read_index", NodeKind::Method, "src/index.rs"),
            ],
            edges: vec![
                edge("main", "app", EdgeKind::Calls),
                edge("app", "handler", EdgeKind::Calls),
                edge("handler", "reader", EdgeKind::Calls),
            ],
        }
    }

    #[test]
    fn builds_entry_first_flow_tree() {
        let report = build_atlas(
            &sample_graph(),
            AtlasOptions {
                entry: Some("main".to_string()),
                max_flows: 1,
                max_depth: 3,
                max_nodes_per_flow: 10,
            },
        );

        assert_eq!(report.title, "havenask Code Atlas");
        assert_eq!(report.flows.len(), 1);
        let flow = &report.flows[0];
        assert_eq!(flow.entry.name, "main");
        assert!(flow.nodes.iter().any(|node| node.name == "create_app"));
        assert!(flow.nodes.iter().any(|node| node.role == "handler"));
        assert!(flow.nodes.iter().any(|node| node.role == "boundary"));
    }

    #[test]
    fn renders_self_contained_html_report() {
        let report = build_atlas(
            &sample_graph(),
            AtlasOptions {
                entry: Some("main".to_string()),
                max_flows: 1,
                max_depth: 3,
                max_nodes_per_flow: 10,
            },
        );

        let html = render_html(&report).unwrap();
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("havenask Code Atlas"));
        assert!(html.contains("type=\"application/json\""));
        assert!(html.contains("atlas-data"));
        assert!(html.contains("main Flow"));
    }
}
