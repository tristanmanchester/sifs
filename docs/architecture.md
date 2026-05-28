# Architecture

SIFS is a Rust search engine for code repositories with two execution shapes:
direct in-process indexing for library and one-shot CLI use, and a shared local
daemon for agent clients that reuse warm indexes across calls. The pipeline
walks supported files, builds syntax-aware chunks when possible, builds a BM25
index, and loads semantic model state only when dense or hybrid search needs
it.

## Pipeline overview

After construction, `SifsIndex` owns everything BM25 search needs, so CLI
commands and MCP tools can run lexical searches without rereading files.

The pipeline stages are:

1. Walk supported files under a local root or temporary Git checkout.
2. Read each file as UTF-8 text.
3. Chunk source with a Tree-sitter parser when one is available.
4. Fall back to character-based chunks with symbol breadcrumbs when
   syntax-aware chunking fails.
5. Build enriched BM25 documents from chunks.
6. Store file and language mappings for filters and chunk lookup.
7. For semantic-capable indexes, lazily load the configured encoder and embed
   chunks when semantic, hybrid, or related-code search first needs dense
   vectors.

## File walking

The walker selects files by extension, skips common generated directories, and
uses the `ignore` crate for nested `.gitignore`, Git excludes, global Git
ignores, and hidden-file behavior. It sorts paths before returning them, so
index construction is deterministic for the same filesystem state.

Default ignored directories:

- `.git`, `.hg`, `.svn`
- `__pycache__`, `.mypy_cache`, `.pytest_cache`, `.ruff_cache`
- `node_modules`, `.venv`, `venv`, `.tox`, `.eggs`
- `.cache`, `.sifs`, `dist`, `build`

`from_path` indexes code extensions and returns a semantic-capable index. Use
`from_path_sparse` for a BM25-only index, or `from_path_with_options` with
`include_text_files=true` to include Markdown, YAML, TOML, and JSON.

## Chunking

Chunking starts with `tree_sitter_language_pack::get_parser` for the detected
language. When a parser is available, SIFS groups child nodes around a target
chunk size and preserves source gaps between adjacent groups.

Syntax-aware chunks store:

- The original source text for the chunk.
- A repository-relative file path.
- One-based start and end lines.
- The detected language name.

When parsing isn't available or returns no useful chunks, SIFS falls back to
character-based chunks of up to 1500 characters each. The fallback still
extracts symbol names and breadcrumbs from the chunk content, so symbol
metadata survives without a tree-sitter parser.

## Embedding model

SIFS loads semantic encoders through `src/model2vec.rs`. The default encoder is
Model2Vec with model `minishlab/potion-code-16M`, and callers can pass a custom
model path through `from_path_with_options`, `from_path_with_model_options`,
`sifs search --model`, or `sifs-embed --model`. The CLI also exposes
`--encoder hashing` as a model-free semantic encoder for smoke tests and local
experiments.

The loader reads tokenizer and tensor files directly. It supports embedding
matrices, optional weights, optional token mappings, truncation settings, and
normalization metadata. Query and chunk embeddings stay in process after the
model loads.

Model loading is lazy for semantic-capable indexes. Explicit sparse-only
construction and BM25-only search never load tokenizers, read safetensors, or
contact Hugging Face. `--no-download` blocks model downloads while allowing
local indexing. `--offline` also rejects remote Git sources.

## Sparse index

The BM25 index stores tokenized, enriched chunk documents. Enrichment adds
code metadata and symbol-like text around the raw chunk content so lexical
search ranks identifiers, file paths, and definitions effectively.

BM25 mode shines when the query contains exact names, acronyms, function
names, file-local terms, or error strings.

## Dense index

The dense index stores normalized embedding vectors for every chunk. It builds
on first semantic, hybrid, or related-code use. Semantic mode embeds the
query, normalizes it, and ranks chunks by vector similarity.

Semantic mode shines when the query describes behavior rather than exact
symbols, such as "where user sessions expire" or "how upload retries work."

## Hybrid ranking

Hybrid search runs dense and sparse retrieval, over-fetches candidates,
normalizes candidate ranks with reciprocal rank fusion, and combines the
scores. It then applies query-aware boosts and reranks the top candidates.

The hybrid alpha controls semantic weight. When callers don't provide one,
SIFS picks an alpha from the query shape:

- Symbol-like queries use more BM25 weight.
- Mixed code phrases use a balanced hybrid weight.
- Natural-language and architecture questions keep more semantic weight.
- Explicit alpha values override the automatic selection.

Hybrid is the default because most developer queries mix semantic intent with
exact code terms.

## Related-code lookup

Related-code lookup starts from a known chunk. SIFS embeds the chunk content
as the query, filters to the same language when possible, and removes the
source chunk from the result set.

`find_related` works well for finding alternate implementations, call site
patterns, duplicated logic, or similar modules.

## Daemon and MCP caching

The shared daemon holds `SifsIndex` instances in an in-memory cache for the
life of the daemon process. Local indexes are keyed by canonical path plus
index options. Git indexes are keyed by URL, optional ref, and index options.
CLI commands use the daemon when its socket is available and fall back to
direct indexing when it isn't.

The cache holds chunks, sparse data, optional semantic state, and lookup maps.
Restarting the daemon clears the live cache; persistent sparse and dense
caches survive according to the selected cache mode.

The stdio MCP server can run without a pinned source. In that mode it defaults
to the server's working directory and still accepts explicit `source`
arguments. The standard long-lived setup on macOS is:

```bash
sifs daemon install-agent
sifs mcp install --client all
```

## Agent artifacts

The `sifs agent` command renders and manages target-specific integration
artifacts on top of the same search contract. Rendering lives in
`src/agent_artifacts.rs`, mutation safety in `src/agent_installer.rs`, and
readiness checks in `src/agent_doctor.rs`.

The canonical artifact is the `sifs-search` skill package under
`skills/sifs-search/`. Target mirrors under `extras/` are local package shapes
for agent-skill consumers like OpenClaw and Hermes.

Instruction snippets are inserted into `AGENTS.md` or `CLAUDE.md` with stable
managed markers and checksums. The installer preserves surrounding user
content, runs idempotently, and requires `--force` to replace a user-modified
managed block.

Generated instructions tell agents to call MCP tools when visible in the
current session and otherwise fall back to shell commands like `sifs search`,
`sifs pack`, `sifs list-files`, `sifs get`, and `sifs agent-context --json`.

## Persistent local indexes

Default local path indexing writes persistent cache entries under the platform
cache directory (`~/Library/Caches/sifs` on macOS,
`${XDG_CACHE_HOME:-~/.cache}/sifs` on Linux). The CLI can switch to a
repo-local `.sifs/` cache with `--project-cache`. SIFS validates each cache
entry against the current sorted file signature list before loading.

The sparse persistent cache stores:

- File signatures for cache validation.
- Chunks and line locations.
- The BM25 index.

Semantic-capable local indexes also write a separate dense cache keyed by the
encoder configuration. Sparse-only indexes never write dense cache files.

Custom extension sets, custom ignore sets, document-file inclusion, and Git
temporary checkouts skip the persistent sparse cache. Those cases build an
index from source so option-specific behavior stays correct.

## Limitations

Live indexes stay in memory after construction. Persistent caches are best
effort: if a cache entry is missing or invalid, SIFS rebuilds from source and
writes a fresh entry when persistent caching is enabled.

Other current limits:

- Files must be readable as UTF-8 text.
- Git indexing uses shallow clones.
- CLI commands use platform caches by default; `--project-cache` opts into
  `.sifs/`.
- Document-like files need `--include-docs` or explicit `--extension` flags.
