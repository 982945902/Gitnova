use crate::json_export;
use crate::migrations;
use anyhow::Result;
use gitnova_core::model::{
    current_unix, CodeGraph, Edge, EdgeKind, Language, Node, NodeKind, Span,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileManifestEntry {
    pub path: String,
    pub content_hash: String,
    pub language: Language,
    pub indexed_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEmbedding {
    pub node_id: String,
    pub provider: String,
    pub vector: Vec<f32>,
}

pub struct GitnovaStore {
    conn: Connection,
    repo_root: PathBuf,
}

impl GitnovaStore {
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let repo_root = repo_root.as_ref().to_path_buf();
        let dir = repo_root.join(".gitnova");
        fs::create_dir_all(&dir)?;
        let conn = Connection::open(dir.join("gitnova.db"))?;
        migrations::run(&conn)?;
        Ok(Self { conn, repo_root })
    }

    pub fn db_path(repo_root: impl AsRef<Path>) -> PathBuf {
        repo_root.as_ref().join(".gitnova/gitnova.db")
    }

    pub fn json_path(&self) -> PathBuf {
        self.repo_root.join(".gitnova/index.json")
    }

    pub fn save_graph(&mut self, graph: &CodeGraph) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM nodes", [])?;
        tx.execute("DELETE FROM edges", [])?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('schema_version', ?1)",
            [graph.schema_version.to_string()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('repo_root', ?1)",
            [&graph.repo_root],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES('indexed_at_unix', ?1)",
            [graph.indexed_at_unix.to_string()],
        )?;
        for node in &graph.nodes {
            let (start_line, start_col, end_line, end_col) = if let Some(span) = node.span {
                (
                    Some(span.start_line as i64),
                    Some(span.start_col as i64),
                    Some(span.end_line as i64),
                    Some(span.end_col as i64),
                )
            } else {
                (None, None, None, None)
            };
            tx.execute(
                r#"INSERT INTO nodes(
                    id, kind, name, qualified_name, path, language, text,
                    start_line, start_col, end_line, end_col, tags_json, metrics_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"#,
                params![
                    node.id,
                    serde_json::to_string(&node.kind)?,
                    node.name,
                    node.qualified_name,
                    node.path,
                    node.language
                        .map(|language| serde_json::to_string(&language))
                        .transpose()?,
                    node.text,
                    start_line,
                    start_col,
                    end_line,
                    end_col,
                    serde_json::to_string(&node.tags)?,
                    serde_json::to_string(&node.metrics)?,
                ],
            )?;
        }
        for edge in &graph.edges {
            tx.execute(
                "INSERT OR REPLACE INTO edges(from_id, to_id, kind, confidence_basis_points) VALUES(?1, ?2, ?3, ?4)",
                params![
                    edge.from,
                    edge.to,
                    serde_json::to_string(&edge.kind)?,
                    edge.confidence_basis_points as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_graph(&self) -> Result<CodeGraph> {
        let schema_version: u32 = self
            .meta("schema_version")?
            .unwrap_or_else(|| "1".into())
            .parse()
            .unwrap_or(1);
        let repo_root = self
            .meta("repo_root")?
            .unwrap_or_else(|| self.repo_root.to_string_lossy().to_string());
        let indexed_at_unix = self
            .meta("indexed_at_unix")?
            .unwrap_or_else(|| current_unix().to_string())
            .parse()
            .unwrap_or_else(|_| current_unix());
        let mut stmt = self.conn.prepare(
            r#"SELECT id, kind, name, qualified_name, path, language, text,
                      start_line, start_col, end_line, end_col, tags_json, metrics_json
               FROM nodes ORDER BY id"#,
        )?;
        let nodes = stmt
            .query_map([], |row| {
                let kind_json: String = row.get(1)?;
                let language_json: Option<String> = row.get(5)?;
                let start_line: Option<i64> = row.get(7)?;
                let span = start_line.map(|start_line| Span {
                    start_line: start_line as usize,
                    start_col: row.get::<_, Option<i64>>(8).unwrap_or(Some(1)).unwrap_or(1)
                        as usize,
                    end_line: row
                        .get::<_, Option<i64>>(9)
                        .unwrap_or(Some(start_line))
                        .unwrap_or(start_line) as usize,
                    end_col: row
                        .get::<_, Option<i64>>(10)
                        .unwrap_or(Some(1))
                        .unwrap_or(1) as usize,
                });
                let tags_json: String = row.get(11)?;
                let metrics_json: String = row.get(12)?;
                Ok(Node {
                    id: row.get(0)?,
                    kind: serde_json::from_str::<NodeKind>(&kind_json).unwrap_or(NodeKind::Unknown),
                    name: row.get(2)?,
                    qualified_name: row.get(3)?,
                    path: row.get(4)?,
                    language: language_json
                        .as_deref()
                        .and_then(|json| serde_json::from_str::<Language>(json).ok()),
                    text: row.get(6)?,
                    span,
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    metrics: serde_json::from_str(&metrics_json).unwrap_or_default(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut stmt = self
            .conn
            .prepare("SELECT from_id, to_id, kind, confidence_basis_points FROM edges ORDER BY from_id, to_id")?;
        let edges = stmt
            .query_map([], |row| {
                let kind_json: String = row.get(2)?;
                Ok(Edge {
                    from: row.get(0)?,
                    to: row.get(1)?,
                    kind: serde_json::from_str::<EdgeKind>(&kind_json)
                        .unwrap_or(EdgeKind::References),
                    confidence_basis_points: row.get::<_, i64>(3)? as u16,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(CodeGraph {
            schema_version,
            repo_root,
            indexed_at_unix,
            nodes,
            edges,
        })
    }

    pub fn export_json(&self, graph: &CodeGraph) -> Result<()> {
        json_export::export_graph(graph, self.json_path())
    }

    pub fn save_manifest(&mut self, entries: &[FileManifestEntry]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM file_manifest", [])?;
        for entry in entries {
            tx.execute(
                "INSERT OR REPLACE INTO file_manifest(path, content_hash, language, indexed_at_unix) VALUES(?1, ?2, ?3, ?4)",
                params![
                    entry.path,
                    entry.content_hash,
                    serde_json::to_string(&entry.language)?,
                    entry.indexed_at_unix as i64,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_manifest(&self) -> Result<HashMap<String, FileManifestEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, content_hash, language, indexed_at_unix FROM file_manifest")?;
        let rows = stmt
            .query_map([], |row| {
                let language_json: String = row.get(2)?;
                let path: String = row.get(0)?;
                Ok((
                    path.clone(),
                    FileManifestEntry {
                        path,
                        content_hash: row.get(1)?,
                        language: serde_json::from_str::<Language>(&language_json)
                            .unwrap_or(Language::Rust),
                        indexed_at_unix: row.get::<_, i64>(3)? as u64,
                    },
                ))
            })?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(rows)
    }

    pub fn save_embeddings(&mut self, embeddings: &[StoredEmbedding]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for embedding in embeddings {
            tx.execute(
                "INSERT OR REPLACE INTO embeddings(node_id, provider, dimension, vector_json) VALUES(?1, ?2, ?3, ?4)",
                params![
                    embedding.node_id,
                    embedding.provider,
                    embedding.vector.len() as i64,
                    serde_json::to_string(&embedding.vector)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_embeddings(&self, provider: &str) -> Result<HashMap<String, Vec<f32>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT node_id, vector_json FROM embeddings WHERE provider = ?1")?;
        let rows = stmt
            .query_map([provider], |row| {
                let vector_json: String = row.get(1)?;
                Ok((
                    row.get::<_, String>(0)?,
                    serde_json::from_str::<Vec<f32>>(&vector_json).unwrap_or_default(),
                ))
            })?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(rows)
    }

    fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::{EdgeKind, NodeKind, NodeMetrics};

    #[test]
    fn sqlite_round_trips_graph_and_exports_json() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut graph = CodeGraph::empty(temp.path().to_string_lossy().to_string());
        graph.nodes.push(Node {
            id: "n1".into(),
            kind: NodeKind::Repository,
            name: "repo".into(),
            qualified_name: "repo".into(),
            path: String::new(),
            span: None,
            language: None,
            text: String::new(),
            tags: vec![],
            metrics: NodeMetrics::default(),
        });
        graph.edges.push(Edge {
            from: "n1".into(),
            to: "n2".into(),
            kind: EdgeKind::Contains,
            confidence_basis_points: 10_000,
        });
        let mut store = GitnovaStore::open(temp.path()).unwrap();
        store.save_graph(&graph).unwrap();
        store.export_json(&graph).unwrap();
        let loaded = store.load_graph().unwrap();
        let imported = crate::json_export::import_graph(store.json_path()).unwrap();
        assert_eq!(loaded.nodes, graph.nodes);
        assert_eq!(imported.nodes, graph.nodes);
        assert!(temp.path().join(".gitnova/index.json").exists());
    }
}
