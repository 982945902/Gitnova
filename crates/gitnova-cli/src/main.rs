use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use gitnova_core::{build_graph_from_entries, model::current_unix, query, scan_repository};
use gitnova_enrich::embeddings::{self, LOCAL_HASH_PROVIDER};
use gitnova_enrich::git::apply_git_churn;
use gitnova_rank::{diff, rank_graph_with_embeddings};
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
    #[command(name = "impact-analysis")]
    ImpactAnalysis(ImpactArgs),
    #[command(name = "architecture-map")]
    ArchitectureMap(ArchitectureArgs),
    #[command(name = "diff-context")]
    DiffContext(DiffArgs),
    Embeddings(EmbeddingsArgs),
    Dashboard(DashboardArgs),
    Serve,
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
            let graph = GitnovaStore::open(&args.repo)?.load_graph()?;
            print_json(&query::summarize(&graph))?;
        }
        Commands::RankContext(args) => {
            let graph = GitnovaStore::open(&args.repo)?.load_graph()?;
            let vectors = GitnovaStore::open(&args.repo)?.load_embeddings(LOCAL_HASH_PROVIDER)?;
            let similarities = if vectors.is_empty() {
                None
            } else {
                Some(embeddings::similarity_map(&graph, &vectors, &args.query))
            };
            print_json(&rank_graph_with_embeddings(
                &graph,
                &args.query,
                args.limit,
                similarities.as_ref(),
            ))?;
        }
        Commands::ExplainSymbol(args) => {
            let graph = GitnovaStore::open(&args.repo)?.load_graph()?;
            print_json(&query::explain_symbol(&graph, &args.symbol))?;
        }
        Commands::ImpactAnalysis(args) => {
            let graph = GitnovaStore::open(&args.repo)?.load_graph()?;
            print_json(&query::impact_analysis(&graph, &args.symbol, args.limit))?;
        }
        Commands::ArchitectureMap(args) => {
            let graph = GitnovaStore::open(&args.repo)?.load_graph()?;
            print_json(&query::architecture_map(&graph, args.focus.as_deref()))?;
        }
        Commands::DiffContext(args) => {
            let graph = GitnovaStore::open(&args.repo)?.load_graph()?;
            print_json(&diff::diff_context(
                &graph, &args.repo, &args.base, args.limit,
            ))?;
        }
        Commands::Embeddings(args) => match args.command {
            EmbeddingCommands::Build(build) => {
                if build.provider != LOCAL_HASH_PROVIDER {
                    anyhow::bail!("unsupported embedding provider: {}", build.provider);
                }
                let graph = GitnovaStore::open(&build.repo)?.load_graph()?;
                let embeddings = embeddings::build_local_hash_embeddings(&graph);
                let count = embeddings.len();
                GitnovaStore::open(&build.repo)?.save_embeddings(&embeddings)?;
                print_json(&json!({
                    "status": "built",
                    "provider": LOCAL_HASH_PROVIDER,
                    "count": count
                }))?;
            }
        },
        Commands::Dashboard(args) => {
            gitnova_dashboard::run_dashboard(args.repo, args.port).await?;
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
    let mut store = GitnovaStore::open(repo)?;
    store.save_graph(&graph)?;
    store.export_json(&graph)?;
    save_manifest_from_files(&mut store, files)?;
    Ok(IndexReport {
        status: "indexed",
        summary: query::summarize(&graph),
    })
}

fn update_repo(repo: &Path) -> Result<UpdateReport> {
    let files = scan_repository(repo)?;
    let mut store = GitnovaStore::open(repo)?;
    let old = store.load_manifest().unwrap_or_default();
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
    store.save_graph(&graph)?;
    store.export_json(&graph)?;
    save_manifest_from_files(&mut store, files)?;
    Ok(UpdateReport {
        status: "updated",
        skipped,
        changed,
        deleted,
        summary: query::summarize(&graph),
    })
}

fn save_manifest_from_files(
    store: &mut GitnovaStore,
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

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
