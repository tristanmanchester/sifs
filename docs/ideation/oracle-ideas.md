Yes — materially better. The biggest red flag from my first pass looks substantially improved: the benchmark-looking hard-coded ranking boosts are gone from `src/ranking.rs` and replaced with generic symbol, path-token, test/docs/example, stem, and public-API features. That is a big credibility improvement.

I still couldn’t run `cargo test` because this sandbox does not have `cargo`/`rustc`, so this is a static review of the updated zip rather than a verified build.

The short version: I’d now shift from “remove suspicious ranking behaviour” to “make the tool internally consistent, cache/freshness-correct, and benchmark-honest”. There are a few likely bugs worth fixing before adding more features.

The best improvements I saw are:

Your ranker is much cleaner. I no longer see direct benchmark-task phrases like `zodtype`, `schemautilityapi`, `format.h`, `fields.py`, etc. in production code. The remaining benchmark-ish strings are only in tests/docs, which is fine.

The cache story is better. `src/index.rs` now uses SHA-256 for cache keys, and `src/model2vec.rs` fingerprints model files with SHA-256 rather than using `DefaultHasher` for persistent identities.

The CLI now exposes `--include-docs` and repeatable `--extension`, and profiles can store those settings.

Per-result explanations exist in `SearchExplanation`, and MCP exposes them in structured result payloads when `explain` is set.

Chunk metadata now has `symbols` and `breadcrumbs`, and BM25 indexes symbol/path/breadcrumb tokens, which is a good pragmatic step.

Unreadable or non-UTF-8 files are now skipped with `IndexWarning` rather than aborting the whole index.

MCP argument validation is tighter: `alpha` bounds, string-array validation, and MCP result structure are all improved.

You added `pack`, `eval`, and `tune` scaffolding. Even if they are basic right now, they are the right product direction for agent-native search.

The biggest remaining issues I’d fix next are these.

First, `SifsIndex::is_fresh()` looks wrong and will probably make MCP freshness misleading. In `src/index.rs:484-489`, it calls:

```rust
let current = current_file_signatures(&cache_entry.root, None, None, false).ok()?;
```

But `cache_entry.root` is the cache directory, not the indexed source directory. For a project cache, that is something like `<repo>/.sifs`; for a platform cache, it is under `~/.cache/sifs/...`. So `is_fresh()` is comparing the original source-file signatures against signatures from the cache directory. In practice this likely returns `false` most of the time, which means the MCP `IndexCache` in `src/mcp.rs:171-177` may refresh the index repeatedly and report `"fresh": false` even immediately after refreshing.

I’d store the source root and index options inside `SifsIndex`, then compare against the real source:

```rust
#[derive(Clone, Debug)]
struct SourceSignatureContext {
    root: PathBuf,
    extensions: Option<HashSet<String>>,
    ignore: Option<HashSet<String>>,
    include_text_files: bool,
}

pub struct SifsIndex {
    // existing fields...
    source_signature_context: Option<SourceSignatureContext>,
}

pub fn is_fresh(&self) -> Option<bool> {
    let ctx = self.source_signature_context.as_ref()?;
    let expected = self.signatures.as_ref()?;
    let current = current_file_signatures(
        &ctx.root,
        ctx.extensions.as_ref(),
        ctx.ignore.as_ref(),
        ctx.include_text_files,
    )
    .ok()?;
    Some(&current == expected)
}
```

For Git URLs, I would either return `None` for freshness or store the checked-out revision explicitly and say “fresh against pinned revision”, not “fresh against remote branch”.

Second, daemon and MCP paths appear to ignore the new document/extension settings. The CLI `search` command resolves `include_docs` and `extensions`, but `try_daemon_search()` builds `IndexRuntimeOptions::sparse(...)` or `IndexRuntimeOptions::with_encoder(...)` without setting `include_text_files` or `extensions` in `src/main.rs:1431-1445`. So if the daemon is running, `sifs search ... --include-docs` may silently search a code-only daemon index instead of the requested docs-inclusive index.

The protocol already has the fields: `IndexRuntimeOptions` includes `extensions` and `include_text_files` in `src/daemon/protocol.rs:192-194`, and `IndexIdentity` includes them in `src/daemon/protocol.rs:250-265`. The missing part is propagation from CLI/MCP into those fields.

I’d add a small builder:

```rust
fn with_index_filters(
    mut options: IndexRuntimeOptions,
    include_docs: bool,
    extensions: Vec<String>,
) -> IndexRuntimeOptions {
    options.include_text_files = include_docs;
    options.extensions = if extensions.is_empty() {
        None
    } else {
        Some(normalize_extensions(extensions))
    };
    options
}

fn normalize_extensions(values: Vec<String>) -> Vec<String> {
    let mut values: Vec<_> = values
        .into_iter()
        .map(|value| {
            let value = value.trim().to_lowercase();
            if value.starts_with('.') {
                value
            } else {
                format!(".{value}")
            }
        })
        .collect();
    values.sort();
    values.dedup();
    values
}
```

Then use it in `try_daemon_search`, `try_daemon_find_related`, `try_daemon_list_files`, `try_daemon_get_chunk`, and MCP daemon delegation.

Third, MCP profiles do not seem to apply full profile indexing options. `selected_profile()` is used for mode/limit/source, but the MCP `IndexCache::get()` always builds `IndexOptions::new(...).with_cache(...)` without profile `include_docs`, `extensions`, `cache_dir`, `encoder`, etc. That means a profile saved with `--include-docs` can work through CLI search but not through MCP search. I’d promote “resolved invocation” into a shared library type and use the same resolution path for CLI, daemon, and MCP.

Fourth, the benchmark story is much better in code, but the checked-in artefacts/docs still look stale or incomplete. `src/bin/sifs-benchmark.rs` now has `--no-cache` and adds `reproducibility` metadata to each repo result, but the checked-in `benchmarks/results/sifs-full.json` does not contain `reproducibility`. Also, `docs/benchmark-report.md:60-67` still shows the benchmark command without `--no-cache`, while the README says `cold_index_ms` means “fresh index, no cache”.

I’d regenerate the checked-in benchmark JSON and docs with something like:

```bash
target/release/sifs-benchmark \
  --benchmarks-dir /path/to/benchmark-corpus \
  --bench-root /path/to/pinned-checkouts \
  --output benchmarks/results/sifs-full.json \
  --no-download \
  --no-cache \
  --include-tasks
```

More importantly, I’d add a separate metric for first-use latency. Right now `cold_index_ms` measures construction of the sparse/chunk index, but semantic embeddings are lazy-loaded/built on first semantic or hybrid search. The benchmark then warms the first query before measuring query latency. That is fine as a warm-query benchmark, but it is not the “from nothing to first hybrid result” experience.

I’d report at least these:

```text
cold_sparse_index_ms
cold_semantic_build_or_load_ms
cold_first_hybrid_search_ms
warm_uncached_query_ms
warm_cached_repeat_query_ms
```

That would make the performance claim much harder to misunderstand.

Fifth, `semantic_index_available()` still checks for `semantic-v2-` in `src/main.rs:1855-1868`, while the current semantic cache prefix is `semantic-v4` in `src/index.rs:44`. So `sifs status` can report that a semantic index is unavailable even when the v4 semantic cache exists. The core test already checks for `semantic-v4-`, so the status helper should be updated or, better, replaced with a library method that uses the same `semantic_cache_path()` logic as the cache writer.

Sixth, cached indexes drop warnings. `CachedIndexPayload` stores `chunks` and `bm25_index`, but not `warnings`. If the first index build skipped unreadable/non-UTF-8 files and then wrote a sparse cache, a later cache hit via `from_cached_parts()` returns `warnings: Vec::new()`. I’d add `warnings: Vec<IndexWarning>` to the cache payload and bump the cache version.

Seventh, `pack` is a good start but currently undersells the idea. It is BM25-only, always includes docs, ignores profiles/cache/extension filters, deduplicates by file by taking only the first chunk, and prints JSON even without `--json`. I’d turn it into the agent-native flagship command:

```bash
sifs pack "how request auth works" \
  --source . \
  --mode hybrid \
  --budget-tokens 6000 \
  --include-neighbours 1 \
  --include-symbol-definitions \
  --json
```

The packer should use hybrid retrieval, select diverse files with maximum marginal relevance, add adjacent chunks when useful, include symbol breadcrumbs/imports, and explain why each chunk was included. For agent work, a high-quality context pack is often more valuable than a ranked list.

Eighth, `eval`/`tune` are promising but currently too BM25-specific. `run_eval()` in `src/main.rs:4093-4098` builds a sparse index and evaluates only BM25. I’d let it evaluate all configured modes and report hit@k, MRR, NDCG, and regressions by category:

```bash
sifs eval --from-feedback --source . --mode hybrid --limit 10 --json
sifs eval --from-feedback --source . --all-modes --json
```

Then `tune --dry-run` could actually do useful work: try alpha values, path-boost weights, test/doc penalties, and rerank settings against local feedback without modifying production defaults.

Ninth, path-intent boosting is now generic, but it may run too broadly. `path_intent_query()` returns true whenever `query_path_terms(lowered).next().is_some()`, which is almost any non-trivial natural-language query. That means most queries get path-token boosting, not just path-intent queries. That may be fine, but I’d rename it conceptually to “path-token feature”, cap its contribution more explicitly, and expose the contribution in explanations.

I’d also add generic candidate expansion for path matches. Right now `boost_path_intent()` boosts only chunks already in the candidate set. If the right file is named `retry/backoff.rs` but BM25/semantic did not retrieve it, the path feature cannot rescue it. A cleaner approach is to retrieve candidates from multiple indexes:

```text
content BM25 candidates
symbol/name candidates
path-token candidates
semantic candidates
```

Then fuse them with RRF. That gives you the benefits of the old path-oracle behaviour without benchmark-specific hard-coding.

Tenth, symbol extraction is useful but still quite shallow. The current line-based extractor in `src/chunker.rs` catches things like `def`, `class`, `fn`, `struct`, `interface`, C macros, C typedef structs/enums, and common visibility/modifier prefixes, but it will miss forms such as `impl Foo`, `const useThing = (...) =>`, decorators, Rust `impl` methods, TypeScript object methods, Python async defs, Java annotations, and so on.

The next upgrade is tree-sitter symbol extraction per language. You do not need perfect LSP-level semantics; even a 70% extractor for top languages would improve ranking and context packaging. The stored `Symbol` could become:

```rust
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub end_line: Option<usize>,
    pub visibility: Option<String>,
    pub container: Option<String>,
}
```

Then breadcrumbs become real structural context rather than “all symbols appearing in this chunk”.

Eleventh, document/config indexing needs guardrails. `--include-docs` currently includes Markdown, JSON, YAML, and TOML. That is useful, but JSON/YAML can include enormous generated files: `package-lock.json`, `pnpm-lock.yaml`, OpenAPI bundles, generated schemas, test fixtures, etc. I’d add default size limits and generated-file exclusions, especially when docs are enabled.

Something like:

```bash
--max-file-bytes 1_000_000
--include-lockfiles
--include-generated
```

And defaults that skip things like `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, generated schema bundles, coverage files, and minified JSON unless explicitly requested.

Twelfth, the query cache is unbounded. `SifsIndex` has `search_cache: Mutex<HashMap<SearchCacheKey, Vec<SearchResult>>>`. In a long-running daemon/MCP session, an agent can easily issue hundreds or thousands of unique queries, and each cached result clones chunks including content. I’d add an LRU or a simple byte/entry cap:

```text
SIFS_QUERY_CACHE_ENTRIES=256
SIFS_QUERY_CACHE_BYTES=64MB
```

For MCP, I’d consider disabling query-cache by default for very large result payloads or caching only chunk IDs/scores rather than full `SearchResult` clones.

Thirteenth, the CLI contract and agent context need updating to match the new surface area. `agent_context.rs` still does not describe `--include-docs`, `--extension`, `--explain`, `pack`, `eval`, or `tune`. Since agents are supposed to use `sifs agent-context --json` as the machine-readable contract, missing flags there will make agent behaviour lag the CLI.

Fourteenth, path and extension normalisation should be stricter. `extension_set()` preserves case, while `walk_files()` lowercases actual file extensions before matching. So `--extension RS` probably will not match `.rs`. I’d normalise user extensions to lowercase, strip whitespace, reject empty values, and maybe reject `*`/glob-looking inputs unless you deliberately support them.

Similarly, `filter_paths` are exact. You already warn if `./src/lib.rs` did not match but `src/lib.rs` exists, but the search still fails. I’d normalise `./`, repeated slashes, and Windows separators before building the selector.

Fifteenth, decide the Windows story. The daemon uses Unix sockets, so Windows support is either absent or partial. If unsupported, make that explicit in README and `doctor`. If you want Windows support, abstract the transport behind a small trait and use named pipes or TCP loopback with a lockfile/token.

My recommended next priority order would be:

Fix `is_fresh()` first, because it can cause repeated MCP refreshes and misleading `"fresh": false` output.

Then make CLI/daemon/MCP honour the same indexing options, especially `include_docs` and `extensions`.

Then regenerate benchmark artefacts with `--no-cache`, add first-use semantic/hybrid latency, and update the README claims accordingly.

Then upgrade `pack` into a proper context-pack generator, because that is likely the most agent-native differentiator.

After that, improve symbol extraction and fielded retrieval. The tool is now in a much more credible place; the remaining work is mostly consistency, evaluation discipline, and turning “search results” into “safe-to-edit context”.
