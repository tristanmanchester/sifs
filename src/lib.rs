pub mod agent_artifacts;
pub mod agent_context;
pub mod agent_doctor;
pub mod agent_installer;
pub mod chunker;
pub mod daemon;
pub mod dense;
pub mod feedback;
pub mod file_walker;
pub mod index;
pub mod mcp;
pub mod metrics;
pub mod model2vec;
pub mod profiles;
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
    Encoder, EncoderSpec, HashingEncoder, ModelLoadPolicy, ModelOptions, ModelStatus,
    encoder_fingerprint, load_encoder, load_model, load_model_with_options, model_fingerprint,
    model_status,
};
pub use types::{
    CacheMode, Chunk, IndexStats, IndexWarning, SearchMode, SearchOptions, SearchResult,
};
pub use utils::{format_results, is_git_url, resolve_chunk};
