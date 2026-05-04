#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import subprocess
from collections import Counter, defaultdict
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SEMBLE_BENCHMARKS = REPO_ROOT.parent / "semble" / "benchmarks"
DEFAULT_BENCH_ROOT = Path.home() / ".cache" / "semble-bench"
TOKENIZER_NAME = "cl100k_base"
KEYWORD_MIN_LEN = 3
MAX_MATCHES_PER_KEYWORD = 500

STOPWORDS = frozenset(
    """
    a an and are as at be by for from has have how in into is it its of on or
    that the their then there these this to with without
    but can not no nor so yet both either neither than will would could should
    may might been being had did they all any each few more most other some such
    only own same too very just about after also before between during through
    under up down over
    """.split()
)


def load_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def path_matches(file_path: str, target_path: str) -> bool:
    norm_file = file_path.replace("\\", "/")
    norm_target = target_path.replace("\\", "/")
    return (
        norm_file == norm_target
        or norm_file.endswith(f"/{norm_target}")
        or norm_target.endswith(f"/{norm_file}")
    )


def benchmark_dir(spec: dict, bench_root: Path) -> Path:
    checkout = bench_root / spec["name"]
    root = spec.get("benchmark_root")
    return checkout if root is None else checkout / root


def normalize_target(raw: object) -> dict:
    if isinstance(raw, str):
        return {"path": raw}
    if isinstance(raw, dict):
        return {"path": str(raw["path"])}
    raise TypeError(f"unexpected target shape: {raw!r}")


def load_specs(semble_benchmarks: Path, bench_root: Path) -> dict[str, dict]:
    specs = {}
    for item in load_json(semble_benchmarks / "repos.json"):
        spec = dict(item)
        if benchmark_dir(spec, bench_root).exists():
            specs[spec["name"]] = spec
    return specs


def load_tasks(semble_benchmarks: Path, specs: dict[str, dict]) -> list[dict]:
    tasks = []
    annotations = semble_benchmarks / "annotations"
    for annotation_file in sorted(annotations.glob("*.json")):
        default_repo = annotation_file.stem
        if default_repo not in specs:
            continue
        for item in load_json(annotation_file):
            repo = item.get("repo", default_repo)
            if repo not in specs:
                continue
            tasks.append(
                {
                    "repo": repo,
                    "language": specs[repo]["language"],
                    "query": item["query"],
                    "category": item.get("category") or infer_category(item["query"]),
                    "relevant": [
                        normalize_target(target)
                        for target in [*item.get("relevant", []), *item.get("secondary", [])]
                    ],
                }
            )
    return tasks


def infer_category(query: str) -> str:
    if " " not in query.strip():
        return "symbol"
    lowered = query.lower()
    if lowered.startswith("how ") or lowered.startswith("how does") or lowered.startswith("how are"):
        return "architecture"
    return "semantic"


def ripgrep_json_matches(pattern: str, root: Path, *, timeout: int) -> list[tuple[str, int]]:
    cmd = [
        "rg",
        "--json",
        "--ignore-case",
        "--hidden",
        "--glob",
        "!.git",
        "--fixed-strings",
        pattern,
        str(root),
    ]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return []
    if proc.returncode not in (0, 1):
        return []

    matches = []
    for line in proc.stdout.splitlines():
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") != "match":
            continue
        data = event.get("data", {})
        path = data.get("path", {}).get("text")
        line_number = data.get("line_number")
        if path and isinstance(line_number, int):
            matches.append((path, line_number))
    return matches


def keywords(query: str) -> list[str]:
    words = re.findall(r"[a-zA-Z][a-zA-Z0-9]*", query)
    seen = set()
    result = []
    for word in words:
        lowered = word.lower()
        if len(lowered) < KEYWORD_MIN_LEN or lowered in STOPWORDS or lowered in seen:
            continue
        seen.add(lowered)
        result.append(word)
    return result


def run_ripgrep_keyword_read(query: str, root: Path, *, top_k: int, timeout: int) -> list[str]:
    query_keywords = keywords(query)
    if not query_keywords:
        query_keywords = [query]
    keyword_hits: dict[str, set[str]] = defaultdict(set)
    match_counts: Counter[str] = Counter()
    for keyword in query_keywords:
        for path, _line in ripgrep_json_matches(keyword, root, timeout=timeout)[:MAX_MATCHES_PER_KEYWORD]:
            keyword_hits[path].add(keyword.lower())
            match_counts[path] += 1
    ranked = sorted(
        keyword_hits,
        key=lambda path: (-len(keyword_hits[path]), -match_counts[path], path),
    )
    return ranked[:top_k]


def token_counter():
    try:
        import tiktoken
    except ImportError:
        return None
    return tiktoken.get_encoding(TOKENIZER_NAME)


def estimate_tokens(path: Path, encoder: object | None) -> int:
    try:
        text = path.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return 1
    if encoder is not None:
        return max(1, len(encoder.encode(text, disallowed_special=())))
    return max(1, len(text.split()))


def line_count(path: Path) -> int:
    try:
        return len(path.read_text(encoding="utf-8", errors="ignore").splitlines())
    except OSError:
        return 1


def relevant_hit(path: str, task: dict) -> bool:
    return any(path_matches(path, target["path"]) for target in task["relevant"])


def file_rank(paths: list[str], target_path: str) -> int | None:
    for index, path in enumerate(paths, 1):
        if path_matches(path, target_path):
            return index
    return None


def grouped_by_repo(tasks: list[dict]) -> dict[str, list[dict]]:
    groups = {}
    for task in tasks:
        groups.setdefault(task["repo"], []).append(task)
    return groups


def build_payload(semble_benchmarks: Path, bench_root: Path, top_k: int, timeout: int) -> dict:
    specs = load_specs(semble_benchmarks, bench_root)
    tasks = load_tasks(semble_benchmarks, specs)
    encoder = token_counter()
    results = []
    for repo, repo_tasks in sorted(grouped_by_repo(tasks).items()):
        spec = specs[repo]
        root = benchmark_dir(spec, bench_root)
        print(f"Running ripgrep context tasks for {repo} ({len(repo_tasks)} tasks)")
        task_results = []
        for task in repo_tasks:
            paths = run_ripgrep_keyword_read(task["query"], root, top_k=top_k, timeout=timeout)
            ranks = [
                rank
                for target in task["relevant"]
                if (rank := file_rank(paths, target["path"])) is not None
            ]
            hits = []
            for path in paths:
                hit_path = Path(path)
                rel_path = str(hit_path.relative_to(root)) if hit_path.is_relative_to(root) else path
                hits.append(
                    {
                        "location": f"{rel_path}:1",
                        "path": rel_path,
                        "start_line": 1,
                        "end_line": line_count(hit_path),
                        "tokens": estimate_tokens(hit_path, encoder),
                        "relevant": relevant_hit(rel_path, task),
                    }
                )
            task_results.append(
                {
                    "query": task["query"],
                    "category": task["category"],
                    "relevant_count": len(task["relevant"]),
                    "ranks": ranks,
                    "top_results": hits,
                }
            )
        results.append(
            {
                "repo": repo,
                "language": spec["language"],
                "tasks": len(repo_tasks),
                "task_results": task_results,
            }
        )
    return {
        "method": "ripgrep + read",
        "top_k": top_k,
        "query_mode": "keyword",
        "keyword_min_len": KEYWORD_MIN_LEN,
        "max_matches_per_keyword": MAX_MATCHES_PER_KEYWORD,
        "tokenizer": TOKENIZER_NAME if encoder is not None else "whitespace",
        "metric": "file_recall_by_retrieved_whole_file_tokens",
        "results": results,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Create a token-aware ripgrep context-curve payload.")
    parser.add_argument("--semble-benchmarks", type=Path, default=DEFAULT_SEMBLE_BENCHMARKS)
    parser.add_argument("--bench-root", type=Path, default=DEFAULT_BENCH_ROOT)
    parser.add_argument("--top-k", type=int, default=100)
    parser.add_argument("--timeout", type=int, default=30)
    parser.add_argument("--output", type=Path, default=REPO_ROOT / "benchmarks" / "results" / "ripgrep-context.json")
    args = parser.parse_args()

    payload = build_payload(args.semble_benchmarks, args.bench_root, args.top_k, args.timeout)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote {args.output}")


if __name__ == "__main__":
    main()
