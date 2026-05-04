use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use sifs::SifsIndex;
use sifs::types::SearchMode;
use sifs::utils::{format_results, is_git_url, resolve_chunk};
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
    Search {
        query: String,
        #[arg(default_value = ".")]
        path: String,
        #[arg(short = 'k', long = "top-k", default_value_t = 5)]
        top_k: usize,
        #[arg(short = 'm', long = "mode", value_enum, default_value_t = ModeArg::Hybrid)]
        mode: ModeArg,
    },
    FindRelated {
        file_path: String,
        line: usize,
        #[arg(default_value = ".")]
        path: String,
        #[arg(short = 'k', long = "top-k", default_value_t = 5)]
        top_k: usize,
    },
    Init {
        #[arg(long)]
        force: bool,
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
        }) => {
            let index = build_index(&path)?;
            let mode = SearchMode::from(mode);
            let results = index.search(&query, top_k, mode, None, None, None);
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
