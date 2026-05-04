use crate::SifsIndex;
use crate::daemon::protocol::{
    CachedIndexStatus, IndexIdentity, IndexRuntimeOptions, IndexStatusResult, SourceKind,
    SourceSpec,
};
use crate::index::{CacheConfig, IndexOptions};
use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

pub struct IndexManager {
    indexes: HashMap<String, CachedIndex>,
}

struct CachedIndex {
    identity: IndexIdentity,
    index: SifsIndex,
    last_used: SystemTime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexManagerStatus {
    pub indexes: Vec<CachedIndexStatus>,
}

impl IndexManager {
    pub fn new() -> Self {
        Self {
            indexes: HashMap::new(),
        }
    }

    pub fn get(&mut self, source: SourceSpec, options: IndexRuntimeOptions) -> Result<&SifsIndex> {
        let identity = IndexIdentity::new(source, &options);
        let key = identity.key();
        if !self.indexes.contains_key(&key) {
            let index = build_index(&identity.source, &options)?;
            self.indexes.insert(
                key.clone(),
                CachedIndex {
                    identity,
                    index,
                    last_used: SystemTime::now(),
                },
            );
        }
        let cached = self.indexes.get_mut(&key).expect("index inserted above");
        cached.last_used = SystemTime::now();
        Ok(&cached.index)
    }

    pub fn refresh(
        &mut self,
        source: SourceSpec,
        options: IndexRuntimeOptions,
    ) -> Result<&SifsIndex> {
        let identity = IndexIdentity::new(source, &options);
        let key = identity.key();
        let index = build_index(&identity.source, &options)?;
        self.indexes.insert(
            key.clone(),
            CachedIndex {
                identity,
                index,
                last_used: SystemTime::now(),
            },
        );
        Ok(&self.indexes.get(&key).expect("index inserted above").index)
    }

    pub fn clear(&mut self, source: SourceSpec, options: IndexRuntimeOptions) -> bool {
        let key = IndexIdentity::new(source, &options).key();
        self.indexes.remove(&key).is_some()
    }

    pub fn status(&self) -> IndexManagerStatus {
        let mut indexes: Vec<_> = self
            .indexes
            .values()
            .map(|cached| CachedIndexStatus {
                source: cached.identity.source.clone(),
                stats: cached.index.stats(),
                semantic_loaded: cached.index.semantic_loaded(),
            })
            .collect();
        indexes.sort_by_key(|status| status.source.display());
        IndexManagerStatus { indexes }
    }

    pub fn prune_idle(&mut self, max_idle: Duration) -> usize {
        let now = SystemTime::now();
        let before = self.indexes.len();
        self.indexes.retain(|_, cached| {
            now.duration_since(cached.last_used)
                .map(|idle| idle <= max_idle)
                .unwrap_or(true)
        });
        before - self.indexes.len()
    }

    pub fn index_status(index: &SifsIndex, source: SourceSpec) -> IndexStatusResult {
        IndexStatusResult {
            source,
            stats: index.stats(),
            semantic_loaded: index.semantic_loaded(),
            warnings: index.warnings().to_vec(),
        }
    }
}

impl Default for IndexManager {
    fn default() -> Self {
        Self::new()
    }
}

fn build_index(source: &SourceSpec, options: &IndexRuntimeOptions) -> Result<SifsIndex> {
    let index_options = IndexOptions::sparse()
        .with_cache(CacheConfig::from(options.cache.clone()))
        .with_extensions(options.extensions_set())
        .with_ignore(options.ignore_set())
        .with_include_text_files(options.include_text_files);
    let index_options = match options.encoder.clone().into_encoder_spec() {
        Some(encoder) => index_options.with_encoder_spec(encoder),
        None => index_options,
    };

    match source.kind {
        SourceKind::LocalPath => {
            SifsIndex::from_path_with_index_options(&source.source, index_options)
        }
        SourceKind::GitUrl => SifsIndex::from_git_with_index_options(
            &source.source,
            source.ref_name.as_deref(),
            index_options,
        ),
    }
}
