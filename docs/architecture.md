# Architecture

SIFS is a single-process Rust search engine for code repositories. The pipeline
walks supported files, builds syntax-aware chunks when possible, builds a BM25
index, and lazily attaches semantic model state only when dense or hybrid search
needs it.

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
7. Lazily load the Model2Vec model and embed chunks when semantic, hybrid, or
   related-code search first needs dense vectors.

## File walking

The file walker selects files by extension, skips common generated directories,
and respects the root `.gitignore` file. It sorts paths before returning them so
index construction is deterministic for the same filesystem state.

Default ignored directories are:

- `.git`, `.hg`, and `.svn`
- `__pycache__`, `.mypy_cache`, `.pytest_cache`, and `.ruff_cache`
- `node_modules`, `.venv`, `venv`, `.tox`, and `.eggs`
- `.cache`, `.sifs`, `dist`, and `build`

By default, the public `from_path` constructor indexes code extensions only.
Use `from_path_with_options` with `include_text_files=true` to include default
document extensions such as Markdown, YAML, TOML, and JSON.

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

SIFS loads a Model2Vec-compatible model through `src/model2vec.rs`. The default
model is `minishlab/potion-code-16M`, and callers can pass a custom model path
through `from_path_with_options`, `from_path_with_model_options`,
`sifs search --model`, or `sifs-embed --model`.

The loader reads tokenizer and tensor files directly. It supports embedding
matrices, optional weights, optional token mappings, truncation settings, and
normalization metadata. Query and chunk embeddings stay in process after the
model loads.

Model loading is lazy. BM25-only construction and BM25-only search do not load
tokenizers, read safetensors, or call Hugging Face. `--no-download` prevents
model downloads while allowing local indexing. `--offline` also rejects remote
Git sources.

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
- Natural-language queries keep more semantic weight.
- Explicit alpha values override the automatic selection.

Hybrid mode is the default because most developer queries contain both semantic
intent and exact code terms.

## Related-code lookup

Related-code lookup starts from a known chunk. SIFS semantically searches using
the chunk content as the query, filters to the same language when possible, and
removes the source chunk from the result set.

This makes `find_related` useful for finding alternate implementations, call
site patterns, duplicated logic, or similar modules.

## MCP caching

The MCP server stores `SifsIndex` instances in an in-memory cache for the life
of the server process. Local indexes are keyed by canonical path. Git indexes
are keyed by URL plus optional ref.

The cache includes chunks, sparse data, optional semantic state, and lookup
maps. Restarting the server clears the cache.

## Persistent local indexes

Default indexing writes persistent cache entries under the platform cache
directory, not inside the searched repository. On macOS this is
`~/Library/Caches/sifs`; on Linux it is `${XDG_CACHE_HOME:-~/.cache}/sifs`.
Project-local `.sifs/` caching is available only when explicitly requested.
SIFS validates persistent caches against the current sorted file signature list
before loading them.

The sparse persistent cache stores:

- File signatures for cache validation.
- Chunks and line locations.
- The BM25 index.

Cache entries are keyed by source identity, indexing options, file signatures,
and cache/chunker version. Semantic cache files add the model name plus a
fingerprint of the resolved tokenizer, safetensors, and config files, so dense
vectors are not reused after a model changes.

## Limitations

SIFS keeps live indexes in memory after construction. Persistent caches are best
effort: if a cache entry is missing or invalid, SIFS rebuilds from source and
writes a fresh entry when persistent caching is enabled.

Other current limits are:

- Files must be readable as UTF-8 text.
- Only the root `.gitignore` file is loaded.
- Git indexing uses shallow clones.
- Document-like files require explicit library options.
