import argparse
import json
from pathlib import Path

import matplotlib.pyplot as plt
from matplotlib.ticker import FuncFormatter, LogLocator, NullFormatter


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SEMBLE_RESULTS = REPO_ROOT.parent / "semble" / "benchmarks" / "results"


BASELINES = {
    "Semble": "semble-hybrid-0332378809c5.json",
    "CodeRankEmbed Hybrid": "coderankembed-0332378809c5.json",
    "CodeRankEmbed": "coderankembed-0332378809c5.json",
    "ColGREP": "colgrep-c8a40fab2235.json",
    "grepai": "grepai-715563a812c3.json",
    "probe": "probe-715563a812c3.json",
    "ripgrep": "ripgrep-fixed-strings-0332378809c5.json",
}


COLORS = {
    "SIFS": "#0f766e",
    "SIFS hybrid": "#0f766e",
    "SIFS BM25": "#2563eb",
    "SIFS semantic": "#b45309",
    "Semble": "#1a5fa8",
    "CodeRankEmbed Hybrid": "#922b21",
    "CodeRankEmbed": "#d9634f",
    "ColGREP": "#e8a838",
    "grepai": "#c0724a",
    "probe": "#9b7bb0",
    "ripgrep": "#606060",
}

LABEL_BACKGROUNDS = {
    "SIFS": "#d7f1ed",
    "Semble": "#dceaf8",
    "CodeRankEmbed Hybrid": "#f5dedb",
    "CodeRankEmbed": "#f9e0d9",
    "ColGREP": "#f8ecd0",
    "grepai": "#f3e3d8",
    "probe": "#eee5f3",
    "ripgrep": "#e7e7e7",
}


def load_json(path: Path) -> dict:
    return json.loads(path.read_text())


def baseline_summary(name: str, baseline_results: Path) -> dict:
    payload = load_json(baseline_results / BASELINES[name])
    if name == "CodeRankEmbed Hybrid":
        return {
            "method": name,
            "ndcg10": payload["by_mode"]["hybrid"]["avg_ndcg10"],
            "index_ms": 57269.4,
            "warm_uncached_query_ms": payload["by_mode"]["hybrid"]["avg_p50_ms"],
            "warm_cached_repeat_query_ms": None,
        }
    if name == "CodeRankEmbed":
        return {
            "method": name,
            "ndcg10": payload["by_mode"]["semantic"]["avg_ndcg10"],
            "index_ms": 57269.4,
            "warm_uncached_query_ms": payload["by_mode"]["semantic"]["avg_p50_ms"],
            "warm_cached_repeat_query_ms": None,
        }
    summary = payload.get("summary") or {}
    method = payload.get("method", name)
    ndcg10 = summary.get("ndcg10") or summary.get("avg_ndcg10") or payload.get("avg_ndcg10")
    p50 = summary.get("p50_ms") or summary.get("avg_p50_ms") or payload.get("avg_p50_ms")
    index_ms = summary.get("index_ms") or payload.get("avg_index_ms")
    if name == "ripgrep":
        index_ms = 0.0
    if name == "probe":
        index_ms = 0.0
    return {
        "method": name,
        "source_method": method,
        "ndcg10": float(ndcg10),
        "index_ms": float(index_ms or 0.0),
        "warm_uncached_query_ms": float(p50),
        "warm_cached_repeat_query_ms": None,
    }


def sifs_summary(path: Path) -> dict:
    payload = load_json(path)
    summary = payload["summary"]
    return {
        "method": "SIFS",
        "ndcg10": float(summary["avg_ndcg10"]),
        "index_ms": float(summary.get("avg_cold_index_ms") or weighted_any(payload["results"], "cold_index_ms", "index_ms")),
        "warm_uncached_query_ms": float(
            summary.get("avg_warm_uncached_query_ms") or summary.get("avg_p50_ms")
        ),
        "warm_cached_repeat_query_ms": summary.get("avg_warm_cached_repeat_query_ms"),
        "repos": summary["repos"],
        "tasks": summary["tasks"],
    }


def weighted(results: list[dict], key: str) -> float:
    total = sum(r["tasks"] for r in results)
    return sum(float(r[key]) * r["tasks"] for r in results) / total if total else 0.0


def weighted_any(results: list[dict], *keys: str) -> float:
    total = sum(r["tasks"] for r in results)
    if not total:
        return 0.0
    return (
        sum(float(next(r[key] for key in keys if key in r)) * r["tasks"] for r in results)
        / total
    )


def all_method_rows(sifs_path: Path, baseline_results: Path) -> list[dict]:
    rows = [sifs_summary(sifs_path)]
    for name in [
        "Semble",
        "CodeRankEmbed Hybrid",
        "CodeRankEmbed",
        "ColGREP",
        "grepai",
        "probe",
        "ripgrep",
    ]:
        rows.append(baseline_summary(name, baseline_results))
    return rows


def fmt_ms(ms: float | None) -> str:
    if ms is None:
        return "n/a"
    if ms >= 1000:
        return f"{ms / 1000:.1f} s"
    if ms < 0.01:
        return f"{ms:.4f} ms"
    if ms < 1:
        return f"{ms:.3f} ms"
    return f"{ms:.1f} ms"


def fmt_axis_ms(ms: float, _pos: int) -> str:
    if ms <= 0:
        return ""
    if ms >= 1000:
        seconds = ms / 1000
        return f"{seconds:g} s"
    if ms < 1:
        return f"{ms:g} ms"
    return f"{ms:g} ms"


def write_markdown(rows: list[dict], sifs_path: Path, out_path: Path) -> None:
    ordered = sorted(rows, key=lambda r: r["ndcg10"], reverse=True)
    lines = [
        "# SIFS Benchmark Report",
        "",
        f"Source SIFS result: `{sifs_path}`",
        "",
        "## Main Results",
        "",
        "| Method | NDCG@10 | Cold index | Warm uncached query | Cached repeat query |",
        "|---|---:|---:|---:|---:|",
    ]
    for row in ordered:
        method = f"**{row['method']}**" if row["method"] == "SIFS" else row["method"]
        lines.append(
            f"| {method} | {row['ndcg10']:.4f} | {fmt_ms(row['index_ms'])} | {fmt_ms(row['warm_uncached_query_ms'])} | {fmt_ms(row.get('warm_cached_repeat_query_ms'))} |"
        )
    lines += [
        "",
        "## Notes",
        "",
        "- SIFS results were produced by the Rust `sifs-benchmark` binary against the annotated pinned-repository corpus.",
        "- Baseline methods use existing comparison result JSON files from the adjacent Python tool checkout.",
        "- Warm uncached query latency bypasses the in-process SIFS query-result cache. Cached repeat query latency measures identical repeated queries after one warm-up.",
        "- Some baseline files only expose precomputed summary timing fields; the report preserves those values.",
        "",
        "## Generated Figures",
        "",
        "- `assets/images/speed_vs_quality_combined.png`",
        "- `assets/images/speed_vs_quality_cold.png`",
        "- `assets/images/speed_vs_quality_warm.png`",
        "- `assets/images/quality_vs_warm_latency.png`",
        "- `assets/images/sifs_context_efficiency.png`",
        "- `assets/images/sifs_by_query_type.png`",
        "- `assets/images/sifs_by_language.png`",
        "- `assets/images/sifs_by_category.png`",
    ]
    out_path.write_text("\n".join(lines) + "\n")


def plot_speed_quality(rows: list[dict], out_path: Path, *, warm: bool) -> None:
    fig, ax = plt.subplots(figsize=(8, 5))
    draw_speed_quality_panel(ax, rows, warm=warm)
    fig.tight_layout()
    fig.savefig(out_path, dpi=180, bbox_inches="tight")
    plt.close(fig)


def plot_combined_speed_quality(rows: list[dict], out_path: Path) -> None:
    fig, axes = plt.subplots(1, 2, figsize=(13, 5), sharey=True)
    draw_speed_quality_panel(axes[0], rows, warm=False)
    draw_speed_quality_panel(axes[1], rows, warm=True)
    axes[0].set_ylabel("NDCG@10")
    axes[1].set_ylabel("")
    fig.suptitle("SIFS speed and quality compared with code-search baselines", y=1.02)
    fig.tight_layout()
    fig.savefig(out_path, dpi=180, bbox_inches="tight")
    plt.close(fig)


def draw_speed_quality_panel(ax, rows: list[dict], *, warm: bool) -> None:
    x_values = [
        row["warm_uncached_query_ms"]
        if warm
        else row["index_ms"] + row["warm_uncached_query_ms"]
        for row in rows
    ]
    min_x = min(x for x in x_values if x > 0)
    max_x = max(x_values)
    for row in rows:
        x = (
            row["warm_uncached_query_ms"]
            if warm
            else row["index_ms"] + row["warm_uncached_query_ms"]
        )
        y = row["ndcg10"]
        name = row["method"]
        ax.scatter(x, y, s=140 if name == "SIFS" else 90, color=COLORS.get(name, "#444"))
        dx, dy, ha = label_offset(name)
        ax.annotate(
            name,
            (x, y),
            xytext=(dx, dy),
            textcoords="offset points",
            va="center",
            ha=ha,
            fontsize=8,
            bbox={
                "boxstyle": "round,pad=0.22,rounding_size=0.12",
                "facecolor": LABEL_BACKGROUNDS.get(name, "#eeeeee"),
                "edgecolor": "none",
                "alpha": 0.92,
            },
        )
    ax.set_xscale("log")
    ax.xaxis.set_major_locator(LogLocator(base=10))
    ax.xaxis.set_major_formatter(FuncFormatter(fmt_axis_ms))
    ax.xaxis.set_minor_locator(LogLocator(base=10, subs=range(2, 10)))
    ax.xaxis.set_minor_formatter(NullFormatter())
    ax.set_xlim(min_x / 2.2, max_x * 8.0)
    ax.set_ylim(0.08, 0.90)
    ax.set_xlabel(
        "Warm uncached query p50 (log scale)"
        if warm
        else "Cold index + warm uncached query p50 (log scale)"
    )
    ax.set_ylabel("NDCG@10")
    ax.set_title("Warm search" if warm else "Cold start")
    ax.grid(True, which="both", alpha=0.25)


def label_offset(name: str) -> tuple[int, int, str]:
    return {
        "SIFS": (8, 0, "left"),
        "Semble": (8, 0, "left"),
        "CodeRankEmbed Hybrid": (10, 0, "left"),
        "CodeRankEmbed": (10, 0, "left"),
        "ColGREP": (8, 0, "left"),
        "grepai": (8, 0, "left"),
        "probe": (8, 0, "left"),
        "ripgrep": (8, 0, "left"),
    }.get(name, (8, 0, "left"))


def plot_sifs_breakdowns(sifs_path: Path, out_dir: Path) -> None:
    payload = load_json(sifs_path)
    results = payload["results"]
    by_lang: dict[str, list[dict]] = {}
    for row in results:
        by_lang.setdefault(row["language"], []).append(row)
    lang_scores = {
        lang: sum(r["ndcg10"] * r["tasks"] for r in rows) / sum(r["tasks"] for r in rows)
        for lang, rows in by_lang.items()
    }
    fig, ax = plt.subplots(figsize=(10, 5))
    items = sorted(lang_scores.items(), key=lambda x: x[1])
    ax.barh([k for k, _ in items], [v for _, v in items], color="#0f766e")
    ax.set_xlim(0, 1)
    ax.set_xlabel("NDCG@10")
    ax.set_title("SIFS quality by language")
    fig.tight_layout()
    fig.savefig(out_dir / "sifs_by_language.png", dpi=180)
    plt.close(fig)

    cats: dict[str, list[tuple[float, int]]] = {}
    for row in results:
        for cat, score in row.get("by_category", {}).items():
            cats.setdefault(cat, []).append((float(score), row["tasks"]))
    cat_scores = {
        cat: sum(score * tasks for score, tasks in values) / sum(tasks for _, tasks in values)
        for cat, values in cats.items()
    }
    fig, ax = plt.subplots(figsize=(6, 4))
    items = sorted(cat_scores.items(), key=lambda x: x[0])
    bars = ax.bar([k for k, _ in items], [v for _, v in items], color="#0f766e")
    ax.bar_label(bars, labels=[f"{v:.3f}" for _, v in items], padding=3, fontsize=8)
    ax.set_ylim(0, 1)
    ax.set_ylabel("NDCG@10")
    ax.set_title("SIFS quality by query type")
    fig.tight_layout()
    fig.savefig(out_dir / "sifs_by_category.png", dpi=180)
    fig.savefig(out_dir / "sifs_by_query_type.png", dpi=180)
    plt.close(fig)


def context_curve(result_path: Path, budgets: list[int]) -> tuple[str, list[float]]:
    payload = load_json(result_path)
    method = payload.get("method", result_path.stem)
    label = {
        "sifs-hybrid": "SIFS hybrid",
        "sifs-bm25": "SIFS BM25",
        "sifs-semantic": "SIFS semantic",
    }.get(method, method)
    totals = [0.0 for _ in budgets]
    task_count = 0
    for repo in payload["results"]:
        for task in repo.get("task_results", []):
            relevant_count = int(task.get("relevant_count") or 0)
            if relevant_count == 0:
                continue
            task_count += 1
            cumulative_tokens = 0
            relevant_paths: set[str] = set()
            points: list[tuple[int, float]] = [(0, 0.0)]
            for hit in task.get("top_results", []):
                cumulative_tokens += int(hit.get("tokens") or 1)
                if hit.get("relevant"):
                    relevant_paths.add(hit.get("path") or hit.get("location") or "")
                recall = min(len(relevant_paths), relevant_count) / relevant_count
                points.append((cumulative_tokens, recall))
            for idx, budget in enumerate(budgets):
                recall_at_budget = 0.0
                for tokens, recall in points:
                    if tokens > budget:
                        break
                    recall_at_budget = recall
                totals[idx] += recall_at_budget
    if task_count == 0:
        return label, [0.0 for _ in budgets]
    return label, [value / task_count for value in totals]


def context_curve_summary(context_results: list[Path], budgets: list[int], summary_path: Path) -> dict:
    raw_paths = [path for path in context_results if path.exists()]
    if not raw_paths and summary_path.exists():
        return load_json(summary_path)
    curves = []
    for path in raw_paths:
        label, recalls = context_curve(path, budgets)
        source = str(path.relative_to(REPO_ROOT)) if path.is_relative_to(REPO_ROOT) else str(path)
        curves.append({"label": label, "recall": recalls, "source": source})
    summary = {
        "budgets": budgets,
        "metric": "file_recall_by_retrieved_chunk_tokens",
        "curves": curves,
    }
    summary_path.write_text(json.dumps(summary, indent=2) + "\n")
    return summary


def plot_context_efficiency(context_results: list[Path], out_path: Path, summary_path: Path) -> None:
    budgets = [0, 100, 250, 500, 1000, 2000, 4000, 8000, 16000]
    summary = context_curve_summary(context_results, budgets, summary_path)
    budgets = summary["budgets"]
    fig, ax = plt.subplots(figsize=(9, 5))
    for curve in summary["curves"]:
        label = curve["label"]
        recalls = curve["recall"]
        ax.plot(
            budgets,
            recalls,
            label=label,
            marker="o",
            markersize=4,
            linewidth=2.4 if label == "SIFS hybrid" else 2.0,
            color=COLORS.get(label),
        )
    ax.set_xlim(0, max(budgets))
    ax.set_ylim(0, 1.0)
    ax.set_xlabel("Retrieved context tokens")
    ax.set_ylabel("Relevant target files found")
    ax.set_title("Context efficiency: recall vs. retrieved tokens")
    ax.grid(True, alpha=0.25)
    ax.legend(loc="lower right")
    fig.tight_layout()
    fig.savefig(out_path, dpi=180, bbox_inches="tight")
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sifs-result", type=Path, required=True)
    parser.add_argument(
        "--baseline-results",
        type=Path,
        default=DEFAULT_SEMBLE_RESULTS,
        help="Directory containing Semble baseline result JSON files.",
    )
    parser.add_argument(
        "--assets-dir",
        type=Path,
        default=REPO_ROOT / "assets" / "images",
        help="Directory where PNG figures are written.",
    )
    parser.add_argument(
        "--summary-md",
        type=Path,
        default=REPO_ROOT / "benchmarks" / "README.generated.md",
        help="Path for a generated Markdown summary table.",
    )
    args = parser.parse_args()
    assets = args.assets_dir
    assets.mkdir(parents=True, exist_ok=True)
    rows = all_method_rows(args.sifs_result, args.baseline_results)
    write_markdown(rows, args.sifs_result, args.summary_md)
    plot_combined_speed_quality(rows, assets / "speed_vs_quality_combined.png")
    plot_speed_quality(rows, assets / "speed_vs_quality_cold.png", warm=False)
    plot_speed_quality(rows, assets / "speed_vs_quality_warm.png", warm=True)
    plot_speed_quality(rows, assets / "quality_vs_warm_latency.png", warm=True)
    plot_sifs_breakdowns(args.sifs_result, assets)
    plot_context_efficiency(
        [
            REPO_ROOT / "benchmarks" / "results" / "sifs-context-hybrid.json",
            REPO_ROOT / "benchmarks" / "results" / "sifs-context-bm25.json",
            REPO_ROOT / "benchmarks" / "results" / "sifs-context-semantic.json",
        ],
        assets / "sifs_context_efficiency.png",
        REPO_ROOT / "benchmarks" / "results" / "sifs-context-curves.json",
    )


if __name__ == "__main__":
    main()
