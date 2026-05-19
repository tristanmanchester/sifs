TL;DR: The real path forward is to stop treating ranking as a bag of boosts and make SIFS a proper multi-stage retrieval system: several cheap candidate generators, explicit per-field scores, then a small generic reranker. The biggest likely quality wins are exact symbol indexing, fielded BM25, metadata-aware embeddings, hierarchical file→chunk retrieval, and query-aware test/docs penalties. The biggest speed wins are avoiding per-query scans over all chunks, storing hits by chunk id instead of cloning chunks, precomputing path/symbol metadata, and making dense vectors more cache-friendly.

I statically inspected the latest zip. I still cannot run `cargo` in this sandbox, so I am basing this on the code and checked-in benchmark artefacts.

One caveat first: I would not trust all checked-in benchmark artefacts equally right now. `benchmarks/results/sifs-full.json` and `docs/benchmark-report.md` report the honest regenerated run: `NDCG@10 = 0.8188`, warm uncached query around `4.8 ms`, and cold semantic first-use around `465 ms`. But `benchmarks/results/sifs-mode-hybrid.json` reports `NDCG@10 = 0.8642` and much lower query latency, while lacking the newer reproducibility and semantic-first-use fields. I would treat that mode-ablation file as stale until regenerated with the same binary, same commit, `--no-cache`, and the same corpus.

The useful signal from the honest run is: symbol search is already strong, around `0.96 NDCG@10`; architecture queries are weakest, around `0.74`; C and TypeScript are the weakest language slices, around `0.68` and `0.71`. That gives us a sensible target: improve architecture/natural-language retrieval and the C/TypeScript analyzers, without damaging exact symbol lookup.

1. Build explicit candidate generators instead of post-hoc boosts.

Right now hybrid search does dense retrieval, BM25 retrieval, RRF fusion, then file/path/query boosts. The problem is that some of those boosts scan or manipulate the candidate set after retrieval. That is brittle: if the right chunk never enters the candidate pool, a boost cannot save it, unless you scan every chunk and inject candidates manually.

A cleaner design would be:

```text
content BM25 candidates
symbol-definition candidates
path/file-name candidates
dense chunk candidates
dense file-summary candidates
neighbour/adjacent candidates
        ↓
candidate union
        ↓
generic feature scoring or RRF fusion
        ↓
diversity/context-pack selection
```

This turns “clever boost rules” into proper retrieval channels. It also makes explanations honest: “this result appeared because the symbol index matched `FromRequest`, the path index matched `extract`, and dense search ranked it 14th.”

Concretely, I would introduce something like:

```rust
pub struct RetrievalIndex {
    content_bm25: FieldBm25,
    symbol_bm25: FieldBm25,
    path_bm25: FieldBm25,
    exact_symbols: HashMap<String, Vec<usize>>,
    path_terms: HashMap<String, Vec<usize>>,
    file_index: FileLevelIndex,
    dense_chunks: Option<DenseIndex>,
    dense_files: Option<DenseIndex>,
    chunk_meta: Vec<ChunkMeta>,
}
```

This is not benchmark hacking because every source is generic and useful on arbitrary repos. It also avoids the current situation where `src/sparse.rs` mixes content, symbol, breadcrumb, stem, and directory tokens into one BM25 field by duplicating tokens. Duplicating tokens works, but it hides the contribution of each signal and makes tuning much harder.

2. Add a real symbol/definition index.

This is probably the highest-return quality improvement.

The current symbol story is halfway there: chunks now store `symbols`, and BM25 indexes symbol tokens. But `boost_symbol_definitions()` still mostly relies on regex matching in candidate chunks, and for non-candidate chunks it only injects definitions when the file stem matches the queried symbol. That misses important cases like Rust traits in `mod.rs`, TypeScript exports from barrel files, Java classes in package folders, C functions in `foo.c` that are declared in `foo.h`, or C++ classes with macros between `class` and the name.

Make symbol definitions first-class postings:

```rust
pub struct SymbolPosting {
    pub chunk_id: usize,
    pub file_path: String,
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub visibility: Option<Visibility>,
    pub container: Option<String>,
}

pub struct SymbolIndex {
    exact: HashMap<String, Vec<SymbolPosting>>,
    folded: HashMap<String, Vec<SymbolPosting>>,
    ngrams: HashMap<String, Vec<usize>>,
}
```

Then a query like `FromRequest`, `Mutex`, `JsonObject`, `transformRequest`, or `uv_run` can retrieve definitions directly, not merely hope BM25 or dense retrieval pulled them into the top 90 candidates.

This would specifically help several failures in the checked-in result. For example, `FromRequest` in axum ranks badly; `Mutex` in abseil ranks fourth; several Java/PHP/TypeScript symbol-ish queries are close but not top. Those are exactly the cases where exact symbol postings should beat generic natural-language ranking.

3. Replace line-based symbol extraction with tree-sitter queries for the weak languages.

The current `src/chunker.rs` extractor is intentionally simple: it scans lines and looks for prefixes such as `fn`, `class`, `def`, `interface`, `const`, `let`, and so on. That is fine as a bootstrap, but it misses common real code.

For TypeScript, it should handle:

```typescript
export const useFoo = () => {}
const createRouter = function () {}
export { something } from './internal'
export default class Foo {}
type Foo = ...
interface Foo<T> ...
class Foo extends Bar {}
describe(...)
it(...)
beforeAll(...)
```

For C/C++, it should handle:

```c
CURLcode Curl_retry_request(...)
#define FOO(...)
typedef struct foo ...
struct foo { ... }
enum foo { ... }
static inline int foo(...)
```

For Java/PHP/C#, it should capture class methods, package/namespace context, visibility, annotations/attributes, and nested classes.

This is where tree-sitter earns its keep. Use language-specific query patterns rather than regexes. You do not need a full LSP. A 70–80% structural extractor would probably improve SIFS more than a dozen hand-written ranking boosts.

4. Add file-level retrieval for architecture queries.

The benchmark tells us architecture queries are the weak point. That makes sense: architecture answers are often not a single perfect chunk. They are spread across a file, neighbouring chunks, imports, class definitions, and comments.

I would add a file-level index where each file gets a compact “file card”:

```text
path: src/routing/mod.rs
language: rust
symbols: Router, Route, MethodRouter, nest, merge, layer
imports: tower::Layer, axum_core::...
comments/docstrings: ...
first 1–2 KB of file-level context
```

Search architecture queries against file cards first, then search chunks only within the top files. This often improves both accuracy and context efficiency. It also helps cases where the correct answer is in the right file but not the exact top chunk.

A good hybrid strategy would be:

```text
if query looks architectural:
    retrieve top 20 files
    retrieve top chunks inside those files
    add neighbouring chunks around strong hits
else:
    do normal chunk retrieval
```

This is not just for the benchmark. It matches how agents actually work: they need enough context to edit safely, not just one isolated snippet.

5. Make `pack` the flagship agent primitive, not a thin wrapper over search.

The current `pack` command is useful but still basic: search, deduplicate by file, truncate to budget. A proper agent-native context pack should use retrieval and selection differently from a search result list.

I would make `pack` do this:

```text
1. Retrieve broad candidates from content, symbol, path, file-level, and dense indexes.
2. Select diverse files, not just top chunks.
3. Include neighbouring chunks when a hit is inside a larger function/class.
4. Include file headers/imports when relevant.
5. Include symbol definitions referenced by selected chunks.
6. Return a reason for each included item.
```

The output should answer: “What should an agent read before editing this?” rather than “What are the top 10 chunks?”

A useful JSON shape would be:

```json
{
  "query": "how request validation is handled",
  "items": [
    {
      "kind": "primary_chunk",
      "file_path": "src/validation/mod.rs",
      "start_line": 42,
      "end_line": 118,
      "why": [
        "content_bm25_rank=2",
        "file_card_rank=1",
        "symbol_match=Validator"
      ],
      "neighbours_included": true,
      "content": "..."
    },
    {
      "kind": "supporting_definition",
      "file_path": "src/error.rs",
      "symbol": "ValidationError",
      "why": ["referenced by primary chunk"]
    }
  ]
}
```

That is likely a stronger differentiator than squeezing another 0.01 out of NDCG.

6. Make penalties query-aware, especially for tests, docs, examples, and `.d.ts`.

The current hybrid reranker penalises tests, examples, compatibility files, and `.d.ts` paths fairly aggressively. That is sensible for many production-code queries, but it hurts queries that explicitly ask about tests, snapshots, lifecycle hooks, test discovery, mocks, examples, or public type declarations.

The TypeScript failures strongly suggest this. Vitest queries such as “discovers and runs test files”, “describe it and test block locations”, “beforeAll afterAll lifecycle hooks”, “snapshot testing”, and “mock and spy utilities” should not receive the normal test/spec penalty. For those queries, test/spec files may be the answer.

I would change path penalties from fixed global multipliers into query-conditioned priors:

```rust
pub struct QueryIntent {
    wants_tests: bool,
    wants_docs: bool,
    wants_examples: bool,
    wants_public_api: bool,
    wants_types: bool,
    wants_source: bool,
}
```

Then:

```text
normal query:
    tests/specs/examples get a mild penalty

test-related query:
    tests/specs/runners/hooks/snapshot files get no penalty or a boost

type/API query:
    .d.ts, interface/type files, index.ts, public exports get no penalty

docs query:
    docs/examples/README files get no penalty
```

This is a real accuracy improvement because it models user intent, not benchmark phrases.

7. Add light stemming and identifier-aware query expansion.

`tokens.rs` splits identifiers well, but it does not really normalise natural-language morphology. That means queries like “transformers”, “connectors”, “fields”, “operations”, “serialisation”, “deserializing”, “middlewares”, or “handlers” may fail to match code tokens like `transform`, `connector`, `field`, `operation`, `serialize`, `deserialize`, `middleware`, `handler`.

This probably explains some near misses: the right file is nearby, but not quite top.

I would not stem everything destructively. Instead, add secondary tokens with lower field weight:

```text
original token: transformers
light stem: transformer
verb-ish stem: transform

original token: deserializing
light stem: deserialize / deserial
```

For code search, a small conservative normaliser is safer than full aggressive stemming. Start with plural stripping, `ing`/`ed` handling, British/American variants, and a few code-domain transforms:

```text
serialise ↔ serialize
deserialise ↔ deserialize
config ↔ configuration
auth ↔ authentication
ctx ↔ context
req ↔ request
resp ↔ response
```

The important bit is to tune these on held-out data and local feedback, not by adding benchmark-specific query rewrites.

8. Increase and instrument hybrid candidate pools.

In `search_hybrid`, `candidate_count` is currently `top_k * 9`. For `top_k = 10`, that is only 90 dense candidates and 90 BM25 candidates before fusion. That may be too narrow for architecture queries.

The cheap experiment is:

```text
candidate_count = max(top_k * 20, 200)
```

or adaptive:

```text
symbol query:        80–120 candidates
natural language:   200–500 candidates
architecture query: 500 candidates + file-level retrieval
```

This should have modest latency impact because dense search already scores every vector; increasing candidate count mainly changes top-k truncation and the downstream candidate union. BM25 may cost a little more, but the current BM25 path is very fast.

However, do not just increase it blindly and declare victory. Add diagnostic fields per task:

```json
{
  "target_bm25_rank": 184,
  "target_dense_rank": null,
  "target_symbol_rank": 1,
  "target_path_rank": 12,
  "target_in_candidate_union": true,
  "target_final_rank": 23
}
```

That splits failures into two types:

```text
candidate-generation failure: the answer never entered the pool
reranking failure: the answer entered but was pushed down
```

Those require different fixes.

9. Use a small generic reranker instead of more hand-tuned boosts.

Once you have explicit features, the ranking problem becomes much cleaner.

Candidate features could include:

```text
content_bm25_score
symbol_bm25_score
path_bm25_score
dense_chunk_score
dense_file_score
exact_symbol_match
symbol_kind_match
path_token_overlap
file_stem_overlap
same_file_candidate_count
chunk_length
is_test_file
is_docs_file
is_example_file
query_intent_flags
```

Start with a transparent linear model:

```text
score =
  w1 * content_bm25
+ w2 * symbol_bm25
+ w3 * path_bm25
+ w4 * dense_chunk
+ w5 * dense_file
+ w6 * exact_symbol_match
+ ...
```

Then tune weights against a development set, leaving a held-out test set untouched. Later, if you want, try LambdaMART or another lightweight learning-to-rank model. But even a linear model would be a big improvement over opaque boost stacking because you can inspect weights, ablate features, and prevent benchmark leakage.

The key guardrail: freeze public benchmark tasks as test-only. Use local feedback and a separate dev set for tuning.

10. Embed metadata, not just raw chunk content.

`encode_chunks_batched()` currently embeds `chunk.content` only. That means semantic retrieval cannot “see” the path, language, symbols, or breadcrumbs unless they appear in the chunk text itself.

A simple experiment:

```rust
fn semantic_text(chunk: &Chunk) -> String {
    format!(
        "path: {}\nlanguage: {}\nsymbols: {}\ncontext: {}\n\n{}",
        chunk.file_path,
        chunk.language.as_deref().unwrap_or("unknown"),
        chunk.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" "),
        chunk.breadcrumbs.join(" > "),
        chunk.content,
    )
}
```

Do not necessarily use that exact format permanently, but test variants:

```text
content only
path + content
path + symbols + content
path + symbols + imports/comments + content
file-card embedding + chunk embedding
```

This can improve natural-language queries like “queue connection resolution”, “request transformers”, “router creation”, or “test discovery” because path and symbol metadata often encode the concept better than the local code body.

11. Add multi-vector retrieval for chunks/files.

A single vector per chunk is a blunt representation. A chunk may contain imports, comments, several symbols, and implementation details. Mean pooling over all of that can blur the signal.

A pragmatic local design:

```text
chunk_content_vector
chunk_symbol_vector
chunk_path_vector
file_card_vector
```

At query time:

```text
symbol query:
    weight symbol/path vectors more

architecture query:
    weight file_card and content vectors more

exact natural-language query:
    weight content and file_card vectors
```

You can implement this without a full late-interaction model. Store multiple dense indexes or one dense index with vector kind metadata. Then fuse candidates.

This is a real accuracy direction, especially for architecture queries, because file-level semantics and chunk-level semantics often disagree.

12. Optimise the actual hot paths, not cached-repeat latency.

The `0.0036 ms` cached-repeat number is not meaningful for normal agent behaviour unless the agent literally repeats the same query. The metric to optimise is warm uncached latency, currently around `4.8 ms`, and first semantic/hybrid use, around `469 ms`.

I would profile with `sifs-benchmark --hybrid-timing` first, but the code already suggests several likely hot paths.

`boost_path_intent()` currently scans chunks and recomputes path terms during query-time boosting. Replace this with precomputed `ChunkMeta` and a path-term inverted index.

`boost_symbol_definitions()` scans candidate content with regex and only injects non-candidate definitions when the file stem matches. Replace this with the symbol index above.

`SearchResult` clones full `Chunk` values, including content. Store `chunk_id` internally and only materialise chunks at the CLI/MCP boundary. Query cache should store `Vec<SearchHit>`, not cloned chunks.

`DenseIndex::query()` computes every dot product and collects all scores into a vector before truncating. For small repos this is fine; for large monorepos, use a row-major `Vec<f32>` plus per-thread top-k heaps. That avoids allocating and sorting a full score vector every query.

The current BM25 implementation uses `HashMap<String, Vec<(usize, u32)>>` and a per-query `HashMap<usize, f32>` for scores. It is already fast, but for scale you can switch to term IDs, precomputed IDF, contiguous postings, and a generation-stamped score array.

13. Reduce semantic memory and first-use cost.

The React smoke result shows around `462 MB` peak RSS for about `21k` chunks. The dense vectors themselves should not account for most of that, so the memory is probably model + tokenizer + chunks + temporary embedding arrays + deserialisation overhead.

Good investigations:

```text
Store dense vectors as f16 or int8 in cache.
Memory-map semantic vectors instead of bincode-decoding the whole payload.
Share one encoder instance across indexes in the daemon.
Avoid cloning chunk content into cached query results.
Build semantic embeddings in parallel batches if tokenizer/model path allows it.
Prewarm semantic indexes in daemon/MCP mode.
```

For local code search, quantised vectors are worth testing. A 256-dim normalised embedding can often tolerate f16, and maybe int8, with little ranking loss. You should measure NDCG and latency, but this is a real systems improvement, not a benchmark trick.

14. Make indexing incremental.

Cold sparse indexing is already pretty good, but a real agent will repeatedly search the same repo while editing files. Rebuilding or revalidating the whole tree is avoidable.

A robust incremental design:

```text
per-file signature
per-file chunks
per-file symbol postings
per-file BM25 postings
per-file dense vectors
global term statistics
```

On file change:

```text
remove old file contributions
rechunk changed file
update symbol/path/content postings
re-embed only changed chunks
refresh file-card vector
```

This is more work than tuning the ranker, but it is the direction that makes SIFS feel genuinely live inside an agent loop.

15. Improve chunking for context usefulness.

The current code-aware chunker groups tree-sitter child nodes around a target size. That is sensible, but for search quality I would add a few refinements:

```text
Keep doc comments with the symbol they document.
Keep imports/package declarations available as file header context.
When splitting a large function/class, inject the enclosing symbol breadcrumb into every child chunk.
Create separate symbol-definition micro-chunks for exact lookup.
Create file-summary chunks for architecture search.
Use neighbouring chunks in pack/context mode, not necessarily in search mode.
```

This matters because NDCG on a chunk benchmark can hide the real question: did the agent receive enough context to edit safely? Better chunking plus pack selection can improve actual agent performance even if top-10 NDCG barely moves.

16. Use the benchmark as a diagnostic, not the objective.

Given the cheating episode, I would add a small discipline layer.

Regenerate all benchmark outputs from one command and one commit. Delete stale mode files or mark them as historical. Split tasks into dev and held-out test sets. Use the dev set for weight tuning; use the held-out set only for release claims. Add a CI check that production ranking files do not contain benchmark repo names or query-specific phrases. Keep per-query diagnostics so you can see whether improvements affect candidate generation, reranking, or context packing.

The evaluation I would want after every experiment is:

```text
NDCG@10 overall
NDCG@10 by language
NDCG@10 by category
hit@1 / hit@5 / hit@10
candidate recall@200 by source
warm uncached p50/p90
first semantic/hybrid p50/p90
context recall at 500/1000/2000/4000 tokens
memory peak
```

The candidate-recall metric is especially important. Without it, you cannot tell whether to improve BM25/dense/symbol retrieval or the final reranker.

My recommended investigation order would be:

First, add candidate diagnostics and regenerate all benchmark artefacts consistently. This prevents another round of accidental self-deception.

Second, implement a symbol-definition index and query-aware test/docs/type penalties. Those are relatively contained and should directly address visible failures.

Third, prototype fielded BM25 plus path/symbol candidate generators. This replaces several fragile boosts with transparent retrieval sources.

Fourth, add file-level retrieval and improve `pack`. That attacks the weakest category: architecture queries.

Fifth, optimise hot paths after profiling: precomputed metadata, no chunk cloning in internal hits, denser vector storage, and faster dense top-k.

That is the path I’d trust: less “make the benchmark green”, more “make the retrieval stack explainable, composable, and measurable.”
