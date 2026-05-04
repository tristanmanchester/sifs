import argparse
import json
from pathlib import Path

import matplotlib.pyplot as plt


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SEMBLE_RESULTS = REPO_ROOT.parent / "semble" / "benchmarks" / "results"


BASELINES = {
    "semble": "semble-hybrid-0332378809c5.json",
    "CodeRankEmbed Hybrid": "coderankembed-0332378809c5.json",
    "CodeRankEmbed": "coderankembed-0332378809c5.json",
    "ColGREP": "colgrep-c8a40fab2235.json",
    "grepai": "grepai-715563a812c3.json",
    "probe": "probe-715563a812c3.json",
    "ripgrep": "ripgrep-fixed-strings-0332378809c5.json",
}


COLORS = {
    "SIFS": "#0f766e",
    "semble": "#1a5fa8",
    "CodeRankEmbed Hybrid": "#922b21",
    "CodeRankEmbed": "#d9634f",
    "ColGREP": "#e8a838",
    "grepai": "#c0724a",
    "probe": "#9b7bb0",
    "ripgrep": "#606060",
}

LABEL_BACKGROUNDS = {
    "SIFS": "#d7f1ed",
    "semble": "#dceaf8",
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
            "query_p50_ms": payload["by_mode"]["hybrid"]["avg_p50_ms"],
        }
    if name == "CodeRankEmbed":
        return {
            "method": name,
            "ndcg10": payload["by_mode"]["semantic"]["avg_ndcg10"],
            "index_ms": 57269.4,
            "query_p50_ms": payload["by_mode"]["semantic"]["avg_p50_ms"],
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
        "query_p50_ms": float(p50),
    }


def sifs_summary(path: Path) -> dict:
    payload = load_json(path)
    return {
        "method": "SIFS",
        "ndcg10": float(payload["summary"]["avg_ndcg10"]),
        "index_ms": weighted(payload["results"], "index_ms"),
        "query_p50_ms": float(payload["summary"]["avg_p50_ms"]),
        "repos": payload["summary"]["repos"],
        "tasks": payload["summary"]["tasks"],
    }


def weighted(results: list[dict], key: str) -> float:
    total = sum(r["tasks"] for r in results)
    return sum(float(r[key]) * r["tasks"] for r in results) / total if total else 0.0


def all_method_rows(sifs_path: Path, baseline_results: Path) -> list[dict]:
    rows = [sifs_summary(sifs_path)]
    for name in [
        "semble",
        "CodeRankEmbed Hybrid",
        "CodeRankEmbed",
        "ColGREP",
        "grepai",
        "probe",
        "ripgrep",
    ]:
        rows.append(baseline_summary(name, baseline_results))
    return rows


def fmt_ms(ms: float) -> str:
    if ms >= 1000:
        return f"{ms / 1000:.1f} s"
    if ms < 0.01:
        return f"{ms:.4f} ms"
    if ms < 1:
        return f"{ms:.3f} ms"
    return f"{ms:.1f} ms"


def write_markdown(rows: list[dict], sifs_path: Path, out_path: Path) -> None:
    ordered = sorted(rows, key=lambda r: r["ndcg10"], reverse=True)
    lines = [
        "# SIFS Benchmark Report",
        "",
        f"Source SIFS result: `{sifs_path}`",
        "",
        "## Main Results",
        "",
        "| Method | NDCG@10 | Index time | Query p50 |",
        "|---|---:|---:|---:|",
    ]
    for row in ordered:
        method = f"**{row['method']}**" if row["method"] == "SIFS" else row["method"]
        lines.append(
            f"| {method} | {row['ndcg10']:.4f} | {fmt_ms(row['index_ms'])} | {fmt_ms(row['query_p50_ms'])} |"
        )
    lines += [
        "",
        "## Notes",
        "",
        "- SIFS results were produced by the Rust `sifs-benchmark` binary against the Semble benchmark annotations and pinned repositories.",
        "- Other methods use the existing Semble benchmark result JSON files in `semble/benchmarks/results`.",
        "- Cold latency is index time plus first query latency. Warm latency is query p50 with an existing index.",
        "- Existing Semble baseline files include some methods with precomputed summary-only timing fields; the report preserves those values.",
        "",
        "## Generated Figures",
        "",
        "- `assets/images/speed_vs_quality_combined.png`",
        "- `assets/images/speed_vs_quality_cold.png`",
        "- `assets/images/speed_vs_quality_warm.png`",
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
        row["query_p50_ms"] if warm else row["index_ms"] + row["query_p50_ms"] for row in rows
    ]
    min_x = min(x for x in x_values if x > 0)
    max_x = max(x_values)
    for row in rows:
        x = row["query_p50_ms"] if warm else row["index_ms"] + row["query_p50_ms"]
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
    ax.set_xlim(min_x / 2.2, max_x * 8.0)
    ax.set_ylim(0.08, 0.90)
    ax.set_xlabel("Query p50 (warm)" if warm else "Index + query p50 (cold)")
    ax.set_ylabel("NDCG@10")
    ax.set_title("Warm search" if warm else "Cold start")
    ax.grid(True, which="both", alpha=0.25)


def label_offset(name: str) -> tuple[int, int, str]:
    return {
        "SIFS": (8, 0, "left"),
        "semble": (8, 0, "left"),
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
    ax.bar([k for k, _ in items], [v for _, v in items], color="#0f766e")
    ax.set_ylim(0, 1)
    ax.set_ylabel("NDCG@10")
    ax.set_title("SIFS quality by query category")
    fig.tight_layout()
    fig.savefig(out_dir / "sifs_by_category.png", dpi=180)
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
    plot_sifs_breakdowns(args.sifs_result, assets)


if __name__ == "__main__":
    main()
