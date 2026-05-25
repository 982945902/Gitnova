use anyhow::Result;
use std::path::{Path, PathBuf};
use surrealdb::engine::local::RocksDb;
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

        let db = if tokio::runtime::Handle::try_current().is_err() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let db = rt.block_on(async { connect_surrealdb(&db_dir).await })?;
            std::mem::forget(rt);
            db
        } else {
            // Already inside a runtime — skip SurrealDB (can't nest)
            return Err(anyhow::anyhow!("SurrealDB requires a dedicated thread"));
        };

        Ok(Self { db, repo_root })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

async fn connect_surrealdb(db_dir: &std::path::Path) -> Result<Db> {
    let db = Surreal::new::<RocksDb>(db_dir.to_string_lossy().as_ref()).await?;
    db.use_ns("gitnova").use_db("havenask").await?;
    ensure_schema(&db).await?;
    Ok(db)
}

async fn ensure_schema(db: &Db) -> Result<()> {
    db.query("DEFINE ANALYZER IF NOT EXISTS simple TOKENIZERS blank, class FILTERS lowercase;").await?;
    db.query("DEFINE TABLE IF NOT EXISTS symbol SCHEMAFULL;
        DEFINE FIELD name ON symbol TYPE string;
        DEFINE FIELD qualified_name ON symbol TYPE string;
        DEFINE FIELD kind ON symbol TYPE string;
        DEFINE FIELD path ON symbol TYPE string;
        DEFINE FIELD text ON symbol TYPE string;
        DEFINE FIELD in_degree ON symbol TYPE int DEFAULT 0;
        DEFINE FIELD out_degree ON symbol TYPE int DEFAULT 0;
        DEFINE FIELD churn_90d ON symbol TYPE int DEFAULT 0;
        DEFINE FIELD is_test ON symbol TYPE bool DEFAULT false;
    ").await?;
    db.query("DEFINE INDEX IF NOT EXISTS idx_sym_name ON symbol FIELDS name;").await?;
    db.query("DEFINE INDEX IF NOT EXISTS idx_sym_kind ON symbol FIELDS kind;").await?;

    // Full-text search index (SurrealDB v2 syntax)
    db.query("DEFINE INDEX IF NOT EXISTS idx_fts ON symbol FIELDS text SEARCH ANALYZER simple BM25 HIGHLIGHTS;").await?;

    // Edge tables
    db.query("DEFINE TABLE IF NOT EXISTS calls_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;
        DEFINE FIELD confidence ON calls_edge TYPE int DEFAULT 6500;
    ").await?;
    db.query("DEFINE TABLE IF NOT EXISTS extends_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;
        DEFINE FIELD confidence ON extends_edge TYPE int DEFAULT 9000;
    ").await?;
    db.query("DEFINE TABLE IF NOT EXISTS contains_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;").await?;
    db.query("DEFINE TABLE IF NOT EXISTS defines_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;").await?;
    db.query("DEFINE TABLE IF NOT EXISTS imports_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;").await?;
    db.query("DEFINE TABLE IF NOT EXISTS references_edge SCHEMAFULL TYPE RELATION IN symbol TO symbol;").await?;

    Ok(())
}
