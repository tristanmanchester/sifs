pub mod chunker;
pub mod dense;
pub mod file_walker;
pub mod index;
pub mod mcp;
pub mod metrics;
pub mod model2vec;
pub mod ranking;
pub mod search;
pub mod sparse;
pub mod tokens;
pub mod types;
pub mod utils;

pub use index::{
    CacheConfig, CacheSummary, IndexOptions, SifsIndex, cache_summary, platform_cache_root,
};
pub use model2vec::{
    Encoder, HashingEncoder, ModelLoadPolicy, ModelOptions, ModelStatus, load_model,
    load_model_with_options, model_fingerprint, model_status,
};
pub use types::{Chunk, IndexStats, SearchMode, SearchOptions, SearchResult};
pub use utils::{format_results, is_git_url, resolve_chunk};
