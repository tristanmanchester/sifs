use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use sifs::{
    CacheConfig, EncoderSpec, IndexOptions, IndexStats, ModelLoadPolicy, ModelOptions, SearchMode,
    SearchOptions, SearchResult, SifsIndex, cache_summary, format_results, is_git_url,
    load_model_with_options, model_status, platform_cache_root, resolve_chunk,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "sifs",
    about = "SIFS Is Fast Search: instant local code search for agents."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
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
        #[arg(
            long = "language",
            help = "Only search chunks with this language label. Repeatable."
        )]
        languages: Vec<String>,
        #[arg(
            long = "path",
            help = "Only search this repository-relative file path. Repeatable."
        )]
        filter_paths: Vec<String>,
        #[arg(
            long,
            default_value_t = 0,
            help = "Include surrounding lines when local source files are available."
        )]
        context_lines: usize,
        #[arg(
            long,
            help = "Include query, ranking, and filter metadata in the output."
        )]
        explain: bool,
        #[command(flatten)]
        output: OutputArgs,
        #[arg(long, help = "Model path or Hugging Face model id.")]
        model: Option<String>,
        #[arg(long, value_enum, default_value_t = EncoderArg::Model2Vec, help = "Encoder for semantic and hybrid search.")]
        encoder: EncoderArg,
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
        #[command(flatten)]
        output: OutputArgs,
        #[arg(long, help = "Model path or Hugging Face model id.")]
        model: Option<String>,
        #[arg(long, value_enum, default_value_t = EncoderArg::Model2Vec, help = "Encoder for related-code search.")]
        encoder: EncoderArg,
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
    #[command(about = "Start the SIFS stdio MCP server.")]
    Mcp {
        #[arg(help = "Local directory or git URL to pre-index.")]
        path: Option<String>,
        #[arg(long = "ref", help = "Branch or tag to check out for a Git URL.")]
        ref_name: Option<String>,
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
    #[command(about = "List repository-relative file paths included in the index.")]
    Files {
        #[arg(
            default_value = ".",
            help = "Local directory or Git URL to index and inspect."
        )]
        path: String,
        #[arg(
            long,
            default_value_t = 200,
            help = "Maximum number of file paths to print."
        )]
        limit: usize,
        #[command(flatten)]
        output: OutputArgs,
        #[arg(long, help = "Model path or Hugging Face model id.")]
        model: Option<String>,
        #[arg(long, help = "Disable model downloads and remote Git sources.")]
        offline: bool,
        #[arg(long = "no-download", help = "Disable model downloads.")]
        no_download: bool,
    },
    #[command(about = "Print index status for a local directory or Git URL.")]
    Status {
        #[arg(
            default_value = ".",
            help = "Local directory or Git URL to index and inspect."
        )]
        path: String,
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
        #[arg(long, help = "Model path or Hugging Face model id.")]
        model: Option<String>,
        #[arg(long, help = "Disable model downloads and remote Git sources.")]
        offline: bool,
        #[arg(long = "no-download", help = "Disable model downloads.")]
        no_download: bool,
    },
    #[command(about = "Print the indexed chunk containing a file and one-based line number.")]
    Get {
        #[arg(help = "Repository-relative file path.")]
        file_path: String,
        #[arg(help = "One-based line number inside the source chunk.")]
        line: usize,
        #[arg(
            default_value = ".",
            help = "Local directory or Git URL to index and inspect."
        )]
        path: String,
        #[command(flatten)]
        output: OutputArgs,
        #[arg(long, help = "Model path or Hugging Face model id.")]
        model: Option<String>,
        #[arg(long, help = "Disable model downloads and remote Git sources.")]
        offline: bool,
        #[arg(long = "no-download", help = "Disable model downloads.")]
        no_download: bool,
    },
    #[command(about = "Remove SIFS cache artifacts from a local directory.")]
    Clean {
        #[arg(
            default_value = ".",
            help = "Local directory whose .sifs cache should be removed."
        )]
        path: String,
    },
    #[command(about = "Inspect or clean SIFS persistent index caches.")]
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum EncoderArg {
    #[value(name = "model2vec")]
    Model2Vec,
    Hashing,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TextFormat {
    Human,
    Compact,
}

#[derive(Args)]
struct OutputArgs {
    #[arg(long, conflicts_with = "jsonl", help = "Print one pretty JSON object.")]
    json: bool,
    #[arg(long, help = "Print newline-delimited JSON records.")]
    jsonl: bool,
    #[arg(long, value_enum, default_value_t = TextFormat::Human, help = "Text output format.")]
    format: TextFormat,
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
            languages,
            filter_paths,
            context_lines,
            explain,
            output,
            model,
            encoder,
            offline,
            no_download,
            cache_dir,
            no_cache,
            project_cache,
        }) => run_search(SearchCommand {
            query,
            path,
            top_k,
            mode: SearchMode::from(mode),
            languages,
            filter_paths,
            context_lines,
            explain,
            output,
            model,
            encoder,
            offline,
            no_download,
            cache: cache_config(cache_dir, no_cache, project_cache),
        })?,
        Some(Command::FindRelated {
            file_path,
            line,
            path,
            top_k,
            output,
            model,
            encoder,
            offline,
            no_download,
            cache_dir,
            no_cache,
            project_cache,
        }) => {
            let policy = model_policy(offline, no_download);
            let started = Instant::now();
            let index = build_hybrid_index(
                &path,
                encoder_spec(encoder, model.as_deref(), policy),
                cache_config(cache_dir, no_cache, project_cache),
                offline,
            )?;
            let Some(chunk) = resolve_chunk(&index.chunks, &file_path, line) else {
                eprintln!("No chunk found at {file_path}:{line}.");
                std::process::exit(1);
            };
            let results = index.find_related(&chunk, top_k)?;
            let elapsed_ms = started.elapsed().as_millis();
            let payload = json!({
                "source": path,
                "file_path": file_path,
                "line": line,
                "top_k": top_k,
                "index_stats": index.stats(),
                "elapsed_ms": elapsed_ms,
                "results": structured_results(&path, &results, 0),
            });
            if output.json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else if output.jsonl {
                for result in &results {
                    println!(
                        "{}",
                        serde_json::to_string(&json!({
                            "source": path,
                            "file_path": file_path,
                            "line": line,
                            "top_k": top_k,
                            "elapsed_ms": elapsed_ms,
                            "result": structured_result(&path, result, 0),
                        }))?
                    );
                }
            } else {
                match output.format {
                    TextFormat::Human => {
                        if results.is_empty() {
                            println!("No related chunks found for {file_path}:{line}.");
                        } else {
                            println!(
                                "{}",
                                format_results(
                                    &format!("Chunks related to {file_path}:{line}"),
                                    &results
                                )
                            );
                        }
                    }
                    TextFormat::Compact => print_compact_results(&results),
                }
            }
        }
        Some(Command::Model { command }) => run_model(command)?,
        Some(Command::Cache { command }) => run_cache(command)?,
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
        Some(Command::Mcp {
            path,
            ref_name,
            model,
            offline,
            no_download,
            cache_dir,
            no_cache,
            project_cache,
        }) => {
            let policy = model_policy(offline, no_download);
            if offline
                && let Some(path) = &path
                && is_git_url(path)
            {
                bail!("--offline does not allow remote Git sources");
            }
            sifs::mcp::serve_with_options(
                path,
                ref_name,
                ModelOptions::new(model.as_deref(), policy),
                cache_config(cache_dir, no_cache, project_cache),
                offline,
            )?;
        }
        Some(Command::Files {
            path,
            limit,
            output,
            model,
            offline,
            no_download,
        }) => run_files(&path, limit, output, model, offline, no_download)?,
        Some(Command::Status {
            path,
            json,
            model,
            offline,
            no_download,
        }) => run_status(&path, json, model, offline, no_download)?,
        Some(Command::Get {
            file_path,
            line,
            path,
            output,
            model,
            offline,
            no_download,
        }) => run_get(&file_path, line, &path, output, model, offline, no_download)?,
        Some(Command::Clean { path }) => run_clean(&path)?,
        Some(Command::Init { force }) => run_init(force)?,
        Some(Command::Capabilities) => print_capabilities(),
        None => {
            Cli::command().print_help()?;
            println!();
        }
    }
    Ok(())
}

struct SearchCommand {
    query: String,
    path: String,
    top_k: usize,
    mode: SearchMode,
    languages: Vec<String>,
    filter_paths: Vec<String>,
    context_lines: usize,
    explain: bool,
    output: OutputArgs,
    model: Option<String>,
    encoder: EncoderArg,
    offline: bool,
    no_download: bool,
    cache: CacheConfig,
}

fn run_search(command: SearchCommand) -> Result<()> {
    let policy = model_policy(command.offline, command.no_download);
    let started = Instant::now();
    let index = build_index_for_mode(
        &command.path,
        command.mode,
        encoder_spec(command.encoder, command.model.as_deref(), policy),
        command.cache,
        command.offline,
    )?;
    let mut options = SearchOptions::new(command.top_k).with_mode(command.mode);
    options.filter_languages = command.languages;
    options.filter_paths = command.filter_paths;
    let results = index.search_with(&command.query, &options)?;
    let elapsed_ms = started.elapsed().as_millis();
    let warnings = context_warnings(&command.path, command.context_lines);
    let payload = search_payload(
        &command.query,
        &command.path,
        &options,
        index.stats(),
        elapsed_ms,
        &warnings,
        &results,
        command.context_lines,
    );

    if command.output.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if command.output.jsonl {
        for result in &results {
            println!(
                "{}",
                serde_json::to_string(&search_result_record(
                    &command.query,
                    &command.path,
                    &options,
                    elapsed_ms,
                    &warnings,
                    result,
                    command.context_lines,
                ))?
            );
        }
    } else {
        if command.explain {
            print_explanation(
                &command.query,
                &command.path,
                &options,
                elapsed_ms,
                &warnings,
            );
        }
        match command.output.format {
            TextFormat::Human => {
                if results.is_empty() {
                    println!("No results found.");
                } else {
                    println!(
                        "{}",
                        format_results(
                            &format!(
                                "Search results for: {:?} (mode={})",
                                command.query, command.mode
                            ),
                            &results,
                        )
                    );
                }
            }
            TextFormat::Compact => print_compact_results(&results),
        }
    }
    Ok(())
}

fn build_index_for_mode(
    path: &str,
    mode: SearchMode,
    encoder_spec: EncoderSpec,
    cache: CacheConfig,
    offline: bool,
) -> Result<SifsIndex> {
    match mode {
        SearchMode::Bm25 => build_sparse_index(path, cache, offline),
        SearchMode::Semantic | SearchMode::Hybrid => {
            build_hybrid_index(path, encoder_spec, cache, offline)
        }
    }
}

fn build_sparse_index(path: &str, cache: CacheConfig, offline: bool) -> Result<SifsIndex> {
    let options = IndexOptions::sparse().with_cache(cache);
    if is_git_url(path) {
        if offline {
            bail!("--offline does not allow remote Git sources");
        }
        SifsIndex::from_git_with_index_options(path, None, options)
    } else {
        SifsIndex::from_path_with_index_options(path, options)
    }
}

fn build_hybrid_index(
    path: &str,
    encoder_spec: EncoderSpec,
    cache: CacheConfig,
    offline: bool,
) -> Result<SifsIndex> {
    let options = IndexOptions::sparse()
        .with_encoder_spec(encoder_spec)
        .with_cache(cache);
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

fn run_files(
    path: &str,
    limit: usize,
    output: OutputArgs,
    model: Option<String>,
    offline: bool,
    no_download: bool,
) -> Result<()> {
    let policy = model_policy(offline, no_download);
    let started = Instant::now();
    let index = build_hybrid_index(
        path,
        EncoderSpec::model2vec(model.as_deref(), policy),
        CacheConfig::Platform,
        offline,
    )?;
    let elapsed_ms = started.elapsed().as_millis();
    let files = index.indexed_files();
    let shown: Vec<_> = files.iter().take(limit).cloned().collect();
    let payload = json!({
        "source": path,
        "total": files.len(),
        "limit": limit,
        "elapsed_ms": elapsed_ms,
        "files": shown,
    });

    if output.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if output.jsonl {
        for file in files.iter().take(limit) {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "source": path,
                    "total": files.len(),
                    "limit": limit,
                    "file_path": file,
                }))?
            );
        }
    } else {
        match output.format {
            TextFormat::Human => {
                println!(
                    "Indexed files for {path:?} (showing {} of {}):",
                    shown.len(),
                    files.len()
                );
                for file in shown {
                    println!("{file}");
                }
            }
            TextFormat::Compact => {
                for file in shown {
                    println!("{file}");
                }
            }
        }
    }
    Ok(())
}

fn run_status(
    path: &str,
    json_output: bool,
    model: Option<String>,
    offline: bool,
    no_download: bool,
) -> Result<()> {
    let policy = model_policy(offline, no_download);
    let started = Instant::now();
    let index = build_hybrid_index(
        path,
        EncoderSpec::model2vec(model.as_deref(), policy),
        CacheConfig::Platform,
        offline,
    )?;
    let elapsed_ms = started.elapsed().as_millis();
    let stats = index.stats();
    let payload = json!({
        "source": path,
        "index_stats": stats,
        "elapsed_ms": elapsed_ms,
        "semantic_index_available": semantic_index_available(path),
        "semantic_index_loaded": index.semantic_loaded(),
    });

    if json_output {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!(
            "Index status for {path:?}: {} files, {} chunks, languages: {}. Semantic index available: {}. Semantic index loaded: {}.",
            stats.indexed_files,
            stats.total_chunks,
            format_languages(&stats.languages),
            semantic_index_available(path),
            index.semantic_loaded()
        );
    }
    Ok(())
}

fn semantic_index_available(path: &str) -> bool {
    if is_git_url(path) {
        return false;
    }
    let cache_dir = Path::new(path).join(".sifs");
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with("semantic-v2-")
    })
}

fn run_get(
    file_path: &str,
    line: usize,
    path: &str,
    output: OutputArgs,
    model: Option<String>,
    offline: bool,
    no_download: bool,
) -> Result<()> {
    let policy = model_policy(offline, no_download);
    let index = build_hybrid_index(
        path,
        EncoderSpec::model2vec(model.as_deref(), policy),
        CacheConfig::Platform,
        offline,
    )?;
    let Some(chunk) = resolve_chunk(&index.chunks, file_path, line) else {
        eprintln!("No chunk found at {file_path}:{line}. Use `sifs files` to check indexed paths.");
        std::process::exit(1);
    };
    let payload = json!({
        "source": path,
        "chunk": chunk,
    });

    if output.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if output.jsonl {
        println!("{}", serde_json::to_string(&payload)?);
    } else {
        match output.format {
            TextFormat::Human => {
                println!(
                    "{}\n\n```{}\n{}\n```",
                    chunk.location(),
                    chunk.language.clone().unwrap_or_default(),
                    chunk.content
                );
            }
            TextFormat::Compact => {
                println!(
                    "{}\t{}\t{}",
                    chunk.location(),
                    chunk.language.clone().unwrap_or_default(),
                    chunk.content.lines().next().unwrap_or_default().trim()
                );
            }
        }
    }
    Ok(())
}

fn run_clean(path: &str) -> Result<()> {
    if is_git_url(path) {
        bail!("clean only supports local directories");
    }
    let cache_dir = Path::new(path).join(".sifs");
    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir)
            .with_context(|| format!("remove SIFS cache {}", cache_dir.display()))?;
        println!("Removed {}", cache_dir.display());
    } else {
        println!("No SIFS cache found at {}", cache_dir.display());
    }
    Ok(())
}

fn search_payload(
    query: &str,
    source: &str,
    options: &SearchOptions,
    stats: IndexStats,
    elapsed_ms: u128,
    warnings: &[String],
    results: &[SearchResult],
    context_lines: usize,
) -> Value {
    json!({
        "query": query,
        "mode": options.mode.to_string(),
        "source": source,
        "top_k": options.top_k,
        "filter_languages": options.filter_languages,
        "filter_paths": options.filter_paths,
        "index_stats": stats,
        "elapsed_ms": elapsed_ms,
        "warnings": warnings,
        "results": structured_results(source, results, context_lines),
    })
}

fn search_result_record(
    query: &str,
    source: &str,
    options: &SearchOptions,
    elapsed_ms: u128,
    warnings: &[String],
    result: &SearchResult,
    context_lines: usize,
) -> Value {
    json!({
        "query": query,
        "mode": options.mode.to_string(),
        "source": source,
        "top_k": options.top_k,
        "filter_languages": options.filter_languages,
        "filter_paths": options.filter_paths,
        "elapsed_ms": elapsed_ms,
        "warnings": warnings,
        "result": structured_result(source, result, context_lines),
    })
}

fn structured_results(source: &str, results: &[SearchResult], context_lines: usize) -> Value {
    json!(
        results
            .iter()
            .map(|result| structured_result(source, result, context_lines))
            .collect::<Vec<_>>()
    )
}

fn structured_result(source: &str, result: &SearchResult, context_lines: usize) -> Value {
    let mut value = json!({
        "file_path": result.chunk.file_path,
        "start_line": result.chunk.start_line,
        "end_line": result.chunk.end_line,
        "language": result.chunk.language,
        "score": result.score,
        "source": result.source.to_string(),
        "content": result.chunk.content,
    });
    if context_lines > 0
        && let Some(context) = expanded_context(
            source,
            &result.chunk.file_path,
            result.chunk.start_line,
            result.chunk.end_line,
            context_lines,
        )
    {
        value["context"] = context;
    }
    value
}

fn expanded_context(
    source: &str,
    file_path: &str,
    start_line: usize,
    end_line: usize,
    context_lines: usize,
) -> Option<Value> {
    if is_git_url(source) {
        return None;
    }
    let full_path = Path::new(source).join(file_path);
    let content = fs::read_to_string(full_path).ok()?;
    let lines: Vec<_> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }
    let context_start = start_line.saturating_sub(context_lines).max(1);
    let context_end = (end_line + context_lines).min(lines.len());
    let text = lines
        .get(context_start - 1..context_end)?
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    Some(json!({
        "start_line": context_start,
        "end_line": context_end,
        "content": text,
    }))
}

fn context_warnings(source: &str, context_lines: usize) -> Vec<String> {
    if context_lines > 0 && is_git_url(source) {
        vec![
            "context_lines is best-effort and is not expanded for Git URL sources in CLI mode"
                .to_owned(),
        ]
    } else {
        Vec::new()
    }
}

fn print_explanation(
    query: &str,
    source: &str,
    options: &SearchOptions,
    elapsed_ms: u128,
    warnings: &[String],
) {
    println!(
        "Query: {query:?}; source: {source}; mode: {}; top_k: {}; languages: [{}]; paths: [{}]; elapsed_ms: {elapsed_ms}",
        options.mode,
        options.top_k,
        options.filter_languages.join(", "),
        options.filter_paths.join(", ")
    );
    for warning in warnings {
        println!("Warning: {warning}");
    }
}

fn print_compact_results(results: &[SearchResult]) {
    for result in results {
        println!(
            "{}\t{:.4}\t{}\t{}",
            result.chunk.location(),
            result.score,
            result.source,
            result
                .chunk
                .content
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
        );
    }
}

fn format_languages(languages: &std::collections::BTreeMap<String, usize>) -> String {
    if languages.is_empty() {
        return "none".to_owned();
    }
    languages
        .iter()
        .map(|(language, count)| format!("{language}={count}"))
        .collect::<Vec<_>>()
        .join(", ")
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
            "- Print structured CLI output with `--json`, `--jsonl`, or `--format compact`.",
            "- Inspect indexes with `sifs files`, `sifs status`, `sifs get`, and `sifs clean`.",
            "- Find related code from a known file and one-based line.",
            "- Run `sifs mcp` as an MCP server with search, find_related, index_status, refresh_index, clear_index, list_indexed_files, get_chunk, and init_agent tools.",
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
