use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sifs::SifsIndex;
use sifs::types::SearchMode;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Parser)]
#[command(about = "Run SIFS quality and speed benchmarks over annotated repositories.")]
struct Args {
    #[arg(long)]
    benchmarks_dir: PathBuf,
    #[arg(long)]
    bench_root: PathBuf,
    #[arg(long)]
    repo: Vec<String>,
    #[arg(long)]
    language: Vec<String>,
    #[arg(long)]
    sync: bool,
    #[arg(long, default_value_t = 10)]
    top_k: usize,
    #[arg(long, default_value_t = 5)]
    latency_runs: usize,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long)]
    include_tasks: bool,
    #[arg(long)]
    alpha: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct RepoSpec {
    name: String,
    language: String,
    url: String,
    revision: String,
    benchmark_root: Option<String>,
}

impl RepoSpec {
    fn checkout_dir(&self, bench_root: &Path) -> PathBuf {
        bench_root.join(&self.name)
    }

    fn benchmark_dir(&self, bench_root: &Path) -> PathBuf {
        match &self.benchmark_root {
            Some(root) => self.checkout_dir(bench_root).join(root),
            None => self.checkout_dir(bench_root),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawTask {
    query: String,
    relevant: Option<Vec<RawTarget>>,
    secondary: Option<Vec<RawTarget>>,
    category: Option<String>,
    repo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTarget {
    Path(String),
    Span {
        path: String,
        start_line: Option<serde_json::Value>,
        end_line: Option<serde_json::Value>,
    },
}

#[derive(Clone, Debug)]
struct Target {
    path: String,
    start_line: Option<usize>,
    end_line: Option<usize>,
}

#[derive(Debug)]
struct Task {
    repo: String,
    query: String,
    relevant: Vec<Target>,
    category: String,
}

#[derive(Debug, Serialize)]
struct RepoResult {
    repo: String,
    language: String,
    chunks: usize,
    files: usize,
    tasks: usize,
    ndcg5: f64,
    ndcg10: f64,
    p50_ms: f64,
    p90_ms: f64,
    index_ms: f64,
    peak_rss_mb: f64,
    by_category: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    task_results: Vec<TaskResult>,
}

#[derive(Debug, Serialize)]
struct TaskResult {
    query: String,
    category: String,
    ranks: Vec<usize>,
    ndcg5: f64,
    ndcg10: f64,
    top_results: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Payload {
    method: String,
    results: Vec<RepoResult>,
    summary: Summary,
}

#[derive(Debug, Serialize)]
struct Summary {
    repos: usize,
    tasks: usize,
    avg_ndcg10: f64,
    avg_p50_ms: f64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let specs = load_specs(&args)?;
    if args.sync {
        sync_repos(&specs, &args.bench_root)?;
    }
    let tasks = load_tasks(&args, &specs)?;
    if tasks.is_empty() {
        bail!("No benchmark tasks matched the requested filters");
    }

    let mut by_repo: BTreeMap<String, Vec<Task>> = BTreeMap::new();
    for task in tasks {
        by_repo.entry(task.repo.clone()).or_default().push(task);
    }

    let mut results = Vec::new();
    for (repo_name, tasks) in by_repo {
        let spec = specs.get(&repo_name).context("missing repo spec")?;
        let root = spec.benchmark_dir(&args.bench_root);
        let start = Instant::now();
        let index = SifsIndex::from_path(&root)?;
        let index_ms = elapsed_ms(start);

        let mut latencies = Vec::new();
        let mut ndcg5_sum = 0.0;
        let mut ndcg10_sum = 0.0;
        let mut by_category_scores: HashMap<String, Vec<f64>> = HashMap::new();
        let mut task_results = Vec::new();

        for task in &tasks {
            let mut last_results = Vec::new();
            std::hint::black_box(index.search(
                &task.query,
                args.top_k,
                SearchMode::Hybrid,
                args.alpha,
                None,
                None,
            ));
            for _ in 0..args.latency_runs.max(1) {
                let start = Instant::now();
                last_results = index.search(
                    &task.query,
                    args.top_k,
                    SearchMode::Hybrid,
                    args.alpha,
                    None,
                    None,
                );
                latencies.push(elapsed_ms(start));
            }
            let ranks: Vec<usize> = task
                .relevant
                .iter()
                .filter_map(|target| target_rank(&last_results, target))
                .collect();
            let ndcg5 = ndcg_at_k(&ranks, task.relevant.len(), 5);
            let ndcg10 = ndcg_at_k(&ranks, task.relevant.len(), 10);
            ndcg5_sum += ndcg5;
            ndcg10_sum += ndcg10;
            by_category_scores
                .entry(task.category.clone())
                .or_default()
                .push(ndcg10);
            if args.include_tasks {
                task_results.push(TaskResult {
                    query: task.query.clone(),
                    category: task.category.clone(),
                    ranks,
                    ndcg5,
                    ndcg10,
                    top_results: last_results
                        .iter()
                        .take(args.top_k.min(10))
                        .map(|result| result.chunk.location())
                        .collect(),
                });
            }
        }

        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let stats = index.stats();
        let mut by_category = BTreeMap::new();
        for (category, scores) in by_category_scores {
            by_category.insert(category, mean(&scores));
        }
        results.push(RepoResult {
            repo: repo_name,
            language: spec.language.clone(),
            chunks: index.chunks.len(),
            files: stats.indexed_files,
            tasks: tasks.len(),
            ndcg5: ndcg5_sum / tasks.len() as f64,
            ndcg10: ndcg10_sum / tasks.len() as f64,
            p50_ms: percentile(&latencies, 0.5),
            p90_ms: percentile(&latencies, 0.9),
            index_ms,
            peak_rss_mb: peak_rss_mb(),
            by_category,
            task_results,
        });
    }

    let total_tasks: usize = results.iter().map(|r| r.tasks).sum();
    let summary = Summary {
        repos: results.len(),
        tasks: total_tasks,
        avg_ndcg10: weighted_mean(&results, |r| r.ndcg10),
        avg_p50_ms: weighted_mean(&results, |r| r.p50_ms),
    };
    let payload = Payload {
        method: "sifs-hybrid".to_owned(),
        results,
        summary,
    };
    let json = serde_json::to_string_pretty(&payload)? + "\n";
    if let Some(output) = args.output {
        fs::write(output, json)?;
    } else {
        print!("{json}");
    }
    Ok(())
}

fn load_specs(args: &Args) -> Result<BTreeMap<String, RepoSpec>> {
    let raw = fs::read_to_string(args.benchmarks_dir.join("repos.json"))?;
    let specs: Vec<RepoSpec> = serde_json::from_str(&raw)?;
    Ok(specs
        .into_iter()
        .filter(|spec| args.repo.is_empty() || args.repo.contains(&spec.name))
        .filter(|spec| args.language.is_empty() || args.language.contains(&spec.language))
        .map(|spec| (spec.name.clone(), spec))
        .collect())
}

fn load_tasks(args: &Args, specs: &BTreeMap<String, RepoSpec>) -> Result<Vec<Task>> {
    let annotations = args.benchmarks_dir.join("annotations");
    let mut tasks = Vec::new();
    for entry in fs::read_dir(annotations)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let default_repo = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let raw_tasks: Vec<RawTask> = serde_json::from_str(&fs::read_to_string(&path)?)?;
        for raw in raw_tasks {
            let repo = raw.repo.unwrap_or_else(|| default_repo.to_owned());
            if !specs.contains_key(&repo) {
                continue;
            }
            let category = raw.category.unwrap_or_else(|| infer_category(&raw.query));
            let relevant = raw
                .relevant
                .unwrap_or_default()
                .into_iter()
                .chain(raw.secondary.unwrap_or_default())
                .map(parse_target)
                .collect();
            tasks.push(Task {
                repo,
                query: raw.query,
                relevant,
                category,
            });
        }
    }
    Ok(tasks)
}

fn parse_target(raw: RawTarget) -> Target {
    match raw {
        RawTarget::Path(path) => Target {
            path,
            start_line: None,
            end_line: None,
        },
        RawTarget::Span {
            path,
            start_line,
            end_line,
        } => Target {
            path,
            start_line: start_line.and_then(value_to_usize),
            end_line: end_line.and_then(value_to_usize),
        },
    }
}

fn value_to_usize(value: serde_json::Value) -> Option<usize> {
    value
        .as_u64()
        .map(|v| v as usize)
        .or_else(|| value.as_str()?.parse().ok())
}

fn sync_repos(specs: &BTreeMap<String, RepoSpec>, bench_root: &Path) -> Result<()> {
    fs::create_dir_all(bench_root)?;
    for spec in specs.values() {
        let checkout = spec.checkout_dir(bench_root);
        if !checkout.exists() {
            run(Command::new("git")
                .arg("clone")
                .arg(&spec.url)
                .arg(&checkout))?;
        }
        run(Command::new("git")
            .arg("fetch")
            .arg("--all")
            .arg("--tags")
            .current_dir(&checkout))?;
        run(Command::new("git")
            .arg("checkout")
            .arg(&spec.revision)
            .current_dir(&checkout))?;
    }
    Ok(())
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.stdin(Stdio::null()).status()?;
    if !status.success() {
        bail!("command failed with status {status}: {cmd:?}");
    }
    Ok(())
}

fn target_rank(results: &[sifs::SearchResult], target: &Target) -> Option<usize> {
    results.iter().enumerate().find_map(|(idx, result)| {
        let chunk = &result.chunk;
        target_matches(&chunk.file_path, chunk.start_line, chunk.end_line, target)
            .then_some(idx + 1)
    })
}

fn target_matches(file_path: &str, start_line: usize, end_line: usize, target: &Target) -> bool {
    if !path_matches(file_path, &target.path) {
        return false;
    }
    match (target.start_line, target.end_line) {
        (Some(start), Some(end)) => !(end_line < start || start_line > end),
        _ => true,
    }
}

fn path_matches(file_path: &str, target_path: &str) -> bool {
    let file = file_path.replace('\\', "/");
    let target = target_path.replace('\\', "/");
    file == target || file.ends_with(&format!("/{target}")) || target.ends_with(&format!("/{file}"))
}

fn ndcg_at_k(ranks: &[usize], n_relevant: usize, k: usize) -> f64 {
    if n_relevant == 0 {
        return 0.0;
    }
    let mut relevances = vec![0.0; k];
    for &rank in ranks {
        if (1..=k).contains(&rank) {
            relevances[rank - 1] = 1.0;
        }
    }
    let ideal = dcg(&vec![1.0; n_relevant.min(k)]);
    if ideal > 0.0 {
        dcg(&relevances) / ideal
    } else {
        0.0
    }
}

fn dcg(relevances: &[f64]) -> f64 {
    relevances
        .iter()
        .enumerate()
        .map(|(i, rel)| rel / ((i + 2) as f64).log2())
        .sum()
}

fn infer_category(query: &str) -> String {
    let trimmed = query.trim();
    if !trimmed.contains(' ') {
        return "symbol".to_owned();
    }
    let lowered = trimmed.to_lowercase();
    if lowered.starts_with("how ")
        || lowered.starts_with("how does")
        || lowered.starts_with("how are")
    {
        "architecture".to_owned()
    } else {
        "semantic".to_owned()
    }
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[cfg(target_os = "macos")]
fn peak_rss_mb() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc == 0 {
        let usage = unsafe { usage.assume_init() };
        usage.ru_maxrss as f64 / (1024.0 * 1024.0)
    } else {
        0.0
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn peak_rss_mb() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc == 0 {
        let usage = unsafe { usage.assume_init() };
        usage.ru_maxrss as f64 / 1024.0
    } else {
        0.0
    }
}

#[cfg(not(unix))]
fn peak_rss_mb() -> f64 {
    0.0
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values[((values.len() as f64 * p).floor() as usize).min(values.len() - 1)]
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn weighted_mean(results: &[RepoResult], f: impl Fn(&RepoResult) -> f64) -> f64 {
    let total: usize = results.iter().map(|r| r.tasks).sum();
    if total == 0 {
        return 0.0;
    }
    results.iter().map(|r| f(r) * r.tasks as f64).sum::<f64>() / total as f64
}
