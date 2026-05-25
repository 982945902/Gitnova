<<<<<<< HEAD
use crate::sqlite::{FileManifestEntry, StoredEmbedding};
use anyhow::{Context, Result};
use gitnova_core::model::{
    current_unix, CodeGraph, Edge, EdgeKind, Language, Node, NodeKind, NodeMetrics,
};
=======
use anyhow::Result;
use gitnova_core::model::{
    current_unix, CodeGraph, Edge, EdgeKind, Language, Node, NodeKind, NodeMetrics, Span,
};
use serde::{Deserialize, Serialize};
>>>>>>> origin/worktree-agent-a617ffcd
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use surrealdb::engine::local::SurrealKv;
use surrealdb::Surreal;

use crate::json_export;
use crate::{FileManifestEntry, StoredEmbedding};

pub type Db = Surreal<surrealdb::engine::local::Db>;

/// Escape a string value for use in a SurrealQL single-quoted string literal.
/// SurrealDB doubles single quotes to escape them.
fn surql_escape(s: &str) -> String {
    s.replace('\'', "''")
}

const EDGE_TABLES: &[&str] = &[
    "calls_edge", "extends_edge", "contains_edge",
    "defines_edge", "imports_edge", "references_edge",
];

const EDGE_TABLE_KINDS: &[(&str, EdgeKind)] = &[
    ("calls_edge", EdgeKind::Calls),
    ("extends_edge", EdgeKind::Extends),
    ("contains_edge", EdgeKind::Contains),
    ("defines_edge", EdgeKind::Defines),
    ("imports_edge", EdgeKind::Imports),
    ("references_edge", EdgeKind::References),
];

pub struct SurrealStore {
    pub db: Db,
    repo_root: PathBuf,
    rt: Option<tokio::runtime::Runtime>,
}

impl SurrealStore {
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let repo_root = repo_root.as_ref().to_path_buf();
        let db_dir = repo_root.join(".gitnova").join("surrealdb");
        std::fs::create_dir_all(&db_dir)?;

<<<<<<< HEAD
        let (db, rt) = if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let db = tokio::task::block_in_place(|| handle.block_on(connect_surrealdb(&db_dir)))?;
            (db, None)
        } else {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let db = rt.block_on(connect_surrealdb(&db_dir))?;
            (db, Some(rt))
        };
=======
        let db = run_async(async { connect_surrealdb(&db_dir).await })?;
>>>>>>> origin/worktree-agent-a617ffcd

        Ok(Self { db, repo_root, rt })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

<<<<<<< HEAD
    fn block_on<F, T>(&self, f: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        if let Some(rt) = &self.rt {
            rt.block_on(f)
        } else {
            tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
        }
    }

    pub fn json_path(&self) -> PathBuf {
        self.repo_root.join(".gitnova/index.json")
    }

    pub fn export_json(&self, graph: &CodeGraph) -> Result<()> {
        crate::json_export::export_graph(graph, &self.json_path())
    }

    // ------------------------------------------------------------------
    // save_graph
    // ------------------------------------------------------------------

    pub fn save_graph(&self, graph: &CodeGraph) -> Result<()> {
        // Pre-extract all data as simple owned strings.
        let nodes: Vec<(String, String, String, String, String, String, String, String, usize, usize, u32, bool, String)> = graph
            .nodes.iter()
            .map(|n| {
                let kind = serde_json::to_string(&n.kind)?;
                let tags = serde_json::to_string(&n.tags)?;
                let metrics = serde_json::to_string(&n.metrics)?;
                let lang = n.language.map(|l| l.as_str().to_string()).unwrap_or_default();
                Ok((
                    n.id.clone(), n.name.clone(), n.qualified_name.clone(),
                    kind, n.path.clone(), n.text.clone(),
                    tags, metrics,
                    n.metrics.in_degree, n.metrics.out_degree, n.metrics.churn_90d, n.metrics.is_test,
                    lang,
                ))
            })
            .collect::<Result<Vec<_>>>()?;

        let edges: Vec<(String, String, String, u16)> = graph.edges.iter().map(|e| {
            (e.from.clone(), e.to.clone(), edge_table_str(&e.kind).to_string(), e.confidence_basis_points)
        }).collect();

        let sv = graph.schema_version.to_string();
        let rr = graph.repo_root.clone();
        let iu = graph.indexed_at_unix.to_string();

        self.block_on(async {
            self.db.query("DELETE FROM symbol").await?;
            // g_meta
            // g_meta — use CONTENT to avoid escaping issues
            self.db.query(format!("CREATE g_meta CONTENT {}", serde_json::to_string(&serde_json::json!({ "id": "schema_version", "val": sv }))?)).await?;
            self.db.query(format!("CREATE g_meta CONTENT {}", serde_json::to_string(&serde_json::json!({ "id": "repo_root", "val": rr }))?)).await?;
            self.db.query(format!("CREATE g_meta CONTENT {}", serde_json::to_string(&serde_json::json!({ "id": "indexed_at_unix", "val": iu }))?)).await?;
            // nodes — use CONTENT with JSON to avoid all string escaping issues
            for (id, name, qn, kind, path, text, tags, metrics, indeg, outdeg, churn, istest, lang) in &nodes {
                let obj = serde_json::json!({
                    "node_id": id, "name": name, "qualified_name": qn,
                    "kind": kind, "path": path, "text": text,
                    "tags": tags, "metrics": metrics,
                    "in_degree": indeg, "out_degree": outdeg,
                    "churn_90d": churn, "is_test": istest, "language": lang,
                });
                let q = format!("CREATE symbol CONTENT {}", serde_json::to_string(&obj)?);
                self.db.query(q.as_str()).await?;
            }
            // edges — use CONTENT with JSON to avoid escaping issues
            for table in EDGE_TABLES {
                self.db.query(format!("DELETE FROM {}", table)).await?;
            }
            for (from, to, tbl, conf) in &edges {
                let q = format!(
                    "LET $src = (SELECT * FROM symbol WHERE node_id = '{}' LIMIT 1); \
                     LET $dst = (SELECT * FROM symbol WHERE node_id = '{}' LIMIT 1); \
                     RELATE $src->{}->$dst CONTENT {}",
                    surql_escape(from), surql_escape(to), tbl,
                    serde_json::to_string(&serde_json::json!({
                        "confidence": conf,
                        "from_node_id": from,
                        "to_node_id": to,
                    }))?,
                );
                self.db.query(q.as_str()).await?;
            }
            Ok::<_, anyhow::Error>(())
        }).context("save_graph failed")?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // load_graph
    // ------------------------------------------------------------------

    pub fn load_graph(&self) -> Result<CodeGraph> {
        self.block_on(async {
            async fn meta_val(db: &Db, key: &str) -> Result<Option<String>> {
                let q = format!("SELECT val FROM g_meta WHERE id = '{}'", key.replace('\'', "''"));
                let mut r = db.query(q.as_str()).await?;
                let rows: Vec<serde_json::Value> = r.take(0)?;
                Ok(rows.first().and_then(|x: &serde_json::Value| x.get("val")).and_then(|v: &serde_json::Value| v.as_str()).map(String::from))
            }

            let schema_version = meta_val(&self.db, "schema_version").await?.unwrap_or_else(|| "1".into()).parse::<u32>().unwrap_or(1);
            let repo_root = meta_val(&self.db, "repo_root").await?.unwrap_or_default();
            let indexed_at_unix = meta_val(&self.db, "indexed_at_unix").await?.unwrap_or_else(|| current_unix().to_string()).parse::<u64>().unwrap_or_else(|_| current_unix());

            let mut resp = self.db.query("SELECT node_id, name, qualified_name, kind, path, text, tags, metrics, in_degree, out_degree, churn_90d, is_test, language FROM symbol ORDER BY node_id").await?;
            let rows: Vec<serde_json::Value> = resp.take(0)?;
            let nodes: Vec<Node> = rows.iter().map(|r| {
                let id = r.get("node_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let kind_str = r.get("kind").and_then(|v| v.as_str()).unwrap_or("unknown");
                let tags_str = r.get("tags").and_then(|v| v.as_str()).unwrap_or("[]");
                let metrics_str = r.get("metrics").and_then(|v| v.as_str()).unwrap_or("{}");
                Node {
                    id,
                    kind: serde_json::from_str(kind_str).unwrap_or(NodeKind::Unknown),
                    name: r.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    qualified_name: r.get("qualified_name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    path: r.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    span: None,
                    language: r.get("language").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(language_from_str),
                    text: r.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    tags: serde_json::from_str(tags_str).unwrap_or_default(),
                    metrics: {
                        let mut m: NodeMetrics = serde_json::from_str(metrics_str).unwrap_or_default();
                        if let Some(v) = r.get("in_degree").and_then(|v| v.as_i64()) { m.in_degree = v as usize; }
                        if let Some(v) = r.get("out_degree").and_then(|v| v.as_i64()) { m.out_degree = v as usize; }
                        if let Some(v) = r.get("churn_90d").and_then(|v| v.as_i64()) { m.churn_90d = v as u32; }
                        if let Some(v) = r.get("is_test").and_then(|v| v.as_bool()) { m.is_test = v; }
                        m
                    },
                }
            }).collect();

            let mut edges = Vec::new();
            for (tbl, kind) in EDGE_TABLE_KINDS {
                let q = format!("SELECT from_node_id, to_node_id, confidence FROM {}", tbl);
                if let Ok(mut resp) = self.db.query(q.as_str()).await {
                    if let Ok(edge_rows) = resp.take::<Vec<serde_json::Value>>(0) {
                        for row in edge_rows {
                            let from = row.get("from_node_id").and_then(|v| v.as_str()).unwrap_or("");
                            let to = row.get("to_node_id").and_then(|v| v.as_str()).unwrap_or("");
                            if !from.is_empty() && !to.is_empty() {
                                edges.push(Edge {
                                    from: from.to_string(), to: to.to_string(),
                                    kind: kind.clone(),
                                    confidence_basis_points: row.get("confidence").and_then(|v| v.as_i64()).unwrap_or(0) as u16,
                                });
                            }
                        }
                    }
                }
            }

            Ok(CodeGraph { schema_version, repo_root, indexed_at_unix, nodes, edges })
        })
    }

    // ------------------------------------------------------------------
    // search_fts
    // ------------------------------------------------------------------

    pub fn search_fts(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let q = query.to_string();
        self.block_on(async {
            // Escape single quotes by doubling them (SurrealDB standard)
            let escaped = q.replace('\'', "''");
            let sql = format!(
                "SELECT node_id, search::score(0) AS score FROM symbol WHERE text @@ '{}' ORDER BY score DESC LIMIT {}",
                escaped, limit
            );
            let mut resp = self.db.query(sql.as_str()).await?;
            let rows: Vec<serde_json::Value> = resp.take(0)?;
            Ok(rows.iter().map(|r| r.get("node_id").and_then(|v| v.as_str()).unwrap_or("").to_string()).collect())
        })
    }

    // ------------------------------------------------------------------
    // file manifest
    // ------------------------------------------------------------------

    pub fn save_manifest(&self, entries: &[FileManifestEntry]) -> Result<()> {
        let rows: Vec<(String, String, String, u64)> = entries.iter().map(|e| {
            (e.path.clone(), e.content_hash.clone(), e.language.as_str().to_string(), e.indexed_at_unix)
        }).collect();
        self.block_on(async {
            self.db.query("DELETE FROM file_manifest").await?;
            for (path, hash, lang, ts) in &rows {
                let obj = serde_json::json!({
                    "path": path, "content_hash": hash,
                    "language": lang, "indexed_at_unix": ts,
                });
                let q = format!("CREATE file_manifest CONTENT {}", serde_json::to_string(&obj)?);
                self.db.query(q.as_str()).await?;
            }
            Ok::<_, anyhow::Error>(())
        })?;
        Ok(())
=======
    fn block_on<F, T>(&self, future: F) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        run_async(future)
    }

    pub fn save_graph(&self, graph: &CodeGraph) -> Result<()> {
        let graph_json = serde_json::to_string(graph)?;
        self.block_on(async {
            // Delete existing data
            self.db.query("DELETE FROM symbol").await?;
            self.db.query("DELETE FROM graph_edge").await?;
            self.db.query("DELETE FROM meta").await?;

            #[derive(Serialize)]
            struct MetaRow {
                key: String,
                value: String,
            }

            // Insert individual nodes (for FTS queries)
            for node in &graph.nodes {
                let kind_str = format!("{:?}", node.kind).to_lowercase();
                let language = node.language.map(|l| format!("{:?}", l).to_lowercase());
                let (start_line, start_col, end_line, end_col) = match node.span {
                    Some(span) => (Some(span.start_line as i64), Some(span.start_col as i64), Some(span.end_line as i64), Some(span.end_col as i64)),
                    None => (None, None, None, None),
                };

                #[derive(Serialize)]
                struct NodeRecord {
                    node_id: String, name: String, qualified_name: String, kind: String,
                    path: String, language: Option<String>, text: String,
                    start_line: Option<i64>, start_col: Option<i64>,
                    end_line: Option<i64>, end_col: Option<i64>,
                    tags_json: String, metrics_json: String,
                    in_degree: i64, out_degree: i64, churn_90d: i64, is_test: bool,
                }

                let record = NodeRecord {
                    node_id: node.id.clone(), name: node.name.clone(),
                    qualified_name: node.qualified_name.clone(), kind: kind_str,
                    path: node.path.clone(), language, text: node.text.clone(),
                    start_line, start_col, end_line, end_col,
                    tags_json: serde_json::to_string(&node.tags).unwrap_or_default(),
                    metrics_json: serde_json::to_string(&node.metrics).unwrap_or_default(),
                    in_degree: node.metrics.in_degree as i64,
                    out_degree: node.metrics.out_degree as i64,
                    churn_90d: node.metrics.churn_90d as i64,
                    is_test: node.metrics.is_test,
                };
                self.db.query("CREATE symbol CONTENT $node")
                    .bind(("node", record)).await?;
            }

            // Insert individual edges
            for edge in &graph.edges {
                let kind_str = format!("{:?}", edge.kind).to_lowercase();
                #[derive(Serialize)]
                struct EdgeRecord {
                    from_id: String, to_id: String, kind: String, confidence: i64,
                }
                self.db.query("CREATE graph_edge CONTENT $edge")
                    .bind(("edge", EdgeRecord {
                        from_id: edge.from.clone(), to_id: edge.to.clone(),
                        kind: kind_str, confidence: edge.confidence_basis_points as i64,
                    })).await?;
            }

            // Save individual meta fields
            for (key, value) in &[
                ("schema_version", graph.schema_version.to_string()),
                ("repo_root", graph.repo_root.clone()),
                ("indexed_at_unix", graph.indexed_at_unix.to_string()),
            ] {
                self.db.query("CREATE meta CONTENT $row")
                    .bind(("row", MetaRow { key: key.to_string(), value: value.clone() }))
                    .await?;
            }

            // Save the entire graph as a single JSON blob LAST
            // This is the final write — it's most likely to survive the RocksDB buffer
            self.db.query("CREATE meta CONTENT $row")
                .bind(("row", MetaRow {
                    key: "graph_json".to_string(),
                    value: graph_json.clone(),
                }))
                .await?;

            Ok(())
        })
    }

    pub fn load_graph(&self) -> Result<CodeGraph> {
        self.block_on(async {
            // Load meta — try graph_json blob first for reliable persistence
            let mut resp = self.db.query("SELECT * FROM meta").await?;
            let meta_rows: Vec<MetaRow> = resp.take(0)?;

            let mut meta_map: HashMap<String, String> = HashMap::new();
            for row in meta_rows {
                meta_map.insert(row.key.clone(), row.value.clone());
            }

            // Priority 1: load entire graph from single JSON blob
            if let Some(json_str) = meta_map.get("graph_json") {
                if let Ok(graph) = serde_json::from_str::<CodeGraph>(json_str) {
                    return Ok(graph);
                }
            }

            // Priority 2: reconstruct from individual symbol/edge records (legacy path)
            let schema_version: u32 = meta_map
                .get("schema_version")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let repo_root = meta_map
                .get("repo_root")
                .cloned()
                .unwrap_or_else(|| self.repo_root.to_string_lossy().to_string());
            let indexed_at_unix: u64 = meta_map
                .get("indexed_at_unix")
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(current_unix);

            // Load nodes
            let mut resp = self.db.query("SELECT * FROM symbol").await?;
            let symbols: Vec<SymbolRow> = resp.take(0)?;

            let nodes: Vec<Node> = symbols
                .into_iter()
                .map(|row| {
                    let kind =
                        serde_json::from_str::<NodeKind>(&format!("\"{}\"", &row.kind))
                            .unwrap_or(NodeKind::Unknown);
                    let language = row
                        .language
                        .as_ref()
                        .and_then(|l| serde_json::from_str::<Language>(&format!("\"{l}\"")).ok());
                    let span = row.start_line.map(|sl| Span {
                        start_line: sl as usize,
                        start_col: row.start_col.unwrap_or(1) as usize,
                        end_line: row.end_line.unwrap_or(sl) as usize,
                        end_col: row.end_col.unwrap_or(1) as usize,
                    });
                    let tags: Vec<String> = row
                        .tags_json
                        .as_deref()
                        .and_then(|j| serde_json::from_str(j).ok())
                        .unwrap_or_default();
                    let metrics: NodeMetrics = row
                        .metrics_json
                        .as_deref()
                        .and_then(|j| serde_json::from_str(j).ok())
                        .unwrap_or_else(|| {
                            let mut m = NodeMetrics::default();
                            m.in_degree = row.in_degree.unwrap_or(0) as usize;
                            m.out_degree = row.out_degree.unwrap_or(0) as usize;
                            m.churn_90d = row.churn_90d.unwrap_or(0) as u32;
                            m.is_test = row.is_test.unwrap_or(false);
                            m
                        });

                    Node {
                        id: row.node_id,
                        kind,
                        name: row.name,
                        qualified_name: row.qualified_name,
                        path: row.path,
                        span,
                        language,
                        text: row.text,
                        tags,
                        metrics,
                    }
                })
                .collect();

            // Load edges
            let mut resp = self.db.query("SELECT * FROM graph_edge").await?;
            let edge_rows: Vec<EdgeRow> = resp.take(0)?;

            let edges: Vec<Edge> = edge_rows
                .into_iter()
                .map(|row| {
                    let kind =
                        serde_json::from_str::<EdgeKind>(&format!("\"{}\"", &row.kind))
                            .unwrap_or(EdgeKind::References);
                    Edge {
                        from: row.from_id,
                        to: row.to_id,
                        kind,
                        confidence_basis_points: row.confidence.unwrap_or(0) as u16,
                    }
                })
                .collect();

            Ok(CodeGraph {
                schema_version,
                repo_root,
                indexed_at_unix,
                nodes,
                edges,
            })
        })
    }

    pub fn export_json(&self, graph: &CodeGraph) -> Result<()> {
        json_export::export_graph(graph, self.repo_root.join(".gitnova/index.json"))
    }

    pub fn save_manifest(&self, entries: &[FileManifestEntry]) -> Result<()> {
        self.block_on(async {
            self.db.query("DELETE FROM file_manifest").await?;
            for entry in entries {
                let lang_str = format!("{:?}", entry.language).to_lowercase();

                #[derive(Serialize)]
                struct ManifestRow {
                    path: String,
                    content_hash: String,
                    language: String,
                    indexed_at_unix: i64,
                }

                let record = ManifestRow {
                    path: entry.path.clone(),
                    content_hash: entry.content_hash.clone(),
                    language: lang_str,
                    indexed_at_unix: entry.indexed_at_unix as i64,
                };
                self.db
                    .query("CREATE file_manifest CONTENT $entry")
                    .bind(("entry", record))
                    .await?;
            }
            Ok(())
        })
>>>>>>> origin/worktree-agent-a617ffcd
    }

    pub fn load_manifest(&self) -> Result<HashMap<String, FileManifestEntry>> {
        self.block_on(async {
<<<<<<< HEAD
            let mut resp = self.db.query("SELECT path, content_hash, language, indexed_at_unix FROM file_manifest").await?;
            let rows: Vec<serde_json::Value> = resp.take(0)?;
            let mut map = HashMap::new();
            for r in rows {
                let path = r.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                map.insert(path.clone(), FileManifestEntry {
                    path,
                    content_hash: r.get("content_hash").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    language: language_from_str(r.get("language").and_then(|v| v.as_str()).unwrap_or("rust")),
                    indexed_at_unix: r.get("indexed_at_unix").and_then(|v| v.as_i64()).unwrap_or(0) as u64,
                });
=======
            let mut resp = self.db.query("SELECT * FROM file_manifest").await?;
            let rows: Vec<ManifestRow> = resp.take(0)?;

            let mut map = HashMap::new();
            for row in rows {
                let language =
                    serde_json::from_str::<Language>(&format!("\"{}\"", &row.language))
                        .unwrap_or(Language::Rust);
                map.insert(
                    row.path.clone(),
                    FileManifestEntry {
                        path: row.path,
                        content_hash: row.content_hash,
                        language,
                        indexed_at_unix: row.indexed_at_unix as u64,
                    },
                );
>>>>>>> origin/worktree-agent-a617ffcd
            }
            Ok(map)
        })
    }

<<<<<<< HEAD
    // ------------------------------------------------------------------
    // embeddings
    // ------------------------------------------------------------------

    pub fn save_embeddings(&self, embeddings: &[StoredEmbedding]) -> Result<()> {
        let rows: Vec<(String, String, String)> = embeddings.iter().map(|e| {
            let vec_json = serde_json::to_string(&e.vector).unwrap_or_default();
            (e.node_id.clone(), e.provider.clone(), vec_json)
        }).collect();
        self.block_on(async {
            self.db.query("DELETE FROM embedding").await?;
            for (nid, prov, vj) in &rows {
                let obj = serde_json::json!({
                    "node_id": nid, "provider": prov, "vector_json": vj,
                });
                let q = format!("CREATE embedding CONTENT {}", serde_json::to_string(&obj)?);
                self.db.query(q.as_str()).await?;
            }
            Ok::<_, anyhow::Error>(())
        })?;
        Ok(())
    }

    pub fn load_embeddings(&self, provider: &str) -> Result<HashMap<String, Vec<f32>>> {
        let p = provider.to_string();
        self.block_on(async {
            let sql = format!("SELECT node_id, vector_json FROM embedding WHERE provider = '{}'", p.replace('\'', "''"));
            let mut resp = self.db.query(sql.as_str()).await?;
            let rows: Vec<serde_json::Value> = resp.take(0)?;
            let mut map = HashMap::new();
            for r in rows {
                let nid = r.get("node_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let vec: Vec<f32> = r.get("vector_json").and_then(|v| v.as_str()).and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
                map.insert(nid, vec);
=======
    pub fn save_embeddings(&self, embeddings: &[StoredEmbedding]) -> Result<()> {
        self.block_on(async {
            self.db.query("DELETE FROM embedding").await?;
            for emb in embeddings {
                #[derive(Serialize)]
                struct EmbeddingRow {
                    node_id: String,
                    provider: String,
                    vector: String,
                }

                let record = EmbeddingRow {
                    node_id: emb.node_id.clone(),
                    provider: emb.provider.clone(),
                    vector: serde_json::to_string(&emb.vector)?,
                };
                self.db
                    .query("CREATE embedding CONTENT $emb")
                    .bind(("emb", record))
                    .await?;
            }
            Ok(())
        })
    }

    pub fn load_embeddings(&self, provider: &str) -> Result<HashMap<String, Vec<f32>>> {
        let provider = provider.to_string();
        self.block_on(async move {
            let mut resp = self
                .db
                .query("SELECT node_id, vector FROM embedding WHERE provider = $provider")
                .bind(("provider", provider))
                .await?;
            let rows: Vec<EmbeddingRow> = resp.take(0)?;

            let mut map = HashMap::new();
            for row in rows {
                let vector: Vec<f32> = serde_json::from_str(&row.vector).unwrap_or_default();
                map.insert(row.node_id, vector);
>>>>>>> origin/worktree-agent-a617ffcd
            }
            Ok(map)
        })
    }
<<<<<<< HEAD
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn edge_table_str(kind: &EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Calls => "calls_edge",
        EdgeKind::Extends => "extends_edge",
        EdgeKind::Contains => "contains_edge",
        EdgeKind::Defines => "defines_edge",
        EdgeKind::Imports => "imports_edge",
        EdgeKind::References => "references_edge",
    }
}

fn language_from_str(s: &str) -> Language {
    match s {
        "rust" => Language::Rust,
        "typescript" => Language::TypeScript,
        "javascript" => Language::JavaScript,
        "python" => Language::Python,
        "cpp" => Language::Cpp,
        _ => Language::Rust,
    }
}

// ---------------------------------------------------------------------------
// Connection & schema
// ---------------------------------------------------------------------------

async fn connect_surrealdb(db_dir: &std::path::Path) -> Result<Db> {
    let db = Surreal::new::<SurrealKv>(db_dir.to_string_lossy().as_ref()).await?;
=======

    /// Explicitly close the SurrealDB connection to release RocksDB locks.
    pub fn close(self) -> Result<()> {
        // Extract db and drop it outside the async context.
        // The Surreal<Db> Drop impl should release RocksDB locks.
        drop(self);
        Ok(())
    }

    pub fn search_fts(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let query = query.to_string();
        self.block_on(async move {
            let mut resp = self
                .db
                .query(
                    "SELECT node_id FROM symbol WHERE text @@ $query ORDER BY BM25(text) DESC LIMIT $limit",
                )
                .bind(("query", query))
                .bind(("limit", limit as i64))
                .await?;
            let rows: Vec<FtsRow> = resp.take(0)?;

            Ok(rows.into_iter().map(|r| r.node_id).collect())
        })
    }
}

/// Run an async operation, adapting to the current runtime context.
fn run_async<F, T>(future: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // Already inside a tokio runtime — use block_in_place to run the future
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        Err(_) => {
            // Not inside a runtime — create one temporarily
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(future)
        }
    }
}

// Schema definitions updated via SurrealDB queries
async fn connect_surrealdb(db_dir: &Path) -> Result<Db> {
    let db = Surreal::new::<RocksDb>(db_dir.to_string_lossy().as_ref()).await?;
>>>>>>> origin/worktree-agent-a617ffcd
    db.use_ns("gitnova").use_db("havenask").await?;
    ensure_schema(&db).await?;
    Ok(db)
}

async fn ensure_schema(db: &Db) -> Result<()> {
<<<<<<< HEAD
    let stmts = [
        "DEFINE ANALYZER IF NOT EXISTS simple TOKENIZERS blank, class FILTERS lowercase",
        "DEFINE TABLE IF NOT EXISTS symbol SCHEMAFULL",
        "DEFINE FIELD node_id ON symbol TYPE string",
        "DEFINE FIELD name ON symbol TYPE string",
        "DEFINE FIELD qualified_name ON symbol TYPE string",
        "DEFINE FIELD kind ON symbol TYPE string",
        "DEFINE FIELD path ON symbol TYPE string",
        "DEFINE FIELD text ON symbol TYPE string",
        "DEFINE FIELD tags ON symbol TYPE string",
        "DEFINE FIELD metrics ON symbol TYPE string",
        "DEFINE FIELD in_degree ON symbol TYPE int DEFAULT 0",
        "DEFINE FIELD out_degree ON symbol TYPE int DEFAULT 0",
        "DEFINE FIELD churn_90d ON symbol TYPE int DEFAULT 0",
        "DEFINE FIELD is_test ON symbol TYPE bool DEFAULT false",
        "DEFINE FIELD language ON symbol TYPE string DEFAULT ''",
        "DEFINE INDEX IF NOT EXISTS idx_sym_node_id ON symbol FIELDS node_id UNIQUE",
        "DEFINE INDEX IF NOT EXISTS idx_sym_name ON symbol FIELDS name",
        "DEFINE INDEX IF NOT EXISTS idx_sym_kind ON symbol FIELDS kind",
        "DEFINE INDEX IF NOT EXISTS idx_fts ON symbol FIELDS text SEARCH ANALYZER simple BM25 HIGHLIGHTS",
    ];
    for s in &stmts {
        db.query(*s).await?;
    }
    for tbl in EDGE_TABLES {
        let q = format!("DEFINE TABLE IF NOT EXISTS {} SCHEMAFULL TYPE RELATION IN symbol TO symbol", tbl);
        db.query(q.as_str()).await?;
        db.query(format!("DEFINE FIELD confidence ON {} TYPE int DEFAULT 6500", tbl).as_str()).await?;
        db.query(format!("DEFINE FIELD from_node_id ON {} TYPE string", tbl).as_str()).await?;
        db.query(format!("DEFINE FIELD to_node_id ON {} TYPE string", tbl).as_str()).await?;
    }
    db.query("DEFINE TABLE IF NOT EXISTS g_meta SCHEMAFULL").await?;
    db.query("DEFINE FIELD id ON g_meta TYPE string").await?;
    db.query("DEFINE FIELD val ON g_meta TYPE string").await?;
    db.query("DEFINE TABLE IF NOT EXISTS file_manifest SCHEMAFULL").await?;
    db.query("DEFINE FIELD path ON file_manifest TYPE string").await?;
    db.query("DEFINE FIELD content_hash ON file_manifest TYPE string").await?;
    db.query("DEFINE FIELD language ON file_manifest TYPE string").await?;
    db.query("DEFINE FIELD indexed_at_unix ON file_manifest TYPE int").await?;
    db.query("DEFINE TABLE IF NOT EXISTS embedding SCHEMAFULL").await?;
    db.query("DEFINE FIELD node_id ON embedding TYPE string").await?;
    db.query("DEFINE FIELD provider ON embedding TYPE string").await?;
    db.query("DEFINE FIELD vector_json ON embedding TYPE string").await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::{EdgeKind, NodeKind, NodeMetrics};

    fn test_graph(repo_root: impl AsRef<str>) -> CodeGraph {
        let mut g = CodeGraph::empty(repo_root.as_ref().to_string());
        g.nodes.push(Node {
            id: "n1".into(), kind: NodeKind::Repository, name: "repo".into(),
            qualified_name: "repo".into(), path: String::new(), span: None, language: None,
            text: String::new(), tags: vec![], metrics: NodeMetrics::default(),
        });
        g.nodes.push(Node {
            id: "n2".into(), kind: NodeKind::Function, name: "foo".into(),
            qualified_name: "pkg::foo".into(), path: "src/lib.rs".into(), span: None,
            language: Some(gitnova_core::Language::Rust),
            text: "pub fn foo() {}".into(), tags: vec!["public".into()],
            metrics: NodeMetrics { in_degree: 1, out_degree: 0, churn_90d: 3, is_test: false, ..Default::default() },
        });
        g.edges.push(Edge { from: "n2".into(), to: "n1".into(), kind: EdgeKind::Contains, confidence_basis_points: 10_000 });
        g
    }

    #[test]
    fn surrealdb_round_trips_graph_and_exports_json() {
        let temp = tempfile::TempDir::new().unwrap();
        let graph = test_graph(temp.path().to_string_lossy().to_string());
        let store = SurrealStore::open(temp.path()).unwrap();
        store.save_graph(&graph).unwrap();
        store.export_json(&graph).unwrap();
        let loaded = store.load_graph().unwrap();
        let imported = crate::json_export::import_graph(store.json_path()).unwrap();
        assert_eq!(loaded.nodes, graph.nodes);
        assert_eq!(imported.nodes, graph.nodes);
        assert!(temp.path().join(".gitnova/index.json").exists());
    }

    #[test]
    fn surrealdb_round_trips_manifest() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = SurrealStore::open(temp.path()).unwrap();
        store.save_manifest(&[FileManifestEntry {
            path: "src/lib.rs".into(), content_hash: "abc".into(),
            language: gitnova_core::Language::Rust, indexed_at_unix: 42,
        }]).unwrap();
        let loaded = store.load_manifest().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["src/lib.rs"].content_hash, "abc");
    }

    #[test]
    fn surrealdb_round_trips_embeddings() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = SurrealStore::open(temp.path()).unwrap();
        store.save_embeddings(&[StoredEmbedding {
            node_id: "n1".into(), provider: "test".into(), vector: vec![0.1, 0.2, 0.3],
        }]).unwrap();
        let loaded = store.load_embeddings("test").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["n1"], vec![0.1, 0.2, 0.3]);
    }
=======
    db.query(
        "DEFINE ANALYZER IF NOT EXISTS simple TOKENIZERS blank, class FILTERS lowercase;",
    )
    .await?;

    // Symbol table with all fields needed for graph storage
    db.query(
        "DEFINE TABLE IF NOT EXISTS symbol SCHEMAFULL;
        DEFINE FIELD node_id ON symbol TYPE string;
        DEFINE FIELD name ON symbol TYPE string;
        DEFINE FIELD qualified_name ON symbol TYPE string;
        DEFINE FIELD kind ON symbol TYPE string;
        DEFINE FIELD path ON symbol TYPE string;
        DEFINE FIELD language ON symbol TYPE string;
        DEFINE FIELD text ON symbol TYPE string;
        DEFINE FIELD start_line ON symbol TYPE int;
        DEFINE FIELD start_col ON symbol TYPE int;
        DEFINE FIELD end_line ON symbol TYPE int;
        DEFINE FIELD end_col ON symbol TYPE int;
        DEFINE FIELD tags_json ON symbol TYPE string;
        DEFINE FIELD metrics_json ON symbol TYPE string;
        DEFINE FIELD in_degree ON symbol TYPE int DEFAULT 0;
        DEFINE FIELD out_degree ON symbol TYPE int DEFAULT 0;
        DEFINE FIELD churn_90d ON symbol TYPE int DEFAULT 0;
        DEFINE FIELD is_test ON symbol TYPE bool DEFAULT false;",
    )
    .await?;

    // Indexes
    db.query("DEFINE INDEX IF NOT EXISTS idx_sym_name ON symbol FIELDS name;")
        .await?;
    db.query("DEFINE INDEX IF NOT EXISTS idx_sym_kind ON symbol FIELDS kind;")
        .await?;
    db.query("DEFINE INDEX IF NOT EXISTS idx_sym_node_id ON symbol FIELDS node_id;")
        .await?;

    // Full-text search index (SurrealDB v2 syntax)
    db.query(
        "DEFINE INDEX IF NOT EXISTS idx_fts ON symbol FIELDS text SEARCH ANALYZER simple BM25 HIGHLIGHTS;",
    )
    .await?;

    // Edge tables (TYPE RELATION for backward compat with existing mirror code)
    db.query(
        "DEFINE TABLE IF NOT EXISTS calls_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;
        DEFINE FIELD confidence ON calls_edge TYPE int DEFAULT 6500;",
    )
    .await?;
    db.query(
        "DEFINE TABLE IF NOT EXISTS extends_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;
        DEFINE FIELD confidence ON extends_edge TYPE int DEFAULT 9000;",
    )
    .await?;
    db.query(
        "DEFINE TABLE IF NOT EXISTS contains_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;",
    )
    .await?;
    db.query(
        "DEFINE TABLE IF NOT EXISTS defines_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;",
    )
    .await?;
    db.query(
        "DEFINE TABLE IF NOT EXISTS imports_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;",
    )
    .await?;
    db.query(
        "DEFINE TABLE IF NOT EXISTS references_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;",
    )
    .await?;

    // Flat graph edge table for our storage (simpler than RELATION)
    db.query(
        "DEFINE TABLE IF NOT EXISTS graph_edge SCHEMAFULL;
        DEFINE FIELD from_id ON graph_edge TYPE string;
        DEFINE FIELD to_id ON graph_edge TYPE string;
        DEFINE FIELD kind ON graph_edge TYPE string;
        DEFINE FIELD confidence ON graph_edge TYPE int;",
    )
    .await?;

    // Meta table for graph metadata (schema_version, indexed_at_unix, etc.)
    db.query(
        "DEFINE TABLE IF NOT EXISTS meta SCHEMAFULL;
        DEFINE FIELD key ON meta TYPE string;
        DEFINE FIELD value ON meta TYPE string;",
    )
    .await?;

    // File manifest
    db.query(
        "DEFINE TABLE IF NOT EXISTS file_manifest SCHEMAFULL;
        DEFINE FIELD path ON file_manifest TYPE string;
        DEFINE FIELD content_hash ON file_manifest TYPE string;
        DEFINE FIELD language ON file_manifest TYPE string;
        DEFINE FIELD indexed_at_unix ON file_manifest TYPE int;",
    )
    .await?;

    // Embeddings
    db.query(
        "DEFINE TABLE IF NOT EXISTS embedding SCHEMAFULL;
        DEFINE FIELD node_id ON embedding TYPE string;
        DEFINE FIELD provider ON embedding TYPE string;
        DEFINE FIELD vector ON embedding TYPE string;",
    )
    .await?;

    Ok(())
}

// -- Serde deserialization helpers for SurrealDB query results --

#[derive(Deserialize)]
struct MetaRow {
    key: String,
    value: String,
}

#[derive(Deserialize)]
struct SymbolRow {
    node_id: String,
    name: String,
    qualified_name: String,
    kind: String,
    path: String,
    language: Option<String>,
    text: String,
    start_line: Option<i64>,
    start_col: Option<i64>,
    end_line: Option<i64>,
    end_col: Option<i64>,
    tags_json: Option<String>,
    metrics_json: Option<String>,
    in_degree: Option<i64>,
    out_degree: Option<i64>,
    churn_90d: Option<i64>,
    is_test: Option<bool>,
}

#[derive(Deserialize)]
struct EdgeRow {
    from_id: String,
    to_id: String,
    kind: String,
    confidence: Option<i64>,
}

#[derive(Deserialize)]
struct ManifestRow {
    path: String,
    content_hash: String,
    language: String,
    indexed_at_unix: i64,
}

#[derive(Deserialize)]
struct EmbeddingRow {
    node_id: String,
    vector: String,
}

#[derive(Deserialize)]
struct FtsRow {
    node_id: String,
>>>>>>> origin/worktree-agent-a617ffcd
}
