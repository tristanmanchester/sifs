# Architecture

SIFS is a Rust search engine for code repositories with two execution shapes:
direct in-process indexing for library and one-shot use, and a shared local
daemon for agent clients that benefit from warm indexes across repeated tool
calls. The pipeline walks supported files, builds syntax-aware chunks when
possible, builds a BM25 index, and lazily attaches semantic model state only
when dense or hybrid search needs it.

## Pipeline overview

The index pipeline is small. `SifsIndex` owns all data needed for BM25 search
after construction, so CLI commands and MCP tools can run model-free lexical
searches without reading files again.

The pipeline stages are:

1. Walk supported files under a local root or temporary Git checkout.
2. Read each file as UTF-8 text.
3. Chunk source with a Tree-sitter parser when one is available.
4. Fall back to overlapping line chunks when syntax-aware chunking fails.
5. Build enriched BM25 documents from chunks.
6. Store file and language mappings for filters and chunk lookup.
7. For semantic-capable indexes, lazily load the configured encoder and embed
   chunks when semantic, hybrid, or related-code search first needs dense
   vectors.

## File walking

The file walker selects files by extension, skips common generated directories,
and uses the `ignore` crate for nested `.gitignore`, Git excludes, global Git
ignores, and hidden-file behavior. It sorts paths before returning them so index
construction is deterministic for the same filesystem state.

Default ignored directories are:

- `.git`, `.hg`, and `.svn`
- `__pycache__`, `.mypy_cache`, `.pytest_cache`, and `.ruff_cache`
- `node_modules`, `.venv`, `venv`, `.tox`, and `.eggs`
- `.cache`, `.sifs`, `dist`, and `build`

By default, the public `from_path` constructor indexes code extensions only and
returns a semantic-capable index. Use `from_path_sparse` for an explicitly
BM25-only index, or `from_path_with_options` with `include_text_files=true` to
include default document extensions such as Markdown, YAML, TOML, and JSON.

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
line chunks with a default maximum of `50` lines and `5` overlapping lines.

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
construction and BM25-only search do not load tokenizers, read safetensors, or
call Hugging Face. `--no-download` prevents model downloads while allowing local
indexing. `--offline` also rejects remote Git sources.

## Sparse index

The BM25 index stores tokenized, enriched chunk documents. Enrichment adds code
metadata and symbol-like text around the raw chunk content so lexical search can
rank identifiers, file paths, and definitions effectively.

BM25 mode is useful when the query contains exact names, acronyms, function
names, file-local terms, or error strings.

## Dense index

The dense index stores normalized embedding vectors for every chunk. It is built
on first semantic, hybrid, or related-code use. Semantic mode embeds the query,
normalizes it, and ranks chunks by vector similarity.

Semantic mode is useful when the query describes behavior rather than exact
symbols, such as "where user sessions expire" or "how upload retries work."

## Hybrid ranking

Hybrid search runs both dense and sparse retrieval, over-fetches candidates,
normalizes candidate ranks with reciprocal rank fusion, and combines the scores.
It then applies query-aware boosts and reranks the top candidates.

The hybrid alpha value controls semantic weight. When callers don't provide an
alpha, SIFS resolves one from the query:

- Symbol-like queries use more BM25 weight.
- Mixed code phrases use a balanced hybrid weight.
- Natural-language and architecture questions keep more semantic weight.
- Explicit alpha values override the automatic selection.

Hybrid mode is the default because most developer queries contain both semantic
intent and exact code terms.

## Related-code lookup

Related-code lookup starts from a known chunk. SIFS semantically searches using
the chunk content as the query, filters to the same language when possible, and
removes the source chunk from the result set.

This makes `find_related` useful for finding alternate implementations, call
site patterns, duplicated logic, or similar modules.

## Daemon and MCP caching

The shared daemon stores `SifsIndex` instances in an in-memory cache for the
life of the daemon process. Local indexes are keyed by canonical path plus index
options. Git indexes are keyed by URL, optional ref, and index options. CLI
commands opportunistically use the daemon when its socket is available and fall
back to direct indexing when it is not.

The cache includes chunks, sparse data, optional semantic state, and lookup
maps. Restarting the daemon clears the live cache; persistent sparse and dense
caches still survive according to the selected cache mode.

The stdio MCP server can be installed without a pinned source. In that mode it
defaults to the server process working directory and still accepts explicit
`source` arguments for local paths or Git URLs. The recommended long-lived setup
on macOS is:

```bash
sifs daemon install-agent
sifs mcp install --client all
```

## Agent artifacts

The `sifs agent` command renders and manages target-specific integration
artifacts on top of the same search contract. Rendering lives in
`src/agent_artifacts.rs`, mutation safety lives in `src/agent_installer.rs`, and
readiness checks live in `src/agent_doctor.rs`.

The canonical artifact is a CLI-first `sifs-search` skill package under
`skills/sifs-search/`. Target mirrors under `extras/` are local package shapes
for agent-skill consumers such as OpenClaw and Hermes; they do not imply public
marketplace discovery.

Instruction snippets are inserted into `AGENTS.md` or `CLAUDE.md` with stable
managed markers and checksums. The installer preserves surrounding user content,
is idempotent on repeated runs, and requires `--force` before replacing a
user-modified managed block.

MCP remains optional for these artifacts. Generated instructions tell agents to
use MCP tools only when visible in the current session and to fall back to shell
commands such as `sifs search`, `sifs list-files`, `sifs get`, and
`sifs agent-context --json`.

## Persistent local indexes

Default local path indexing writes persistent cache entries under the platform
cache directory, such as `~/Library/Caches/sifs` on macOS. The CLI can opt into
a repository-local `.sifs/` cache with `--project-cache`. SIFS validates each
cache entry against the current sorted file signature list before loading it.

The sparse persistent cache stores:

- File signatures for cache validation.
- Chunks and line locations.
- The BM25 index.

Semantic-capable local indexes also write a separate dense cache keyed by the
encoder configuration. Sparse-only indexes never write dense cache files.

SIFS doesn't use the persistent sparse cache for custom extension sets, custom
ignore sets, document-file inclusion, or Git temporary checkouts. Those cases
build an index from source so option-specific behavior stays correct.

## Limitations

SIFS keeps live indexes in memory after construction. Persistent caches are best
effort: if a cache entry is missing or invalid, SIFS rebuilds from source and
writes a fresh entry when persistent caching is enabled.

Other current limits are:

- Files must be readable as UTF-8 text.
- Git indexing uses shallow clones.
- Direct CLI commands use platform caches by default and only write `.sifs/`
  when `--project-cache` is set.
- Document-like files require explicit library options.
