use crate::sqlite::{FileManifestEntry, StoredEmbedding};
use anyhow::{Context, Result};
use gitnova_core::model::{
    current_unix, CodeGraph, Edge, EdgeKind, Language, Node, NodeKind, NodeMetrics,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use surrealdb::engine::local::SurrealKv;
use surrealdb::Surreal;

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

        Ok(Self { db, repo_root, rt })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

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
    }

    pub fn load_manifest(&self) -> Result<HashMap<String, FileManifestEntry>> {
        self.block_on(async {
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
            }
            Ok(map)
        })
    }

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
            }
            Ok(map)
        })
    }
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
    db.use_ns("gitnova").use_db("havenask").await?;
    ensure_schema(&db).await?;
    Ok(db)
}

async fn ensure_schema(db: &Db) -> Result<()> {
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
}
