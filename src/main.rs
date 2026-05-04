use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use sifs::daemon::{
    DaemonClient, DaemonRequest, DaemonResult, DaemonRuntimeOptions, IndexRuntimeOptions,
    SearchOptionsWire, SourceSpec, default_daemon_paths, run_foreground,
};
use sifs::{
    CacheConfig, EncoderSpec, IndexOptions, IndexStats, ModelLoadPolicy, ModelOptions, SearchMode,
    SearchOptions, SearchResult, SifsIndex, cache_summary, format_results, is_git_url,
    load_model_with_options, model_status, platform_cache_root, resolve_chunk,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(
    name = "sifs",
    version,
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
    #[command(about = "Start, install, or inspect the SIFS stdio MCP server.")]
    Mcp {
        #[command(subcommand)]
        command: Option<McpCommand>,
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
    #[command(about = "Run or inspect the shared SIFS daemon used by agent integrations.")]
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
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
enum McpCommand {
    #[command(about = "Configure SIFS as a local stdio MCP server for Codex and/or Claude Code.")]
    Install {
        #[arg(long, value_enum, default_value_t = McpClientArg::All)]
        client: McpClientArg,
        #[arg(long, value_enum)]
        scope: Option<McpScopeArg>,
        #[arg(
            long,
            help = "Local directory or Git URL to expose through the MCP server."
        )]
        source: Option<String>,
        #[arg(
            long = "ref",
            help = "Branch or tag to check out for a Git URL source."
        )]
        ref_name: Option<String>,
        #[arg(long, default_value = "sifs", help = "MCP server name to configure.")]
        name: String,
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
        #[arg(long, help = "Replace an existing same-name MCP server.")]
        force: bool,
        #[arg(
            long,
            help = "Print commands and config without changing client state."
        )]
        dry_run: bool,
    },
    #[command(about = "Inspect local MCP install readiness.")]
    Doctor {
        #[arg(
            default_value = ".",
            help = "Local directory or Git URL the MCP server would expose."
        )]
        source: String,
        #[arg(
            long = "ref",
            help = "Branch or tag to check out for a Git URL source."
        )]
        ref_name: Option<String>,
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
}

#[derive(Subcommand)]
enum DaemonCommand {
    #[command(about = "Run the SIFS daemon in the foreground.")]
    Run {
        #[arg(
            long,
            help = "Remove an existing socket before binding. Useful after an unclean shutdown."
        )]
        replace_existing_socket: bool,
    },
    #[command(about = "Ping the running SIFS daemon.")]
    Ping,
    #[command(about = "Print daemon process and cached-index status.")]
    Status {
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Install a macOS LaunchAgent that keeps the SIFS daemon running.")]
    InstallAgent {
        #[arg(long, help = "Print the LaunchAgent plist without writing it.")]
        dry_run: bool,
        #[arg(long, help = "Replace an existing SIFS LaunchAgent plist.")]
        force: bool,
    },
    #[command(about = "Remove the macOS LaunchAgent for the SIFS daemon.")]
    UninstallAgent {
        #[arg(long, help = "Print what would be removed without changing files.")]
        dry_run: bool,
    },
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
enum McpClientArg {
    Codex,
    Claude,
    All,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum McpScopeArg {
    Local,
    Project,
    User,
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
            if let Some(result) = try_daemon_find_related(
                &path,
                &file_path,
                line,
                top_k,
                encoder,
                model.as_deref(),
                offline,
                no_download,
                cache_config(cache_dir.clone(), no_cache, project_cache),
            )? {
                print_find_related_output(
                    &result.source.source,
                    &file_path,
                    line,
                    top_k,
                    result.stats,
                    u128::from(result.elapsed_ms),
                    &result.results,
                    &output,
                )?;
                return Ok(());
            }
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
            print_find_related_output(
                &path,
                &file_path,
                line,
                top_k,
                index.stats(),
                elapsed_ms,
                &results,
                &output,
            )?;
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
            command,
            path,
            ref_name,
            model,
            offline,
            no_download,
            cache_dir,
            no_cache,
            project_cache,
        }) => match command {
            Some(McpCommand::Install {
                client,
                scope,
                source,
                ref_name,
                name,
                offline,
                no_download,
                cache_dir,
                no_cache,
                project_cache,
                force,
                dry_run,
            }) => run_mcp_install(McpInstallOptions {
                client,
                scope,
                source,
                ref_name,
                name,
                offline,
                no_download,
                cache_dir,
                no_cache,
                project_cache,
                force,
                dry_run,
            })?,
            Some(McpCommand::Doctor {
                source,
                ref_name,
                offline,
                no_download,
                cache_dir,
                no_cache,
                project_cache,
            }) => run_mcp_doctor(McpDoctorOptions {
                source,
                ref_name,
                offline,
                no_download,
                cache_dir,
                no_cache,
                project_cache,
            })?,
            None => {
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
        },
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
        Some(Command::Daemon { command }) => run_daemon_command(command)?,
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
    if let Some(result) = try_daemon_search(&command)? {
        print_search_output(
            &result.query,
            &result.source.source,
            &SearchOptions {
                top_k: command.top_k,
                mode: result.mode,
                alpha: None,
                filter_languages: command.languages.clone(),
                filter_paths: command.filter_paths.clone(),
                use_query_cache: true,
            },
            result.stats,
            u128::from(result.elapsed_ms),
            &daemon_warnings(&result.warnings),
            &result.results,
            command.context_lines,
            command.explain,
            &command.output,
        )?;
        return Ok(());
    }

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

    print_search_output(
        &command.query,
        &command.path,
        &options,
        index.stats(),
        elapsed_ms,
        &warnings,
        &results,
        command.context_lines,
        command.explain,
        &command.output,
    )
}

#[allow(clippy::too_many_arguments)]
fn print_search_output(
    query: &str,
    source: &str,
    options: &SearchOptions,
    stats: IndexStats,
    elapsed_ms: u128,
    warnings: &[String],
    results: &[SearchResult],
    context_lines: usize,
    explain: bool,
    output: &OutputArgs,
) -> Result<()> {
    let payload = search_payload(
        query,
        source,
        options,
        stats,
        elapsed_ms,
        warnings,
        results,
        context_lines,
    );
    if output.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if output.jsonl {
        for result in results {
            println!(
                "{}",
                serde_json::to_string(&search_result_record(
                    query,
                    source,
                    options,
                    elapsed_ms,
                    warnings,
                    result,
                    context_lines,
                ))?
            );
        }
    } else {
        if explain {
            print_explanation(query, source, options, elapsed_ms, warnings);
        }
        match output.format {
            TextFormat::Human => {
                if results.is_empty() {
                    println!("No results found.");
                } else {
                    println!(
                        "{}",
                        format_results(
                            &format!("Search results for: {:?} (mode={})", query, options.mode),
                            results,
                        )
                    );
                }
            }
            TextFormat::Compact => print_compact_results(results),
        }
    }
    Ok(())
}

fn try_daemon_search(
    command: &SearchCommand,
) -> Result<Option<sifs::daemon::protocol::SearchResultSet>> {
    let Some(client) = daemon_client_if_running()? else {
        return Ok(None);
    };
    let source = SourceSpec::resolve(&command.path, None, command.offline)?;
    let policy = model_policy(command.offline, command.no_download);
    let runtime_options = match command.mode {
        SearchMode::Bm25 => IndexRuntimeOptions::sparse(command.cache.clone()),
        SearchMode::Semantic | SearchMode::Hybrid => IndexRuntimeOptions::with_encoder(
            encoder_spec(command.encoder, command.model.as_deref(), policy),
            command.cache.clone(),
        ),
    };
    let mut search = SearchOptions::new(command.top_k).with_mode(command.mode);
    search.filter_languages = command.languages.clone();
    search.filter_paths = command.filter_paths.clone();
    match client.send(DaemonRequest::Search {
        source,
        options: runtime_options,
        query: command.query.clone(),
        search: SearchOptionsWire::from(search),
    }) {
        Ok(DaemonResult::Search(results)) => Ok(Some(results)),
        Ok(other) => bail!("unexpected daemon response: {other:?}"),
        Err(_) => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
fn try_daemon_find_related(
    path: &str,
    file_path: &str,
    line: usize,
    top_k: usize,
    encoder: EncoderArg,
    model: Option<&str>,
    offline: bool,
    no_download: bool,
    cache: CacheConfig,
) -> Result<Option<sifs::daemon::protocol::SearchResultSet>> {
    let Some(client) = daemon_client_if_running()? else {
        return Ok(None);
    };
    let source = SourceSpec::resolve(path, None, offline)?;
    let policy = model_policy(offline, no_download);
    match client.send(DaemonRequest::FindRelated {
        source,
        options: IndexRuntimeOptions::with_encoder(encoder_spec(encoder, model, policy), cache),
        file_path: file_path.to_owned(),
        line,
        top_k,
    }) {
        Ok(DaemonResult::FindRelated(results)) => Ok(Some(results)),
        Ok(other) => bail!("unexpected daemon response: {other:?}"),
        Err(_) => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
fn print_find_related_output(
    source: &str,
    file_path: &str,
    line: usize,
    top_k: usize,
    stats: IndexStats,
    elapsed_ms: u128,
    results: &[SearchResult],
    output: &OutputArgs,
) -> Result<()> {
    let payload = json!({
        "source": source,
        "file_path": file_path,
        "line": line,
        "top_k": top_k,
        "index_stats": stats,
        "elapsed_ms": elapsed_ms,
        "results": structured_results(source, results, 0),
    });
    if output.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if output.jsonl {
        for result in results {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "source": source,
                    "file_path": file_path,
                    "line": line,
                    "top_k": top_k,
                    "elapsed_ms": elapsed_ms,
                    "result": structured_result(source, result, 0),
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
                        format_results(&format!("Chunks related to {file_path}:{line}"), results)
                    );
                }
            }
            TextFormat::Compact => print_compact_results(results),
        }
    }
    Ok(())
}

fn daemon_client_if_running() -> Result<Option<DaemonClient>> {
    let paths = default_daemon_paths()?;
    if paths.socket.exists() {
        Ok(Some(DaemonClient::new(paths)))
    } else {
        Ok(None)
    }
}

fn daemon_warnings(warnings: &[sifs::IndexWarning]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| {
            if warning.path.is_empty() {
                warning.message.clone()
            } else {
                format!("{}: {}", warning.path, warning.message)
            }
        })
        .collect()
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
    if let Some(DaemonResult::ListFiles {
        source,
        total,
        files,
    }) = try_daemon_list_files(path, limit, model.as_deref(), offline, no_download)?
    {
        print_files_output(&source.source, total, limit, 0, files, output)?;
        return Ok(());
    }
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
    print_files_output(path, files.len(), limit, elapsed_ms, shown, output)
}

fn print_files_output(
    source: &str,
    total: usize,
    limit: usize,
    elapsed_ms: u128,
    shown: Vec<String>,
    output: OutputArgs,
) -> Result<()> {
    let payload = json!({
        "source": source,
        "total": total,
        "limit": limit,
        "elapsed_ms": elapsed_ms,
        "files": shown,
    });
    if output.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if output.jsonl {
        for file in &shown {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "source": source,
                    "total": total,
                    "limit": limit,
                    "file_path": file,
                }))?
            );
        }
    } else {
        match output.format {
            TextFormat::Human => {
                println!(
                    "Indexed files for {source:?} (showing {} of {}):",
                    shown.len(),
                    total
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

fn try_daemon_list_files(
    path: &str,
    limit: usize,
    model: Option<&str>,
    offline: bool,
    no_download: bool,
) -> Result<Option<sifs::daemon::protocol::DaemonResult>> {
    let Some(client) = daemon_client_if_running()? else {
        return Ok(None);
    };
    let source = SourceSpec::resolve(path, None, offline)?;
    let policy = model_policy(offline, no_download);
    match client.send(DaemonRequest::ListFiles {
        source,
        options: IndexRuntimeOptions::with_encoder(
            EncoderSpec::model2vec(model, policy),
            CacheConfig::Platform,
        ),
        limit,
    }) {
        Ok(result @ DaemonResult::ListFiles { .. }) => Ok(Some(result)),
        Ok(other) => bail!("unexpected daemon response: {other:?}"),
        Err(_) => Ok(None),
    }
}

fn run_status(
    path: &str,
    json_output: bool,
    model: Option<String>,
    offline: bool,
    no_download: bool,
) -> Result<()> {
    if let Some(DaemonResult::IndexStatus(status)) =
        try_daemon_index_status(path, model.as_deref(), offline, no_download)?
    {
        print_status_output(
            &status.source.source,
            json_output,
            status.stats,
            status.semantic_loaded,
            status.semantic_loaded,
            0,
        )?;
        return Ok(());
    }
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
    print_status_output(
        path,
        json_output,
        stats,
        semantic_index_available(path),
        index.semantic_loaded(),
        elapsed_ms,
    )
}

fn print_status_output(
    source: &str,
    json_output: bool,
    stats: IndexStats,
    semantic_index_available: bool,
    semantic_index_loaded: bool,
    elapsed_ms: u128,
) -> Result<()> {
    let payload = json!({
        "source": source,
        "index_stats": stats,
        "elapsed_ms": elapsed_ms,
        "semantic_index_available": semantic_index_available,
        "semantic_index_loaded": semantic_index_loaded,
    });
    if json_output {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!(
            "Index status for {source:?}: {} files, {} chunks, languages: {}. Semantic index available: {}. Semantic index loaded: {}.",
            stats.indexed_files,
            stats.total_chunks,
            format_languages(&stats.languages),
            semantic_index_available,
            semantic_index_loaded
        );
    }
    Ok(())
}

fn try_daemon_index_status(
    path: &str,
    model: Option<&str>,
    offline: bool,
    no_download: bool,
) -> Result<Option<DaemonResult>> {
    let Some(client) = daemon_client_if_running()? else {
        return Ok(None);
    };
    let source = SourceSpec::resolve(path, None, offline)?;
    let policy = model_policy(offline, no_download);
    match client.send(DaemonRequest::IndexStatus {
        source,
        options: IndexRuntimeOptions::with_encoder(
            EncoderSpec::model2vec(model, policy),
            CacheConfig::Platform,
        ),
    }) {
        Ok(result @ DaemonResult::IndexStatus(_)) => Ok(Some(result)),
        Ok(other) => bail!("unexpected daemon response: {other:?}"),
        Err(_) => Ok(None),
    }
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
    if let Some(DaemonResult::GetChunk { source, chunk }) = try_daemon_get_chunk(
        file_path,
        line,
        path,
        model.as_deref(),
        offline,
        no_download,
    )? {
        print_get_output(&source.source, &chunk, output)?;
        return Ok(());
    }
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
    print_get_output(path, &chunk, output)
}

fn print_get_output(source: &str, chunk: &sifs::Chunk, output: OutputArgs) -> Result<()> {
    let payload = json!({
        "source": source,
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

fn try_daemon_get_chunk(
    file_path: &str,
    line: usize,
    path: &str,
    model: Option<&str>,
    offline: bool,
    no_download: bool,
) -> Result<Option<DaemonResult>> {
    let Some(client) = daemon_client_if_running()? else {
        return Ok(None);
    };
    let source = SourceSpec::resolve(path, None, offline)?;
    let policy = model_policy(offline, no_download);
    match client.send(DaemonRequest::GetChunk {
        source,
        options: IndexRuntimeOptions::with_encoder(
            EncoderSpec::model2vec(model, policy),
            CacheConfig::Platform,
        ),
        file_path: file_path.to_owned(),
        line,
    }) {
        Ok(result @ DaemonResult::GetChunk { .. }) => Ok(Some(result)),
        Ok(other) => bail!("unexpected daemon response: {other:?}"),
        Err(_) => Ok(None),
    }
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

#[allow(clippy::too_many_arguments)]
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
        .to_vec()
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

fn run_daemon_command(command: DaemonCommand) -> Result<()> {
    match command {
        DaemonCommand::Run {
            replace_existing_socket,
        } => {
            let paths = default_daemon_paths()?;
            eprintln!("SIFS daemon listening on {}", paths.socket.display());
            run_foreground(DaemonRuntimeOptions {
                paths,
                replace_existing_socket,
            })
        }
        DaemonCommand::Ping => {
            let client = DaemonClient::new(default_daemon_paths()?);
            match client.send(DaemonRequest::Ping)? {
                DaemonResult::Pong { version } => {
                    println!("SIFS daemon is running: {version}");
                    Ok(())
                }
                other => bail!("unexpected daemon response: {other:?}"),
            }
        }
        DaemonCommand::Status { json } => {
            let client = DaemonClient::new(default_daemon_paths()?);
            match client.send(DaemonRequest::Status)? {
                DaemonResult::Status(status) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&status)?);
                    } else {
                        println!(
                            "SIFS daemon {} (pid {}, protocol {})",
                            status.version, status.pid, status.protocol_version
                        );
                        if status.indexes.is_empty() {
                            println!("Cached indexes: none");
                        } else {
                            println!("Cached indexes:");
                            for index in status.indexes {
                                println!(
                                    "- {}: {} files, {} chunks, semantic_loaded={}",
                                    index.source.display(),
                                    index.stats.indexed_files,
                                    index.stats.total_chunks,
                                    index.semantic_loaded
                                );
                            }
                        }
                    }
                    Ok(())
                }
                other => bail!("unexpected daemon response: {other:?}"),
            }
        }
        DaemonCommand::InstallAgent { dry_run, force } => install_launch_agent(dry_run, force),
        DaemonCommand::UninstallAgent { dry_run } => uninstall_launch_agent(dry_run),
    }
}

fn install_launch_agent(dry_run: bool, force: bool) -> Result<()> {
    let exe = std::env::current_exe().context("resolve current sifs executable")?;
    warn_if_development_binary(&exe);
    if !dry_run && stable_binary_path(&exe) == "no" {
        bail!(
            "refusing to install LaunchAgent for development binary {}. Install with `cargo install --locked sifs` or Homebrew first.",
            exe.display()
        );
    }
    let plist_path = launch_agent_path()?;
    let plist = launch_agent_plist(&exe)?;
    if dry_run {
        println!("{}", plist);
        return Ok(());
    }
    if plist_path.exists() && !force {
        bail!(
            "SIFS LaunchAgent already exists at {}. Re-run with --force to replace it.",
            plist_path.display()
        );
    }
    let parent = plist_path
        .parent()
        .context("LaunchAgent path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create LaunchAgents directory {}", parent.display()))?;
    fs::write(&plist_path, plist)
        .with_context(|| format!("write LaunchAgent {}", plist_path.display()))?;
    let _ = ProcessCommand::new("launchctl")
        .args([
            "bootout",
            &format!("gui/{}", current_uid_string()),
            plist_path.to_str().unwrap(),
        ])
        .output();
    let output = ProcessCommand::new("launchctl")
        .args([
            "bootstrap",
            &format!("gui/{}", current_uid_string()),
            plist_path.to_str().unwrap(),
        ])
        .output()
        .context("run launchctl bootstrap")?;
    if !output.status.success() {
        bail!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    println!("Installed SIFS LaunchAgent at {}", plist_path.display());
    Ok(())
}

fn uninstall_launch_agent(dry_run: bool) -> Result<()> {
    let plist_path = launch_agent_path()?;
    if dry_run {
        println!("Would unload and remove {}", plist_path.display());
        return Ok(());
    }
    if plist_path.exists() {
        let _ = ProcessCommand::new("launchctl")
            .args([
                "bootout",
                &format!("gui/{}", current_uid_string()),
                plist_path.to_str().unwrap(),
            ])
            .output();
        fs::remove_file(&plist_path)
            .with_context(|| format!("remove LaunchAgent {}", plist_path.display()))?;
        println!("Removed SIFS LaunchAgent at {}", plist_path.display());
    } else {
        println!("No SIFS LaunchAgent found at {}", plist_path.display());
    }
    Ok(())
}

fn launch_agent_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents/dev.sifs.daemon.plist"))
}

fn launch_agent_plist(exe: &Path) -> Result<String> {
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>dev.sifs.daemon</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>daemon</string>
    <string>run</string>
    <string>--replace-existing-socket</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{}</string>
  <key>StandardErrorPath</key>
  <string>{}</string>
</dict>
</plist>
"#,
        xml_escape(&exe.to_string_lossy()),
        xml_escape(&default_daemon_paths()?.log_file.to_string_lossy()),
        xml_escape(&default_daemon_paths()?.log_file.to_string_lossy()),
    ))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn current_uid_string() -> String {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid().to_string() }
    }
    #[cfg(not(unix))]
    {
        "0".to_owned()
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

struct McpInstallOptions {
    client: McpClientArg,
    scope: Option<McpScopeArg>,
    source: Option<String>,
    ref_name: Option<String>,
    name: String,
    offline: bool,
    no_download: bool,
    cache_dir: Option<PathBuf>,
    no_cache: bool,
    project_cache: bool,
    force: bool,
    dry_run: bool,
}

struct McpDoctorOptions {
    source: String,
    ref_name: Option<String>,
    offline: bool,
    no_download: bool,
    cache_dir: Option<PathBuf>,
    no_cache: bool,
    project_cache: bool,
}

struct McpConfig {
    sifs_path: PathBuf,
    source: Option<String>,
    ref_name: Option<String>,
    offline: bool,
    no_download: bool,
    cache_dir: Option<PathBuf>,
    no_cache: bool,
    project_cache: bool,
}

fn run_mcp_install(options: McpInstallOptions) -> Result<()> {
    let source = options
        .source
        .as_deref()
        .map(|source| resolve_mcp_source(Some(source), options.offline))
        .transpose()?;
    let config = McpConfig {
        sifs_path: std::env::current_exe().context("resolve current sifs executable")?,
        source,
        ref_name: options.ref_name,
        offline: options.offline,
        no_download: options.no_download,
        cache_dir: options.cache_dir,
        no_cache: options.no_cache,
        project_cache: options.project_cache,
    };
    warn_if_development_binary(&config.sifs_path);
    if !options.dry_run && stable_binary_path(&config.sifs_path) == "no" {
        bail!(
            "refusing to write durable MCP config for development binary {}. Install with `cargo install --locked sifs` or Homebrew, then rerun this command.",
            config.sifs_path.display()
        );
    }

    match options.client {
        McpClientArg::Codex => {
            install_codex(&options.name, &config, options.force, options.dry_run)?
        }
        McpClientArg::Claude => install_claude(
            &options.name,
            options.scope,
            &config,
            options.force,
            options.dry_run,
        )?,
        McpClientArg::All => {
            install_codex(&options.name, &config, options.force, options.dry_run)?;
            install_claude(
                &options.name,
                options.scope,
                &config,
                options.force,
                options.dry_run,
            )?;
        }
    }
    Ok(())
}

fn run_mcp_doctor(options: McpDoctorOptions) -> Result<()> {
    let source = resolve_mcp_source(Some(&options.source), options.offline)?;
    let config = McpConfig {
        sifs_path: std::env::current_exe().context("resolve current sifs executable")?,
        source: Some(source),
        ref_name: options.ref_name,
        offline: options.offline,
        no_download: options.no_download,
        cache_dir: options.cache_dir,
        no_cache: options.no_cache,
        project_cache: options.project_cache,
    };
    println!("SIFS MCP doctor");
    println!("SIFS executable: {}", config.sifs_path.display());
    println!(
        "Stable install path: {}",
        stable_binary_path(&config.sifs_path)
    );
    println!("Codex CLI: {}", command_status("codex"));
    println!("Claude CLI: {}", command_status("claude"));
    println!(
        "MCP command: {}",
        display_command(&mcp_server_command(&config))
    );
    report_mcp_handshake(&config, HandshakeFraming::LineDelimited);
    report_mcp_handshake(&config, HandshakeFraming::ContentLength);

    if let Some(source) = &config.source
        && !is_git_url(source)
    {
        let smoke = ProcessCommand::new(&config.sifs_path)
            .args([
                "search",
                "sifs_mcp_smoke",
                source,
                "--mode",
                "bm25",
                "--offline",
                "--no-cache",
            ])
            .output();
        match smoke {
            Ok(output) if output.status.success() => println!("BM25 smoke: passed"),
            Ok(output) => println!(
                "BM25 smoke: failed ({})",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(error) => println!("BM25 smoke: could not run ({error})"),
        }
    } else if config.source.as_deref().map(is_git_url).unwrap_or(false) {
        println!("BM25 smoke: skipped for Git URL source");
    } else {
        println!(
            "BM25 smoke: skipped because MCP install is daemon-first and not pinned to a source"
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum HandshakeFraming {
    LineDelimited,
    ContentLength,
}

impl HandshakeFraming {
    fn label(self) -> &'static str {
        match self {
            HandshakeFraming::LineDelimited => "newline",
            HandshakeFraming::ContentLength => "Content-Length",
        }
    }
}

fn report_mcp_handshake(config: &McpConfig, framing: HandshakeFraming) {
    match mcp_handshake_smoke(config, framing) {
        Ok(elapsed) => println!(
            "MCP handshake ({}): passed ({} ms)",
            framing.label(),
            elapsed.as_millis()
        ),
        Err(error) => println!("MCP handshake ({}): failed ({error})", framing.label()),
    }
}

fn mcp_handshake_smoke(config: &McpConfig, framing: HandshakeFraming) -> Result<Duration> {
    let input = mcp_initialize_probe(framing)?;
    let started = Instant::now();
    let output = run_mcp_probe(config, &input, Duration::from_secs(5))?;
    let elapsed = started.elapsed();
    if !output.status.success() {
        bail!(
            "server exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    validate_mcp_initialize_response(framing, &output.stdout)?;
    Ok(elapsed)
}

fn mcp_initialize_probe(framing: HandshakeFraming) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "sifs-doctor", "version": env!("CARGO_PKG_VERSION")}
        }
    }))?;
    Ok(match framing {
        HandshakeFraming::LineDelimited => {
            let mut input = body;
            input.push(b'\n');
            input
        }
        HandshakeFraming::ContentLength => {
            let mut input = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
            input.extend(body);
            input
        }
    })
}

fn run_mcp_probe(config: &McpConfig, input: &[u8], timeout: Duration) -> Result<Output> {
    let mut child = ProcessCommand::new(&config.sifs_path)
        .args(mcp_args(config))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "launch MCP command {}",
                display_command(&mcp_server_command(config))
            )
        })?;

    child
        .stdin
        .as_mut()
        .context("open MCP probe stdin")?
        .write_all(input)
        .context("write MCP initialize probe")?;
    drop(child.stdin.take());

    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            bail!(
                "timed out after {} ms; stderr: {}",
                timeout.as_millis(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn validate_mcp_initialize_response(framing: HandshakeFraming, stdout: &[u8]) -> Result<()> {
    let value = match framing {
        HandshakeFraming::LineDelimited => {
            let line = std::str::from_utf8(stdout)
                .context("decode newline MCP response")?
                .lines()
                .next()
                .context("missing newline MCP response")?;
            serde_json::from_str::<Value>(line).context("parse newline MCP response")?
        }
        HandshakeFraming::ContentLength => parse_content_length_response(stdout)?,
    };
    if value.get("id").and_then(Value::as_i64) != Some(1) {
        bail!("initialize response id mismatch: {value}");
    }
    if value
        .pointer("/result/protocolVersion")
        .and_then(Value::as_str)
        .is_none()
    {
        bail!("initialize response missing protocolVersion: {value}");
    }
    Ok(())
}

fn parse_content_length_response(stdout: &[u8]) -> Result<Value> {
    let separator = b"\r\n\r\n";
    let header_end = stdout
        .windows(separator.len())
        .position(|window| window == separator)
        .context("missing Content-Length response separator")?;
    let header = std::str::from_utf8(&stdout[..header_end]).context("decode MCP headers")?;
    let length = header
        .strip_prefix("Content-Length: ")
        .context("missing Content-Length response header")?
        .parse::<usize>()
        .context("parse response Content-Length")?;
    let body_start = header_end + separator.len();
    let body_end = body_start + length;
    if stdout.len() < body_end {
        bail!("short Content-Length response body");
    }
    serde_json::from_slice(&stdout[body_start..body_end]).context("parse Content-Length body")
}

fn install_codex(name: &str, config: &McpConfig, force: bool, dry_run: bool) -> Result<()> {
    let add_args = codex_add_args(name, config);
    println!("Codex MCP:");
    println!(
        "  {}",
        display_command(&prepend_command("codex", &add_args))
    );
    if dry_run {
        println!(
            "\nCodex config.toml fallback:\n{}",
            codex_toml(name, config)
        );
        return Ok(());
    }
    if command_path("codex").is_none() {
        println!("codex was not found on PATH. Add this to ~/.codex/config.toml instead:");
        println!("{}", codex_toml(name, config));
        return Ok(());
    }
    if mcp_server_exists("codex", name, None)? {
        if !force {
            bail!("Codex MCP server {name:?} already exists. Re-run with --force to replace it.");
        }
        run_checked("codex", &["mcp", "remove", name])?;
    }
    run_checked_owned("codex", &add_args)?;
    println!("Configured Codex MCP server {name:?}.");
    Ok(())
}

fn install_claude(
    name: &str,
    scope: Option<McpScopeArg>,
    config: &McpConfig,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let scope = scope.unwrap_or(McpScopeArg::Local);
    let add_args = claude_add_args(name, scope, config)?;
    println!("Claude Code MCP:");
    println!(
        "  {}",
        display_command(&prepend_command("claude", &add_args))
    );
    if dry_run {
        println!(
            "\nClaude .mcp.json fallback:\n{}",
            claude_project_json(name, config)?
        );
        return Ok(());
    }
    if command_path("claude").is_none() {
        if matches!(scope, McpScopeArg::Project) {
            bail!(
                "claude was not found on PATH. Project-scoped .mcp.json should be written through Claude Code; install Claude or use --dry-run to print the fallback config."
            );
        }
        println!(
            "claude was not found on PATH. Add this project config only in trusted repositories:"
        );
        println!("{}", claude_project_json(name, config)?);
        return Ok(());
    }
    if mcp_server_exists("claude", name, Some(scope))? {
        if !force {
            bail!("Claude MCP server {name:?} already exists. Re-run with --force to replace it.");
        }
        let mut remove_args = vec!["mcp".to_owned(), "remove".to_owned(), name.to_owned()];
        remove_args.push("--scope".to_owned());
        remove_args.push(scope_arg(scope).to_owned());
        run_checked_owned("claude", &remove_args)?;
    }
    run_checked_owned("claude", &add_args)?;
    println!(
        "Configured Claude Code MCP server {name:?} at {} scope.",
        scope_arg(scope)
    );
    Ok(())
}

fn resolve_mcp_source(source: Option<&str>, offline: bool) -> Result<String> {
    let source = match source {
        Some(source) => source.to_owned(),
        None => std::env::current_dir()
            .context("resolve current directory")?
            .to_string_lossy()
            .into_owned(),
    };
    if is_git_url(&source) {
        if offline {
            bail!("--offline does not allow remote Git sources");
        }
        return Ok(source);
    }
    let path = PathBuf::from(source);
    if !path.exists() {
        bail!("local MCP source does not exist: {}", path.display());
    }
    if !path.is_dir() {
        bail!("local MCP source is not a directory: {}", path.display());
    }
    Ok(path
        .canonicalize()
        .with_context(|| format!("canonicalize MCP source {}", path.display()))?
        .to_string_lossy()
        .into_owned())
}

fn mcp_args(config: &McpConfig) -> Vec<String> {
    let mut args = vec!["mcp".to_owned()];
    if let Some(source) = &config.source {
        args.push(source.clone());
    }
    if let Some(ref_name) = &config.ref_name {
        args.push("--ref".to_owned());
        args.push(ref_name.clone());
    }
    if config.offline {
        args.push("--offline".to_owned());
    }
    if config.no_download {
        args.push("--no-download".to_owned());
    }
    if let Some(cache_dir) = &config.cache_dir {
        args.push("--cache-dir".to_owned());
        args.push(cache_dir.to_string_lossy().into_owned());
    }
    if config.no_cache {
        args.push("--no-cache".to_owned());
    }
    if config.project_cache {
        args.push("--project-cache".to_owned());
    }
    args
}

fn mcp_server_command(config: &McpConfig) -> Vec<String> {
    let mut command = vec![config.sifs_path.to_string_lossy().into_owned()];
    command.extend(mcp_args(config));
    command
}

fn codex_add_args(name: &str, config: &McpConfig) -> Vec<String> {
    let mut args = vec![
        "mcp".to_owned(),
        "add".to_owned(),
        name.to_owned(),
        "--".to_owned(),
    ];
    args.extend(mcp_server_command(config));
    args
}

fn claude_add_args(name: &str, scope: McpScopeArg, config: &McpConfig) -> Result<Vec<String>> {
    Ok(vec![
        "mcp".to_owned(),
        "add-json".to_owned(),
        name.to_owned(),
        claude_server_json(config)?,
        "--scope".to_owned(),
        scope_arg(scope).to_owned(),
    ])
}

fn codex_toml(name: &str, config: &McpConfig) -> String {
    let args = mcp_args(config)
        .iter()
        .map(|arg| serde_json::to_string(arg).unwrap())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "[mcp_servers.{name}]\ncommand = {}\nargs = [{args}]\nstartup_timeout_sec = 20\ntool_timeout_sec = 60\n",
        toml_string(&config.sifs_path.to_string_lossy())
    )
}

fn claude_server_json(config: &McpConfig) -> Result<String> {
    Ok(serde_json::to_string(&json!({
        "type": "stdio",
        "command": config.sifs_path.to_string_lossy(),
        "args": mcp_args(config),
        "env": {},
    }))?)
}

fn claude_project_json(name: &str, config: &McpConfig) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({
        "mcpServers": {
            name: {
                "type": "stdio",
                "command": config.sifs_path.to_string_lossy(),
                "args": mcp_args(config),
                "env": {},
            }
        }
    }))?)
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

fn scope_arg(scope: McpScopeArg) -> &'static str {
    match scope {
        McpScopeArg::Local => "local",
        McpScopeArg::Project => "project",
        McpScopeArg::User => "user",
    }
}

fn mcp_server_exists(command: &str, name: &str, _scope: Option<McpScopeArg>) -> Result<bool> {
    let args = vec!["mcp".to_owned(), "get".to_owned(), name.to_owned()];
    let output = ProcessCommand::new(command)
        .args(&args)
        .output()
        .with_context(|| format!("run {command} {}", args.join(" ")))?;
    Ok(output.status.success())
}

fn run_checked(command: &str, args: &[&str]) -> Result<()> {
    let output = ProcessCommand::new(command)
        .args(args)
        .output()
        .with_context(|| format!("run {command} {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "{} failed: {}",
            display_command(&prepend_command(
                command,
                &args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>()
            )),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn run_checked_owned(command: &str, args: &[String]) -> Result<()> {
    let output = ProcessCommand::new(command)
        .args(args)
        .output()
        .with_context(|| format!("run {command} {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "{} failed: {}",
            display_command(&prepend_command(command, args)),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn prepend_command(command: &str, args: &[String]) -> Vec<String> {
    let mut full = vec![command.to_owned()];
    full.extend(args.iter().cloned());
    full
}

fn display_command(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '='))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn command_status(command: &str) -> String {
    command_path(command)
        .map(|path| format!("found at {}", path.display()))
        .unwrap_or_else(|| "not found on PATH".to_owned())
}

fn command_path(command: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

fn stable_binary_path(path: &Path) -> &'static str {
    if path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("target" | "debug" | "release")
        )
    }) {
        "no"
    } else {
        "yes"
    }
}

fn warn_if_development_binary(path: &Path) {
    if stable_binary_path(path) == "no" {
        eprintln!(
            "Warning: sifs is running from {}. Install with `cargo install --locked sifs` or Homebrew before creating durable MCP config.",
            path.display()
        );
    }
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
