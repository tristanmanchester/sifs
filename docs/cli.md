# Command-line usage

SIFS includes a command-line interface for one-shot searches, related-code
lookups, agent-file generation, and MCP server startup. Build the release
binary before running these examples from the repository root.

```bash
cargo build --release
```

## Search command

The `search` command builds an index for the target path or Git URL, runs a
query, and prints ranked chunks with file locations and scores. The default path
is the current directory, the default result count is `5`, and the default mode
is `hybrid`.

```bash
target/release/sifs search <query> [path] [--top-k <count>] [--mode <mode>] [--language <language>] [--path <file>] [--json|--jsonl|--format <format>] [--encoder <encoder>] [--model <model>] [--offline] [--no-download] [--cache-dir <path>] [--no-cache] [--project-cache]
```

Use `hybrid` for general code search, `semantic` for meaning-heavy queries, and
`bm25` for exact identifiers or file-local terminology.

```bash
target/release/sifs search "where is the login redirect handled" .
target/release/sifs search "SessionToken" /path/to/project --mode bm25 -k 10
target/release/sifs search "stream upload backpressure" https://github.com/owner/project
target/release/sifs search "token validation" . --mode bm25 --language rust --path src/auth.rs --json
```

The mode values are:

- `hybrid`: Combine semantic and BM25 rankings with query-aware reranking.
- `semantic`: Rank by embedding similarity only.
- `bm25`: Rank by sparse lexical matching only. This mode is model-free and
  never loads tokenizers, safetensors, or Hugging Face model files.

Semantic and hybrid search default to `--encoder model2vec`. Use
`--encoder hashing` for a model-free dense encoder that is useful for smoke
tests and fully local experiments.

Use `--offline` to prevent all network access by SIFS, including remote Git
clones and model downloads. Use `--no-download` to prevent model downloads while
still allowing local path indexing and remote Git sources.

Use repeatable `--language` and `--path` filters to narrow searches to exact
language labels or repository-relative file paths. Use `--context-lines` to ask
for surrounding source lines in structured output when local files are
available. Use `--explain` to print query, ranking, filter, timing, and warning
metadata before human-readable results.

Persistent index caches use platform cache directories by default:
`~/Library/Caches/sifs` on macOS and `${XDG_CACHE_HOME:-~/.cache}/sifs` on
Linux. Use `--cache-dir` to choose another cache root, `--no-cache` to disable
persistent caches, or `--project-cache` to opt into a repository-local `.sifs/`
cache.

## Find-related command

The `find-related` command resolves a file and line into the indexed chunk that
contains that line. It then searches for semantically related chunks in the same
language when language metadata is available.

```bash
target/release/sifs find-related <file-path> <line> [path] [--top-k <count>] [--json|--jsonl|--format <format>] [--encoder <encoder>] [--model <model>] [--offline] [--no-download] [--cache-dir <path>] [--no-cache] [--project-cache]
```

Pass the file path as it appears in search results or as a path relative to the
indexed repository root.

```bash
target/release/sifs find-related src/auth/session.rs 42 /path/to/project -k 8
target/release/sifs find-related src/auth/session.rs 42 /path/to/project --json
```

If SIFS can't resolve the location, it exits with an error that names the file
and line. Check that the file extension is indexed and that the line falls
inside a non-empty chunk.

## Model command

Semantic, hybrid, `find-related`, and `sifs-embed` need a Model2Vec-compatible
embedding model. BM25 search does not.

By default SIFS uses `SIFS_MODEL` when set, otherwise
`minishlab/potion-code-16M`. Use `--model` to override that per command.

```bash
target/release/sifs model status
target/release/sifs model status --json
target/release/sifs model pull
target/release/sifs model pull --model minishlab/potion-code-16M
target/release/sifs model fetch --model minishlab/potion-code-16M
```

`model status` checks local files only and never downloads. `model pull`
downloads or validates the model through the normal Hugging Face cache.
`model fetch` is an alias for `model pull`.

## Doctor command

The `doctor` command prints local readiness for a path, cache directory, and
semantic encoder. It is a broader diagnostic than `model status`.

```bash
target/release/sifs doctor [path] [--encoder <encoder>] [--model <model>] [--offline] [--no-download]
```

Use it before offline semantic or hybrid search to confirm whether model files
are already local.

## Cache command

SIFS stores persistent sparse and semantic index caches outside searched
repositories by default. Use `cache status` to inspect the cache and
`cache clean` to remove it.

```bash
target/release/sifs cache status
target/release/sifs cache status --json
target/release/sifs cache clean --dry-run
target/release/sifs cache clean
target/release/sifs cache clean --cache-dir /tmp/sifs-cache
```

`cache clean` removes the whole SIFS platform cache by default. It does not
search for or remove project-local `.sifs/` directories unless you explicitly
point `--cache-dir` at one.

## Init command

The `init` command writes a ready-to-use agent description for clients that
support local agent files. It creates `.claude/agents/sifs-search.md`.

```bash
target/release/sifs init
```

Use `--force` to overwrite an existing file at that path.

```bash
target/release/sifs init --force
```

## Capabilities command

The `capabilities` command prints the main CLI and MCP capabilities in one
place. Use it when onboarding a user or checking what an agent can discover
without opening the full documentation.

```bash
target/release/sifs capabilities
```

## MCP server mode

Running `sifs` without a subcommand prints help. Use the explicit `mcp`
subcommand to start the MCP stdio server. You can pass a local path or Git URL
as the default source. The optional `--ref` value selects a branch or tag when
the default source is a Git URL.

```bash
target/release/sifs mcp [path-or-git-url] [--ref <branch-or-tag>] [--model <model>] [--offline] [--no-download] [--cache-dir <path>] [--no-cache] [--project-cache]
```

These examples start MCP server mode with different default sources.

```bash
target/release/sifs mcp /path/to/project
target/release/sifs mcp https://github.com/owner/project --ref main
```

When you provide a default source, MCP tool calls can omit the `repo` argument.
When you don't provide one, every `search` and `find_related` call must include
a local path or Git URL in `repo`.

## Output format

The default CLI output is human-readable text. Each search result includes the
chunk location, ranking score, source mode, and code content. Use
`--format compact` when you want one concise text line per result.

Use `--json` for one pretty JSON object or `--jsonl` for newline-delimited JSON
records. Search JSON includes query, mode, source, filters, index stats,
elapsed timing, warnings, and result rows with file path, line range, language,
score, source mode, and content.

```bash
target/release/sifs search "auth flow" . --json
target/release/sifs search "auth flow" . --jsonl
target/release/sifs search "auth flow" . --format compact
```

## Operational notes

SIFS indexes on demand for each direct CLI invocation. For repeated searches
against the same repository, use MCP server mode or call the Rust library from a
long-lived process so the index stays in memory.

Git URL indexing uses a shallow clone into a temporary directory. Local path
indexing canonicalizes the path and respects the root `.gitignore` file.

BM25 mode is the safest network-free smoke path for package managers:

```bash
target/release/sifs search "SessionToken" /path/to/project --mode bm25 --offline
```

## Next steps

Read [MCP server usage](mcp.md) to wire SIFS into an agent client, or read
[Benchmarking](benchmarks.md) to measure latency and search quality.
