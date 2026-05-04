use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use sifs::{SearchMode, SearchOptions, SifsIndex, format_results, is_git_url, resolve_chunk};
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
    },
    #[command(about = "Create the Claude agent file at .claude/agents/sifs-search.md.")]
    Init {
        #[arg(long, help = "Overwrite an existing generated agent file.")]
        force: bool,
    },
    #[command(about = "Print SIFS agent, CLI, and MCP capabilities.")]
    Capabilities,
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
        }) => {
            let index = build_index(&path)?;
            let mode = SearchMode::from(mode);
            let results = index.search_with(&query, &SearchOptions::new(top_k).with_mode(mode));
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
        }) => {
            let index = build_index(&path)?;
            let Some(chunk) = resolve_chunk(&index.chunks, &file_path, line) else {
                eprintln!("No chunk found at {file_path}:{line}.");
                std::process::exit(1);
            };
            let results = index.find_related(&chunk, top_k);
            if results.is_empty() {
                println!("No related chunks found for {file_path}:{line}.");
            } else {
                println!(
                    "{}",
                    format_results(&format!("Chunks related to {file_path}:{line}"), &results)
                );
            }
        }
        Some(Command::Init { force }) => run_init(force)?,
        Some(Command::Capabilities) => print_capabilities(),
        None => {
            sifs::mcp::serve(cli.path, cli.ref_name)?;
        }
    }
    Ok(())
}

fn build_index(path: &str) -> Result<SifsIndex> {
    if is_git_url(path) {
        SifsIndex::from_git(path, None)
    } else {
        SifsIndex::from_path(path)
    }
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
