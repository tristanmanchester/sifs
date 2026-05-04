# Rust library usage

The `sifs` crate exposes the same indexing and search engine used by the CLI
and MCP server. Use the library when you want structured results, long-lived
indexes, custom filters, or direct integration inside a Rust application.

## Public API

The crate re-exports the main index and result types from `src/lib.rs`. These
types are the stable surface to use from downstream Rust code.

```rust
use sifs::{Chunk, IndexStats, SearchMode, SearchOptions, SearchResult, SifsIndex};
```

The core types are:

- `SifsIndex`: Owns chunks, BM25 data, embeddings, and lookup maps.
- `Chunk`: Stores content, file path, line range, and optional language.
- `SearchMode`: Selects `Hybrid`, `Semantic`, or `Bm25` ranking.
- `SearchResult`: Returns a `Chunk`, score, and source mode.
- `IndexStats`: Reports indexed file count, chunk count, and language counts.

## Index a local path

Use `SifsIndex::from_path` for the default local indexing behavior. It loads the
default embedding model, walks supported code files, chunks them, and builds
both sparse and dense indexes.

```rust
use sifs::{SearchMode, SearchOptions, SifsIndex};

fn main() -> anyhow::Result<()> {
    let index = SifsIndex::from_path("/path/to/project")?;
    let results = index.search_with(
        "where is authentication handled",
        &SearchOptions::new(5).with_mode(SearchMode::Hybrid),
    );

    for result in results {
        println!("{} {}", result.chunk.location(), result.score);
    }

    Ok(())
}
```

`from_path` returns an error when the path doesn't exist, isn't a directory, or
contains no supported non-empty files.

## Customize indexing

Use `SifsIndex::from_path_with_options` when you need a custom model path,
extension set, ignored directory names, or document-file inclusion. The
extension set must use leading-dot values such as `.rs` or `.ts`.

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
chunking. You must provide a loaded encoder and a non-empty chunk list.

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

`from_chunks` builds BM25 documents from enriched chunk text and embeds the raw
chunk content for dense search.

## Search an index

Use `SifsIndex::search_with` for all ranking modes. `SearchOptions` keeps
ranking, result count, hybrid alpha, and filters self-describing. The `alpha`
field is only used by hybrid search. When `alpha` is `None`, SIFS selects a
weight from the query shape.

```rust
let results = index.search_with(
    "parse oauth callback",
    &SearchOptions::new(10).with_mode(SearchMode::Hybrid),
);
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
);
```

If both filters are present, SIFS searches chunks that match either filter set.
If no filter matches any chunk, SIFS falls back to searching the full index.

## Find related chunks

Use `find_related` when you already have a `Chunk` and want nearby concepts or
similar implementations. The method performs semantic search with a same-language
filter when the source chunk has language metadata.

```rust
let source = &index.chunks[0];
let related = index.find_related(source, 5);
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

## Next steps

Read [Architecture](architecture.md) for the indexing pipeline, or read
[Command-line usage](cli.md) when you need equivalent behavior from a shell.
