use crate::json_export;
use crate::types::{FileManifestEntry, StoredEmbedding};
use anyhow::{Context, Result};
use gitnova_core::model::{CodeGraph, NodeKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use surrealdb::engine::local::SurrealKv;
use surrealdb::Surreal;

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

        // SurrealDB needs a dedicated thread. If we're inside an existing runtime,
        // we can't create a nested one. Fall back gracefully.
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(anyhow::anyhow!("SurrealDB: nested runtime not supported"));
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let db = rt.block_on(async { connect(&db_dir).await })?;
        std::mem::forget(rt);
        Ok(Self { db, repo_root })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
    pub fn json_path(&self) -> PathBuf {
        self.repo_root.join(".gitnova/index.json")
    }

    // ── Graph persistence ──

    pub fn save_graph(&self, graph: &CodeGraph) -> Result<()> {
        // Always export index.json as reliable backup
        json_export::export_graph(graph, &self.json_path())?;

        // Write full graph as single JSON blob to SurrealDB (fast, atomic)
        let graph_json = serde_json::to_string(graph)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(async {
            let _ = self.db.query("DELETE FROM g_meta WHERE id = 'codegraph'").await;
            let escaped = graph_json.replace('\\', "\\\\").replace('\'', "\\'");
            self.db.query(format!(
                "CREATE g_meta CONTENT {{ id: 'codegraph', value: '{}' }}", escaped
            )).await?;

            // Also write individual symbols for FTS (best-effort, batch INSERT)
            let fts_entries: Vec<serde_json::Value> = graph.nodes.iter()
                .filter(|n| !matches!(n.kind, NodeKind::Repository | NodeKind::Import))
                .map(|n| serde_json::json!({
                    "id": n.id,
                    "name": n.name,
                    "qualified_name": n.qualified_name,
                    "kind": serde_json::to_string(&n.kind).unwrap_or_default().trim_matches('"').to_string(),
                    "path": n.path,
                    "text": n.text,
                }))
                .collect();

            // Batch INSERT in chunks of 500
            for chunk in fts_entries.chunks(500) {
                let arr = serde_json::to_string(chunk)?;
                let _ = self.db.query(format!("INSERT INTO symbol {}", arr)).await;
            }

            Ok::<_, anyhow::Error>(())
        })?;
        std::mem::forget(rt);
        Ok(())
    }

    pub fn load_graph(&self) -> Result<CodeGraph> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let graph: Option<CodeGraph> = rt.block_on(async {
            if let Ok(mut result) = self
                .db
                .query("SELECT value FROM g_meta WHERE id = 'codegraph'")
                .await
            {
                if let Ok(rows) = result.take::<Vec<serde_json::Value>>(0) {
                    if let Some(v) = rows.first().and_then(|r| r["value"].as_str()) {
                        return serde_json::from_str(v).ok();
                    }
                }
            }
            None
        });

        if let Some(g) = graph {
            std::mem::forget(rt);
            return Ok(g);
        }

        // Fallback: load from index.json
        drop(rt);
        json_export::import_graph(&self.json_path())
            .context("No graph data in SurrealDB or index.json")
    }

    pub fn export_json(&self, graph: &CodeGraph) -> Result<()> {
        json_export::export_graph(graph, &self.json_path())
    }

    // ── FTS search ──

    pub fn search_fts(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let ids: Vec<String> = rt.block_on(async {
            let q = query.replace('\'', "\\'");
            let sql = format!(
                "SELECT id, search::score(0) AS score FROM symbol WHERE text @@ '{}' ORDER BY score DESC LIMIT {}",
                q, limit
            );
            match self.db.query(&sql).await {
                Ok(mut result) => {
                    match result.take::<Vec<serde_json::Value>>(0) {
                        Ok(rows) => rows.iter()
                            .filter_map(|r| r["id"].as_str().map(|s| s.to_string()))
                            .collect(),
                        Err(_) => Vec::new(),
                    }
                }
                Err(_) => Vec::new(),
            }
        });
        std::mem::forget(rt);
        Ok(ids)
    }

    // ── File manifest ──

    pub fn save_manifest(&self, entries: &[FileManifestEntry]) -> Result<()> {
        let entries_json = serde_json::to_string(entries)?;
        let escaped = entries_json.replace('\\', "\\\\").replace('\'', "\\'");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(async {
            let _ = self
                .db
                .query("DELETE FROM g_meta WHERE id = 'file_manifest'")
                .await;
            self.db
                .query(format!(
                    "CREATE g_meta CONTENT {{ id: 'file_manifest', value: '{}' }}",
                    escaped
                ))
                .await?;
            Ok::<_, anyhow::Error>(())
        })?;
        std::mem::forget(rt);
        Ok(())
    }

    pub fn load_manifest(&self) -> Result<HashMap<String, FileManifestEntry>> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let map = rt.block_on(async {
            if let Ok(mut result) = self
                .db
                .query("SELECT value FROM g_meta WHERE id = 'file_manifest'")
                .await
            {
                if let Ok(rows) = result.take::<Vec<serde_json::Value>>(0) {
                    if let Some(v) = rows.first().and_then(|r| r["value"].as_str()) {
                        if let Ok(entries) = serde_json::from_str::<Vec<FileManifestEntry>>(v) {
                            return entries.into_iter().map(|e| (e.path.clone(), e)).collect();
                        }
                    }
                }
            }
            HashMap::new()
        });
        std::mem::forget(rt);
        Ok(map)
    }

    // ── Embeddings ──

    pub fn save_embeddings(&self, embeddings: &[StoredEmbedding]) -> Result<()> {
        let emb_json = serde_json::to_string(embeddings)?;
        let escaped = emb_json.replace('\\', "\\\\").replace('\'', "\\'");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(async {
            let _ = self
                .db
                .query("DELETE FROM g_meta WHERE id = 'embeddings'")
                .await;
            self.db
                .query(format!(
                    "CREATE g_meta CONTENT {{ id: 'embeddings', value: '{}' }}",
                    escaped
                ))
                .await?;
            Ok::<_, anyhow::Error>(())
        })?;
        std::mem::forget(rt);
        Ok(())
    }

    pub fn load_embeddings(&self, provider: &str) -> Result<HashMap<String, Vec<f32>>> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let map = rt.block_on(async {
            if let Ok(mut result) = self
                .db
                .query("SELECT value FROM g_meta WHERE id = 'embeddings'")
                .await
            {
                if let Ok(rows) = result.take::<Vec<serde_json::Value>>(0) {
                    if let Some(v) = rows.first().and_then(|r| r["value"].as_str()) {
                        if let Ok(all) = serde_json::from_str::<Vec<StoredEmbedding>>(v) {
                            return all
                                .into_iter()
                                .filter(|e| e.provider == provider)
                                .map(|e| (e.node_id.clone(), e.vector.clone()))
                                .collect();
                        }
                    }
                }
            }
            HashMap::new()
        });
        std::mem::forget(rt);
        Ok(map)
    }
}

async fn connect(db_dir: &Path) -> Result<Db> {
    let db = Surreal::new::<SurrealKv>(db_dir.to_string_lossy().as_ref()).await?;
    db.use_ns("gitnova").use_db("havenask").await?;
    ensure_schema(&db).await?;
    Ok(db)
}

async fn ensure_schema(db: &Db) -> Result<()> {
    db.query("DEFINE ANALYZER IF NOT EXISTS simple TOKENIZERS blank, class FILTERS lowercase;")
        .await?;
    db.query("DEFINE TABLE IF NOT EXISTS symbol SCHEMALESS;")
        .await?;
    db.query("DEFINE INDEX IF NOT EXISTS idx_fts ON symbol FIELDS text SEARCH ANALYZER simple BM25 HIGHLIGHTS;").await?;
    db.query("DEFINE TABLE IF NOT EXISTS g_meta SCHEMALESS;")
        .await?;
    Ok(())
}
