use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::Serialize;

use crate::types::{ContentFormat, Evidence, PageKind, PatchMode, WikiNode, WikiPage, WikiSchema};

pub struct WikiStore {
    root: PathBuf,
}

impl WikiStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("pages"))?;
        fs::create_dir_all(root.join("blocks"))?;
        fs::create_dir_all(root.join("blocks_private"))?;
        let store = Self { root };
        if !store.schema_path().exists() {
            store.write_schema(&WikiSchema::default())?;
        }
        Ok(store)
    }

    pub fn read_schema(&self) -> Result<WikiSchema> {
        let raw = fs::read_to_string(self.schema_path()).context("read wiki schema")?;
        serde_json::from_str(&raw).context("parse wiki schema")
    }

    pub fn upsert_page(&self, page: WikiPage) -> Result<()> {
        let id = normalize_id(&page.id)?;
        let mut schema = self.read_schema()?;
        let parent = page
            .parent
            .clone()
            .map(normalize_id_result)
            .transpose()?
            .or_else(|| parent_id(&id));
        let content_ref = block_ref(&id, page.content_format);
        let page_ref = page_ref(&id, page.kind);
        let now = now();

        match schema.node_mut(&id) {
            Some(node) => {
                node.title = page.title;
                node.kind = page.kind;
                node.summary = page.summary;
                node.parent = parent;
                node.content_format = page.content_format;
                node.content_ref = content_ref;
                node.page_ref = page_ref;
                node.updated_at = now;
            }
            None => schema.nodes.push(WikiNode {
                id: id.clone(),
                title: page.title,
                kind: page.kind,
                summary: page.summary,
                parent,
                content_ref,
                page_ref,
                content_format: page.content_format,
                evidence: Vec::new(),
                updated_at: now,
            }),
        }

        self.ensure_block_exists(&id, page.content_format)?;
        self.write_schema(&schema)?;
        self.append_journal("upsert_page", &id)?;
        self.render_all(&schema)
    }

    pub fn patch_page(
        &self,
        id: &str,
        format: ContentFormat,
        content: &str,
        mode: PatchMode,
    ) -> Result<()> {
        let id = normalize_id(id)?;
        let mut schema = self.read_schema()?;
        let node = schema
            .node_mut(&id)
            .ok_or_else(|| anyhow!("wiki page not found: {id}"))?;
        node.content_format = format;
        node.content_ref = block_ref(&id, format);
        node.updated_at = now();

        let block_path = self.root.join(&node.content_ref);
        if let Some(parent) = block_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let next = match mode {
            PatchMode::Replace => content.to_owned(),
            PatchMode::Append => {
                let mut existing = fs::read_to_string(&block_path).unwrap_or_default();
                if !existing.is_empty() && !existing.ends_with('\n') {
                    existing.push('\n');
                }
                existing.push_str(content);
                existing
            }
        };
        fs::write(&block_path, next)?;

        self.write_schema(&schema)?;
        self.append_journal("patch_page", &id)?;
        self.render_all(&schema)
    }

    pub fn append_evidence(&self, id: &str, evidence: Evidence) -> Result<()> {
        let id = normalize_id(id)?;
        let mut schema = self.read_schema()?;
        let node = schema
            .node_mut(&id)
            .ok_or_else(|| anyhow!("wiki page not found: {id}"))?;
        node.evidence.push(evidence);
        node.updated_at = now();
        self.write_schema(&schema)?;
        self.append_journal("append_evidence", &id)?;
        self.render_all(&schema)
    }

    pub fn patch_private_note(&self, id: &str, content: &str, mode: PatchMode) -> Result<()> {
        let id = normalize_id(id)?;
        let schema = self.read_schema()?;
        if schema.node(&id).is_none() {
            return Err(anyhow!("wiki page not found: {id}"));
        }

        let note_path = self.root.join(private_note_ref(&id));
        if let Some(parent) = note_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let next = match mode {
            PatchMode::Replace => content.to_owned(),
            PatchMode::Append => {
                let mut existing = fs::read_to_string(&note_path).unwrap_or_default();
                if !existing.is_empty() && !existing.ends_with('\n') {
                    existing.push('\n');
                }
                existing.push_str(content);
                existing
            }
        };
        fs::write(note_path, next)?;
        self.append_journal("patch_private_note", &id)
    }

    pub fn read_private_note(&self, id: &str) -> Result<String> {
        let id = normalize_id(id)?;
        Ok(fs::read_to_string(self.root.join(private_note_ref(&id))).unwrap_or_default())
    }

    pub fn read_page(&self, id: &str) -> Result<WikiPage> {
        let id = normalize_id(id)?;
        let schema = self.read_schema()?;
        let node = schema
            .node(&id)
            .ok_or_else(|| anyhow!("wiki page not found: {id}"))?;
        let content = fs::read_to_string(self.root.join(&node.content_ref)).unwrap_or_default();
        Ok(WikiPage {
            id: node.id.clone(),
            title: node.title.clone(),
            kind: node.kind,
            summary: node.summary.clone(),
            parent: node.parent.clone(),
            content_format: node.content_format,
            content,
            evidence: node.evidence.clone(),
        })
    }

    fn write_schema(&self, schema: &WikiSchema) -> Result<()> {
        let raw = serde_json::to_string_pretty(schema)?;
        fs::write(self.schema_path(), raw)?;
        Ok(())
    }

    fn render_all(&self, schema: &WikiSchema) -> Result<()> {
        for node in &schema.nodes {
            self.render_node(schema, node)?;
        }
        Ok(())
    }

    fn render_node(&self, schema: &WikiSchema, node: &WikiNode) -> Result<()> {
        let output_path = self.root.join(&node.page_ref);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = fs::read_to_string(self.root.join(&node.content_ref)).unwrap_or_default();
        let html = render_page(schema, node, &content);
        fs::write(output_path, html)?;
        Ok(())
    }

    fn ensure_block_exists(&self, id: &str, format: ContentFormat) -> Result<()> {
        let path = self.root.join(block_ref(id, format));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::write(path, "")?;
        }
        Ok(())
    }

    fn append_journal(&self, event: &str, id: &str) -> Result<()> {
        #[derive(Serialize)]
        struct JournalEntry<'a> {
            event: &'a str,
            id: &'a str,
            at: String,
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("journal.jsonl"))?;
        let entry = JournalEntry {
            event,
            id,
            at: now(),
        };
        writeln!(file, "{}", serde_json::to_string(&entry)?)?;
        Ok(())
    }

    fn schema_path(&self) -> PathBuf {
        self.root.join("schema.json")
    }
}

fn normalize_id(id: &str) -> Result<String> {
    normalize_id_result(id.to_owned())
}

fn normalize_id_result(id: String) -> Result<String> {
    let id = id.trim().trim_matches('/').to_owned();
    if id.is_empty() {
        return Err(anyhow!("wiki page id cannot be empty"));
    }
    if id
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(anyhow!(
            "wiki page id contains an unsafe path segment: {id}"
        ));
    }
    Ok(id)
}

fn parent_id(id: &str) -> Option<String> {
    id.rsplit_once('/').map(|(parent, _)| parent.to_owned())
}

fn block_ref(id: &str, format: ContentFormat) -> String {
    let extension = match format {
        ContentFormat::Markdown => "md",
        ContentFormat::Html => "html",
    };
    format!("blocks/{id}.{extension}")
}

fn private_note_ref(id: &str) -> String {
    format!("blocks_private/{id}.md")
}

fn page_ref(id: &str, kind: PageKind) -> String {
    match kind {
        PageKind::Index => format!("pages/{id}/index.html"),
        PageKind::Article => format!("pages/{id}.html"),
    }
}

fn children_of<'a>(schema: &'a WikiSchema, parent: &str) -> Vec<&'a WikiNode> {
    let mut children = schema
        .nodes
        .iter()
        .filter(|node| node.parent.as_deref() == Some(parent))
        .collect::<Vec<_>>();
    children.sort_by(|left, right| left.title.cmp(&right.title));
    children
}

fn render_page(schema: &WikiSchema, node: &WikiNode, content: &str) -> String {
    let children = children_of(schema, &node.id);
    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str(&format!("<title>{}</title>", escape_html(&node.title)));
    html.push_str(
        r#"<style>
:root { color-scheme: light; --bg: #f7f8fb; --ink: #172033; --muted: #667085; --line: #d7dce5; --panel: #ffffff; --accent: #0f766e; --soft: #e7f3f1; --active: #dff0ed; }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink); font: 15px/1.58 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
.app-shell { display: grid; grid-template-columns: 280px minmax(0, 1fr); min-height: 100vh; }
.wiki-sidebar { border-right: 1px solid var(--line); background: #fff; padding: 26px 18px; position: sticky; top: 0; height: 100vh; overflow: auto; }
.sidebar-title { color: var(--muted); font-size: 12px; font-weight: 800; letter-spacing: .08em; text-transform: uppercase; margin: 0 0 14px; }
.wiki-tree, .wiki-tree ul { list-style: none; margin: 0; padding: 0; }
.wiki-tree ul { margin-left: 12px; padding-left: 12px; border-left: 1px solid var(--line); }
.wiki-tree li { margin: 4px 0; }
.wiki-tree a, .wiki-tree span { display: block; border-radius: 6px; padding: 7px 9px; color: var(--ink); font-weight: 650; text-decoration: none; }
.wiki-tree a:hover { background: var(--soft); color: var(--accent); }
.wiki-tree [aria-current="page"] { background: var(--active); color: var(--accent); }
main { max-width: 1040px; width: 100%; margin: 0 auto; padding: 40px 28px 72px; }
.page-header { border-bottom: 1px solid var(--line); padding-bottom: 22px; margin-bottom: 26px; }
.page-kind { color: var(--accent); font-size: 12px; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
.breadcrumb { color: var(--muted); font-size: 13px; margin-bottom: 12px; }
.breadcrumb ol { display: flex; flex-wrap: wrap; gap: 6px; list-style: none; margin: 0; padding: 0; }
.breadcrumb li:not(:last-child)::after { content: "/"; color: var(--muted); margin-left: 6px; }
.breadcrumb a { font-weight: 700; }
h1 { font-size: 34px; line-height: 1.15; margin: 8px 0 10px; }
h2 { font-size: 20px; margin: 28px 0 12px; }
p { margin: 8px 0; }
.summary { max-width: 780px; color: var(--muted); font-size: 16px; }
.child-pages ul { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 12px; padding: 0; margin: 0; list-style: none; align-items: stretch; }
.child-pages li { min-height: 132px; }
.page-card { display: block; height: 100%; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 14px 16px; color: var(--ink); font-weight: 400; text-decoration: none; box-shadow: 0 0 0 rgba(0,0,0,0); transition: background-color .12s ease, box-shadow .12s ease; }
.page-card strong { display: block; color: var(--accent); font-weight: 800; }
.page-card p { color: var(--ink); }
.page-card:hover, .page-card:focus-visible { background: #fbfdfc; box-shadow: 0 3px 14px rgba(23, 32, 51, .08); text-decoration: none; }
a { color: var(--accent); font-weight: 700; text-decoration: none; }
a:hover { text-decoration: none; }
article { margin-top: 26px; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 18px 20px; }
article h2:first-child { margin-top: 0; }
.diagram-block { margin: 18px 0; overflow-x: auto; border: 1px solid var(--line); border-radius: 8px; background: #fcfefd; padding: 14px; }
.diagram-block svg { display: block; max-width: 100%; height: auto; }
@media (max-width: 640px) { .app-shell { display: block; } .wiki-sidebar { position: static; height: auto; border-right: 0; border-bottom: 1px solid var(--line); } }
</style>"#,
    );
    html.push_str("</head><body>");
    html.push_str("<div class=\"app-shell\">");
    html.push_str("<aside class=\"wiki-sidebar\"><p class=\"sidebar-title\">Wiki Tree</p>");
    html.push_str("<nav aria-label=\"Wiki tree\">");
    html.push_str(&render_wiki_tree(schema, node));
    html.push_str("</nav></aside>");
    html.push_str(&format!(
        "<main data-gitnova-page-id=\"{}\" data-gitnova-page-kind=\"{:?}\">",
        escape_html(&node.id),
        node.kind
    ));
    html.push_str("<header class=\"page-header\">");
    html.push_str(&render_breadcrumb(schema, node));
    html.push_str(&format!("<div class=\"page-kind\">{:?}</div>", node.kind));
    html.push_str(&format!("<h1>{}</h1>", escape_html(&node.title)));
    if let Some(summary) = &node.summary {
        html.push_str(&format!(
            "<p class=\"summary\">{}</p>",
            escape_html(summary)
        ));
    }
    html.push_str("</header>");
    if !children.is_empty() {
        html.push_str(
            "<nav class=\"child-pages\" aria-label=\"Child pages\"><h2>Child Pages</h2><ul>",
        );
        for child in children {
            html.push_str(&format!(
                "<li><a class=\"page-card\" href=\"{}\"><strong>{}</strong>{}</a></li>",
                escape_html(&relative_href(&node.page_ref, &child.page_ref)),
                escape_html(&child.title),
                child
                    .summary
                    .as_ref()
                    .map(|summary| format!("<p>{}</p>", escape_html(summary)))
                    .unwrap_or_default()
            ));
        }
        html.push_str("</ul></nav>");
    }
    html.push_str("<article>");
    match node.content_format {
        ContentFormat::Markdown => html.push_str(&render_markdown(content)),
        ContentFormat::Html => html.push_str(content),
    }
    html.push_str("</article>");
    if !node.evidence.is_empty() {
        html.push_str(&format!(
            "<script type=\"application/json\" data-gitnova-evidence>{}</script>",
            escape_script_json(&serde_json::to_string(&node.evidence).unwrap_or_default())
        ));
    }
    html.push_str("</main></div></body></html>");
    html
}

fn render_breadcrumb(schema: &WikiSchema, node: &WikiNode) -> String {
    let mut crumbs = ancestors(schema, node);
    crumbs.push(node);
    let mut html = String::from("<nav class=\"breadcrumb\" aria-label=\"Breadcrumb\"><ol>");
    for crumb in crumbs {
        if crumb.id == node.id {
            html.push_str(&format!(
                "<li><span>{}</span></li>",
                escape_html(&crumb.title)
            ));
        } else {
            html.push_str(&format!(
                "<li><a href=\"{}\">{}</a></li>",
                escape_html(&relative_href(&node.page_ref, &crumb.page_ref)),
                escape_html(&crumb.title)
            ));
        }
    }
    html.push_str("</ol></nav>");
    html
}

fn render_wiki_tree(schema: &WikiSchema, current: &WikiNode) -> String {
    let mut html = String::from("<ul class=\"wiki-tree\">");
    let mut roots = schema
        .nodes
        .iter()
        .filter(|node| node.parent.is_none())
        .collect::<Vec<_>>();
    roots.sort_by(|left, right| left.title.cmp(&right.title));
    for root in roots {
        render_tree_node(schema, current, root, &mut html);
    }
    html.push_str("</ul>");
    html
}

fn render_tree_node(schema: &WikiSchema, current: &WikiNode, node: &WikiNode, html: &mut String) {
    html.push_str("<li>");
    if node.id == current.id {
        html.push_str(&format!(
            "<span aria-current=\"page\">{}</span>",
            escape_html(&node.title)
        ));
    } else {
        html.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            escape_html(&relative_href(&current.page_ref, &node.page_ref)),
            escape_html(&node.title)
        ));
    }

    let children = children_of(schema, &node.id);
    if !children.is_empty() {
        html.push_str("<ul>");
        for child in children {
            render_tree_node(schema, current, child, html);
        }
        html.push_str("</ul>");
    }
    html.push_str("</li>");
}

fn ancestors<'a>(schema: &'a WikiSchema, node: &'a WikiNode) -> Vec<&'a WikiNode> {
    let mut ancestors = Vec::new();
    let mut parent = node.parent.as_deref();
    while let Some(parent_id) = parent {
        let Some(parent_node) = schema.node(parent_id) else {
            break;
        };
        ancestors.push(parent_node);
        parent = parent_node.parent.as_deref();
    }
    ancestors.reverse();
    ancestors
}

fn relative_href(from_page_ref: &str, to_page_ref: &str) -> String {
    let from_dir = from_page_ref
        .split('/')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let to_parts = to_page_ref.split('/').collect::<Vec<_>>();
    let mut shared = 0;
    while shared < from_dir.len() && shared < to_parts.len() && from_dir[shared] == to_parts[shared]
    {
        shared += 1;
    }

    let mut parts = Vec::new();
    for _ in shared..from_dir.len() {
        parts.push("..");
    }
    parts.extend(to_parts[shared..].iter().copied());
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}

fn render_markdown(content: &str) -> String {
    let mut html = String::new();
    let mut svg_block = None::<String>;
    for line in content.lines() {
        if line.trim() == "```svg" {
            svg_block = Some(String::new());
        } else if line.trim() == "```" && svg_block.is_some() {
            let svg = svg_block.take().unwrap_or_default();
            html.push_str("<div class=\"diagram-block\">");
            html.push_str(&svg);
            html.push_str("</div>");
        } else if let Some(svg) = svg_block.as_mut() {
            svg.push_str(line);
            svg.push('\n');
        } else if let Some(text) = line.strip_prefix("## ") {
            html.push_str(&format!("<h2>{}</h2>", escape_html(text)));
        } else if let Some(text) = line.strip_prefix("# ") {
            html.push_str(&format!("<h1>{}</h1>", escape_html(text)));
        } else if line.trim().is_empty() {
            continue;
        } else {
            html.push_str(&format!("<p>{}</p>", escape_html(line)));
        }
    }
    if let Some(svg) = svg_block {
        html.push_str("<pre><code>");
        html.push_str(&escape_html(&svg));
        html.push_str("</code></pre>");
    }
    html
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_script_json(value: &str) -> String {
    value.replace("</", "<\\/")
}

fn now() -> String {
    Utc::now().to_rfc3339()
}
