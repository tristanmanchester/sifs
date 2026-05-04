# Rust library usage

The `sifs` crate exposes the indexing and search engine used by the CLI and MCP
server. Use the library when you want structured results, long-lived indexes,
custom filters, or direct integration inside a Rust application.

## Public API

The crate re-exports the main index and result types from `src/lib.rs`. These
types are the stable surface to use from downstream Rust code.

```rust
use sifs::{Chunk, EncoderSpec, IndexStats, SearchMode, SearchOptions, SearchResult, SifsIndex};
```

The core types are:

- `SifsIndex`: Owns chunks, BM25 data, lazy semantic state, and lookup maps.
- `Chunk`: Stores content, file path, line range, and optional language.
- `SearchMode`: Selects `Hybrid`, `Semantic`, or `Bm25` ranking.
- `SearchResult`: Returns a `Chunk`, score, and source mode.
- `IndexStats`: Reports indexed file count, chunk count, and language counts.

## Index a local path

Use `SifsIndex::from_path` for the default semantic-capable local indexing
behavior. It walks supported code files, chunks them, and builds a sparse BM25
index. It does not load the embedding model until semantic, hybrid, or
related-code search needs the dense index.

```rust
use sifs::{SearchMode, SearchOptions, SifsIndex};

fn main() -> anyhow::Result<()> {
    let index = SifsIndex::from_path("/path/to/project")?;
    let results = index.search_with(
        "where is authentication handled",
        &SearchOptions::new(5).with_mode(SearchMode::Hybrid),
    )?;

    for result in results {
        println!("{} {}", result.chunk.location(), result.score);
    }

    Ok(())
}
```

`from_path` returns an error when the path doesn't exist, isn't a directory, or
contains no supported non-empty files. Model-loading errors are returned later
from semantic or hybrid search.

Use `SifsIndex::from_path_sparse` when you want an explicitly sparse-only index
that can never initialize semantic state. BM25 search works normally; semantic,
hybrid, and related-code search return an error telling callers to build a
hybrid index or use `SearchMode::Bm25`.

```rust
use sifs::{SearchMode, SearchOptions, SifsIndex};

let index = SifsIndex::from_path_sparse("/path/to/project")?;
let results = index.search_with(
    "SessionToken",
    &SearchOptions::new(10).with_mode(SearchMode::Bm25),
)?;
```

Use `SifsIndex::from_path_hybrid` when you want the default lazy semantic
capability with explicit model policy.

```rust
use sifs::{ModelLoadPolicy, ModelOptions, SifsIndex};

let index = SifsIndex::from_path_hybrid(
    "/path/to/project",
    ModelOptions::new(None, ModelLoadPolicy::NoDownload),
)?;
```

## Customize indexing

Use `SifsIndex::from_path_with_options` when you need a custom model path,
extension set, ignored directory names, or document file inclusion. Use
`SifsIndex::from_path_with_model_options` when you also need explicit model
download policy. The extension set must use leading-dot values such as `.rs` or
`.ts`.

```rust
use sifs::SifsIndex;
use std::collections::HashSet;

fn main() -> anyhow::Result<()> {
    let extensions = HashSet::from([".rs".to_owned(), ".toml".to_owned()]);
    let ignore = HashSet::from(["fixtures".to_owned()]);

    let index = SifsIndex::from_path_with_options(
        "/path/to/project",
        None,
        Some(extensions),
        Some(ignore),
        true,
    )?;

    println!("{:?}", index.stats());
    Ok(())
}
```

```rust
use sifs::{ModelLoadPolicy, ModelOptions, SifsIndex};

let index = SifsIndex::from_path_with_model_options(
    "/path/to/project",
    ModelOptions::new(None, ModelLoadPolicy::NoDownload),
    None,
    None,
    false,
)?;
```

Use `SifsIndex::from_path_with_encoder_spec` for non-Model2Vec encoders such
as the built-in hashing encoder.

```rust
use sifs::{EncoderSpec, SifsIndex};

let index = SifsIndex::from_path_with_encoder_spec(
    "/path/to/project",
    EncoderSpec::hashing(),
    None,
    None,
    false,
)?;
```

The `include_text_files` flag controls whether default document-like extensions
such as Markdown, YAML, TOML, and JSON are included when you don't pass an
explicit extension set.

## Index a Git repository

Use `SifsIndex::from_git` to clone and index a remote repository. SIFS performs
a shallow clone into a temporary directory and can check out a branch or tag.

```rust
use sifs::SifsIndex;

fn main() -> anyhow::Result<()> {
    let index = SifsIndex::from_git("https://github.com/owner/project", Some("main"))?;
    println!("{:?}", index.stats());
    Ok(())
}
```

The Git command must be available on `PATH`. Clone failures return an error
that includes the Git stderr output.

## Build from existing chunks

Use `SifsIndex::from_chunks` when your application owns file discovery or
chunking. You must provide a loaded encoder and a non-empty list of chunks.

```rust
use sifs::{Chunk, SifsIndex};
use sifs::model2vec::load_model;

fn main() -> anyhow::Result<()> {
    let model = load_model(None)?;
    let chunks = vec![Chunk {
        content: "fn authenticate() {}".to_owned(),
        file_path: "src/auth.rs".to_owned(),
        start_line: 1,
        end_line: 1,
        language: Some("rust".to_owned()),
    }];

    let index = SifsIndex::from_chunks(model, chunks)?;
    println!("{:?}", index.stats());
    Ok(())
}
```

`from_chunks` preserves compatibility for callers that already have an encoder:
it builds BM25 data and preloads semantic state. Use `from_chunks_sparse` for a
sparse-only chunk index, `from_chunks_hybrid` for a lazy Model2Vec-backed
semantic-capable index, or `from_chunks_with_encoder_spec` for hashing.

## Search an index

Use `SifsIndex::search_with` for all ranking modes. It returns
`Result<Vec<SearchResult>>` because semantic and hybrid search may need to load
or download a model. BM25 mode does not touch the model path. `SearchOptions`
keeps ranking, result count, hybrid alpha, and filters self-describing. The
`alpha` field is only used by hybrid search. When `alpha` is `None`, SIFS
selects a weight from the query shape.

```rust
let results = index.search_with(
    "parse oauth callback",
    &SearchOptions::new(10).with_mode(SearchMode::Hybrid),
)?;
```

Use language or path filters to search a subset of the index. Filters are exact
matches against chunk language strings and repository-relative file paths.

```rust
let results = index.search_with(
    "session expiry",
    &SearchOptions::new(5)
        .with_mode(SearchMode::Hybrid)
        .with_alpha(0.5)
        .with_languages(["rust".to_owned()])
        .with_paths(["src/auth.rs".to_owned()]),
)?;
```

If both filters are present, SIFS searches chunks that match both filter sets.
If no filter matches any chunk, SIFS falls back to searching the full index.

Disable the in-process query-result cache when measuring uncached warm query
latency or when a caller wants every request to execute ranking work.

```rust
let results = index.search_with(
    "parse oauth callback",
    &SearchOptions::new(10)
        .with_mode(SearchMode::Hybrid)
        .with_cache(false),
)?;
```

## Find related chunks

Use `find_related` when you already have a `Chunk` and want nearby concepts or
similar implementations. The method performs semantic search with a same
language filter when the source chunk has language metadata.

```rust
let source = &index.chunks[0];
let related = index.find_related(source, 5)?;
```

The source chunk itself is removed from the returned results.

## Get index statistics

Use `stats` to inspect index size and language coverage. This is useful for
debugging file selection and benchmark output.

```rust
let stats = index.stats();
println!("{} files, {} chunks", stats.indexed_files, stats.total_chunks);
```

The `languages` map stores chunk counts by language, not file counts.
