use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sifs::search::{hybrid_timing, reset_hybrid_timing};
use sifs::{
    CacheConfig, IndexOptions, ModelLoadPolicy, ModelOptions, SearchMode, SearchOptions, SifsIndex,
    encoder_fingerprint, metrics::peak_rss_mb,
};
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
    #[arg(long, help = "Include per-task candidate-generation diagnostics.")]
    candidate_diagnostics: bool,
    #[arg(long, default_value_t = 200)]
    candidate_diagnostics_depth: usize,
    #[arg(long)]
    alpha: Option<f32>,
    #[arg(long, default_value_t = SearchMode::Hybrid)]
    mode: SearchMode,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    offline: bool,
    #[arg(long = "no-download")]
    no_download: bool,
    #[arg(long)]
    hybrid_timing: bool,
    /// Disable persistent index caches for cold-index timing.
    #[arg(long)]
    no_cache: bool,
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
    cold_index_ms: f64,
    cold_semantic_build_or_load_ms: Option<f64>,
    cold_first_search_ms: f64,
    warm_uncached_query_ms: f64,
    warm_uncached_query_p90_ms: f64,
    warm_cached_repeat_query_ms: f64,
    warm_cached_repeat_query_p90_ms: f64,
    peak_rss_mb: f64,
    reproducibility: BenchmarkMetadata,
    by_category: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_diagnostic_summary: Option<CandidateDiagnosticSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    task_results: Vec<TaskResult>,
}

#[derive(Debug, Serialize)]
struct BenchmarkMetadata {
    cache_mode: String,
    sifs_version: String,
    rustc_version: Option<String>,
    os: String,
    cpu: Option<String>,
    repo_revision: Option<String>,
    model_fingerprint: Option<String>,
    indexed_files: usize,
    indexed_chunks: usize,
}

#[derive(Debug, Serialize)]
struct TaskResult {
    query: String,
    category: String,
    relevant_count: usize,
    ranks: Vec<usize>,
    ndcg5: f64,
    ndcg10: f64,
    top_results: Vec<TaskHit>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidate_diagnostics: Vec<TargetDiagnostic>,
}

#[derive(Debug, Serialize)]
struct TaskHit {
    location: String,
    path: String,
    start_line: usize,
    end_line: usize,
    tokens: usize,
    relevant: bool,
}

#[derive(Debug, Serialize)]
struct TargetDiagnostic {
    target: String,
    target_final_rank: Option<usize>,
    target_bm25_rank: Option<usize>,
    target_semantic_rank: Option<usize>,
    target_in_candidate_union: bool,
    failure_stage: String,
}

#[derive(Debug, Default, Serialize)]
struct CandidateDiagnosticSummary {
    targets: usize,
    top10: usize,
    reranking: usize,
    reranking_or_depth: usize,
    candidate_generation: usize,
}

impl CandidateDiagnosticSummary {
    fn record(&mut self, diagnostic: &TargetDiagnostic) {
        self.targets += 1;
        match diagnostic.failure_stage.as_str() {
            "top10" => self.top10 += 1,
            "reranking" => self.reranking += 1,
            "reranking_or_depth" => self.reranking_or_depth += 1,
            "candidate_generation" => self.candidate_generation += 1,
            _ => {}
        }
    }

    fn is_empty(&self) -> bool {
        self.targets == 0
    }
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
    avg_cold_index_ms: f64,
    avg_cold_semantic_build_or_load_ms: f64,
    avg_cold_first_search_ms: f64,
    avg_warm_uncached_query_ms: f64,
    avg_warm_cached_repeat_query_ms: f64,
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
    if args.hybrid_timing {
        reset_hybrid_timing();
    }

    let mut by_repo: BTreeMap<String, Vec<Task>> = BTreeMap::new();
    for task in tasks {
        by_repo.entry(task.repo.clone()).or_default().push(task);
    }

    let mut results = Vec::new();
    for (repo_name, tasks) in by_repo {
        let spec = specs.get(&repo_name).context("missing repo spec")?;
        let root = spec.benchmark_dir(&args.bench_root);
        eprintln!(
            "Indexing benchmark repo {repo_name} at {} ({} tasks)",
            root.display(),
            tasks.len()
        );
        let model_options = ModelOptions::new(
            args.model.as_deref(),
            model_policy(args.offline, args.no_download),
        );
        let index_options = IndexOptions::new(model_options.clone()).with_cache(if args.no_cache {
            CacheConfig::Disabled
        } else {
            CacheConfig::Platform
        });
        let start = Instant::now();
        let index = SifsIndex::from_path_with_index_options(&root, index_options)?;
        let cold_index_ms = elapsed_ms(start);
        let semantic_start = Instant::now();
        let cold_semantic_build_or_load_ms =
            index.warm_semantic()?.then(|| elapsed_ms(semantic_start));

        let mut uncached_latencies = Vec::new();
        let mut cached_latencies = Vec::new();
        let mut ndcg5_sum = 0.0;
        let mut ndcg10_sum = 0.0;
        let mut by_category_scores: HashMap<String, Vec<f64>> = HashMap::new();
        let mut task_results = Vec::new();
        let mut candidate_diagnostic_summary = CandidateDiagnosticSummary::default();

        let mut first_options = SearchOptions::new(args.top_k)
            .with_mode(args.mode)
            .with_cache(false);
        first_options.alpha = args.alpha;
        let first_task = tasks
            .first()
            .context("repo benchmark task group was unexpectedly empty")?;
        let first_start = Instant::now();
        std::hint::black_box(index.search_with(&first_task.query, &first_options)?);
        let cold_first_search_ms =
            cold_semantic_build_or_load_ms.unwrap_or(0.0) + elapsed_ms(first_start);

        for (task_idx, task) in tasks.iter().enumerate() {
            eprintln!(
                "Running benchmark task {}/{} for {repo_name}: {}",
                task_idx + 1,
                tasks.len(),
                task.query
            );
            let mut last_results = Vec::new();
            let mut uncached_options = SearchOptions::new(args.top_k)
                .with_mode(args.mode)
                .with_cache(false);
            uncached_options.alpha = args.alpha;
            let mut cached_options = SearchOptions::new(args.top_k)
                .with_mode(args.mode)
                .with_cache(true);
            cached_options.alpha = args.alpha;

            std::hint::black_box(index.search_with(&task.query, &uncached_options)?);
            for _ in 0..args.latency_runs.max(1) {
                let start = Instant::now();
                last_results = index.search_with(&task.query, &uncached_options)?;
                uncached_latencies.push(elapsed_ms(start));
            }

            std::hint::black_box(index.search_with(&task.query, &cached_options)?);
            for _ in 0..args.latency_runs.max(1) {
                let start = Instant::now();
                std::hint::black_box(index.search_with(&task.query, &cached_options)?);
                cached_latencies.push(elapsed_ms(start));
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
                let candidate_diagnostics = if args.candidate_diagnostics {
                    let diagnostic_depth = args.candidate_diagnostics_depth.max(args.top_k);
                    let mut diagnostic_options = SearchOptions::new(diagnostic_depth)
                        .with_mode(args.mode)
                        .with_cache(false)
                        .with_explain(true);
                    diagnostic_options.alpha = args.alpha;
                    let diagnostic_results = index.search_with(&task.query, &diagnostic_options)?;
                    let bm25_diagnostic_results = index.search_with(
                        &task.query,
                        &SearchOptions::new(diagnostic_depth)
                            .with_mode(SearchMode::Bm25)
                            .with_cache(false),
                    )?;
                    let semantic_diagnostic_results = index.search_with(
                        &task.query,
                        &SearchOptions::new(diagnostic_depth)
                            .with_mode(SearchMode::Semantic)
                            .with_cache(false),
                    )?;
                    task_candidate_diagnostics(
                        &diagnostic_results,
                        &bm25_diagnostic_results,
                        &semantic_diagnostic_results,
                        task,
                    )
                } else {
                    Vec::new()
                };
                for diagnostic in &candidate_diagnostics {
                    candidate_diagnostic_summary.record(diagnostic);
                }
                task_results.push(TaskResult {
                    query: task.query.clone(),
                    category: task.category.clone(),
                    relevant_count: task.relevant.len(),
                    ranks,
                    ndcg5,
                    ndcg10,
                    top_results: last_results
                        .iter()
                        .map(|result| task_hit(result, task))
                        .collect(),
                    candidate_diagnostics,
                });
            }
        }

        uncached_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        cached_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
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
            cold_index_ms,
            cold_semantic_build_or_load_ms,
            cold_first_search_ms,
            warm_uncached_query_ms: percentile(&uncached_latencies, 0.5),
            warm_uncached_query_p90_ms: percentile(&uncached_latencies, 0.9),
            warm_cached_repeat_query_ms: percentile(&cached_latencies, 0.5),
            warm_cached_repeat_query_p90_ms: percentile(&cached_latencies, 0.9),
            peak_rss_mb: peak_rss_mb(),
            reproducibility: BenchmarkMetadata {
                cache_mode: if args.no_cache {
                    "disabled".to_owned()
                } else {
                    "platform".to_owned()
                },
                sifs_version: env!("CARGO_PKG_VERSION").to_owned(),
                rustc_version: command_stdout("rustc", &["--version"]),
                os: std::env::consts::OS.to_owned(),
                cpu: cpu_name(),
                repo_revision: git_revision(&root),
                model_fingerprint: encoder_fingerprint(&sifs::EncoderSpec::Model2Vec(
                    model_options,
                ))
                .ok(),
                indexed_files: stats.indexed_files,
                indexed_chunks: index.chunks.len(),
            },
            by_category,
            candidate_diagnostic_summary: (!candidate_diagnostic_summary.is_empty())
                .then_some(candidate_diagnostic_summary),
            task_results,
        });
    }

    let total_tasks: usize = results.iter().map(|r| r.tasks).sum();
    let summary = Summary {
        repos: results.len(),
        tasks: total_tasks,
        avg_ndcg10: weighted_mean(&results, |r| r.ndcg10),
        avg_cold_index_ms: weighted_mean(&results, |r| r.cold_index_ms),
        avg_cold_semantic_build_or_load_ms: weighted_mean(&results, |r| {
            r.cold_semantic_build_or_load_ms.unwrap_or(0.0)
        }),
        avg_cold_first_search_ms: weighted_mean(&results, |r| r.cold_first_search_ms),
        avg_warm_uncached_query_ms: weighted_mean(&results, |r| r.warm_uncached_query_ms),
        avg_warm_cached_repeat_query_ms: weighted_mean(&results, |r| r.warm_cached_repeat_query_ms),
    };
    let payload = Payload {
        method: format!("sifs-{}", args.mode),
        results,
        summary,
    };
    let json = serde_json::to_string_pretty(&payload)? + "\n";
    if let Some(output) = args.output {
        fs::write(&output, json)?;
        eprintln!(
            "Wrote benchmark results to {} ({} repos, {} tasks, avg_ndcg10={:.3}, avg_warm_uncached_query_ms={:.1}, avg_warm_cached_repeat_query_ms={:.4}).",
            output.display(),
            payload.summary.repos,
            payload.summary.tasks,
            payload.summary.avg_ndcg10,
            payload.summary.avg_warm_uncached_query_ms,
            payload.summary.avg_warm_cached_repeat_query_ms
        );
    } else {
        print!("{json}");
    }
    if args.hybrid_timing {
        let timing = hybrid_timing();
        let divisor = timing.queries.max(1) as f64;
        eprintln!(
            "Hybrid timing per query: queries={} encode_ms={:.4} dense_ms={:.4} bm25_ms={:.4} fuse_ms={:.4} file_boost_ms={:.4} query_boost_ms={:.4} rerank_ms={:.4} collect_ms={:.4}",
            timing.queries,
            timing.encode.as_secs_f64() * 1000.0 / divisor,
            timing.dense.as_secs_f64() * 1000.0 / divisor,
            timing.bm25.as_secs_f64() * 1000.0 / divisor,
            timing.fuse.as_secs_f64() * 1000.0 / divisor,
            timing.file_boost.as_secs_f64() * 1000.0 / divisor,
            timing.query_boost.as_secs_f64() * 1000.0 / divisor,
            timing.rerank.as_secs_f64() * 1000.0 / divisor,
            timing.collect.as_secs_f64() * 1000.0 / divisor,
        );
    }
    Ok(())
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

fn git_revision(root: &Path) -> Option<String> {
    command_stdout_in(root, "git", &["rev-parse", "HEAD"])
}

fn cpu_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        command_stdout("sysctl", &["-n", "machdep.cpu.brand_string"])
    }
    #[cfg(not(target_os = "macos"))]
    {
        command_stdout("uname", &["-m"])
    }
}

fn command_stdout(command: &str, args: &[&str]) -> Option<String> {
    command_stdout_in(Path::new("."), command, args)
}

fn command_stdout_in(cwd: &Path, command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
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
            eprintln!("Cloning {} into {}", spec.url, checkout.display());
            run(Command::new("git")
                .arg("clone")
                .arg(&spec.url)
                .arg(&checkout))?;
        }
        eprintln!("Fetching {} in {}", spec.name, checkout.display());
        run(Command::new("git")
            .arg("fetch")
            .arg("--all")
            .arg("--tags")
            .current_dir(&checkout))?;
        eprintln!("Checking out {} at {}", spec.name, spec.revision);
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

fn task_candidate_diagnostics(
    results: &[sifs::SearchResult],
    bm25_results: &[sifs::SearchResult],
    semantic_results: &[sifs::SearchResult],
    task: &Task,
) -> Vec<TargetDiagnostic> {
    task.relevant
        .iter()
        .map(|target| {
            let target_final_rank = target_rank(results, target);
            let target_bm25_rank = target_rank(bm25_results, target);
            let target_semantic_rank = target_rank(semantic_results, target);
            let target_in_candidate_union =
                target_bm25_rank.is_some() || target_semantic_rank.is_some();
            let failure_stage = match (target_final_rank, target_in_candidate_union) {
                (Some(rank), _) if rank <= 10 => "top10",
                (Some(_), _) => "reranking",
                (None, true) => "reranking_or_depth",
                (None, false) => "candidate_generation",
            }
            .to_owned();
            TargetDiagnostic {
                target: target_label(target),
                target_final_rank,
                target_bm25_rank,
                target_semantic_rank,
                target_in_candidate_union,
                failure_stage,
            }
        })
        .collect()
}

fn task_hit(result: &sifs::SearchResult, task: &Task) -> TaskHit {
    let chunk = &result.chunk;
    TaskHit {
        location: chunk.location(),
        path: chunk.file_path.clone(),
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        tokens: estimate_tokens(&chunk.content),
        relevant: task.relevant.iter().any(|target| {
            target_matches(&chunk.file_path, chunk.start_line, chunk.end_line, target)
        }),
    }
}

fn target_label(target: &Target) -> String {
    match (target.start_line, target.end_line) {
        (Some(start), Some(end)) => format!("{}:{start}-{end}", target.path),
        (Some(start), None) => format!("{}:{start}", target.path),
        _ => target.path.clone(),
    }
}

fn estimate_tokens(content: &str) -> usize {
    content.split_whitespace().count().max(1)
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
