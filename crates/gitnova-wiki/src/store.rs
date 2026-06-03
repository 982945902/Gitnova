use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use pulldown_cmark::{html, CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
use serde::Serialize;

use crate::types::{
    ContentFormat, DeepTask, Evidence, OutlinePage, PageKind, PatchMode, TaskStatus, WikiNode,
    WikiOutline, WikiPage, WikiSchema,
};

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

    pub fn apply_outline(&self, mut outline: WikiOutline) -> Result<()> {
        outline.version = 1;
        for page in &outline.pages {
            let mut wiki_page = WikiPage::new(&page.id, &page.title, page.kind);
            wiki_page.summary = page.summary.clone();
            wiki_page.parent = page.parent.clone();
            self.upsert_page(wiki_page)?;
            if let Some(content) = &page.content {
                self.patch_page(
                    &page.id,
                    ContentFormat::Markdown,
                    content,
                    PatchMode::Replace,
                )?;
            }
            let private_note = outline_private_note(page.purpose.as_deref(), &page.deep_tasks);
            if !private_note.is_empty() {
                self.patch_private_note(&page.id, &private_note, PatchMode::Replace)?;
            }
        }
        self.write_outline(&outline)?;
        self.append_journal("apply_outline", &outline.root)
    }

    pub fn read_outline(&self) -> Result<WikiOutline> {
        let path = self.outline_path();
        if !path.exists() {
            return Ok(WikiOutline::default());
        }
        let raw = fs::read_to_string(path).context("read wiki outline")?;
        serde_json::from_str(&raw).context("parse wiki outline")
    }

    pub fn write_outline(&self, outline: &WikiOutline) -> Result<()> {
        let raw = serde_json::to_string_pretty(outline)?;
        fs::write(self.outline_path(), raw)?;
        Ok(())
    }

    pub fn pending_tasks(
        &self,
        root_page_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, DeepTask)>> {
        self.expandable_tasks(root_page_id, limit, false)
    }

    pub fn retryable_tasks(
        &self,
        root_page_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, DeepTask)>> {
        self.expandable_tasks(root_page_id, limit, true)
    }

    fn expandable_tasks(
        &self,
        root_page_id: &str,
        limit: usize,
        retry_failed: bool,
    ) -> Result<Vec<(String, DeepTask)>> {
        let outline = self.read_outline()?;
        let mut tasks = Vec::new();
        for page in outline.pages {
            if !is_page_under_root(&page.id, root_page_id) {
                continue;
            }
            for task in page.deep_tasks {
                if task.status == TaskStatus::Pending
                    || (retry_failed && task.status == TaskStatus::Failed)
                {
                    tasks.push((page.id.clone(), task));
                    if tasks.len() >= limit {
                        return Ok(tasks);
                    }
                }
            }
        }
        Ok(tasks)
    }

    pub fn update_task_status(
        &self,
        page_id: &str,
        task_id: &str,
        status: TaskStatus,
        confidence: Option<f32>,
        error: Option<String>,
    ) -> Result<()> {
        let mut outline = self.read_outline()?;
        let page = outline
            .pages
            .iter_mut()
            .find(|page| page.id == page_id)
            .ok_or_else(|| anyhow!("outline page not found: {page_id}"))?;
        let task = page
            .deep_tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or_else(|| anyhow!("outline task not found: {task_id}"))?;
        task.status = status;
        task.confidence = confidence;
        task.error = error;
        let private_note = refreshed_private_note_for_page(&self.root, page)?;
        self.write_outline(&outline)?;
        if !private_note.is_empty() {
            self.patch_private_note(page_id, &private_note, PatchMode::Replace)?;
        }
        self.append_journal("update_task_status", page_id)
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

    pub fn publish_source_file(
        &self,
        repo_path: impl AsRef<Path>,
        source_file: &str,
    ) -> Result<bool> {
        let Some(source_ref) = source_page_ref(source_file) else {
            return Ok(false);
        };
        let source_path = repo_path.as_ref().join(source_file);
        if !source_path.is_file() {
            return Ok(false);
        }
        let content = fs::read_to_string(&source_path)
            .with_context(|| format!("read source file: {source_file}"))?;
        let output_path = self.root.join(source_ref);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_path, render_source_page(source_file, &content))?;
        Ok(true)
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

    fn outline_path(&self) -> PathBuf {
        self.root.join("outline.json")
    }
}

fn outline_private_note(purpose: Option<&str>, tasks: &[DeepTask]) -> String {
    let mut note = String::new();
    if let Some(purpose) = purpose {
        if !purpose.trim().is_empty() {
            note.push_str("Purpose\n");
            note.push_str(purpose.trim());
            note.push('\n');
        }
    }
    if !tasks.is_empty() {
        if !note.is_empty() {
            note.push('\n');
        }
        note.push_str("Deep Tasks\n");
        for task in tasks {
            note.push_str("- [");
            note.push_str(match task.status {
                TaskStatus::Pending => "pending",
                TaskStatus::Running => "running",
                TaskStatus::Done => "done",
                TaskStatus::Failed => "failed",
            });
            note.push_str("] ");
            note.push_str(&task.id);
            note.push_str(": ");
            note.push_str(&task.question);
            note.push('\n');
        }
    }
    note.trim_end().to_string()
}

fn refreshed_private_note_for_page(root: &Path, page: &OutlinePage) -> Result<String> {
    let mut note = outline_private_note(page.purpose.as_deref(), &page.deep_tasks);
    let existing = fs::read_to_string(root.join(private_note_ref(&page.id))).unwrap_or_default();
    if let Some(index) = existing.find("Atlas Follow-ups\n") {
        if !note.is_empty() {
            note.push_str("\n\n");
        }
        note.push_str(existing[index..].trim_end());
    }
    Ok(note)
}

fn is_page_under_root(page_id: &str, root_page_id: &str) -> bool {
    page_id == root_page_id || page_id.starts_with(&format!("{root_page_id}/"))
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
ul, ol { padding-left: 24px; margin: 10px 0 14px; }
li { margin: 6px 0; }
code { background: #edf2f0; border: 1px solid #dce6e2; border-radius: 5px; padding: 1px 5px; font-size: .92em; }
pre { margin: 14px 0; border: 1px solid var(--line); border-radius: 8px; background: #f6f8fa; overflow: hidden; }
pre code { display: block; padding: 12px 14px; overflow-x: auto; background: transparent; border: 0; border-radius: 0; }
table { width: 100%; border-collapse: collapse; margin: 14px 0 18px; font-size: .95rem; }
th, td { border: 1px solid var(--line); padding: 8px 10px; text-align: left; vertical-align: top; }
th { background: #f1f5f4; font-weight: 800; }
tr:nth-child(even) td { background: #fbfcfc; }
strong { font-weight: 800; }
.summary { max-width: 780px; color: var(--muted); font-size: 16px; }
.child-pages ul { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 12px; padding: 0; margin: 0; list-style: none; align-items: stretch; }
.child-pages li { min-height: 132px; }
.page-card { display: block; height: 100%; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 14px 16px; color: var(--ink); font-weight: 400; text-decoration: none; box-shadow: 0 0 0 rgba(0,0,0,0); transition: background-color .12s ease, box-shadow .12s ease; }
.page-card strong { display: block; color: var(--accent); font-weight: 800; }
.page-card p { color: var(--ink); }
.page-card:hover, .page-card:focus-visible { background: #fbfdfc; box-shadow: 0 3px 14px rgba(23, 32, 51, .08); text-decoration: none; }
a { color: var(--accent); font-weight: 700; text-decoration: none; }
a:hover { text-decoration: none; }
.source-cite { display: inline-flex; align-items: center; justify-content: center; min-width: 1.45em; min-height: 1.45em; margin-left: 3px; border: 1px solid #a7d5ce; border-radius: 999px; background: #e7f3f1; color: var(--accent); font-size: .78em; font-weight: 800; vertical-align: super; }
.source-cite:hover, .source-cite:focus-visible { background: #d4ece8; text-decoration: none; }
article { margin-top: 26px; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 18px 20px; }
article h2:first-child { margin-top: 0; }
.diagram-block { margin: 18px 0; overflow-x: auto; border: 1px solid var(--line); border-radius: 8px; background: #fcfefd; padding: 14px; }
.diagram-block svg { display: block; max-width: 100%; height: auto; }
.diagram-block pre { margin: 0; white-space: pre; font-size: 0.92rem; line-height: 1.55; }
.source-spans { margin-top: 22px; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 16px 18px; }
.source-spans h2 { margin: 0 0 12px; }
.source-spans ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 9px; }
.source-spans li { margin: 0; }
.source-span { display: grid; gap: 5px; border: 1px solid #e3e8ef; border-radius: 8px; padding: 10px 12px; background: #fbfcfc; }
.source-span code { width: fit-content; max-width: 100%; overflow-wrap: anywhere; }
.source-span a { width: fit-content; max-width: 100%; }
.source-note { margin: 0; color: var(--muted); font-size: 0.94rem; }
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
    let has_inline_sources = matches!(node.content_format, ContentFormat::Markdown)
        && content_has_source_markers(content);
    match node.content_format {
        ContentFormat::Markdown => {
            html.push_str(&render_markdown_with_sources(
                content,
                &node.page_ref,
                &node.evidence,
            ));
        }
        ContentFormat::Html => html.push_str(content),
    }
    html.push_str("</article>");
    if !has_inline_sources {
        html.push_str(&render_source_spans(&node.page_ref, &node.evidence));
    }
    if !node.evidence.is_empty() {
        html.push_str(&format!(
            "<script type=\"application/json\" data-gitnova-evidence>{}</script>",
            escape_script_json(&serde_json::to_string(&node.evidence).unwrap_or_default())
        ));
    }
    html.push_str("</main></div>");
    if page_uses_mermaid(node.content_format, content) {
        html.push_str(
            r#"<script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script><script>
if (window.mermaid) {
  window.mermaid.initialize({ startOnLoad: true, securityLevel: "strict" });
}
</script>"#,
        );
    }
    html.push_str("</body></html>");
    html
}

fn render_source_spans(from_page_ref: &str, evidence: &[Evidence]) -> String {
    if evidence.is_empty() {
        return String::new();
    }

    let mut html = String::from(
        "<aside class=\"source-spans\" aria-label=\"Source spans\"><h2>Source Spans</h2><ul>",
    );
    for item in evidence {
        html.push_str("<li><div class=\"source-span\">");
        let label = source_span_label(item);
        if let Some(href) = source_span_href(from_page_ref, item) {
            html.push_str("<a href=\"");
            html.push_str(&escape_html(&href));
            html.push_str("\"><code>");
            html.push_str(&escape_html(&label));
            html.push_str("</code></a>");
        } else {
            html.push_str("<code>");
            html.push_str(&escape_html(&label));
            html.push_str("</code>");
        }
        if let Some(note) = &item.note {
            if !note.trim().is_empty() {
                html.push_str("<p class=\"source-note\">");
                html.push_str(&escape_html(note.trim()));
                html.push_str("</p>");
            }
        }
        html.push_str("</div></li>");
    }
    html.push_str("</ul></aside>");
    html
}

fn source_span_href(from_page_ref: &str, evidence: &Evidence) -> Option<String> {
    let mut href = relative_href(from_page_ref, &source_page_ref(&evidence.file)?);
    if let Some(start_line) = evidence.start_line {
        href.push_str("#L");
        href.push_str(&start_line.to_string());
        if let Some(end_line) = evidence.end_line {
            if end_line != start_line {
                href.push_str("-L");
                href.push_str(&end_line.to_string());
            }
        }
    }
    Some(href)
}

fn source_span_label(evidence: &Evidence) -> String {
    match (evidence.start_line, evidence.end_line) {
        (Some(start), Some(end)) if start == end => format!("{}:{start}", evidence.file),
        (Some(start), Some(end)) => format!("{}:{start}-{end}", evidence.file),
        (Some(start), None) => format!("{}:{start}", evidence.file),
        (None, Some(end)) => format!("{}:1-{end}", evidence.file),
        (None, None) => evidence.file.clone(),
    }
}

fn source_page_ref(source_file: &str) -> Option<String> {
    let path = Path::new(source_file);
    if path.is_absolute() {
        return None;
    }
    let mut parts = Vec::new();
    for component in path.components() {
        let std::path::Component::Normal(part) = component else {
            return None;
        };
        let part = part.to_str()?;
        if part.is_empty() {
            return None;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("pages/_sources/{}.html", parts.join("/")))
}

fn render_source_page(source_file: &str, content: &str) -> String {
    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    html.push_str(&format!("<title>{}</title>", escape_html(source_file)));
    html.push_str(
        r#"<style>
:root { color-scheme: light; --bg: #f7f8fb; --ink: #172033; --muted: #667085; --line: #d7dce5; --panel: #ffffff; --accent: #0f766e; --target: #fff4c2; }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink); font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace; }
main { max-width: 1180px; margin: 0 auto; padding: 28px 18px 64px; }
.source-header { margin-bottom: 16px; border-bottom: 1px solid var(--line); padding-bottom: 14px; }
.source-header a { color: var(--accent); font: 700 13px/1.4 ui-sans-serif, system-ui, sans-serif; text-decoration: none; }
h1 { margin: 8px 0 0; font-size: 20px; line-height: 1.25; overflow-wrap: anywhere; }
pre { margin: 0; border: 1px solid var(--line); border-radius: 8px; background: var(--panel); overflow: auto; }
code { display: table; width: 100%; border-collapse: collapse; }
.source-line { display: table-row; }
.line-no, .line-code { display: table-cell; white-space: pre; }
.line-no { width: 1%; padding: 0 12px; color: var(--muted); text-align: right; border-right: 1px solid #edf0f4; user-select: none; }
.line-code { padding: 0 14px; }
.source-line:target .line-no, .source-line:target .line-code { background: var(--target); }
.source-line.selected .line-no, .source-line.selected .line-code { background: var(--target); }
</style>"#,
    );
    html.push_str("</head><body><main>");
    html.push_str("<header class=\"source-header\"><a href=\"javascript:history.back()\">Back</a>");
    html.push_str(&format!("<h1>{}</h1></header>", escape_html(source_file)));
    html.push_str("<pre><code>");
    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        html.push_str(&format!(
            "<span class=\"source-line\" id=\"L{line_number}\"><span class=\"line-no\">{line_number}</span><span class=\"line-code\">{}</span></span>\n",
            escape_html(line)
        ));
    }
    html.push_str(
        r#"</code></pre></main><script>
function highlightHashSpan() {
  document.querySelectorAll(".source-line.selected").forEach((line) => line.classList.remove("selected"));
  const match = location.hash.match(/^#L(\d+)(?:-L?(\d+))?$/);
  if (!match) return;
  const start = Number(match[1]);
  const end = Number(match[2] || match[1]);
  const min = Math.min(start, end);
  const max = Math.max(start, end);
  for (let line = min; line <= max; line += 1) {
    document.getElementById(`L${line}`)?.classList.add("selected");
  }
  document.getElementById(`L${min}`)?.scrollIntoView({ block: "center" });
}
window.addEventListener("hashchange", highlightHashSpan);
highlightHashSpan();
</script></body></html>"#,
    );
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
    parts.extend(std::iter::repeat_n("..", from_dir.len() - shared));
    parts.extend(to_parts[shared..].iter().copied());
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}

fn render_markdown(content: &str) -> String {
    let mut html = String::new();
    let parser = Parser::new_ext(
        content,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS,
    );
    let mut events = Vec::new();
    let mut diagram_block = None::<(String, String)>;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language)))
                if matches!(language.as_ref(), "svg" | "mermaid") =>
            {
                diagram_block = Some((language.to_string(), String::new()));
            }
            Event::End(TagEnd::CodeBlock) if diagram_block.is_some() => {
                let (format, diagram) = diagram_block.take().unwrap_or_default();
                events.push(Event::Html(CowStr::from(render_diagram_block(
                    &format, &diagram,
                ))));
            }
            Event::Text(text) if diagram_block.is_some() => {
                if let Some((_, diagram)) = diagram_block.as_mut() {
                    diagram.push_str(&text);
                }
            }
            Event::Code(text) if diagram_block.is_some() => {
                if let Some((_, diagram)) = diagram_block.as_mut() {
                    diagram.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak if diagram_block.is_some() => {
                if let Some((_, diagram)) = diagram_block.as_mut() {
                    diagram.push('\n');
                }
            }
            _event if diagram_block.is_some() => {}
            event => events.push(event),
        }
    }

    if let Some((format, diagram)) = diagram_block {
        events.push(Event::Html(CowStr::from(format!(
            "<pre><code>{}\n{}</code></pre>",
            escape_html(&format),
            escape_html(&diagram)
        ))));
    }

    html::push_html(&mut html, events.into_iter());
    html
}

fn render_markdown_with_sources(
    content: &str,
    from_page_ref: &str,
    evidence: &[Evidence],
) -> String {
    render_markdown(&expand_source_markers(content, from_page_ref, evidence))
}

fn content_has_source_markers(content: &str) -> bool {
    content.contains("{{source:") || content.contains("{{src:")
}

fn expand_source_markers(content: &str, from_page_ref: &str, evidence: &[Evidence]) -> String {
    let mut output = String::new();
    let mut remaining = content;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find("}}") else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let marker = &after_start[..end];
        if let Some(source_html) = render_source_marker(marker, from_page_ref, evidence) {
            output.push_str(&source_html);
        } else {
            output.push_str("{{");
            output.push_str(marker);
            output.push_str("}}");
        }
        remaining = &after_start[end + 2..];
    }
    output.push_str(remaining);
    output
}

fn render_source_marker(
    marker: &str,
    from_page_ref: &str,
    evidence: &[Evidence],
) -> Option<String> {
    let index = marker
        .strip_prefix("source:")
        .or_else(|| marker.strip_prefix("src:"))?
        .trim()
        .parse::<usize>()
        .ok()?;
    let evidence_item = evidence.get(index.checked_sub(1)?)?;
    let href = source_span_href(from_page_ref, evidence_item)?;
    let label = format!("[{index}]");
    let title = source_marker_title(evidence_item);
    Some(format!(
        "<a class=\"source-cite\" href=\"{}\" title=\"{}\" aria-label=\"{}\">{}</a>",
        escape_html(&href),
        escape_html(&title),
        escape_html(&title),
        label
    ))
}

fn source_marker_title(evidence: &Evidence) -> String {
    let mut title = source_span_label(evidence);
    if let Some(note) = &evidence.note {
        if !note.trim().is_empty() {
            title.push_str(" - ");
            title.push_str(note.trim());
        }
    }
    title
}

fn render_diagram_block(format: &str, diagram: &str) -> String {
    match format {
        "svg" => format!("<div class=\"diagram-block\">{diagram}</div>"),
        "mermaid" => format!(
            "<div class=\"diagram-block\" data-diagram-format=\"mermaid\"><pre class=\"mermaid\">{}</pre></div>",
            escape_html(&sanitize_mermaid_labels(diagram))
        ),
        _ => format!(
            "<pre><code>{}\n{}</code></pre>",
            escape_html(format),
            escape_html(diagram)
        ),
    }
}

fn sanitize_mermaid_labels(diagram: &str) -> String {
    let mut output = String::with_capacity(diagram.len());
    for line in diagram.lines() {
        output.push_str(&sanitize_mermaid_line_labels(line));
        output.push('\n');
    }
    output.trim_end_matches('\n').to_string()
}

fn sanitize_mermaid_line_labels(line: &str) -> String {
    let mut output = String::new();
    let mut remaining = line;
    while let Some(open) = remaining.find('[') {
        output.push_str(&remaining[..open + 1]);
        let after_open = &remaining[open + 1..];
        let Some(close) = after_open.find(']') else {
            output.push_str(after_open);
            return output;
        };
        let label = &after_open[..close];
        if label.starts_with('"') || label.starts_with('\'') || label.starts_with('`') {
            output.push_str(label);
        } else {
            output.push('"');
            output.push_str(&label.replace('"', "\\\""));
            output.push('"');
        }
        output.push(']');
        remaining = &after_open[close + 1..];
    }
    output.push_str(remaining);
    output
}

fn page_uses_mermaid(format: ContentFormat, content: &str) -> bool {
    match format {
        ContentFormat::Markdown => content
            .lines()
            .any(|line| line.trim_start().starts_with("```mermaid")),
        ContentFormat::Html => {
            content.contains("class=\"mermaid\"") || content.contains("class='mermaid'")
        }
    }
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
