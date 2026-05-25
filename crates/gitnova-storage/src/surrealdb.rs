use anyhow::Result;
use gitnova_core::model::{
    current_unix, CodeGraph, Edge, EdgeKind, Language, Node, NodeKind, NodeMetrics, Span,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use surrealdb::engine::local::RocksDb;
use surrealdb::Surreal;

use crate::json_export;
use crate::{FileManifestEntry, StoredEmbedding};

pub type Db = Surreal<surrealdb::engine::local::Db>;

pub struct SurrealStore {
    pub db: Db,
    repo_root: PathBuf,
}

impl SurrealStore {
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let repo_root = repo_root.as_ref().to_path_buf();
        let db_dir = repo_root.join(".gitnova").join("surrealdb");
        std::fs::create_dir_all(&db_dir)?;

        let db = run_async(async { connect_surrealdb(&db_dir).await })?;

        Ok(Self { db, repo_root })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

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
    }

    pub fn load_manifest(&self) -> Result<HashMap<String, FileManifestEntry>> {
        self.block_on(async {
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
            }
            Ok(map)
        })
    }

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
            }
            Ok(map)
        })
    }

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
    db.use_ns("gitnova").use_db("havenask").await?;
    ensure_schema(&db).await?;
    Ok(db)
}

async fn ensure_schema(db: &Db) -> Result<()> {
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
}
