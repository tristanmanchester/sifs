use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use sifs::{
    EncoderSpec, ModelLoadPolicy, ModelOptions, SearchMode, SearchOptions, SifsIndex,
    format_results, is_git_url, load_model_with_options, model_status, resolve_chunk,
};
use std::fs;
use std::path::PathBuf;

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
        #[arg(long, value_enum, default_value_t = EncoderArg::Model2Vec, help = "Encoder for semantic and hybrid search.")]
        encoder: EncoderArg,
        #[arg(long, help = "Disable model downloads and remote Git sources.")]
        offline: bool,
        #[arg(long = "no-download", help = "Disable model downloads.")]
        no_download: bool,
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
        #[arg(long, value_enum, default_value_t = EncoderArg::Model2Vec, help = "Encoder for related-code search.")]
        encoder: EncoderArg,
        #[arg(long, help = "Disable model downloads and remote Git sources.")]
        offline: bool,
        #[arg(long = "no-download", help = "Disable model downloads.")]
        no_download: bool,
    },
    #[command(about = "Manage the SIFS embedding model cache.")]
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    #[command(about = "Check SIFS model, cache, and offline readiness.")]
    Doctor {
        #[arg(long, help = "Model path or Hugging Face model id.")]
        model: Option<String>,
        #[arg(long, value_enum, default_value_t = EncoderArg::Model2Vec)]
        encoder: EncoderArg,
        #[arg(
            default_value = ".",
            help = "Local directory to inspect for cache readiness."
        )]
        path: String,
        #[arg(long, help = "Disable model downloads and remote Git sources.")]
        offline: bool,
        #[arg(long = "no-download", help = "Disable model downloads.")]
        no_download: bool,
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
    #[command(about = "Alias for `pull`: download or validate the embedding model.")]
    Fetch {
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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ModeArg {
    Hybrid,
    Semantic,
    Bm25,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum EncoderArg {
    #[value(name = "model2vec")]
    Model2Vec,
    Hashing,
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
            encoder,
            offline,
            no_download,
        }) => {
            let mode = SearchMode::from(mode);
            let policy = model_policy(offline, no_download);
            let encoder_spec = encoder_spec(encoder, model.as_deref(), policy);
            let index = build_index_for_mode(&path, mode, encoder_spec, offline)?;
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
            encoder,
            offline,
            no_download,
        }) => {
            let policy = model_policy(offline, no_download);
            let encoder_spec = encoder_spec(encoder, model.as_deref(), policy);
            let index = build_hybrid_index(&path, encoder_spec, offline)?;
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
        Some(Command::Doctor {
            model,
            encoder,
            path,
            offline,
            no_download,
        }) => run_doctor(
            &path,
            encoder,
            model.as_deref(),
            model_policy(offline, no_download),
            offline,
        )?,
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
                cli.offline,
            )?;
        }
    }
    Ok(())
}

fn build_index_for_mode(
    path: &str,
    mode: SearchMode,
    encoder_spec: EncoderSpec,
    offline: bool,
) -> Result<SifsIndex> {
    match mode {
        SearchMode::Bm25 => build_sparse_index(path, offline),
        SearchMode::Semantic | SearchMode::Hybrid => {
            build_hybrid_index(path, encoder_spec, offline)
        }
    }
}

fn build_sparse_index(path: &str, offline: bool) -> Result<SifsIndex> {
    if is_git_url(path) {
        if offline {
            bail!("--offline does not allow remote Git sources");
        }
        SifsIndex::from_git_sparse(path, None)
    } else {
        SifsIndex::from_path_sparse(path)
    }
}

fn build_hybrid_index(path: &str, encoder_spec: EncoderSpec, offline: bool) -> Result<SifsIndex> {
    if is_git_url(path) {
        if offline {
            bail!("--offline does not allow remote Git sources");
        }
        SifsIndex::from_git_with_encoder_spec(path, None, encoder_spec)
    } else {
        SifsIndex::from_path_with_encoder_spec(path, encoder_spec, None, None, false)
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

fn encoder_spec(encoder: EncoderArg, model: Option<&str>, policy: ModelLoadPolicy) -> EncoderSpec {
    match encoder {
        EncoderArg::Model2Vec => EncoderSpec::model2vec(model, policy),
        EncoderArg::Hashing => EncoderSpec::hashing(),
    }
}

fn run_model(command: ModelCommand) -> Result<()> {
    match command {
        ModelCommand::Pull { model } | ModelCommand::Fetch { model } => {
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

fn run_doctor(
    path: &str,
    encoder: EncoderArg,
    model: Option<&str>,
    policy: ModelLoadPolicy,
    offline: bool,
) -> Result<()> {
    println!("SIFS doctor");
    println!("Path: {path}");
    if is_git_url(path) {
        println!("Source: remote Git URL");
        println!(
            "Remote Git under offline mode: {}",
            if offline { "blocked" } else { "allowed" }
        );
    } else {
        let path = PathBuf::from(path);
        println!("Source: local directory");
        println!("Path exists: {}", path.exists());
        println!("Path is directory: {}", path.is_dir());
        if path.is_dir() {
            let cache_dir = path.join(".sifs");
            let cache_writable = if cache_dir.exists() {
                fs::metadata(&cache_dir)
                    .map(|metadata| !metadata.permissions().readonly())
                    .unwrap_or(false)
            } else {
                fs::metadata(&path)
                    .map(|metadata| !metadata.permissions().readonly())
                    .unwrap_or(false)
            };
            println!("Cache directory: {}", cache_dir.display());
            println!("Cache writable: {cache_writable}");
        }
    }

    match encoder {
        EncoderArg::Hashing => {
            println!("Encoder: hashing");
            println!("Semantic readiness: ready without model files");
        }
        EncoderArg::Model2Vec => {
            let options = ModelOptions::new(model, policy);
            let status = model_status(Some(&options.model));
            println!("Encoder: model2vec");
            println!("Model: {}", status.model);
            println!("Tokenizer: {}", file_status(status.tokenizer.as_ref()));
            println!("Safetensors: {}", file_status(status.safetensors.as_ref()));
            println!("Config: {}", file_status(status.config.as_ref()));
            println!(
                "Semantic readiness: {}",
                if status.available() {
                    "ready from local files"
                } else if policy == ModelLoadPolicy::AllowDownload {
                    "will download on first semantic/hybrid search"
                } else {
                    "not ready in offline/no-download mode"
                }
            );
        }
    }
    Ok(())
}

fn file_status(path: Option<&PathBuf>) -> String {
    path.map(|path| format!("found at {}", path.display()))
        .unwrap_or_else(|| "missing".to_owned())
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
