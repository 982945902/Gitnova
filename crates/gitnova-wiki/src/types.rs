use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    Index,
    Article,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentFormat {
    Markdown,
    Html,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchMode {
    Replace,
    Append,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
}

fn default_task_status() -> TaskStatus {
    TaskStatus::Pending
}

fn default_outline_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Evidence {
    pub file: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub note: Option<String>,
}

impl Evidence {
    pub fn new(file: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            start_line: None,
            end_line: None,
            note: None,
        }
    }

    pub fn with_span(mut self, start_line: u32, end_line: u32) -> Self {
        self.start_line = Some(start_line);
        self.end_line = Some(end_line);
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WikiPage {
    pub id: String,
    pub title: String,
    pub kind: PageKind,
    pub summary: Option<String>,
    pub parent: Option<String>,
    pub content_format: ContentFormat,
    pub content: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeepTask {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub scope_paths: Vec<String>,
    #[serde(default)]
    pub scope_symbols: Vec<String>,
    #[serde(default)]
    pub expected_outputs: Vec<String>,
    #[serde(default = "default_task_status")]
    pub status: TaskStatus,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutlinePage {
    pub id: String,
    pub title: String,
    pub kind: PageKind,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub deep_tasks: Vec<DeepTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WikiOutline {
    #[serde(default = "default_outline_version")]
    pub version: u32,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub pages: Vec<OutlinePage>,
}

impl Default for WikiOutline {
    fn default() -> Self {
        Self {
            version: 1,
            root: String::new(),
            pages: Vec::new(),
        }
    }
}

impl WikiPage {
    pub fn new(id: impl Into<String>, title: impl Into<String>, kind: PageKind) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind,
            summary: None,
            parent: None,
            content_format: ContentFormat::Markdown,
            content: String::new(),
            evidence: Vec::new(),
        }
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WikiNode {
    pub id: String,
    pub title: String,
    pub kind: PageKind,
    pub summary: Option<String>,
    pub parent: Option<String>,
    pub content_ref: String,
    pub page_ref: String,
    pub content_format: ContentFormat,
    pub evidence: Vec<Evidence>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WikiSchema {
    pub version: u32,
    pub nodes: Vec<WikiNode>,
}

impl Default for WikiSchema {
    fn default() -> Self {
        Self {
            version: 1,
            nodes: Vec::new(),
        }
    }
}

impl WikiSchema {
    pub fn node(&self, id: &str) -> Option<&WikiNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub(crate) fn node_mut(&mut self, id: &str) -> Option<&mut WikiNode> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }
}
