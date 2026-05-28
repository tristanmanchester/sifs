#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import time
from pathlib import Path
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare SIFS and codedb on equivalent local search, symbol, and outline tasks."
    )
    parser.add_argument("--sifs-bin", default="target/release/sifs")
    parser.add_argument("--codedb-bin", required=True)
    parser.add_argument("--tasks", required=True, help="JSON file with repos and tasks")
    parser.add_argument("--work-dir", default="/tmp/sifs-codedb-compare")
    parser.add_argument("--output", default="benchmarks/results/codedb-compare.json")
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument(
        "--sifs-daemon",
        choices=["off", "auto"],
        default="off",
        help="Use 'auto' to allow an existing SIFS daemon; default isolates one-shot CLI timings.",
    )
    parser.add_argument(
        "--reuse-cache",
        action="store_true",
        help="Keep prior isolated SIFS/codedb caches under --work-dir.",
    )
    return parser.parse_args()


def run(cmd: list[str], env: dict[str, str] | None, timeout: float) -> dict[str, Any]:
    started = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
            timeout=timeout,
        )
        elapsed_ms = (time.perf_counter() - started) * 1000
    except subprocess.TimeoutExpired as exc:
        elapsed_ms = (time.perf_counter() - started) * 1000
        stdout = exc.stdout or ""
        stderr = exc.stderr or ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode(errors="replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode(errors="replace")
        return {
            "returncode": None,
            "elapsed_ms": elapsed_ms,
            "stdout": stdout,
            "stderr": stderr or f"timed out after {timeout:.1f}s",
            "timed_out": True,
        }
    return {
        "returncode": proc.returncode,
        "elapsed_ms": elapsed_ms,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "timed_out": False,
    }


def codedb_env(home: Path) -> dict[str, str]:
    env = os.environ.copy()
    env["HOME"] = str(home)
    env["CODEDB_NO_TELEMETRY"] = "1"
    env["CODEDB_NO_AUTO_UPDATE"] = "1"
    return env


def sifs_env(mode: str, work_dir: Path) -> dict[str, str]:
    env = os.environ.copy()
    if mode == "off":
        env["SIFS_DAEMON_RUNTIME_DIR"] = str(work_dir / "no-sifs-daemon")
    return env


def parse_codedb_paths(output: str) -> list[str]:
    paths = []
    path_line = re.compile(r"(?:^|\s)([^\s:]+(?:/[^\s:]+)*\.[A-Za-z0-9][^\s:]*):(\d+)")
    for line in output.splitlines():
        match = path_line.search(line)
        if match:
            paths.append(match.group(1))
    return paths


def expected_rank(paths: list[str], expected: str | None) -> int | None:
    if not expected:
        return None
    for index, path in enumerate(paths, start=1):
        if path == expected or path.endswith(f"/{expected}") or path.endswith(expected):
            return index
    return None


def median(values: list[float]) -> float | None:
    return statistics.median(values) if values else None


def load_tasks(path: Path) -> tuple[dict[str, str], list[dict[str, Any]]]:
    data = json.loads(path.read_text(encoding="utf-8"))
    repos = {name: str(Path(repo).expanduser()) for name, repo in data["repos"].items()}
    return repos, data["tasks"]


def sifs_search_command(
    sifs_bin: str,
    repo: str,
    cache_dir: Path,
    task: dict[str, Any],
) -> list[str]:
    mode = task.get("mode", "bm25")
    return [
        sifs_bin,
        "search",
        task["query"],
        "--source",
        repo,
        "--offline",
        "--cache-dir",
        str(cache_dir),
        "--mode",
        mode,
        "--limit",
        str(task.get("limit", 10)),
        "--json",
    ]


def sifs_symbol_command(
    sifs_bin: str,
    repo: str,
    cache_dir: Path,
    task: dict[str, Any],
) -> list[str]:
    return [
        sifs_bin,
        "symbol",
        task["query"],
        "--source",
        repo,
        "--offline",
        "--cache-dir",
        str(cache_dir),
        "--limit",
        str(task.get("limit", 10)),
        "--json",
    ]


def sifs_outline_command(
    sifs_bin: str,
    repo: str,
    cache_dir: Path,
    task: dict[str, Any],
) -> list[str]:
    return [
        sifs_bin,
        "outline",
        task["file"],
        "--source",
        repo,
        "--offline",
        "--cache-dir",
        str(cache_dir),
        "--symbols-limit",
        str(task.get("symbols_limit", 1000)),
        "--no-chunks",
        "--json",
    ]


def codedb_command(codedb_bin: str, repo: str, task: dict[str, Any]) -> list[str]:
    kind = task["kind"]
    if kind == "search":
        return [codedb_bin, repo, "search", task["query"]]
    if kind == "symbol":
        return [codedb_bin, repo, "find", task["query"]]
    if kind == "outline":
        return [codedb_bin, repo, "outline", task["file"]]
    raise ValueError(f"unsupported task kind: {kind}")


def sifs_command(
    sifs_bin: str,
    repo: str,
    cache_dir: Path,
    task: dict[str, Any],
) -> list[str]:
    kind = task["kind"]
    if kind == "search":
        return sifs_search_command(sifs_bin, repo, cache_dir, task)
    if kind == "symbol":
        return sifs_symbol_command(sifs_bin, repo, cache_dir, task)
    if kind == "outline":
        return sifs_outline_command(sifs_bin, repo, cache_dir, task)
    raise ValueError(f"unsupported task kind: {kind}")


def parse_sifs_result(kind: str, output: str) -> tuple[list[str], int | None]:
    if not output.strip():
        return [], None
    data = json.loads(output)
    if kind == "search":
        return [hit["file_path"] for hit in data.get("results", [])], None
    if kind == "symbol":
        return [symbol["file_path"] for symbol in data.get("symbols", [])], None
    if kind == "outline":
        return [], data.get("total_symbols")
    return [], None


def parse_codedb_result(kind: str, output: str) -> tuple[list[str], int | None]:
    if kind in {"search", "symbol"}:
        return parse_codedb_paths(output), None
    if kind == "outline":
        count = sum(1 for line in output.splitlines() if re.match(r"\s*L\d+\s+", line))
        return [], count
    return [], None


def summarize_runs(
    kind: str,
    expected: str | None,
    runs: list[dict[str, Any]],
    parser,
) -> dict[str, Any]:
    timings = [run["elapsed_ms"] for run in runs]
    ranks = []
    counts = []
    for run in runs:
        if run.get("timed_out") or run["returncode"] != 0:
            paths, count = [], None
        else:
            paths, count = parser(kind, run["stdout"])
        ranks.append(expected_rank(paths, expected))
        counts.append(count if count is not None else len(paths))
    first = runs[0] if runs else {"stdout": "", "stderr": "", "returncode": None}
    return {
        "median_ms": median(timings),
        "min_ms": min(timings) if timings else None,
        "max_ms": max(timings) if timings else None,
        "returncodes": [run["returncode"] for run in runs],
        "timeouts": [run.get("timed_out", False) for run in runs],
        "ranks": ranks,
        "counts": counts,
        "sample": "\n".join((first["stdout"] or first["stderr"]).splitlines()[:8]),
    }


def setup_repo(
    sifs_bin: str,
    codedb_bin: str,
    repo_name: str,
    repo: str,
    sifs_cache: Path,
    sifs_process_env: dict[str, str],
    codedb_process_env: dict[str, str],
    timeout: float,
) -> dict[str, Any]:
    sifs = run(
        [
            sifs_bin,
            "status",
            "--source",
            repo,
            "--offline",
            "--cache-dir",
            str(sifs_cache),
            "--json",
        ],
        sifs_process_env,
        timeout,
    )
    codedb = run([codedb_bin, repo, "tree"], codedb_process_env, timeout)
    files = None
    if sifs["returncode"] == 0:
        files = json.loads(sifs["stdout"])["index_stats"]["indexed_files"]
    return {
        "repo": repo_name,
        "sifs_ms": sifs["elapsed_ms"],
        "sifs_files": files,
        "codedb_ms": codedb["elapsed_ms"],
        "codedb_sample": "\n".join(codedb["stdout"].splitlines()[:3]),
        "returncodes": {"sifs": sifs["returncode"], "codedb": codedb["returncode"]},
        "timeouts": {
            "sifs": sifs.get("timed_out", False),
            "codedb": codedb.get("timed_out", False),
        },
    }


def version_line(cmd: list[str], env: dict[str, str] | None, timeout: float) -> str:
    result = run(cmd, env, timeout)
    lines = (result["stdout"] or result["stderr"]).splitlines()
    return lines[0] if lines else ""


def main() -> None:
    args = parse_args()
    work_dir = Path(args.work_dir)
    if work_dir.exists() and not args.reuse_cache:
        shutil.rmtree(work_dir)
    sifs_cache = work_dir / "sifs-cache"
    codedb_home = work_dir / "codedb-home"
    sifs_cache.mkdir(parents=True, exist_ok=True)
    codedb_home.mkdir(parents=True, exist_ok=True)
    codedb_process_env = codedb_env(codedb_home)
    sifs_process_env = sifs_env(args.sifs_daemon, work_dir)

    repos, tasks = load_tasks(Path(args.tasks))
    setup = [
        setup_repo(
            args.sifs_bin,
            args.codedb_bin,
            name,
            repo,
            sifs_cache,
            sifs_process_env,
            codedb_process_env,
            args.timeout,
        )
        for name, repo in repos.items()
    ]

    results = []
    for task in tasks:
        repo = repos[task["repo"]]
        sifs_runs = [
            run(
                sifs_command(args.sifs_bin, repo, sifs_cache, task),
                sifs_process_env,
                args.timeout,
            )
            for _ in range(args.repetitions)
        ]
        codedb_runs = [
            run(codedb_command(args.codedb_bin, repo, task), codedb_process_env, args.timeout)
            for _ in range(args.repetitions)
        ]
        results.append(
            {
                "repo": task["repo"],
                "kind": task["kind"],
                "query": task.get("query") or task.get("file"),
                "expected": task.get("expected"),
                "sifs": summarize_runs(task["kind"], task.get("expected"), sifs_runs, parse_sifs_result),
                "codedb": summarize_runs(
                    task["kind"], task.get("expected"), codedb_runs, parse_codedb_result
                ),
            }
        )

    output = {
        "versions": {
            "sifs": version_line([args.sifs_bin, "--version"], None, args.timeout),
            "codedb": version_line([args.codedb_bin, "--help"], codedb_process_env, args.timeout),
        },
        "sifs_daemon": args.sifs_daemon,
        "setup": setup,
        "tasks": results,
    }
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(output, indent=2), encoding="utf-8")
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
