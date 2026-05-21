use serde::{Deserialize, Serialize};

pub type NodeId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeGraph {
    pub schema_version: u32,
    pub repo_root: String,
    pub indexed_at_unix: u64,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl CodeGraph {
    pub fn empty(repo_root: String) -> Self {
        Self {
            schema_version: 1,
            repo_root,
            indexed_at_unix: current_unix(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub path: String,
    pub span: Option<Span>,
    pub language: Option<Language>,
    pub text: String,
    pub tags: Vec<String>,
    pub metrics: NodeMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NodeMetrics {
    pub in_degree: usize,
    pub out_degree: usize,
    pub churn_90d: u32,
    pub last_changed_unix: Option<u64>,
    pub is_test: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Repository,
    File,
    Module,
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Union,
    Typedef,
    Variable,
    Macro,
    Trait,
    Interface,
    Import,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub confidence_basis_points: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Defines,
    Imports,
    Calls,
    References,
    Extends,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Cpp,
}

impl Language {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Cpp => "cpp",
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Span {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

pub fn current_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
