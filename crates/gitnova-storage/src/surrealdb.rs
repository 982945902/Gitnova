use anyhow::Result;
use std::path::{Path, PathBuf};
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::doc;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::tokenizer::*;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument};

pub struct FtsStore {
    index: Index,
    reader: IndexReader,
    schema: Schema,
    text_field: Field,
    searchable_field: Field,
    id_field: Field,
    name_field: Field,
    qname_field: Field,
    kind_field: Field,
    path_field: Field,
    repo_root: PathBuf,
}

impl FtsStore {
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let repo_root = repo_root.as_ref().to_path_buf();
        let index_dir = repo_root.join(".gitnova").join("fts_index");
        std::fs::create_dir_all(&index_dir)?;

        let mut schema_builder = Schema::builder();
        // Combined searchable field: name + qualified_name + source text
        let text_field = schema_builder.add_text_field("text", TEXT | STORED);
        let id_field = schema_builder.add_text_field("node_id", STRING | STORED);
        let name_field = schema_builder.add_text_field("name", STRING | STORED);
        let qname_field = schema_builder.add_text_field("qualified_name", STRING | STORED);
        let kind_field = schema_builder.add_text_field("kind", STRING | STORED);
        let path_field = schema_builder.add_text_field("path", STRING | STORED);
        // Separate field for name+qname text (used for FTS with higher weight)
        let searchable_field = schema_builder.add_text_field("searchable", TEXT);
        let schema = schema_builder.build();

        // Always create fresh index to avoid schema mismatch issues
        if index_dir.exists() {
            std::fs::remove_dir_all(&index_dir)?;
        }
        let index = Index::create_in_dir(&index_dir, schema.clone())?;
        let tokenizer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(LowerCaser)
            .build();
        index.tokenizers().register("gitnova_en", tokenizer);

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            schema,
            text_field,
            searchable_field,
            id_field,
            name_field,
            qname_field,
            kind_field,
            path_field,
            repo_root,
        })
    }

    pub fn rebuild_index(&self, symbols: &[FtsSymbol]) -> Result<()> {
        let mut writer: IndexWriter = self.index.writer(50_000_000)?;
        writer.delete_all_documents()?;

        for sym in symbols {
            let searchable = format!("{} {} {} {}", sym.name, sym.qualified_name, sym.kind, sym.text);
            writer.add_document(doc!(
                self.id_field => sym.node_id.as_str(),
                self.name_field => sym.name.as_str(),
                self.qname_field => sym.qualified_name.as_str(),
                self.kind_field => sym.kind.as_str(),
                self.path_field => sym.path.as_str(),
                self.text_field => sym.text.as_str(),
                self.searchable_field => searchable.as_str(),
            ))?;
        }

        writer.commit()?;
        Ok(())
    }

    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<String>> {
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.searchable_field]);
        let query = query_parser.parse_query(query_str)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut ids = Vec::new();
        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            if let Some(id_val) = doc.get_first(self.id_field) {
                if let Some(text) = id_val.as_str() {
                    ids.push(text.to_string());
                }
            }
        }
        Ok(ids)
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

#[derive(Debug, Clone)]
pub struct FtsSymbol {
    pub node_id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub path: String,
    pub text: String,
}
