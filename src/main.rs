use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use sifs::agent_artifacts::{AgentArtifact, AgentTarget, render_artifact};
use sifs::agent_installer::{AgentMutationOptions, AgentOperation, apply_mutation};
use sifs::daemon::{
    DaemonClient, DaemonRequest, DaemonResult, DaemonRuntimeOptions, IndexRuntimeOptions,
    SearchOptionsWire, SourceSpec, default_daemon_paths, run_foreground,
};
use sifs::update::{UpdateMode, UpdateOptions, run_update};
use sifs::{
    CacheConfig, EncoderSpec, IndexOptions, IndexStats, ModelLoadPolicy, ModelOptions, SearchMode,
    SearchOptions, SearchResult, SifsIndex, cache_summary, format_results, is_git_url,
    load_model_with_options, model_status, platform_cache_root, resolve_chunk,
};
use sifs::{agent_context, feedback, profiles};
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
    #[arg(
        long,
        global = true,
        help = "Assert non-interactive execution; SIFS never prompts and closes child stdin where applicable."
    )]
    no_input: bool,
    #[arg(
        long,
        global = true,
        default_value_t = 30,
        help = "Timeout in seconds for daemon, MCP probe, and external client operations."
    )]
    timeout: u64,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Search a local directory or Git URL with natural-language or code queries.")]
    Search {
        #[arg(help = "Natural-language, code, symbol, or literal query to search for.")]
        query: String,
        #[arg(long, help = "Local directory or Git URL to index and search.")]
        source: Option<String>,
        #[arg(long, help = "Saved profile to use for source and search defaults.")]
        profile: Option<String>,
        #[arg(
            short = 'k',
            long = "limit",
            help = "Maximum number of ranked chunks to print."
        )]
        limit: Option<usize>,
        #[arg(
            short = 'm',
            long = "mode",
            value_enum,
            help = "Ranking mode: hybrid for most searches, bm25 for exact symbols, semantic for conceptual queries."
        )]
        mode: Option<ModeArg>,
        #[arg(
            long = "language",
            help = "Only search chunks with this language label. Repeatable."
        )]
        languages: Vec<String>,
        #[arg(
            long = "filter-path",
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
        #[arg(long, help = "Local directory or Git URL to index and search.")]
        source: Option<String>,
        #[arg(long, help = "Saved profile to use for source and search defaults.")]
        profile: Option<String>,
        #[arg(
            short = 'k',
            long = "limit",
            help = "Maximum number of related chunks to print."
        )]
        limit: Option<usize>,
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
        #[arg(long, help = "Default local directory or Git URL exposed through MCP.")]
        source: Option<String>,
        #[arg(long, help = "Saved profile to use for MCP defaults.")]
        profile: Option<String>,
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
    ListFiles {
        #[arg(long, help = "Local directory or Git URL to index and inspect.")]
        source: Option<String>,
        #[arg(long, help = "Saved profile to use for source defaults.")]
        profile: Option<String>,
        #[arg(long, help = "Maximum number of file paths to print.")]
        limit: Option<usize>,
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
        #[arg(long, help = "Local directory or Git URL to index and inspect.")]
        source: Option<String>,
        #[arg(long, help = "Saved profile to use for source defaults.")]
        profile: Option<String>,
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
        #[arg(long, help = "Local directory or Git URL to index and inspect.")]
        source: Option<String>,
        #[arg(long, help = "Saved profile to use for source defaults.")]
        profile: Option<String>,
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
        #[arg(long, help = "Local directory whose .sifs cache should be removed.")]
        source: Option<String>,
        #[arg(long, help = "Preview removal without changing files.")]
        dry_run: bool,
        #[arg(long, help = "Required to remove a project-local .sifs cache.")]
        force: bool,
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
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
        #[arg(long, help = "Local directory to inspect for cache readiness.")]
        source: Option<String>,
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
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
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Print, install, inspect, or remove SIFS agent integration artifacts.")]
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    #[command(about = "Print SIFS agent, CLI, and MCP capabilities.")]
    Capabilities {
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Print the machine-readable SIFS agent-native contract.")]
    AgentContext {
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Manage persistent SIFS profiles.")]
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    #[command(about = "Record and list local agent feedback.")]
    Feedback {
        #[command(subcommand)]
        command: FeedbackCommand,
    },
    #[command(about = "Check for or install the latest SIFS release through Cargo or Homebrew.")]
    Update {
        #[arg(
            long,
            help = "Check update status without planning or running mutation."
        )]
        check: bool,
        #[arg(
            long,
            conflicts_with = "check",
            help = "Print the package-manager command that would run without changing state."
        )]
        dry_run: bool,
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
        #[arg(
            long,
            default_value_t = 600,
            help = "Timeout in seconds for package-manager update execution."
        )]
        update_timeout: u64,
    },
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
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Inspect local MCP install readiness.")]
    Doctor {
        #[arg(long, help = "Local directory or Git URL the MCP server would expose.")]
        source: Option<String>,
        #[arg(long, help = "Saved profile to use for source defaults.")]
        profile: Option<String>,
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
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
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
    Ping {
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
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
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Remove the macOS LaunchAgent for the SIFS daemon.")]
    UninstallAgent {
        #[arg(long, help = "Print what would be removed without changing files.")]
        dry_run: bool,
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ModelCommand {
    #[command(about = "Download or validate the embedding model in the Hugging Face cache.")]
    Pull {
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Alias for `pull`: download or validate the embedding model.")]
    Fetch {
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
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
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ProfileCommand {
    #[command(about = "Save a persistent SIFS profile.")]
    Save {
        name: String,
        #[arg(long)]
        source: Option<String>,
        #[arg(long = "ref")]
        ref_name: Option<String>,
        #[arg(long, value_enum)]
        mode: Option<ModeArg>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, value_enum)]
        encoder: Option<EncoderArg>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        offline: bool,
        #[arg(long = "no-download")]
        no_download: bool,
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        project_cache: bool,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "List saved profiles.")]
    List {
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Show one saved profile.")]
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Delete one saved profile.")]
    Delete {
        name: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum FeedbackCommand {
    #[command(about = "Record local feedback for SIFS maintainers.")]
    Create {
        message: String,
        #[arg(long)]
        command_context: Option<String>,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "List local feedback entries.")]
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum AgentCommand {
    #[command(about = "Print a target-specific SIFS skill, snippet, or MCP guidance artifact.")]
    Print {
        #[arg(long, value_enum)]
        target: AgentTarget,
        #[arg(long, value_enum)]
        artifact: AgentArtifact,
        #[arg(long, help = "Destination hint used in JSON output and next actions.")]
        destination: Option<PathBuf>,
        #[arg(long, help = "Instruction file hint used for snippets.")]
        file: Option<PathBuf>,
        #[arg(
            long,
            help = "Project source to include in generated project-local guidance."
        )]
        source: Option<String>,
        #[arg(
            long,
            help = "Profile name to include in generated project-local guidance."
        )]
        profile: Option<String>,
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Install a target-specific SIFS skill, snippet, or MCP guidance artifact.")]
    Install {
        #[arg(long, value_enum)]
        target: AgentTarget,
        #[arg(long, value_enum)]
        artifact: AgentArtifact,
        #[arg(long, help = "Skill/package destination path.")]
        destination: Option<PathBuf>,
        #[arg(long, help = "Instruction file for snippet insertion.")]
        file: Option<PathBuf>,
        #[arg(
            long,
            help = "Project source to include in generated project-local guidance."
        )]
        source: Option<String>,
        #[arg(
            long,
            help = "Profile name to include in generated project-local guidance."
        )]
        profile: Option<String>,
        #[arg(long, help = "Preview planned writes without changing files.")]
        dry_run: bool,
        #[arg(long, help = "Replace stale or user-modified managed artifacts.")]
        force: bool,
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Inspect installed SIFS agent artifacts and fallback readiness.")]
    Doctor {
        #[arg(long, value_enum)]
        target: AgentTarget,
        #[arg(long, value_enum, default_value_t = AgentArtifact::All)]
        artifact: AgentArtifact,
        #[arg(long, help = "Print structured JSON output.")]
        json: bool,
    },
    #[command(about = "Remove SIFS-managed skill files or instruction snippets.")]
    Uninstall {
        #[arg(long, value_enum)]
        target: AgentTarget,
        #[arg(long, value_enum)]
        artifact: AgentArtifact,
        #[arg(long, help = "Skill/package destination path.")]
        destination: Option<PathBuf>,
        #[arg(long, help = "Instruction file for snippet removal.")]
        file: Option<PathBuf>,
        #[arg(long, help = "Preview planned removals without changing files.")]
        dry_run: bool,
        #[arg(long, help = "Remove stale or user-modified managed artifacts.")]
        force: bool,
        #[arg(long, help = "Print structured JSON output.")]
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
    let timeout = Duration::from_secs(cli.timeout);
    let _no_input = cli.no_input;
    match cli.command {
        Some(Command::Search {
            query,
            source,
            profile,
            limit,
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
        }) => {
            let resolved = resolve_invocation(
                profile.as_deref(),
                source,
                mode,
                limit,
                model,
                encoder,
                offline,
                no_download,
                cache_config(cache_dir, no_cache, project_cache),
            )?;
            run_search(SearchCommand {
                query,
                source: resolved.source,
                limit: resolved.limit,
                mode: resolved.mode,
                languages,
                filter_paths,
                context_lines,
                explain,
                output,
                model: resolved.model,
                encoder: resolved.encoder,
                offline: resolved.offline,
                no_download: resolved.no_download,
                cache: resolved.cache,
            })?
        }
        Some(Command::FindRelated {
            file_path,
            line,
            source,
            profile,
            limit,
            output,
            model,
            encoder,
            offline,
            no_download,
            cache_dir,
            no_cache,
            project_cache,
        }) => {
            let resolved = resolve_invocation(
                profile.as_deref(),
                source,
                None,
                limit,
                model,
                encoder,
                offline,
                no_download,
                cache_config(cache_dir, no_cache, project_cache),
            )?;
            if let Some(result) = try_daemon_find_related(
                &resolved.source,
                &file_path,
                line,
                resolved.limit,
                resolved.encoder,
                resolved.model.as_deref(),
                resolved.offline,
                resolved.no_download,
                resolved.cache.clone(),
            )? {
                print_find_related_output(
                    &result.source.source,
                    &file_path,
                    line,
                    resolved.limit,
                    result.stats,
                    u128::from(result.elapsed_ms),
                    &result.results,
                    &output,
                )?;
                return Ok(());
            }
            let policy = model_policy(resolved.offline, resolved.no_download);
            let started = Instant::now();
            let index = build_hybrid_index(
                &resolved.source,
                encoder_spec(resolved.encoder, resolved.model.as_deref(), policy),
                resolved.cache,
                resolved.offline,
            )?;
            let Some(chunk) = resolve_chunk(&index.chunks, &file_path, line) else {
                eprintln!("No chunk found at {file_path}:{line}.");
                std::process::exit(1);
            };
            let results = index.find_related(&chunk, resolved.limit)?;
            let elapsed_ms = started.elapsed().as_millis();
            print_find_related_output(
                &resolved.source,
                &file_path,
                line,
                resolved.limit,
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
            source,
            json,
            offline,
            no_download,
        }) => run_doctor(
            source.as_deref().unwrap_or("."),
            encoder,
            model.as_deref(),
            model_policy(offline, no_download),
            offline,
            json,
        )?,
        Some(Command::Mcp {
            command,
            source,
            profile,
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
                json,
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
                json,
            })?,
            Some(McpCommand::Doctor {
                source,
                profile,
                ref_name,
                offline,
                no_download,
                cache_dir,
                no_cache,
                project_cache,
                json,
            }) => run_mcp_doctor(McpDoctorOptions {
                source,
                profile,
                ref_name,
                offline,
                no_download,
                cache_dir,
                no_cache,
                project_cache,
                json,
                timeout,
            })?,
            None => {
                let resolved = resolve_invocation(
                    profile.as_deref(),
                    source,
                    None,
                    None,
                    model,
                    EncoderArg::Model2Vec,
                    offline,
                    no_download,
                    cache_config(cache_dir, no_cache, project_cache),
                )?;
                let policy = model_policy(resolved.offline, resolved.no_download);
                if offline && is_git_url(&resolved.source) {
                    bail!("--offline does not allow remote Git sources");
                }
                sifs::mcp::serve_with_options(
                    Some(resolved.source),
                    ref_name,
                    ModelOptions::new(resolved.model.as_deref(), policy),
                    resolved.cache,
                    resolved.offline,
                )?;
            }
        },
        Some(Command::ListFiles {
            source,
            profile,
            limit,
            output,
            model,
            offline,
            no_download,
        }) => {
            let resolved = resolve_invocation(
                profile.as_deref(),
                source,
                None,
                None,
                model,
                EncoderArg::Model2Vec,
                offline,
                no_download,
                CacheConfig::Platform,
            )?;
            run_files(
                &resolved.source,
                limit.unwrap_or(200),
                output,
                resolved.model,
                resolved.offline,
                resolved.no_download,
            )?
        }
        Some(Command::Status {
            source,
            profile,
            json,
            model,
            offline,
            no_download,
        }) => {
            let resolved = resolve_invocation(
                profile.as_deref(),
                source,
                None,
                None,
                model,
                EncoderArg::Model2Vec,
                offline,
                no_download,
                CacheConfig::Platform,
            )?;
            run_status(
                &resolved.source,
                json,
                resolved.model,
                resolved.offline,
                resolved.no_download,
            )?
        }
        Some(Command::Get {
            file_path,
            line,
            source,
            profile,
            output,
            model,
            offline,
            no_download,
        }) => {
            let resolved = resolve_invocation(
                profile.as_deref(),
                source,
                None,
                None,
                model,
                EncoderArg::Model2Vec,
                offline,
                no_download,
                CacheConfig::Platform,
            )?;
            run_get(
                &file_path,
                line,
                &resolved.source,
                output,
                resolved.model,
                resolved.offline,
                resolved.no_download,
            )?
        }
        Some(Command::Clean {
            source,
            dry_run,
            force,
            json,
        }) => run_clean(source.as_deref().unwrap_or("."), dry_run, force, json)?,
        Some(Command::Init { force, json }) => run_init(force, json)?,
        Some(Command::Agent { command }) => run_agent(command)?,
        Some(Command::Capabilities { json }) => print_capabilities(json)?,
        Some(Command::AgentContext { json }) => print_agent_context(json)?,
        Some(Command::Profile { command }) => run_profile(command)?,
        Some(Command::Feedback { command }) => run_feedback(command)?,
        Some(Command::Daemon { command }) => run_daemon_command(command, timeout)?,
        Some(Command::Update {
            check,
            dry_run,
            json,
            update_timeout,
        }) => run_update_command(
            check,
            dry_run,
            json,
            timeout,
            Duration::from_secs(update_timeout),
        )?,
        None => {
            Cli::command().print_help()?;
            println!();
        }
    }
    Ok(())
}

struct SearchCommand {
    query: String,
    source: String,
    limit: usize,
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

struct ResolvedInvocation {
    source: String,
    mode: SearchMode,
    limit: usize,
    model: Option<String>,
    encoder: EncoderArg,
    offline: bool,
    no_download: bool,
    cache: CacheConfig,
}

#[allow(clippy::too_many_arguments)]
fn resolve_invocation(
    profile_name: Option<&str>,
    source: Option<String>,
    mode: Option<ModeArg>,
    limit: Option<usize>,
    model: Option<String>,
    encoder: EncoderArg,
    offline: bool,
    no_download: bool,
    cache: CacheConfig,
) -> Result<ResolvedInvocation> {
    let profile = match profile_name {
        Some(name) => Some(profiles::get_profile(&platform_cache_root()?, name)?),
        None => None,
    };
    let source = source
        .or_else(|| std::env::var("SIFS_SOURCE").ok())
        .or_else(|| profile.as_ref().and_then(|profile| profile.source.clone()))
        .unwrap_or_else(|| ".".to_owned());
    let mode = mode
        .map(SearchMode::from)
        .or_else(|| profile.as_ref().and_then(|profile| profile.mode))
        .unwrap_or(SearchMode::Hybrid);
    let limit = limit
        .or_else(|| profile.as_ref().and_then(|profile| profile.limit))
        .unwrap_or(5);
    if limit == 0 {
        bail!("--limit must be at least 1");
    }
    let model = model.or_else(|| profile.as_ref().and_then(|profile| profile.model.clone()));
    let encoder = profile
        .as_ref()
        .and_then(|profile| profile.encoder.as_deref())
        .and_then(parse_encoder_name)
        .unwrap_or(encoder);
    let offline = offline
        || profile
            .as_ref()
            .and_then(|profile| profile.offline)
            .unwrap_or(false);
    let no_download = no_download
        || profile
            .as_ref()
            .and_then(|profile| profile.no_download)
            .unwrap_or(false);
    let cache = match profile.as_ref() {
        Some(profile) if matches!(cache, CacheConfig::Platform) => {
            if profile.no_cache.unwrap_or(false) {
                CacheConfig::Disabled
            } else if profile.project_cache.unwrap_or(false) {
                CacheConfig::Project
            } else if let Some(path) = &profile.cache_dir {
                CacheConfig::Custom(path.clone())
            } else {
                cache
            }
        }
        _ => cache,
    };
    Ok(ResolvedInvocation {
        source,
        mode,
        limit,
        model,
        encoder,
        offline,
        no_download,
        cache,
    })
}

fn parse_encoder_name(value: &str) -> Option<EncoderArg> {
    match value {
        "model2vec" => Some(EncoderArg::Model2Vec),
        "hashing" => Some(EncoderArg::Hashing),
        _ => None,
    }
}

fn run_search(command: SearchCommand) -> Result<()> {
    if let Some(result) = try_daemon_search(&command)? {
        print_search_output(
            &result.query,
            &result.source.source,
            &SearchOptions {
                top_k: command.limit,
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
        &command.source,
        command.mode,
        encoder_spec(command.encoder, command.model.as_deref(), policy),
        command.cache,
        command.offline,
    )?;
    let warnings = search_warnings(&index, &command.filter_paths, &command.languages)
        .into_iter()
        .chain(context_warnings(&command.source, command.context_lines))
        .collect::<Vec<_>>();
    let mut options = SearchOptions::new(command.limit).with_mode(command.mode);
    options.filter_languages = command.languages;
    options.filter_paths = command.filter_paths;
    let results = index.search_with(&command.query, &options)?;
    let elapsed_ms = started.elapsed().as_millis();

    print_search_output(
        &command.query,
        &command.source,
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
    let source = SourceSpec::resolve(&command.source, None, command.offline)?;
    let policy = model_policy(command.offline, command.no_download);
    let runtime_options = match command.mode {
        SearchMode::Bm25 => IndexRuntimeOptions::sparse(command.cache.clone()),
        SearchMode::Semantic | SearchMode::Hybrid => IndexRuntimeOptions::with_encoder(
            encoder_spec(command.encoder, command.model.as_deref(), policy),
            command.cache.clone(),
        ),
    };
    let mut search = SearchOptions::new(command.limit).with_mode(command.mode);
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
        "limit": top_k,
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
                    "limit": top_k,
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
        "truncated": total > shown.len(),
        "hint": if total > shown.len() { Some("Increase --limit or narrow by source to inspect more indexed files.") } else { None },
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
        eprintln!(
            "No chunk found at {file_path}:{line}. Use `sifs list-files` to check indexed paths."
        );
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

fn run_clean(path: &str, dry_run: bool, force: bool, json_output: bool) -> Result<()> {
    if is_git_url(path) {
        bail!("clean only supports local directories");
    }
    let cache_dir = Path::new(path).join(".sifs");
    if dry_run {
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "dry_run": true,
                    "would_change": cache_dir.exists(),
                    "cache_dir": cache_dir,
                }))?
            );
        } else if cache_dir.exists() {
            println!("Would remove {}", cache_dir.display());
        } else {
            println!("No SIFS cache found at {}", cache_dir.display());
        }
    } else if cache_dir.exists() {
        if !force {
            bail!("clean requires --force; run with --dry-run to preview");
        }
        fs::remove_dir_all(&cache_dir)
            .with_context(|| format!("remove SIFS cache {}", cache_dir.display()))?;
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "changed": true,
                    "cache_dir": cache_dir,
                }))?
            );
        } else {
            println!("Removed {}", cache_dir.display());
        }
    } else {
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "changed": false,
                    "cache_dir": cache_dir,
                }))?
            );
        } else {
            println!("No SIFS cache found at {}", cache_dir.display());
        }
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
        "filter_languages": options.filter_languages,
        "filter_paths": options.filter_paths,
        "index_stats": stats,
        "elapsed_ms": elapsed_ms,
        "warnings": warnings,
        "limit": options.top_k,
        "truncated": results.len() >= options.top_k,
        "hint": if results.len() >= options.top_k { Some("Increase --limit or add --language/--filter-path to narrow the search.") } else { None },
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
        "limit": options.top_k,
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

fn search_warnings(
    index: &SifsIndex,
    filter_paths: &[String],
    filter_languages: &[String],
) -> Vec<String> {
    let mut warnings = Vec::new();
    if !filter_paths.is_empty() {
        let indexed_files = index.indexed_files();
        for path in filter_paths {
            if !indexed_files.iter().any(|indexed| indexed == path) {
                let normalized = path.strip_prefix("./").unwrap_or(path);
                let suggestion = indexed_files
                    .iter()
                    .find(|indexed| indexed.as_str() == normalized);
                if let Some(suggestion) = suggestion {
                    warnings.push(format!(
                        "No indexed file exactly matched filter-path {path:?}. Did you mean {suggestion:?}?"
                    ));
                } else {
                    warnings.push(format!(
                        "No indexed file matched filter-path {path:?}. Use `sifs list-files --json` to inspect indexed paths."
                    ));
                }
            }
        }
    }
    if !filter_languages.is_empty() {
        let languages = index.stats().languages;
        for language in filter_languages {
            if !languages.contains_key(language) {
                let valid = languages.keys().cloned().collect::<Vec<_>>().join(", ");
                warnings.push(format!(
                    "No indexed chunks matched language {language:?}. Valid indexed languages: {valid}"
                ));
            }
        }
    }
    warnings
}

fn print_explanation(
    query: &str,
    source: &str,
    options: &SearchOptions,
    elapsed_ms: u128,
    warnings: &[String],
) {
    println!(
        "Query: {query:?}; source: {source}; mode: {}; limit: {}; languages: [{}]; paths: [{}]; elapsed_ms: {elapsed_ms}",
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

fn run_daemon_command(command: DaemonCommand, timeout: Duration) -> Result<()> {
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
        DaemonCommand::Ping { json } => {
            let client = DaemonClient::new(default_daemon_paths()?).with_timeout(timeout);
            match client.send(DaemonRequest::Ping)? {
                DaemonResult::Pong { version } => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "status": "running",
                                "version": version,
                            }))?
                        );
                    } else {
                        println!("SIFS daemon is running: {version}");
                    }
                    Ok(())
                }
                other => bail!("unexpected daemon response: {other:?}"),
            }
        }
        DaemonCommand::Status { json } => {
            let client = DaemonClient::new(default_daemon_paths()?).with_timeout(timeout);
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
        DaemonCommand::InstallAgent {
            dry_run,
            force,
            json,
        } => install_launch_agent(dry_run, force, json),
        DaemonCommand::UninstallAgent { dry_run, json } => uninstall_launch_agent(dry_run, json),
    }
}

fn install_launch_agent(dry_run: bool, force: bool, json_output: bool) -> Result<()> {
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
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "dry_run": true,
                    "would_change": true,
                    "plist_path": plist_path,
                    "plist": plist,
                    "program": exe,
                    "args": ["daemon", "run", "--replace-existing-socket"],
                }))?
            );
        } else {
            println!("{}", plist);
        }
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
        .stdin(Stdio::null())
        .output();
    let output = ProcessCommand::new("launchctl")
        .args([
            "bootstrap",
            &format!("gui/{}", current_uid_string()),
            plist_path.to_str().unwrap(),
        ])
        .stdin(Stdio::null())
        .output()
        .context("run launchctl bootstrap")?;
    if !output.status.success() {
        bail!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "changed": true,
                "plist_path": plist_path,
                "program": exe,
            }))?
        );
    } else {
        println!("Installed SIFS LaunchAgent at {}", plist_path.display());
    }
    Ok(())
}

fn uninstall_launch_agent(dry_run: bool, json_output: bool) -> Result<()> {
    let plist_path = launch_agent_path()?;
    if dry_run {
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "dry_run": true,
                    "would_change": plist_path.exists(),
                    "plist_path": plist_path,
                }))?
            );
        } else {
            println!("Would unload and remove {}", plist_path.display());
        }
        return Ok(());
    }
    if plist_path.exists() {
        let _ = ProcessCommand::new("launchctl")
            .args([
                "bootout",
                &format!("gui/{}", current_uid_string()),
                plist_path.to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .output();
        fs::remove_file(&plist_path)
            .with_context(|| format!("remove LaunchAgent {}", plist_path.display()))?;
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "changed": true,
                    "plist_path": plist_path,
                }))?
            );
        } else {
            println!("Removed SIFS LaunchAgent at {}", plist_path.display());
        }
    } else {
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "changed": false,
                    "plist_path": plist_path,
                }))?
            );
        } else {
            println!("No SIFS LaunchAgent found at {}", plist_path.display());
        }
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
        ModelCommand::Pull { model, json } | ModelCommand::Fetch { model, json } => {
            let options = ModelOptions::new(model.as_deref(), ModelLoadPolicy::AllowDownload);
            let started = Instant::now();
            load_model_with_options(&options)?;
            let status = model_status(Some(&options.model));
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "model": options.model,
                        "available": status.available(),
                        "elapsed_ms": started.elapsed().as_millis(),
                        "changed": true,
                    }))?
                );
            } else {
                println!("Model is available: {}", options.model);
            }
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
    json_output: bool,
) -> Result<()> {
    if json_output {
        let source = if is_git_url(path) {
            json!({
                "kind": "git_url",
                "source": path,
                "offline_allowed": !offline,
            })
        } else {
            let path_buf = PathBuf::from(path);
            let cache_dir = path_buf.join(".sifs");
            json!({
                "kind": "local_path",
                "path": path,
                "exists": path_buf.exists(),
                "is_directory": path_buf.is_dir(),
                "cache_dir": cache_dir,
                "cache_writable": if path_buf.is_dir() {
                    fs::metadata(if cache_dir.exists() { &cache_dir } else { &path_buf })
                        .map(|metadata| !metadata.permissions().readonly())
                        .unwrap_or(false)
                } else {
                    false
                },
            })
        };
        let semantic = match encoder {
            EncoderArg::Hashing => json!({
                "encoder": "hashing",
                "ready": true,
                "message": "ready without model files",
            }),
            EncoderArg::Model2Vec => {
                let options = ModelOptions::new(model, policy);
                let status = model_status(Some(&options.model));
                json!({
                    "encoder": "model2vec",
                    "model": status.model,
                    "available": status.available(),
                    "tokenizer": status.tokenizer,
                    "safetensors": status.safetensors,
                    "config": status.config,
                    "load_policy": format!("{:?}", policy),
                })
            }
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "source": source,
                "semantic": semantic,
                "offline": offline,
            }))?
        );
        return Ok(());
    }
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
    json: bool,
}

struct McpDoctorOptions {
    source: Option<String>,
    profile: Option<String>,
    ref_name: Option<String>,
    offline: bool,
    no_download: bool,
    cache_dir: Option<PathBuf>,
    no_cache: bool,
    project_cache: bool,
    json: bool,
    timeout: Duration,
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
    if options.dry_run && options.json {
        let mut clients = Vec::new();
        if matches!(options.client, McpClientArg::Codex | McpClientArg::All) {
            clients.push(json!({
                "client": "codex",
                "name": options.name,
                "commands": [{"program": "codex", "args": codex_add_args(&options.name, &config)}],
                "fallback_config": {"path_hint": "~/.codex/config.toml", "toml": codex_toml(&options.name, &config)}
            }));
        }
        if matches!(options.client, McpClientArg::Claude | McpClientArg::All) {
            let scope = options.scope.unwrap_or(McpScopeArg::Local);
            clients.push(json!({
                "client": "claude",
                "name": options.name,
                "scope": scope_arg(scope),
                "commands": [{"program": "claude", "args": claude_add_args(&options.name, scope, &config)?}],
                "fallback_config": {"path_hint": ".mcp.json", "json": serde_json::from_str::<Value>(&claude_project_json(&options.name, &config)?)?}
            }));
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "dry_run": true,
                "would_modify": false,
                "clients": clients,
            }))?
        );
        return Ok(());
    }

    match options.client {
        McpClientArg::Codex => install_codex(
            &options.name,
            &config,
            options.force,
            options.dry_run,
            options.json,
        )?,
        McpClientArg::Claude => install_claude(
            &options.name,
            options.scope,
            &config,
            options.force,
            options.dry_run,
            options.json,
        )?,
        McpClientArg::All => {
            install_codex(
                &options.name,
                &config,
                options.force,
                options.dry_run,
                options.json,
            )?;
            install_claude(
                &options.name,
                options.scope,
                &config,
                options.force,
                options.dry_run,
                options.json,
            )?;
        }
    }
    Ok(())
}

fn run_mcp_doctor(options: McpDoctorOptions) -> Result<()> {
    let resolved = resolve_invocation(
        options.profile.as_deref(),
        options.source,
        None,
        None,
        None,
        EncoderArg::Model2Vec,
        options.offline,
        options.no_download,
        cache_config(
            options.cache_dir.clone(),
            options.no_cache,
            options.project_cache,
        ),
    )?;
    let source = resolve_mcp_source(Some(&resolved.source), resolved.offline)?;
    let config = McpConfig {
        sifs_path: std::env::current_exe().context("resolve current sifs executable")?,
        source: Some(source),
        ref_name: options.ref_name,
        offline: resolved.offline,
        no_download: resolved.no_download,
        cache_dir: match resolved.cache.clone() {
            CacheConfig::Custom(path) => Some(path),
            _ => None,
        },
        no_cache: matches!(resolved.cache, CacheConfig::Disabled),
        project_cache: matches!(resolved.cache, CacheConfig::Project),
    };
    if options.json {
        let newline =
            mcp_handshake_smoke(&config, HandshakeFraming::LineDelimited, options.timeout)
                .map(|elapsed| json!({"status": "passed", "elapsed_ms": elapsed.as_millis()}))
                .unwrap_or_else(|error| json!({"status": "failed", "error": error.to_string()}));
        let content_length =
            mcp_handshake_smoke(&config, HandshakeFraming::ContentLength, options.timeout)
                .map(|elapsed| json!({"status": "passed", "elapsed_ms": elapsed.as_millis()}))
                .unwrap_or_else(|error| json!({"status": "failed", "error": error.to_string()}));
        let bm25_smoke = bm25_smoke_json(&config);
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "sifs_executable": config.sifs_path,
                "stable_install_path": stable_binary_path(&config.sifs_path) == "yes",
                "clients": {
                    "codex": {"available": command_path("codex").is_some(), "path": command_path("codex")},
                    "claude": {"available": command_path("claude").is_some(), "path": command_path("claude")}
                },
                "mcp_command": mcp_server_command(&config),
                "source": config.source,
                "handshake": {
                    "newline": newline,
                    "content_length": content_length,
                },
                "bm25_smoke": bm25_smoke,
            }))?
        );
        return Ok(());
    }
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
    report_mcp_handshake(&config, HandshakeFraming::LineDelimited, options.timeout);
    report_mcp_handshake(&config, HandshakeFraming::ContentLength, options.timeout);

    if let Some(source) = &config.source
        && !is_git_url(source)
    {
        let smoke = ProcessCommand::new(&config.sifs_path)
            .args([
                "search",
                "sifs_mcp_smoke",
                "--source",
                source,
                "--mode",
                "bm25",
                "--offline",
                "--no-cache",
            ])
            .stdin(Stdio::null())
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

fn bm25_smoke_json(config: &McpConfig) -> Value {
    if let Some(source) = &config.source
        && !is_git_url(source)
    {
        let smoke = ProcessCommand::new(&config.sifs_path)
            .args([
                "search",
                "sifs_mcp_smoke",
                "--source",
                source,
                "--mode",
                "bm25",
                "--offline",
                "--no-cache",
            ])
            .stdin(Stdio::null())
            .output();
        return match smoke {
            Ok(output) if output.status.success() => json!({"status": "passed"}),
            Ok(output) => {
                json!({"status": "failed", "error": String::from_utf8_lossy(&output.stderr).trim()})
            }
            Err(error) => json!({"status": "failed", "error": error.to_string()}),
        };
    }
    json!({"status": "skipped"})
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

fn report_mcp_handshake(config: &McpConfig, framing: HandshakeFraming, timeout: Duration) {
    match mcp_handshake_smoke(config, framing, timeout) {
        Ok(elapsed) => println!(
            "MCP handshake ({}): passed ({} ms)",
            framing.label(),
            elapsed.as_millis()
        ),
        Err(error) => println!("MCP handshake ({}): failed ({error})", framing.label()),
    }
}

fn mcp_handshake_smoke(
    config: &McpConfig,
    framing: HandshakeFraming,
    timeout: Duration,
) -> Result<Duration> {
    let input = mcp_initialize_probe(framing)?;
    let started = Instant::now();
    let output = run_mcp_probe(config, &input, timeout)?;
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

fn install_codex(
    name: &str,
    config: &McpConfig,
    force: bool,
    dry_run: bool,
    json_output: bool,
) -> Result<()> {
    let add_args = codex_add_args(name, config);
    if dry_run && json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "client": "codex",
                "name": name,
                "dry_run": true,
                "would_modify": false,
                "commands": [{
                    "program": "codex",
                    "args": add_args,
                }],
                "fallback_config": {
                    "path_hint": "~/.codex/config.toml",
                    "toml": codex_toml(name, config),
                }
            }))?
        );
        return Ok(());
    }
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
    json_output: bool,
) -> Result<()> {
    let scope = scope.unwrap_or(McpScopeArg::Local);
    let add_args = claude_add_args(name, scope, config)?;
    if dry_run && json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "client": "claude",
                "name": name,
                "scope": scope_arg(scope),
                "dry_run": true,
                "would_modify": false,
                "commands": [{
                    "program": "claude",
                    "args": add_args,
                }],
                "fallback_config": {
                    "path_hint": ".mcp.json",
                    "json": serde_json::from_str::<Value>(&claude_project_json(name, config)?)?,
                }
            }))?
        );
        return Ok(());
    }
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
        args.push("--source".to_owned());
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
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("run {command} {}", args.join(" ")))?;
    Ok(output.status.success())
}

fn run_checked(command: &str, args: &[&str]) -> Result<()> {
    let output = ProcessCommand::new(command)
        .args(args)
        .stdin(Stdio::null())
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
        .stdin(Stdio::null())
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
        CacheCommand::Clean {
            cache_dir,
            dry_run,
            force,
            json,
        } => {
            let root = cache_dir.unwrap_or(platform_cache_root()?);
            let summary = cache_summary(&root);
            if dry_run {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "dry_run": true,
                            "would_change": summary.exists,
                            "root": summary.root,
                            "files": summary.files,
                            "bytes": summary.bytes,
                        }))?
                    );
                } else {
                    println!(
                        "Would remove {} files ({} bytes) from {}.",
                        summary.files,
                        summary.bytes,
                        summary.root.display()
                    );
                }
            } else {
                if !force {
                    bail!("cache clean requires --force; run with --dry-run to preview");
                }
                remove_cache_root(&root)?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "changed": summary.exists,
                            "root": root,
                            "files_removed": summary.files,
                            "bytes_removed": summary.bytes,
                        }))?
                    );
                } else {
                    println!("Removed cache: {}", root.display());
                }
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

fn run_init(force: bool, json_output: bool) -> Result<()> {
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
    let rendered = render_artifact(AgentTarget::ClaudeCode, AgentArtifact::Skill, None, None)?;
    fs::write(&dest, &rendered.content)?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "changed": true,
                "destination": dest,
                "force": force,
                "checksum": rendered.checksum,
                "next_actions": [
                    "sifs agent install --target claude-code --artifact skill --destination .claude/agents/sifs-search.md"
                ],
            }))?
        );
    } else {
        println!("Created {}", dest.display());
    }
    Ok(())
}

fn run_agent(command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Print {
            target,
            artifact,
            destination,
            file,
            source,
            profile,
            json,
        } => run_agent_print(target, artifact, destination, file, source, profile, json),
        AgentCommand::Install {
            target,
            artifact,
            destination,
            file,
            source,
            profile,
            dry_run,
            force,
            json,
        } => run_agent_mutation(
            AgentOperation::Install,
            target,
            artifact,
            destination,
            file,
            source,
            profile,
            dry_run,
            force,
            json,
        ),
        AgentCommand::Doctor {
            target,
            artifact,
            json,
        } => run_agent_doctor(target, artifact, json),
        AgentCommand::Uninstall {
            target,
            artifact,
            destination,
            file,
            dry_run,
            force,
            json,
        } => run_agent_mutation(
            AgentOperation::Uninstall,
            target,
            artifact,
            destination,
            file,
            None,
            None,
            dry_run,
            force,
            json,
        ),
    }
}

fn run_agent_print(
    target: AgentTarget,
    artifact: AgentArtifact,
    destination: Option<PathBuf>,
    file: Option<PathBuf>,
    source: Option<String>,
    profile: Option<String>,
    json_output: bool,
) -> Result<()> {
    let outputs = render_agent_artifacts(
        target,
        artifact,
        destination,
        file,
        source.as_deref(),
        profile.as_deref(),
    )?;
    if json_output {
        let values: Vec<_> = outputs
            .iter()
            .map(|rendered| rendered.print_output())
            .collect();
        if values.len() == 1 {
            println!("{}", serde_json::to_string_pretty(&values[0])?);
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema_version": sifs::agent_artifacts::AGENT_ARTIFACT_SCHEMA_VERSION,
                    "artifacts": values,
                }))?
            );
        }
    } else {
        for (index, rendered) in outputs.iter().enumerate() {
            if outputs.len() > 1 {
                if index > 0 {
                    println!();
                }
                println!(
                    "# target={} artifact={}",
                    rendered.target, rendered.artifact
                );
            }
            print!("{}", rendered.content);
            if !rendered.content.ends_with('\n') {
                println!();
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_agent_mutation(
    operation: AgentOperation,
    target: AgentTarget,
    artifact: AgentArtifact,
    destination: Option<PathBuf>,
    file: Option<PathBuf>,
    source: Option<String>,
    profile: Option<String>,
    dry_run: bool,
    force: bool,
    json_output: bool,
) -> Result<()> {
    let outputs = render_agent_artifacts(
        target,
        artifact,
        destination.clone(),
        file.clone(),
        source.as_deref(),
        profile.as_deref(),
    )?;
    let mut reports = Vec::new();
    for rendered in outputs {
        let options = AgentMutationOptions {
            target: rendered.target,
            artifact: rendered.artifact,
            destination: destination.clone(),
            file: file.clone(),
            dry_run,
            force,
        };
        reports.push(apply_mutation(operation, &rendered, &options)?);
    }
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version": sifs::agent_artifacts::AGENT_ARTIFACT_SCHEMA_VERSION,
                "dry_run": dry_run,
                "force": force,
                "results": reports,
            }))?
        );
    } else {
        for report in reports {
            let destination = report
                .destination
                .as_deref()
                .map(|destination| format!(" at {destination}"))
                .unwrap_or_default();
            println!(
                "{} {}: {}{}",
                report.target, report.artifact, report.status, destination
            );
            for warning in report.warnings {
                println!("warning: {warning}");
            }
        }
    }
    Ok(())
}

fn run_agent_doctor(target: AgentTarget, artifact: AgentArtifact, json_output: bool) -> Result<()> {
    let report = sifs::agent_doctor::doctor(target, artifact);
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("SIFS agent doctor");
        for target in report.targets {
            println!("{}: {}", target.target, target.status);
            for check in target.checks {
                println!("  {}: {} - {}", check.name, check.state, check.evidence);
            }
            for action in target.next_actions {
                println!("  next: {action}");
            }
        }
    }
    Ok(())
}

fn render_agent_artifacts(
    target: AgentTarget,
    artifact: AgentArtifact,
    destination: Option<PathBuf>,
    file: Option<PathBuf>,
    source: Option<&str>,
    profile: Option<&str>,
) -> Result<Vec<sifs::agent_artifacts::RenderedArtifact>> {
    let mut rendered = Vec::new();
    for concrete_target in target.concrete_targets() {
        for concrete_artifact in artifact.concrete_artifacts(concrete_target) {
            let mut artifact =
                render_artifact(concrete_target, concrete_artifact, source, profile)?;
            if concrete_artifact == AgentArtifact::Skill {
                if let Some(destination) = &destination {
                    artifact.destination_hint = Some(destination.clone());
                }
            } else if concrete_artifact == AgentArtifact::Snippet {
                if let Some(file) = &file {
                    artifact.destination_hint = Some(file.clone());
                }
            }
            rendered.push(artifact);
        }
    }
    Ok(rendered)
}

fn print_capabilities(json_output: bool) -> Result<()> {
    let capabilities = [
        "SIFS capabilities:",
        "- Search local directories and Git URLs with hybrid, semantic, or BM25 ranking.",
        "- Print structured CLI output with `--json` and result streams with `--jsonl`.",
        "- Inspect indexes with `sifs list-files`, `sifs status`, `sifs get`, and cache commands.",
        "- Find related code from a known file and one-based line.",
        "- Run `sifs mcp` as an MCP server with search, find_related, index_status, refresh_index, clear_index, list_files, get_chunk, profile, feedback, and init_agent tools.",
        "- Discover the machine-readable agent contract with `sifs agent-context --json`.",
        "- Save reusable source/search defaults with `sifs profile`.",
        "- Record local agent feedback with `sifs feedback`.",
        "- Run BM25 search without loading or downloading an embedding model.",
        "- Manage embedding models with `sifs model pull` and `sifs model status`.",
        "- Generate a Claude agent file with `sifs init`.",
        "- Print, install, inspect, and remove agent skills and instruction snippets with `sifs agent`.",
        "- Use `sifs-benchmark` for quality and latency benchmarks.",
        "- Use `sifs-embed` for embedding diagnostics.",
        "",
        "Discovery:",
        "- `sifs --help` and subcommand help show CLI usage.",
        "- MCP clients can call `tools/list` and read `sifs://server/context`.",
    ];
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "capabilities": capabilities,
                "agent_context": "sifs agent-context --json",
                "profiles": "sifs profile list --json",
                "feedback": "sifs feedback list --json",
            }))?
        );
    } else {
        println!("{}", capabilities.join("\n"));
    }
    Ok(())
}

fn print_agent_context(json_output: bool) -> Result<()> {
    if !json_output {
        bail!("agent-context is machine-readable; rerun with --json");
    }
    let names = profiles::profile_names(&platform_cache_root()?).unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string_pretty(&agent_context::agent_context(names, true))?
    );
    Ok(())
}

fn run_update_command(
    check: bool,
    dry_run: bool,
    json_output: bool,
    timeout: Duration,
    update_timeout: Duration,
) -> Result<()> {
    let mode = if check {
        UpdateMode::Check
    } else if dry_run {
        UpdateMode::DryRun
    } else {
        UpdateMode::Execute
    };
    let report = run_update(&UpdateOptions {
        mode,
        timeout,
        update_timeout,
    })?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return if update_should_exit_nonzero(mode, &report.status) {
            std::process::exit(1);
        } else {
            Ok(())
        };
    }
    print_update_report(&report);
    if update_should_exit_nonzero(mode, &report.status) {
        std::process::exit(1);
    }
    Ok(())
}

fn update_should_exit_nonzero(mode: UpdateMode, status: &str) -> bool {
    status == "failed"
        || (mode == UpdateMode::Execute && matches!(status, "unsupported" | "blocked"))
}

fn print_update_report(report: &sifs::update::UpdateReport) {
    match report.status.as_str() {
        "unchanged" => {
            println!("SIFS is up to date: {}", report.versions.current_version);
        }
        "update_available" => {
            let latest = report
                .versions
                .actionable_latest_version
                .as_deref()
                .unwrap_or("unknown");
            println!(
                "Update available: sifs {} -> {}",
                report.versions.current_version, latest
            );
        }
        "planned" => {
            println!("SIFS update plan:");
            for command in &report.planned_commands {
                println!("  {} {}", command.program, command.args.join(" "));
            }
        }
        "updated" => {
            println!("Updated SIFS.");
        }
        "unsupported" | "blocked" => {
            println!("sifs update cannot safely mutate this install.");
            for condition in &report.ownership.blocking_conditions {
                println!("- {condition}");
            }
        }
        "unknown" => {
            println!("Could not determine whether a SIFS update is available.");
        }
        "failed" => {
            eprintln!("sifs update failed.");
            if let Some(runner) = &report.runner {
                if !runner.stderr.trim().is_empty() {
                    eprintln!("{}", runner.stderr.trim());
                }
            }
        }
        other => println!("sifs update status: {other}"),
    }
    for warning in &report.warnings {
        eprintln!("Warning: {warning}");
    }
    if !report.next_actions.is_empty() {
        println!("Next actions:");
        for action in &report.next_actions {
            println!("- {action}");
        }
    }
}

fn run_profile(command: ProfileCommand) -> Result<()> {
    let root = platform_cache_root()?;
    match command {
        ProfileCommand::Save {
            name,
            source,
            ref_name,
            mode,
            limit,
            encoder,
            model,
            offline,
            no_download,
            cache_dir,
            no_cache,
            project_cache,
            json,
        } => {
            if let Some(limit) = limit
                && limit == 0
            {
                bail!("--limit must be at least 1");
            }
            let profile = profiles::Profile {
                name: name.clone(),
                source,
                ref_name,
                mode: mode.map(SearchMode::from),
                limit,
                encoder: encoder.map(|encoder| match encoder {
                    EncoderArg::Model2Vec => "model2vec".to_owned(),
                    EncoderArg::Hashing => "hashing".to_owned(),
                }),
                model,
                offline: offline.then_some(true),
                no_download: no_download.then_some(true),
                cache_dir,
                no_cache: no_cache.then_some(true),
                project_cache: project_cache.then_some(true),
            };
            profiles::save_profile(&root, profile.clone())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "changed": true,
                        "profile": profile,
                        "path": profiles::profile_store_path(&root),
                    }))?
                );
            } else {
                println!("Saved profile {name:?}.");
            }
        }
        ProfileCommand::List { json } => {
            let list = profiles::load_profiles(&root)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "profiles": list,
                        "total": list.len(),
                        "path": profiles::profile_store_path(&root),
                    }))?
                );
            } else {
                for profile in list {
                    println!("{}", profile.name);
                }
            }
        }
        ProfileCommand::Show { name, json } => {
            let profile = profiles::get_profile(&root, &name)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "profile": profile,
                        "path": profiles::profile_store_path(&root),
                    }))?
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&profile)?);
            }
        }
        ProfileCommand::Delete { name, force, json } => {
            if !force {
                bail!("profile delete requires --force");
            }
            let removed = profiles::delete_profile(&root, &name)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "changed": removed,
                        "profile": name,
                        "path": profiles::profile_store_path(&root),
                    }))?
                );
            } else if removed {
                println!("Deleted profile {name:?}.");
            } else {
                println!("No profile named {name:?}.");
            }
        }
    }
    Ok(())
}

fn run_feedback(command: FeedbackCommand) -> Result<()> {
    let root = platform_cache_root()?;
    match command {
        FeedbackCommand::Create {
            message,
            command_context,
            json,
        } => {
            let entry = feedback::create_feedback(&root, &message, command_context)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "changed": true,
                        "feedback": entry,
                        "path": feedback::feedback_log_path(&root),
                    }))?
                );
            } else {
                println!("Feedback recorded locally: {}", entry.id);
            }
        }
        FeedbackCommand::List { limit, json } => {
            if limit == 0 {
                bail!("--limit must be at least 1");
            }
            let (entries, total) = feedback::list_feedback(&root, limit)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "feedback": entries,
                        "total": total,
                        "limit": limit,
                        "truncated": total > limit,
                        "hint": if total > limit { Some("Increase --limit to inspect more local feedback entries.") } else { None },
                        "path": feedback::feedback_log_path(&root),
                    }))?
                );
            } else {
                for entry in entries {
                    println!("{}\t{}", entry.id, entry.message);
                }
            }
        }
    }
    Ok(())
}
