use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use sifs::{
    CacheConfig, IndexOptions, ModelLoadPolicy, ModelOptions, SearchMode, SearchOptions, SifsIndex,
    cache_summary, format_results, is_git_url, load_model_with_options, model_status,
    platform_cache_root, resolve_chunk,
};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "sifs",
    about = "SIFS Is Fast Search: instant local code search for agents."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(help = "Local directory or git URL to pre-index for MCP server mode.")]
    path: Option<String>,
    #[arg(long = "ref", help = "Branch or tag to check out in MCP server mode.")]
    ref_name: Option<String>,
    #[arg(
        long,
        help = "Model path or Hugging Face model id for MCP server mode."
    )]
    model: Option<String>,
    #[arg(long, help = "Disable model downloads and remote Git sources.")]
    offline: bool,
    #[arg(long = "no-download", help = "Disable model downloads.")]
    no_download: bool,
    #[arg(long, help = "Use a custom persistent index cache directory.")]
    cache_dir: Option<PathBuf>,
    #[arg(long, help = "Disable persistent index caches.")]
    no_cache: bool,
    #[arg(long, help = "Use a project-local .sifs cache.")]
    project_cache: bool,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Search a local directory or Git URL with natural-language or code queries.")]
    Search {
        #[arg(help = "Natural-language, code, symbol, or literal query to search for.")]
        query: String,
        #[arg(
            default_value = ".",
            help = "Local directory or Git URL to index and search."
        )]
        path: String,
        #[arg(
            short = 'k',
            long = "top-k",
            default_value_t = 5,
            help = "Maximum number of ranked chunks to print."
        )]
        top_k: usize,
        #[arg(short = 'm', long = "mode", value_enum, default_value_t = ModeArg::Hybrid, help = "Ranking mode: hybrid for most searches, bm25 for exact symbols, semantic for conceptual queries.")]
        mode: ModeArg,
        #[arg(long, help = "Model path or Hugging Face model id.")]
        model: Option<String>,
        #[arg(long, help = "Disable model downloads and remote Git sources.")]
        offline: bool,
        #[arg(long = "no-download", help = "Disable model downloads.")]
        no_download: bool,
        #[arg(long, help = "Use a custom persistent index cache directory.")]
        cache_dir: Option<PathBuf>,
        #[arg(long, help = "Disable persistent index caches.")]
        no_cache: bool,
        #[arg(long, help = "Use a project-local .sifs cache.")]
        project_cache: bool,
    },
    #[command(about = "Find chunks related to a known file and one-based line number.")]
    FindRelated {
        #[arg(help = "Repository-relative file path, usually copied from a search result.")]
        file_path: String,
        #[arg(help = "One-based line number inside the source chunk.")]
        line: usize,
        #[arg(
            default_value = ".",
            help = "Local directory or Git URL to index and search."
        )]
        path: String,
        #[arg(
            short = 'k',
            long = "top-k",
            default_value_t = 5,
            help = "Maximum number of related chunks to print."
        )]
        top_k: usize,
        #[arg(long, help = "Model path or Hugging Face model id.")]
        model: Option<String>,
        #[arg(long, help = "Disable model downloads and remote Git sources.")]
        offline: bool,
        #[arg(long = "no-download", help = "Disable model downloads.")]
        no_download: bool,
        #[arg(long, help = "Use a custom persistent index cache directory.")]
        cache_dir: Option<PathBuf>,
        #[arg(long, help = "Disable persistent index caches.")]
        no_cache: bool,
        #[arg(long, help = "Use a project-local .sifs cache.")]
        project_cache: bool,
    },
    #[command(about = "Inspect or clean SIFS persistent index caches.")]
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
    #[command(about = "Manage the SIFS embedding model cache.")]
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    #[command(about = "Create the Claude agent file at .claude/agents/sifs-search.md.")]
    Init {
        #[arg(long, help = "Overwrite an existing generated agent file.")]
        force: bool,
    },
    #[command(about = "Print SIFS agent, CLI, and MCP capabilities.")]
    Capabilities,
}

#[derive(Subcommand)]
enum ModelCommand {
    #[command(about = "Download or validate the embedding model in the Hugging Face cache.")]
    Pull {
        #[arg(long)]
        model: Option<String>,
    },
    #[command(about = "Report whether the embedding model is available locally.")]
    Status {
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum CacheCommand {
    #[command(about = "Report persistent index cache location and size.")]
    Status {
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Delete the SIFS persistent index cache.")]
    Clean {
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ModeArg {
    Hybrid,
    Semantic,
    Bm25,
}

impl From<ModeArg> for SearchMode {
    fn from(value: ModeArg) -> Self {
        match value {
            ModeArg::Hybrid => SearchMode::Hybrid,
            ModeArg::Semantic => SearchMode::Semantic,
            ModeArg::Bm25 => SearchMode::Bm25,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Search {
            query,
            path,
            top_k,
            mode,
            model,
            offline,
            no_download,
            cache_dir,
            no_cache,
            project_cache,
        }) => {
            let mode = SearchMode::from(mode);
            let policy = model_policy(offline, no_download);
            let cache = cache_config(cache_dir, no_cache, project_cache);
            let index = build_index(
                &path,
                ModelOptions::new(model.as_deref(), policy),
                cache,
                offline,
            )?;
            let results = index.search_with(&query, &SearchOptions::new(top_k).with_mode(mode))?;
            if results.is_empty() {
                println!("No results found.");
            } else {
                println!(
                    "{}",
                    format_results(
                        &format!("Search results for: {query:?} (mode={mode})"),
                        &results
                    )
                );
            }
        }
        Some(Command::FindRelated {
            file_path,
            line,
            path,
            top_k,
            model,
            offline,
            no_download,
            cache_dir,
            no_cache,
            project_cache,
        }) => {
            let policy = model_policy(offline, no_download);
            let cache = cache_config(cache_dir, no_cache, project_cache);
            let index = build_index(
                &path,
                ModelOptions::new(model.as_deref(), policy),
                cache,
                offline,
            )?;
            let Some(chunk) = resolve_chunk(&index.chunks, &file_path, line) else {
                eprintln!("No chunk found at {file_path}:{line}.");
                std::process::exit(1);
            };
            let results = index.find_related(&chunk, top_k)?;
            if results.is_empty() {
                println!("No related chunks found for {file_path}:{line}.");
            } else {
                println!(
                    "{}",
                    format_results(&format!("Chunks related to {file_path}:{line}"), &results)
                );
            }
        }
        Some(Command::Model { command }) => run_model(command)?,
        Some(Command::Cache { command }) => run_cache(command)?,
        Some(Command::Init { force }) => run_init(force)?,
        Some(Command::Capabilities) => print_capabilities(),
        None => {
            let policy = model_policy(cli.offline, cli.no_download);
            if cli.offline
                && let Some(path) = &cli.path
                && is_git_url(path)
            {
                bail!("--offline does not allow remote Git sources");
            }
            sifs::mcp::serve_with_options(
                cli.path,
                cli.ref_name,
                ModelOptions::new(cli.model.as_deref(), policy),
                cache_config(cli.cache_dir, cli.no_cache, cli.project_cache),
                cli.offline,
            )?;
        }
    }
    Ok(())
}

fn build_index(
    path: &str,
    model_options: ModelOptions,
    cache: CacheConfig,
    offline: bool,
) -> Result<SifsIndex> {
    let options = IndexOptions::new(model_options).with_cache(cache);
    if is_git_url(path) {
        if offline {
            bail!("--offline does not allow remote Git sources");
        }
        SifsIndex::from_git_with_index_options(path, None, options)
    } else {
        SifsIndex::from_path_with_index_options(path, options)
    }
}

fn cache_config(cache_dir: Option<PathBuf>, no_cache: bool, project_cache: bool) -> CacheConfig {
    if no_cache {
        CacheConfig::Disabled
    } else if project_cache {
        CacheConfig::Project
    } else if let Some(path) = cache_dir {
        CacheConfig::Custom(path)
    } else {
        CacheConfig::Platform
    }
}

fn model_policy(offline: bool, no_download: bool) -> ModelLoadPolicy {
    if offline {
        ModelLoadPolicy::Offline
    } else if no_download {
        ModelLoadPolicy::NoDownload
    } else {
        ModelLoadPolicy::AllowDownload
    }
}

fn run_model(command: ModelCommand) -> Result<()> {
    match command {
        ModelCommand::Pull { model } => {
            let options = ModelOptions::new(model.as_deref(), ModelLoadPolicy::AllowDownload);
            load_model_with_options(&options)?;
            println!("Model is available: {}", options.model);
        }
        ModelCommand::Status { model, json } => {
            let status = model_status(model.as_deref());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "model": status.model,
                        "available": status.available(),
                        "tokenizer": status.tokenizer,
                        "safetensors": status.safetensors,
                        "config": status.config,
                    }))?
                );
            } else if status.available() {
                println!("Model is available locally: {}", status.model);
            } else {
                println!("Model is not available locally: {}", status.model);
            }
        }
    }
    Ok(())
}

fn run_cache(command: CacheCommand) -> Result<()> {
    match command {
        CacheCommand::Status { cache_dir, json } => {
            let root = cache_dir.unwrap_or(platform_cache_root()?);
            let summary = cache_summary(&root);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "root": summary.root,
                        "exists": summary.exists,
                        "entries": summary.entries,
                        "files": summary.files,
                        "bytes": summary.bytes,
                    }))?
                );
            } else {
                println!("Cache root: {}", summary.root.display());
                println!("Exists: {}", summary.exists);
                println!("Entries: {}", summary.entries);
                println!("Files: {}", summary.files);
                println!("Bytes: {}", summary.bytes);
            }
        }
        CacheCommand::Clean { cache_dir, dry_run } => {
            let root = cache_dir.unwrap_or(platform_cache_root()?);
            let summary = cache_summary(&root);
            if dry_run {
                println!(
                    "Would remove {} files ({} bytes) from {}.",
                    summary.files,
                    summary.bytes,
                    summary.root.display()
                );
            } else {
                remove_cache_root(&root)?;
                println!("Removed cache: {}", root.display());
            }
        }
    }
    Ok(())
}

fn remove_cache_root(root: &Path) -> Result<()> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    Ok(())
}

fn run_init(force: bool) -> Result<()> {
    let dest = PathBuf::from(".claude")
        .join("agents")
        .join("sifs-search.md");
    if dest.exists() && !force {
        eprintln!(
            "{} already exists. Run with --force to overwrite.",
            dest.display()
        );
        std::process::exit(1);
    }
    fs::create_dir_all(dest.parent().unwrap())?;
    fs::write(&dest, include_str!("agents/sifs-search.md"))?;
    println!("Created {}", dest.display());
    Ok(())
}

fn print_capabilities() {
    println!(
        "{}",
        [
            "SIFS capabilities:",
            "- Search local directories and Git URLs with hybrid, semantic, or BM25 ranking.",
            "- Find related code from a known file and one-based line.",
            "- Run as an MCP server with search, find_related, index_status, refresh_index, clear_index, list_indexed_files, get_chunk, and init_agent tools.",
            "- Run BM25 search without loading or downloading an embedding model.",
            "- Manage embedding models with `sifs model pull` and `sifs model status`.",
            "- Manage persistent index caches with `sifs cache status` and `sifs cache clean`.",
            "- Generate a Claude agent file with `sifs init`.",
            "- Use `sifs-benchmark` for quality and latency benchmarks.",
            "- Use `sifs-embed` for embedding diagnostics.",
            "",
            "Discovery:",
            "- `sifs --help` and subcommand help show CLI usage.",
            "- MCP clients can call `tools/list` and read `sifs://server/context`.",
        ]
        .join("\n")
    );
}
