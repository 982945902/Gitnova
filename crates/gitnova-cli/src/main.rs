use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use gitnova_core::{build_graph_from_entries, model::current_unix, query, scan_repository};
use gitnova_enrich::embeddings::{self, LOCAL_HASH_PROVIDER};
use gitnova_enrich::git::apply_git_churn;
use gitnova_enrich::lsp::apply_lsp_metadata;
use gitnova_rank::{diff, rank_graph_with_fts};
use gitnova_storage::{FileManifestEntry, GitnovaStore};
use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;

#[derive(Parser)]
#[command(name = "gitnova", version, about = "Local-first MCP code intelligence")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Index(IndexArgs),
    Update(RepoArgs),
    Watch(RepoArgs),
    Stats(RepoArgs),
    #[command(name = "rank-context")]
    RankContext(RankArgs),
    #[command(name = "explain-symbol")]
    ExplainSymbol(SymbolArgs),
    #[command(name = "graph-context")]
    GraphContext(GraphContextArgs),
    #[command(name = "impact-analysis")]
    ImpactAnalysis(ImpactArgs),
    #[command(name = "architecture-map")]
    ArchitectureMap(ArchitectureArgs),
    #[command(name = "diff-context")]
    DiffContext(DiffArgs),
    Embeddings(EmbeddingsArgs),
    Dashboard(DashboardArgs),
    Serve,
    #[command(name = "train")]
    Train(TrainArgs),
    #[command(name = "retrieve-graph")]
    RetrieveGraph(RetrieveGraphArgs),
}

#[derive(Args)]
struct IndexArgs {
    path: PathBuf,
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct RepoArgs {
    #[arg(long)]
    repo: PathBuf,
}

#[derive(Args)]
struct RankArgs {
    query: String,
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

#[derive(Args)]
struct SymbolArgs {
    symbol: String,
    #[arg(long)]
    repo: PathBuf,
}

#[derive(Args)]
struct GraphContextArgs {
    selector: String,
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value_t = 1)]
    depth: usize,
    #[arg(long, default_value_t = 40)]
    limit: usize,
}

#[derive(Args)]
struct ImpactArgs {
    symbol: String,
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Args)]
struct ArchitectureArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    focus: Option<String>,
}

#[derive(Args)]
struct DiffArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = "main")]
    base: String,
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

#[derive(Args)]
struct DashboardArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value_t = 4567)]
    port: u16,
}

#[derive(Args)]
struct EmbeddingsArgs {
    #[command(subcommand)]
    command: EmbeddingCommands,
}

#[derive(Subcommand)]
enum EmbeddingCommands {
    Build(EmbeddingBuildArgs),
}

#[derive(Args)]
struct EmbeddingBuildArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = LOCAL_HASH_PROVIDER)]
    provider: String,
}

#[derive(Args)]
struct TrainArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = "local-hash")]
    provider: String,
    #[arg(long, default_value_t = 50)]
    epochs: usize,
    #[arg(long)]
    model: Option<PathBuf>,
}

#[derive(Args)]
struct RetrieveGraphArgs {
    query: String,
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value_t = 10)]
    limit: usize,
    #[arg(long)]
    model: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct IndexReport {
    status: &'static str,
    summary: query::GraphSummary,
}

#[derive(Debug, Serialize)]
struct UpdateReport {
    status: &'static str,
    skipped: usize,
    changed: usize,
    deleted: usize,
    summary: query::GraphSummary,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Index(args) => print_json(&index_repo(&args.path, args.force)?)?,
        Commands::Update(args) => print_json(&update_repo(&args.repo)?)?,
        Commands::Watch(args) => watch_repo(&args.repo)?,
        Commands::Stats(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            print_json(&query::summarize(&graph))?;
        }
        Commands::RankContext(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            let vectors = GitnovaStore::open(&args.repo)
                .ok()
                .and_then(|s| s.load_embeddings(LOCAL_HASH_PROVIDER).ok())
                .unwrap_or_default();
            let similarities = if vectors.is_empty() {
                None
            } else {
                Some(embeddings::similarity_map(&graph, &vectors, &args.query))
            };
            // FTS pre-filter: narrow candidates via full-text search
            let fts_candidates = GitnovaStore::open(&args.repo)
                .ok()
                .and_then(|store| store.search_fts(&args.query, 200).ok())
                .map(|ids| ids.into_iter().collect::<HashSet<_>>());
            print_json(&rank_graph_with_fts(
                &graph,
                &args.query,
                args.limit,
                similarities.as_ref(),
                fts_candidates.as_ref(),
            ))?;
        }
        Commands::ExplainSymbol(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            print_json(&query::explain_symbol(&graph, &args.symbol))?;
        }
        Commands::GraphContext(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            print_json(&query::graph_context(
                &graph,
                &args.selector,
                args.depth,
                args.limit,
            ))?;
        }
        Commands::ImpactAnalysis(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            print_json(&query::impact_analysis(&graph, &args.symbol, args.limit))?;
        }
        Commands::ArchitectureMap(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            print_json(&query::architecture_map(&graph, args.focus.as_deref()))?;
        }
        Commands::DiffContext(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            print_json(&diff::diff_context(
                &graph, &args.repo, &args.base, args.limit,
            ))?;
        }
        Commands::Embeddings(args) => match args.command {
            EmbeddingCommands::Build(build) => {
                let graph = load_graph_with_fallback(&build.repo)?;
                let embeddings = embeddings::build_embeddings(&graph, &build.provider)?;
                let count = embeddings.len();
                if let Ok(store) = GitnovaStore::open(&build.repo) { let _ = store.save_embeddings(&embeddings); }
                print_json(&json!({
                    "status": "built",
                    "provider": build.provider,
                    "count": count
                }))?;
            }
        },
        Commands::Dashboard(args) => {
            gitnova_dashboard::run_dashboard(args.repo, args.port).await?;
        }
        Commands::Train(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            let store = GitnovaStore::open(&args.repo)?;
            let embeddings = store.load_embeddings(&args.provider).unwrap_or_default();

            if embeddings.is_empty() {
                eprintln!("No embeddings found for provider '{}'. Build them first with: gitnova embeddings build --repo <path>", args.provider);
                std::process::exit(1);
            }

            let examples = gitnova_train::data::generate_training_examples(
                &graph, &embeddings, &args.provider, 2000,
            )?;

            if examples.is_empty() {
                eprintln!("No training examples could be generated from the graph. Ensure the repo has been indexed.");
                std::process::exit(1);
            }

            eprintln!("Generated {} training examples", examples.len());

            let dim = embeddings::embedding_dim(&args.provider);
            let model_config = gitnova_train::ModelConfig {
                input_dim: dim,
                hidden_dim: dim.max(128),
                ..Default::default()
            };
            let train_config = gitnova_train::training::TrainConfig {
                epochs: args.epochs,
                ..Default::default()
            };

            let save_path = args
                .model
                .unwrap_or_else(|| args.repo.join(".gitnova/model/model.safetensors"));

            gitnova_train::train_and_save(
                &examples, &graph, &embeddings,
                &model_config, &train_config, &save_path,
            )?;

            eprintln!("Model saved to {}", save_path.display());
        }
        Commands::RetrieveGraph(args) => {
            let graph = load_graph_with_fallback(&args.repo)?;
            let store = GitnovaStore::open(&args.repo)?;
            let embeddings = store.load_embeddings("local-hash").unwrap_or_default();

            let model_path = args
                .model
                .unwrap_or_else(|| args.repo.join(".gitnova/model/model.safetensors"));

            if !model_path.exists() {
                eprintln!(
                    "No trained model found at {}. Train one with: gitnova train --repo <path>",
                    model_path.display()
                );
                eprintln!("Falling back to dense embedding retrieval.");
                let results = embeddings::search_embeddings(
                    &graph, &embeddings, &args.query, args.limit,
                );
                print_json(&results)?;
                return Ok(());
            }

            let dim = embeddings.values().next().map(|v| v.len()).unwrap_or(64);
            let model_config = gitnova_train::ModelConfig {
                input_dim: dim,
                hidden_dim: dim.max(128),
                ..Default::default()
            };
            let retriever = gitnova_train::SeedERRetriever::load(
                &model_path, &model_config, Default::default(),
            )?;

            let query_vec = embeddings::embed_text(&args.query);
            let results = retriever.retrieve(&graph, &query_vec, &embeddings, args.limit);
            print_json(&results)?;
        }
        Commands::Serve => {
            gitnova_mcp::serve_stdio().await?;
        }
    }
    Ok(())
}

fn index_repo(repo: &Path, _force: bool) -> Result<IndexReport> {
    let files = scan_repository(repo)?;
    let mut graph = build_graph_from_entries(repo, &files)?;
    apply_git_churn(repo, &mut graph)?;
    apply_lsp_metadata(&mut graph);

    // Always export index.json as reliable backup
    gitnova_storage::json_export::export_graph(&graph, &repo.join(".gitnova/index.json"))?;

    // Try SurrealDB persistence (best-effort, non-fatal in nested runtime contexts)
    if let Ok(store) = GitnovaStore::open(repo) {
        let _ = store.save_graph(&graph);
        let _ = store.export_json(&graph);
        let _ = save_manifest_from_files(&store, files);
    }

    Ok(IndexReport {
        status: "indexed",
        summary: query::summarize(&graph),
    })
}

fn update_repo(repo: &Path) -> Result<UpdateReport> {
    let files = scan_repository(repo)?;
    let store = GitnovaStore::open(repo);
    let old = store.as_ref().map(|s| s.load_manifest().unwrap_or_default()).unwrap_or_default();
    let current_paths: HashSet<_> = files
        .iter()
        .map(|file| file.relative_path.clone())
        .collect();
    let skipped = files
        .iter()
        .filter(|file| {
            old.get(&file.relative_path)
                .map(|entry| entry.content_hash == file.content_hash)
                .unwrap_or(false)
        })
        .count();
    let changed = files.len().saturating_sub(skipped);
    let deleted = old
        .keys()
        .filter(|path| !current_paths.contains(*path))
        .count();
    let mut graph = build_graph_from_entries(repo, &files)?;
    apply_git_churn(repo, &mut graph)?;
    apply_lsp_metadata(&mut graph);
    gitnova_storage::json_export::export_graph(&graph, &repo.join(".gitnova/index.json"))?;
    if let Ok(ref s) = store {
        let _ = s.save_graph(&graph);
        let _ = s.export_json(&graph);
        let _ = save_manifest_from_files(s, files);
    }
    Ok(UpdateReport {
        status: "updated",
        skipped,
        changed,
        deleted,
        summary: query::summarize(&graph),
    })
}

fn save_manifest_from_files(
    store: &GitnovaStore,
    files: Vec<gitnova_core::scan::SourceFile>,
) -> Result<()> {
    let now = current_unix();
    let manifest = files
        .into_iter()
        .map(|file| FileManifestEntry {
            path: file.relative_path,
            content_hash: file.content_hash,
            language: file.language,
            indexed_at_unix: now,
        })
        .collect::<Vec<_>>();
    store.save_manifest(&manifest)
}

fn watch_repo(repo: &Path) -> Result<()> {
    let repo = repo.to_path_buf();
    let (tx, rx) = channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    watcher.watch(&repo, RecursiveMode::Recursive)?;
    let _ = update_repo(&repo);
    for event in rx {
        match event {
            Ok(_) => {
                let _ = update_repo(&repo);
            }
            Err(err) => eprintln!("watch error: {err}"),
        }
    }
    Ok(())
}

fn load_graph_with_fallback(repo: &Path) -> Result<gitnova_core::CodeGraph> {
    if let Ok(store) = GitnovaStore::open(repo) {
        if let Ok(graph) = store.load_graph() {
            return Ok(graph);
        }
    }
    // Fallback: load from index.json
    gitnova_storage::json_export::import_graph(&repo.join(".gitnova/index.json"))
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
