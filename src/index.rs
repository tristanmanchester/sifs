use crate::chunker::chunk_source;
use crate::dense::DenseIndex;
use crate::file_walker::{filter_extensions, language_for_path, walk_files};
use crate::model2vec::{Encoder, EncoderSpec, ModelOptions, encoder_fingerprint, load_encoder};
use crate::search::{search_bm25, search_hybrid, search_semantic};
use crate::sparse::Bm25Index;
use crate::symbol_index::{self, FileOutline, SymbolIndex, SymbolPosting};
use crate::types::{
    Chunk, IndexStats, IndexWarning, SearchHit, SearchMode, SearchOptions, SearchResult,
};
use anyhow::{Context, Result, bail};
use ndarray::{Array2, s};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

pub struct SifsIndex {
    bm25_index: Bm25Index,
    semantic_state: Mutex<Option<SemanticState>>,
    semantic_config: Option<EncoderSpec>,
    pub chunks: Vec<Chunk>,
    file_mapping: HashMap<String, Vec<usize>>,
    language_mapping: HashMap<String, Vec<usize>>,
    symbol_index: SymbolIndex,
    search_cache: Mutex<HashMap<SearchCacheKey, Vec<SearchHit>>>,
    cache_entry: Option<CacheEntry>,
    signatures: Option<Vec<FileSignature>>,
    signature_context: Option<SourceSignatureContext>,
    cache_context: Option<CacheContext>,
    warnings: Vec<IndexWarning>,
}

#[derive(Clone, Debug)]
pub struct StructuralIndex {
    files: Vec<String>,
    stats: IndexStats,
    symbol_index: SymbolIndex,
    outlines: HashMap<String, FileOutline>,
    warnings: Vec<IndexWarning>,
}

#[derive(Clone, Debug)]
pub struct NavigationIndex {
    files: Vec<String>,
    stats: IndexStats,
    symbol_index: SymbolIndex,
    warnings: Vec<IndexWarning>,
}

#[derive(Clone, Debug)]
pub struct NavigationSymbolLookup {
    pub stats: IndexStats,
    pub warnings: Vec<IndexWarning>,
    pub total: usize,
    pub postings: Vec<SymbolPosting>,
}

#[derive(Clone, Debug)]
pub struct NavigationFileOutline {
    pub stats: IndexStats,
    pub warnings: Vec<IndexWarning>,
    pub outline: FileOutline,
}

struct SemanticState {
    model: Box<dyn Encoder>,
    index: DenseIndex,
}

const EMBED_BATCH_SIZE: usize = 1024;
const CACHE_VERSION: u32 = 6;
const CACHE_DIR: &str = ".sifs";
const PLATFORM_CACHE_DIR: &str = "sifs";
const SPARSE_CACHE_FILE: &str = "index-v6-sparse.bin";
const STRUCTURAL_CACHE_FILE: &str = "index-v6-structural.bin";
const NAVIGATION_CACHE_FILE: &str = "index-v6-navigation.bin";
const NAVIGATION_MANIFEST_FILE: &str = "index-v6-navigation-manifest.bin";
const NAVIGATION_SYMBOL_DIR: &str = "index-v6-navigation-symbols";
const NAVIGATION_OUTLINE_DIR: &str = "index-v6-navigation-outlines";
const SEMANTIC_CACHE_PREFIX: &str = "semantic-v6";
const DEFAULT_QUERY_CACHE_ENTRIES: usize = 256;

type IndexMappings = (HashMap<String, Vec<usize>>, HashMap<String, Vec<usize>>);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CacheConfig {
    #[default]
    Platform,
    Project,
    Custom(PathBuf),
    Disabled,
}

#[derive(Clone, Debug)]
pub struct IndexOptions {
    pub semantic_config: Option<EncoderSpec>,
    pub extensions: Option<HashSet<String>>,
    pub ignore: Option<HashSet<String>>,
    pub include_text_files: bool,
    pub cache: CacheConfig,
    source_id_override: Option<String>,
}

impl IndexOptions {
    pub fn new(model_options: ModelOptions) -> Self {
        Self {
            semantic_config: Some(EncoderSpec::Model2Vec(model_options)),
            extensions: None,
            ignore: None,
            include_text_files: false,
            cache: CacheConfig::default(),
            source_id_override: None,
        }
    }

    pub fn sparse() -> Self {
        Self {
            semantic_config: None,
            extensions: None,
            ignore: None,
            include_text_files: false,
            cache: CacheConfig::default(),
            source_id_override: None,
        }
    }

    pub fn with_encoder_spec(mut self, encoder_spec: EncoderSpec) -> Self {
        self.semantic_config = Some(encoder_spec);
        self
    }

    pub fn with_extensions(mut self, extensions: Option<HashSet<String>>) -> Self {
        self.extensions = extensions;
        self
    }

    pub fn with_ignore(mut self, ignore: Option<HashSet<String>>) -> Self {
        self.ignore = ignore;
        self
    }

    pub fn with_include_text_files(mut self, include_text_files: bool) -> Self {
        self.include_text_files = include_text_files;
        self
    }

    pub fn with_cache(mut self, cache: CacheConfig) -> Self {
        self.cache = cache;
        self
    }

    fn with_source_id(mut self, source_id: String) -> Self {
        self.source_id_override = Some(source_id);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CacheContext {
    source_id: String,
    options_id: String,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    root: PathBuf,
}

#[derive(Clone, Debug)]
struct SourceSignatureContext {
    root: PathBuf,
    extensions: Option<HashSet<String>>,
    ignore: Option<HashSet<String>>,
    include_text_files: bool,
}

impl CacheContext {
    fn for_source(
        source_id: String,
        _root: &Path,
        extensions: &Option<HashSet<String>>,
        ignore: &Option<HashSet<String>>,
        include_text_files: bool,
    ) -> Self {
        let options_id = cache_hash(&(
            "index-options-v1",
            normalized_set(extensions),
            normalized_set(ignore),
            include_text_files,
            CACHE_VERSION,
        ));
        Self {
            source_id,
            options_id,
        }
    }

    fn entry_key(&self) -> String {
        cache_hash(&(CACHE_VERSION, &self.source_id, &self.options_id))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SearchCacheKey {
    query: String,
    top_k: usize,
    mode: SearchMode,
    alpha_bits: Option<u32>,
    filter_languages: Option<Vec<String>>,
    filter_paths: Option<Vec<String>>,
    explain: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedIndexPayload {
    version: u32,
    context: CacheContext,
    #[serde(default)]
    source_fingerprint: Option<String>,
    signatures: Vec<FileSignature>,
    chunks: Vec<Chunk>,
    bm25_index: Bm25Index,
    #[serde(default)]
    file_mapping: Option<HashMap<String, Vec<usize>>>,
    #[serde(default)]
    language_mapping: Option<HashMap<String, Vec<usize>>>,
    #[serde(default)]
    symbol_index: Option<SymbolIndex>,
    warnings: Vec<IndexWarning>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedStructuralPayload {
    version: u32,
    context: CacheContext,
    #[serde(default)]
    source_fingerprint: Option<String>,
    signatures: Vec<FileSignature>,
    files: Vec<String>,
    stats: IndexStats,
    symbol_index: SymbolIndex,
    outlines: HashMap<String, FileOutline>,
    warnings: Vec<IndexWarning>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedNavigationPayload {
    version: u32,
    context: CacheContext,
    #[serde(default)]
    source_fingerprint: Option<String>,
    signatures: Vec<FileSignature>,
    files: Vec<String>,
    stats: IndexStats,
    #[serde(default)]
    symbol_postings: Vec<SymbolPosting>,
    #[serde(default)]
    symbol_index: Option<SymbolIndex>,
    warnings: Vec<IndexWarning>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedNavigationManifest {
    version: u32,
    context: CacheContext,
    #[serde(default)]
    source_fingerprint: Option<String>,
    signatures: Vec<FileSignature>,
    files: Vec<String>,
    stats: IndexStats,
    warnings: Vec<IndexWarning>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedSymbolLookupPayload {
    version: u32,
    context: CacheContext,
    #[serde(default)]
    source_fingerprint: Option<String>,
    key: String,
    postings: Vec<SymbolPosting>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedFileOutlinePayload {
    version: u32,
    context: CacheContext,
    #[serde(default)]
    source_fingerprint: Option<String>,
    file_path: String,
    outline: FileOutline,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedSemanticPayload {
    version: u32,
    context: CacheContext,
    #[serde(default)]
    source_fingerprint: Option<String>,
    model_fingerprint: String,
    signatures: Vec<FileSignature>,
    semantic_index: DenseIndex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileSignature {
    path: String,
    len: u64,
    modified_ns: u128,
}

impl SifsIndex {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_path_with_options(path, None, None, None, false)
    }

    pub fn from_path_with_options(
        path: impl AsRef<Path>,
        model_path: Option<&str>,
        extensions: Option<HashSet<String>>,
        ignore: Option<HashSet<String>>,
        include_text_files: bool,
    ) -> Result<Self> {
        Self::from_path_with_model_options(
            path,
            ModelOptions::new(model_path, crate::model2vec::ModelLoadPolicy::AllowDownload),
            extensions,
            ignore,
            include_text_files,
        )
    }

    pub fn from_path_with_model_options(
        path: impl AsRef<Path>,
        model_options: ModelOptions,
        extensions: Option<HashSet<String>>,
        ignore: Option<HashSet<String>>,
        include_text_files: bool,
    ) -> Result<Self> {
        let options = IndexOptions::new(model_options)
            .with_extensions(extensions)
            .with_ignore(ignore)
            .with_include_text_files(include_text_files);
        Self::from_path_with_index_options(path, options)
    }

    pub fn from_path_sparse(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_path_with_index_options(path, IndexOptions::sparse())
    }

    pub fn from_path_hybrid(path: impl AsRef<Path>, model_options: ModelOptions) -> Result<Self> {
        Self::from_path_with_index_options(path, IndexOptions::new(model_options))
    }

    pub fn from_path_with_encoder_spec(
        path: impl AsRef<Path>,
        encoder_spec: EncoderSpec,
        extensions: Option<HashSet<String>>,
        ignore: Option<HashSet<String>>,
        include_text_files: bool,
    ) -> Result<Self> {
        let options = IndexOptions::sparse()
            .with_encoder_spec(encoder_spec)
            .with_extensions(extensions)
            .with_ignore(ignore)
            .with_include_text_files(include_text_files);
        Self::from_path_with_index_options(path, options)
    }

    pub fn from_path_with_index_options(
        path: impl AsRef<Path>,
        options: IndexOptions,
    ) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("Path is not a directory: {}", path.display());
        }
        let root = path.canonicalize()?;
        let context = CacheContext::for_source(
            options
                .source_id_override
                .clone()
                .unwrap_or_else(|| format!("path:{}", root.to_string_lossy())),
            &root,
            &options.extensions,
            &options.ignore,
            options.include_text_files,
        );
        let cache_entry = resolve_cache_entry(&options.cache, &root, &context)?;
        let signature_context = SourceSignatureContext {
            root: root.clone(),
            extensions: options.extensions.clone(),
            ignore: options.ignore.clone(),
            include_text_files: options.include_text_files,
        };
        if let Some(cache_entry) = &cache_entry
            && let Some(payload) =
                load_cached_index_payload_by_source_fingerprint(cache_entry, &context, &root)
        {
            let index = Self::from_cached_parts(
                options.semantic_config.clone(),
                payload,
                Some(cache_entry.clone()),
                Some(context),
                Some(signature_context),
            )?;
            write_cached_index_payload(&index);
            return Ok(index);
        }
        let signatures = current_file_signatures(
            &root,
            options.extensions.as_ref(),
            options.ignore.as_ref(),
            options.include_text_files,
        )
        .ok();
        if let (Some(cache_entry), Some(signatures)) = (&cache_entry, &signatures)
            && let Some(payload) = load_cached_index_payload(cache_entry, &context, signatures)
        {
            let index = Self::from_cached_parts(
                options.semantic_config.clone(),
                payload,
                Some(cache_entry.clone()),
                Some(context),
                Some(signature_context),
            )?;
            write_cached_index_payload(&index);
            return Ok(index);
        }
        let (chunks, warnings) = create_chunks_from_path_with_warnings(
            &root,
            options.extensions,
            options.ignore.as_ref(),
            options.include_text_files,
            &root,
        )?;
        let mut index = Self::from_chunks_with_semantic_config(options.semantic_config, chunks)?;
        index.warnings = warnings;
        if let (Some(cache_entry), Some(signatures)) = (cache_entry, signatures) {
            index.attach_cache(cache_entry, signatures, context, signature_context);
            write_cached_index_payload(&index);
            write_cached_navigation_payload(&index);
            write_cached_structural_payload(&index);
        }
        Ok(index)
    }

    pub fn from_git(url: &str, ref_name: Option<&str>) -> Result<Self> {
        Self::from_git_with_model_options(url, ref_name, ModelOptions::default())
    }

    pub fn from_git_with_model_options(
        url: &str,
        ref_name: Option<&str>,
        model_options: ModelOptions,
    ) -> Result<Self> {
        Self::from_git_with_index_options(url, ref_name, IndexOptions::new(model_options))
    }

    pub fn from_git_sparse(url: &str, ref_name: Option<&str>) -> Result<Self> {
        Self::from_git_with_index_options(url, ref_name, IndexOptions::sparse())
    }

    pub fn from_git_hybrid(
        url: &str,
        ref_name: Option<&str>,
        model_options: ModelOptions,
    ) -> Result<Self> {
        Self::from_git_with_index_options(url, ref_name, IndexOptions::new(model_options))
    }

    pub fn from_git_with_encoder_spec(
        url: &str,
        ref_name: Option<&str>,
        encoder_spec: EncoderSpec,
    ) -> Result<Self> {
        Self::from_git_with_index_options(
            url,
            ref_name,
            IndexOptions::sparse().with_encoder_spec(encoder_spec),
        )
    }

    pub fn from_git_with_index_options(
        url: &str,
        ref_name: Option<&str>,
        options: IndexOptions,
    ) -> Result<Self> {
        let tmp = tempfile::tempdir()?;
        let mut cmd = Command::new("git");
        cmd.arg("clone").arg("--depth").arg("1");
        if let Some(ref_name) = ref_name {
            cmd.arg("--branch").arg(ref_name);
        }
        cmd.arg("--").arg(url).arg(tmp.path());
        let output = cmd.stdin(Stdio::null()).output().context("run git clone")?;
        if !output.status.success() {
            bail!(
                "git clone failed for {url:?}:\n{}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let source_id = ref_name
            .map(|name| format!("git:{url}@{name}"))
            .unwrap_or_else(|| format!("git:{url}"));
        Self::from_path_with_index_options(tmp.path(), options.with_source_id(source_id))
    }

    pub fn from_chunks(model: Box<dyn Encoder>, chunks: Vec<Chunk>) -> Result<Self> {
        Self::from_chunks_with_semantic_state(None, model, chunks)
    }

    pub fn from_chunks_sparse(chunks: Vec<Chunk>) -> Result<Self> {
        Self::from_chunks_with_semantic_config(None, chunks)
    }

    pub fn from_chunks_hybrid(chunks: Vec<Chunk>, model_options: ModelOptions) -> Result<Self> {
        Self::from_chunks_with_semantic_config(Some(EncoderSpec::Model2Vec(model_options)), chunks)
    }

    pub fn from_chunks_with_model_options(
        model_options: ModelOptions,
        chunks: Vec<Chunk>,
    ) -> Result<Self> {
        Self::from_chunks_hybrid(chunks, model_options)
    }

    pub fn from_chunks_with_encoder_spec(
        encoder_spec: EncoderSpec,
        chunks: Vec<Chunk>,
    ) -> Result<Self> {
        Self::from_chunks_with_semantic_config(Some(encoder_spec), chunks)
    }

    fn from_chunks_with_semantic_config(
        semantic_config: Option<EncoderSpec>,
        chunks: Vec<Chunk>,
    ) -> Result<Self> {
        if chunks.is_empty() {
            bail!("No supported files found.");
        }
        let bm25_index = Bm25Index::build_from_chunks(&chunks);
        let (file_mapping, language_mapping) = populate_mapping(&chunks);
        let symbol_index = SymbolIndex::from_chunks(&chunks);
        Ok(Self {
            bm25_index,
            semantic_state: Mutex::new(None),
            semantic_config,
            chunks,
            file_mapping,
            language_mapping,
            symbol_index,
            search_cache: Mutex::new(HashMap::new()),
            cache_entry: None,
            signatures: None,
            signature_context: None,
            cache_context: None,
            warnings: Vec::new(),
        })
    }

    fn from_chunks_with_semantic_state(
        semantic_config: Option<EncoderSpec>,
        model: Box<dyn Encoder>,
        chunks: Vec<Chunk>,
    ) -> Result<Self> {
        let index = Self::from_chunks_with_semantic_config(semantic_config, chunks)?;
        let embeddings = encode_chunks_batched(model.as_ref(), &index.chunks);
        let semantic_index = DenseIndex::new(embeddings);
        if let Ok(mut state) = index.semantic_state.lock() {
            *state = Some(SemanticState {
                model,
                index: semantic_index,
            });
        }
        Ok(index)
    }

    fn from_cached_parts(
        semantic_config: Option<EncoderSpec>,
        payload: CachedIndexPayload,
        cache_entry: Option<CacheEntry>,
        context: Option<CacheContext>,
        signature_context: Option<SourceSignatureContext>,
    ) -> Result<Self> {
        if payload.chunks.is_empty() {
            bail!("No supported files found.");
        }
        let signatures = payload.signatures.clone();
        let (file_mapping, language_mapping) =
            match (payload.file_mapping, payload.language_mapping) {
                (Some(file_mapping), Some(language_mapping)) => (file_mapping, language_mapping),
                _ => populate_mapping(&payload.chunks),
            };
        let symbol_index = payload
            .symbol_index
            .unwrap_or_else(|| SymbolIndex::from_chunks(&payload.chunks));
        Ok(Self {
            bm25_index: payload.bm25_index,
            semantic_state: Mutex::new(None),
            semantic_config,
            chunks: payload.chunks,
            file_mapping,
            language_mapping,
            symbol_index,
            search_cache: Mutex::new(HashMap::new()),
            cache_entry,
            signatures: Some(signatures),
            signature_context,
            cache_context: context,
            warnings: payload.warnings,
        })
    }

    pub fn stats(&self) -> IndexStats {
        let mut languages = BTreeMap::new();
        for chunk in &self.chunks {
            if let Some(language) = &chunk.language {
                *languages.entry(language.clone()).or_default() += 1;
            }
        }
        IndexStats {
            indexed_files: self.file_mapping.len(),
            total_chunks: self.chunks.len(),
            languages,
        }
    }

    pub fn indexed_files(&self) -> Vec<String> {
        let mut files: Vec<_> = self.file_mapping.keys().cloned().collect();
        files.sort();
        files
    }

    pub fn indexed_files_with_prefix(&self, prefix: Option<&str>, limit: usize) -> Vec<String> {
        let normalized_prefix = prefix
            .map(normalize_filter_path)
            .filter(|prefix| !prefix.is_empty());
        let mut files: Vec<_> = self
            .file_mapping
            .keys()
            .filter(|file| {
                normalized_prefix
                    .as_ref()
                    .is_none_or(|prefix| file.starts_with(prefix))
            })
            .cloned()
            .collect();
        files.sort();
        files.truncate(limit);
        files
    }

    pub fn symbol_lookup(&self, name: &str, limit: usize) -> Vec<SymbolPosting> {
        self.symbol_index.lookup(name, limit)
    }

    pub fn symbol_lookup_total(&self, name: &str) -> usize {
        self.symbol_index.lookup_total(name)
    }

    pub fn file_outline(&self, file_path: &str) -> Option<FileOutline> {
        let normalized = normalize_filter_path(file_path);
        let chunk_ids = self.file_mapping.get(&normalized)?;
        symbol_index::file_outline(&normalized, chunk_ids, &self.chunks)
    }

    pub fn warnings(&self) -> &[IndexWarning] {
        &self.warnings
    }

    pub fn is_fresh(&self) -> Option<bool> {
        let context = self.signature_context.as_ref()?;
        let signatures = self.signatures.as_ref()?;
        let current = current_file_signatures(
            &context.root,
            context.extensions.as_ref(),
            context.ignore.as_ref(),
            context.include_text_files,
        )
        .ok()?;
        Some(&current == signatures)
    }

    pub fn chunks_for_file(&self, file_path: &str) -> Vec<&Chunk> {
        self.file_mapping
            .get(file_path)
            .map(|ids| ids.iter().map(|id| &self.chunks[*id]).collect())
            .unwrap_or_default()
    }

    pub fn search(
        &self,
        query: &str,
        top_k: usize,
        mode: SearchMode,
        alpha: Option<f32>,
        filter_languages: Option<&[String]>,
        filter_paths: Option<&[String]>,
    ) -> Result<Vec<SearchResult>> {
        let mut options = SearchOptions::new(top_k).with_mode(mode);
        options.alpha = alpha;
        if let Some(languages) = filter_languages {
            options.filter_languages = languages.to_vec();
        }
        if let Some(paths) = filter_paths {
            options.filter_paths = paths.to_vec();
        }
        self.search_with(query, &options)
    }

    pub fn search_with(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>> {
        if self.chunks.is_empty() || query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let filter_languages = empty_to_none(&options.filter_languages);
        let filter_paths = empty_to_none(&options.filter_paths);
        let cache_key = SearchCacheKey {
            query: query.to_owned(),
            top_k: options.top_k,
            mode: options.mode,
            alpha_bits: options.alpha.map(f32::to_bits),
            filter_languages: filter_languages.map(<[String]>::to_vec),
            filter_paths: filter_paths.map(<[String]>::to_vec),
            explain: options.explain,
        };
        if options.use_query_cache
            && let Some(results) = self
                .search_cache
                .lock()
                .ok()
                .and_then(|cache| cache.get(&cache_key).cloned())
        {
            return Ok(self.materialize_hits(results));
        }
        let selector = self.selector(filter_languages, filter_paths);
        let selector_ref = selector.as_deref();
        let results = match options.mode {
            SearchMode::Bm25 => search_bm25(
                query,
                &self.bm25_index,
                &self.chunks,
                options.top_k,
                selector_ref,
                options.explain,
            ),
            SearchMode::Semantic => {
                let state = self.ensure_semantic()?;
                let state = state.as_ref().expect("semantic state was initialized");
                search_semantic(
                    query,
                    state.model.as_ref(),
                    &state.index,
                    &self.chunks,
                    options.top_k,
                    selector_ref,
                    options.explain,
                )
            }
            SearchMode::Hybrid => {
                let state = self.ensure_semantic()?;
                let state = state.as_ref().expect("semantic state was initialized");
                search_hybrid(
                    query,
                    state.model.as_ref(),
                    &state.index,
                    &self.bm25_index,
                    &self.chunks,
                    Some(&self.file_mapping),
                    Some(self.symbol_index.chunk_mapping()),
                    options.top_k,
                    options.alpha,
                    selector_ref,
                    options.explain,
                )
            }
        };
        if options.use_query_cache
            && let Ok(mut cache) = self.search_cache.lock()
        {
            let max_entries = query_cache_entry_limit();
            if max_entries == 0 {
                return Ok(self.materialize_hits(results));
            }
            if cache.len() >= max_entries && !cache.contains_key(&cache_key) {
                cache.clear();
            }
            cache.insert(cache_key, results.clone());
        }
        Ok(self.materialize_hits(results))
    }

    pub fn find_related(&self, source: &Chunk, top_k: usize) -> Result<Vec<SearchResult>> {
        let filter_languages = source.language.as_ref().map(|l| vec![l.clone()]);
        let selector = self.selector(filter_languages.as_deref(), None);
        let state = self.ensure_semantic()?;
        let state = state.as_ref().expect("semantic state was initialized");
        let mut results = search_semantic(
            &source.content,
            state.model.as_ref(),
            &state.index,
            &self.chunks,
            top_k + 1,
            selector.as_deref(),
            false,
        );
        results.retain(|hit| self.chunks.get(hit.chunk_id) != Some(source));
        results.truncate(top_k);
        Ok(self.materialize_hits(results))
    }

    fn materialize_hits(&self, hits: Vec<SearchHit>) -> Vec<SearchResult> {
        hits.into_iter()
            .filter_map(|hit| {
                self.chunks.get(hit.chunk_id).map(|chunk| SearchResult {
                    chunk: chunk.clone(),
                    score: hit.score,
                    source: hit.source,
                    explanation: hit.explanation,
                })
            })
            .collect()
    }

    pub fn semantic_loaded(&self) -> bool {
        self.semantic_state
            .lock()
            .map(|state| state.is_some())
            .unwrap_or(false)
    }

    pub fn warm_semantic(&self) -> Result<bool> {
        if self.semantic_config.is_none() {
            return Ok(false);
        }
        drop(self.ensure_semantic()?);
        Ok(true)
    }

    #[cfg(test)]
    fn query_cache_len(&self) -> usize {
        self.search_cache
            .lock()
            .map(|cache| cache.len())
            .unwrap_or_default()
    }

    fn ensure_semantic(&self) -> Result<std::sync::MutexGuard<'_, Option<SemanticState>>> {
        let mut state = self
            .semantic_state
            .lock()
            .map_err(|_| anyhow::anyhow!("semantic index lock is poisoned"))?;
        if state.is_none() {
            let Some(semantic_config) = self.semantic_config.as_ref() else {
                bail!(
                    "semantic search is not available on this sparse-only index. Build with SifsIndex::from_path_hybrid or use SearchMode::Bm25."
                );
            };
            let model = load_encoder(semantic_config)?;
            let fingerprint = encoder_fingerprint(semantic_config)?;
            let semantic_index = self
                .load_cached_semantic_index(&fingerprint)
                .unwrap_or_else(|| {
                    let embeddings = encode_chunks_batched(model.as_ref(), &self.chunks);
                    let semantic_index = DenseIndex::new(embeddings);
                    self.write_cached_semantic_index(&semantic_index, &fingerprint);
                    semantic_index
                });
            *state = Some(SemanticState {
                model,
                index: semantic_index,
            });
        }
        Ok(state)
    }

    fn attach_cache(
        &mut self,
        cache_entry: CacheEntry,
        signatures: Vec<FileSignature>,
        context: CacheContext,
        signature_context: SourceSignatureContext,
    ) {
        self.signatures = Some(signatures);
        self.signature_context = Some(signature_context);
        self.cache_entry = Some(cache_entry);
        self.cache_context = Some(context);
    }

    fn cache_source_fingerprint(&self) -> Option<String> {
        git_source_fingerprint(&self.signature_context.as_ref()?.root)
    }

    fn load_cached_semantic_index(&self, model_fingerprint: &str) -> Option<DenseIndex> {
        let cache_entry = self.cache_entry.as_ref()?;
        let signatures = self.signatures.as_ref()?;
        let context = self.cache_context.as_ref()?;
        let bytes = fs::read(semantic_cache_path(
            cache_entry,
            self.semantic_config.as_ref()?,
            model_fingerprint,
        ))
        .ok()?;
        let (payload, _): (CachedSemanticPayload, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
        (payload.version == CACHE_VERSION
            && payload.context == *context
            && payload.model_fingerprint == model_fingerprint
            && payload.signatures == *signatures)
            .then_some(payload.semantic_index)
    }

    fn write_cached_semantic_index(&self, semantic_index: &DenseIndex, model_fingerprint: &str) {
        let Some(cache_entry) = self.cache_entry.as_ref() else {
            return;
        };
        let Some(signatures) = self.signatures.as_ref() else {
            return;
        };
        let Some(context) = self.cache_context.as_ref() else {
            return;
        };
        let payload = CachedSemanticPayload {
            version: CACHE_VERSION,
            context: context.clone(),
            source_fingerprint: self.cache_source_fingerprint(),
            model_fingerprint: model_fingerprint.to_owned(),
            signatures: signatures.clone(),
            semantic_index: semantic_index.clone(),
        };
        let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
            return;
        };
        let Some(semantic_config) = self.semantic_config.as_ref() else {
            return;
        };
        let cache_path = semantic_cache_path(cache_entry, semantic_config, model_fingerprint);
        if let Some(parent) = cache_path.parent()
            && fs::create_dir_all(parent).is_err()
        {
            return;
        }
        let tmp_path = cache_path.with_extension("bin.tmp");
        if fs::write(&tmp_path, bytes).is_ok() {
            let _ = fs::rename(tmp_path, cache_path);
        }
    }

    fn selector(
        &self,
        filter_languages: Option<&[String]>,
        filter_paths: Option<&[String]>,
    ) -> Option<Vec<usize>> {
        let mut language_ids = Vec::new();
        if let Some(languages) = filter_languages {
            for language in languages {
                if let Some(values) = self.language_mapping.get(language) {
                    language_ids.extend(values);
                }
            }
        }
        language_ids.sort_unstable();
        language_ids.dedup();

        let mut path_ids = Vec::new();
        if let Some(paths) = filter_paths {
            for path in paths {
                let normalized = normalize_filter_path(path);
                if let Some(values) = self.file_mapping.get(&normalized) {
                    path_ids.extend(values);
                }
            }
        }
        path_ids.sort_unstable();
        path_ids.dedup();

        match (filter_languages, filter_paths) {
            (Some(_), Some(_)) => {
                let ids = language_ids
                    .into_iter()
                    .filter(|id| path_ids.binary_search(id).is_ok())
                    .collect::<Vec<_>>();
                Some(ids)
            }
            (Some(_), None) => Some(language_ids),
            (None, Some(_)) => Some(path_ids),
            (None, None) => None,
        }
    }
}

impl StructuralIndex {
    pub fn from_path_with_index_options(
        path: impl AsRef<Path>,
        mut options: IndexOptions,
    ) -> Result<Self> {
        options.semantic_config = None;
        let path = path.as_ref();
        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("Path is not a directory: {}", path.display());
        }
        let root = path.canonicalize()?;
        let context = CacheContext::for_source(
            options
                .source_id_override
                .clone()
                .unwrap_or_else(|| format!("path:{}", root.to_string_lossy())),
            &root,
            &options.extensions,
            &options.ignore,
            options.include_text_files,
        );
        let cache_entry = resolve_cache_entry(&options.cache, &root, &context)?;
        if let Some(cache_entry) = &cache_entry
            && let Some(payload) =
                load_cached_structural_payload_by_source_fingerprint(cache_entry, &context, &root)
        {
            return Ok(Self::from_cached_payload(payload));
        }
        let signatures = current_file_signatures(
            &root,
            options.extensions.as_ref(),
            options.ignore.as_ref(),
            options.include_text_files,
        )
        .ok();
        if let (Some(cache_entry), Some(signatures)) = (&cache_entry, &signatures)
            && let Some(payload) = load_cached_structural_payload(cache_entry, &context, signatures)
        {
            return Ok(Self::from_cached_payload(payload));
        }
        let index = SifsIndex::from_path_with_index_options(&root, options)?;
        write_cached_navigation_payload(&index);
        Ok(Self::from_sifs_index(&index))
    }

    pub fn from_git_with_index_options(
        url: &str,
        ref_name: Option<&str>,
        options: IndexOptions,
    ) -> Result<Self> {
        let tmp = tempfile::tempdir()?;
        let mut cmd = Command::new("git");
        cmd.arg("clone").arg("--depth").arg("1");
        if let Some(ref_name) = ref_name {
            cmd.arg("--branch").arg(ref_name);
        }
        cmd.arg("--").arg(url).arg(tmp.path());
        let output = cmd.stdin(Stdio::null()).output().context("run git clone")?;
        if !output.status.success() {
            bail!(
                "git clone failed for {url:?}:\n{}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let source_id = ref_name
            .map(|name| format!("git:{url}@{name}"))
            .unwrap_or_else(|| format!("git:{url}"));
        Self::from_path_with_index_options(tmp.path(), options.with_source_id(source_id))
    }

    pub fn indexed_files_with_prefix(&self, prefix: Option<&str>, limit: usize) -> Vec<String> {
        let normalized_prefix = prefix
            .map(normalize_filter_path)
            .filter(|prefix| !prefix.is_empty());
        let mut files = self
            .files
            .iter()
            .filter(|file| {
                normalized_prefix
                    .as_ref()
                    .is_none_or(|prefix| file.starts_with(prefix))
            })
            .cloned()
            .collect::<Vec<_>>();
        files.truncate(limit);
        files
    }

    pub fn symbol_lookup(&self, name: &str, limit: usize) -> Vec<SymbolPosting> {
        self.symbol_index.lookup(name, limit)
    }

    pub fn symbol_lookup_total(&self, name: &str) -> usize {
        self.symbol_index.lookup_total(name)
    }

    pub fn file_outline(&self, file_path: &str) -> Option<FileOutline> {
        let normalized = normalize_filter_path(file_path);
        self.outlines.get(&normalized).cloned()
    }

    pub fn stats(&self) -> &IndexStats {
        &self.stats
    }

    pub fn warnings(&self) -> &[IndexWarning] {
        &self.warnings
    }

    fn from_sifs_index(index: &SifsIndex) -> Self {
        let outlines = outlines_from_chunks(&index.file_mapping, &index.chunks);
        let mut files = index.file_mapping.keys().cloned().collect::<Vec<_>>();
        files.sort();
        Self {
            files,
            stats: index.stats(),
            symbol_index: index.symbol_index.clone(),
            outlines,
            warnings: index.warnings.clone(),
        }
    }

    fn from_cached_payload(payload: CachedStructuralPayload) -> Self {
        Self {
            files: payload.files,
            stats: payload.stats,
            symbol_index: payload.symbol_index,
            outlines: payload.outlines,
            warnings: payload.warnings,
        }
    }
}

impl NavigationIndex {
    pub fn cached_symbol_lookup_from_path_with_index_options(
        path: impl AsRef<Path>,
        mut options: IndexOptions,
        name: &str,
        limit: usize,
    ) -> Result<Option<NavigationSymbolLookup>> {
        options.semantic_config = None;
        let path = path.as_ref();
        if !path.exists() || !path.is_dir() || name.trim().is_empty() {
            return Ok(None);
        }
        let root = path.canonicalize()?;
        let context = CacheContext::for_source(
            options
                .source_id_override
                .clone()
                .unwrap_or_else(|| format!("path:{}", root.to_string_lossy())),
            &root,
            &options.extensions,
            &options.ignore,
            options.include_text_files,
        );
        let Some(cache_entry) = resolve_cache_entry(&options.cache, &root, &context)? else {
            return Ok(None);
        };
        let Some(manifest) =
            load_cached_navigation_manifest_by_source_fingerprint(&cache_entry, &context, &root)
        else {
            return Ok(None);
        };
        let exact_key = symbol_shard_key(name);
        let folded_key = symbol_shard_key(&symbol_index::fold_symbol_key(name));
        let payload = load_cached_symbol_lookup_payload(&cache_entry, &context, &root, &exact_key)
            .or_else(|| {
                if folded_key == exact_key {
                    None
                } else {
                    load_cached_symbol_lookup_payload(&cache_entry, &context, &root, &folded_key)
                }
            });
        let Some(mut payload) = payload else {
            return Ok(None);
        };
        let total = payload.postings.len();
        payload.postings.truncate(limit);
        Ok(Some(NavigationSymbolLookup {
            stats: manifest.stats,
            warnings: manifest.warnings,
            total,
            postings: payload.postings,
        }))
    }

    pub fn cached_file_outline_from_path_with_index_options(
        path: impl AsRef<Path>,
        mut options: IndexOptions,
        file_path: &str,
    ) -> Result<Option<NavigationFileOutline>> {
        options.semantic_config = None;
        let path = path.as_ref();
        if !path.exists() || !path.is_dir() || file_path.trim().is_empty() {
            return Ok(None);
        }
        let root = path.canonicalize()?;
        let context = CacheContext::for_source(
            options
                .source_id_override
                .clone()
                .unwrap_or_else(|| format!("path:{}", root.to_string_lossy())),
            &root,
            &options.extensions,
            &options.ignore,
            options.include_text_files,
        );
        let Some(cache_entry) = resolve_cache_entry(&options.cache, &root, &context)? else {
            return Ok(None);
        };
        let Some(manifest) =
            load_cached_navigation_manifest_by_source_fingerprint(&cache_entry, &context, &root)
        else {
            return Ok(None);
        };
        let normalized = normalize_filter_path(file_path);
        let payload = load_cached_file_outline_payload(&cache_entry, &context, &root, &normalized);
        let Some(payload) = payload else {
            return Ok(None);
        };
        Ok(Some(NavigationFileOutline {
            stats: manifest.stats,
            warnings: manifest.warnings,
            outline: payload.outline,
        }))
    }

    pub fn from_path_with_index_options(
        path: impl AsRef<Path>,
        mut options: IndexOptions,
    ) -> Result<Self> {
        options.semantic_config = None;
        let path = path.as_ref();
        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("Path is not a directory: {}", path.display());
        }
        let root = path.canonicalize()?;
        let context = CacheContext::for_source(
            options
                .source_id_override
                .clone()
                .unwrap_or_else(|| format!("path:{}", root.to_string_lossy())),
            &root,
            &options.extensions,
            &options.ignore,
            options.include_text_files,
        );
        let cache_entry = resolve_cache_entry(&options.cache, &root, &context)?;
        if let Some(cache_entry) = &cache_entry
            && let Some(payload) =
                load_cached_navigation_payload_by_source_fingerprint(cache_entry, &context, &root)
        {
            let source_fingerprint = payload.source_fingerprint.clone();
            let signatures = payload.signatures.clone();
            let navigation = Self::from_cached_payload(payload);
            write_cached_navigation_manifest_and_shards_from_parts(
                cache_entry,
                &context,
                source_fingerprint,
                &signatures,
                &navigation,
            );
            return Ok(navigation);
        }
        let signatures = current_file_signatures(
            &root,
            options.extensions.as_ref(),
            options.ignore.as_ref(),
            options.include_text_files,
        )
        .ok();
        if let (Some(cache_entry), Some(signatures)) = (&cache_entry, &signatures)
            && let Some(payload) = load_cached_navigation_payload(cache_entry, &context, signatures)
        {
            let source_fingerprint = payload
                .source_fingerprint
                .clone()
                .or_else(|| git_source_fingerprint(&root));
            let navigation = Self::from_cached_payload(payload);
            write_cached_navigation_manifest_and_shards_from_parts(
                cache_entry,
                &context,
                source_fingerprint,
                signatures,
                &navigation,
            );
            return Ok(navigation);
        }
        let index = SifsIndex::from_path_with_index_options(&root, options)?;
        Ok(Self::from_sifs_index(&index))
    }

    pub fn from_git_with_index_options(
        url: &str,
        ref_name: Option<&str>,
        options: IndexOptions,
    ) -> Result<Self> {
        let tmp = tempfile::tempdir()?;
        let mut cmd = Command::new("git");
        cmd.arg("clone").arg("--depth").arg("1");
        if let Some(ref_name) = ref_name {
            cmd.arg("--branch").arg(ref_name);
        }
        cmd.arg("--").arg(url).arg(tmp.path());
        let output = cmd.stdin(Stdio::null()).output().context("run git clone")?;
        if !output.status.success() {
            bail!(
                "git clone failed for {url:?}:\n{}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let source_id = ref_name
            .map(|name| format!("git:{url}@{name}"))
            .unwrap_or_else(|| format!("git:{url}"));
        Self::from_path_with_index_options(tmp.path(), options.with_source_id(source_id))
    }

    pub fn indexed_files_with_prefix(&self, prefix: Option<&str>, limit: usize) -> Vec<String> {
        let normalized_prefix = prefix
            .map(normalize_filter_path)
            .filter(|prefix| !prefix.is_empty());
        let mut files = self
            .files
            .iter()
            .filter(|file| {
                normalized_prefix
                    .as_ref()
                    .is_none_or(|prefix| file.starts_with(prefix))
            })
            .cloned()
            .collect::<Vec<_>>();
        files.truncate(limit);
        files
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn symbol_lookup(&self, name: &str, limit: usize) -> Vec<SymbolPosting> {
        self.symbol_index.lookup(name, limit)
    }

    pub fn symbol_lookup_total(&self, name: &str) -> usize {
        self.symbol_index.lookup_total(name)
    }

    pub fn file_symbol_outline(&self, file_path: &str) -> Option<FileOutline> {
        let normalized = normalize_filter_path(file_path);
        if self.files.binary_search(&normalized).is_err() {
            return None;
        }
        let mut symbols = self
            .symbol_index
            .postings()
            .into_iter()
            .filter(|posting| posting.file_path == normalized)
            .collect::<Vec<_>>();
        if symbols.is_empty() {
            return None;
        }
        symbols.sort_by(|left, right| {
            left.line
                .cmp(&right.line)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.kind.cmp(&right.kind))
        });
        let start_line = symbols
            .iter()
            .map(|posting| posting.start_line)
            .min()
            .unwrap_or(1);
        let end_line = symbols
            .iter()
            .map(|posting| posting.end_line)
            .max()
            .unwrap_or(start_line);
        let mut language_counts = BTreeMap::<String, usize>::new();
        let mut chunks = HashSet::<(usize, usize)>::new();
        for posting in &symbols {
            if let Some(language) = &posting.language {
                *language_counts.entry(language.clone()).or_default() += 1;
            }
            chunks.insert((posting.start_line, posting.end_line));
        }
        let language = language_counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(language, _)| language);
        Some(FileOutline {
            file_path: normalized,
            language,
            start_line,
            end_line,
            chunk_count: chunks.len(),
            symbol_count: symbols.len(),
            symbols,
            chunks: Vec::new(),
        })
    }

    pub fn stats(&self) -> &IndexStats {
        &self.stats
    }

    pub fn warnings(&self) -> &[IndexWarning] {
        &self.warnings
    }

    fn from_sifs_index(index: &SifsIndex) -> Self {
        let mut files = index.file_mapping.keys().cloned().collect::<Vec<_>>();
        files.sort();
        Self {
            files,
            stats: index.stats(),
            symbol_index: index.symbol_index.clone(),
            warnings: index.warnings.clone(),
        }
    }

    fn from_cached_payload(payload: CachedNavigationPayload) -> Self {
        let symbol_index = payload
            .symbol_index
            .unwrap_or_else(|| SymbolIndex::from_postings(payload.symbol_postings));
        Self {
            files: payload.files,
            stats: payload.stats,
            symbol_index,
            warnings: payload.warnings,
        }
    }
}

fn load_cached_index_payload(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    signatures: &[FileSignature],
) -> Option<CachedIndexPayload> {
    let bytes = fs::read(sparse_cache_path(cache_entry)).ok()?;
    let (payload, _): (CachedIndexPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    (payload.version == CACHE_VERSION
        && payload.context == *context
        && payload.signatures == signatures)
        .then_some(payload)
}

fn load_cached_index_payload_by_source_fingerprint(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    root: &Path,
) -> Option<CachedIndexPayload> {
    let bytes = fs::read(sparse_cache_path(cache_entry)).ok()?;
    let (payload, _): (CachedIndexPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    cached_payload_source_matches(
        payload.version,
        &payload.context,
        payload.source_fingerprint.as_deref(),
        context,
        root,
    )
    .then_some(payload)
}

fn load_cached_structural_payload(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    signatures: &[FileSignature],
) -> Option<CachedStructuralPayload> {
    let bytes = fs::read(structural_cache_path(cache_entry)).ok()?;
    let (payload, _): (CachedStructuralPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    (payload.version == CACHE_VERSION
        && payload.context == *context
        && payload.signatures == signatures)
        .then_some(payload)
}

fn load_cached_structural_payload_by_source_fingerprint(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    root: &Path,
) -> Option<CachedStructuralPayload> {
    let bytes = fs::read(structural_cache_path(cache_entry)).ok()?;
    let (payload, _): (CachedStructuralPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    cached_payload_source_matches(
        payload.version,
        &payload.context,
        payload.source_fingerprint.as_deref(),
        context,
        root,
    )
    .then_some(payload)
}

fn load_cached_navigation_payload(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    signatures: &[FileSignature],
) -> Option<CachedNavigationPayload> {
    let bytes = fs::read(navigation_cache_path(cache_entry)).ok()?;
    let (payload, _): (CachedNavigationPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    (payload.version == CACHE_VERSION
        && payload.context == *context
        && payload.signatures == signatures)
        .then_some(payload)
}

fn load_cached_navigation_payload_by_source_fingerprint(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    root: &Path,
) -> Option<CachedNavigationPayload> {
    let bytes = fs::read(navigation_cache_path(cache_entry)).ok()?;
    let (payload, _): (CachedNavigationPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    cached_payload_source_matches(
        payload.version,
        &payload.context,
        payload.source_fingerprint.as_deref(),
        context,
        root,
    )
    .then_some(payload)
}

fn load_cached_navigation_manifest_by_source_fingerprint(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    root: &Path,
) -> Option<CachedNavigationManifest> {
    let bytes = fs::read(navigation_manifest_path(cache_entry)).ok()?;
    let (payload, _): (CachedNavigationManifest, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    cached_payload_source_matches(
        payload.version,
        &payload.context,
        payload.source_fingerprint.as_deref(),
        context,
        root,
    )
    .then_some(payload)
}

fn load_cached_symbol_lookup_payload(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    root: &Path,
    key: &str,
) -> Option<CachedSymbolLookupPayload> {
    let bytes = fs::read(navigation_symbol_shard_path(cache_entry, key)).ok()?;
    let (payload, _): (CachedSymbolLookupPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    (payload.key == key
        && cached_payload_source_matches(
            payload.version,
            &payload.context,
            payload.source_fingerprint.as_deref(),
            context,
            root,
        ))
    .then_some(payload)
}

fn load_cached_file_outline_payload(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    root: &Path,
    file_path: &str,
) -> Option<CachedFileOutlinePayload> {
    let bytes = fs::read(navigation_outline_shard_path(cache_entry, file_path)).ok()?;
    let (payload, _): (CachedFileOutlinePayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    (payload.file_path == file_path
        && cached_payload_source_matches(
            payload.version,
            &payload.context,
            payload.source_fingerprint.as_deref(),
            context,
            root,
        ))
    .then_some(payload)
}

fn cached_payload_source_matches(
    version: u32,
    payload_context: &CacheContext,
    payload_fingerprint: Option<&str>,
    context: &CacheContext,
    root: &Path,
) -> bool {
    version == CACHE_VERSION
        && payload_context == context
        && payload_fingerprint
            .zip(git_source_fingerprint(root))
            .is_some_and(|(left, right)| left == right)
}

fn git_source_fingerprint(root: &Path) -> Option<String> {
    let root = root.canonicalize().ok()?;
    let cache = GIT_SOURCE_FINGERPRINTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock()
        && let Some(fingerprint) = cache.get(&root)
    {
        return fingerprint.clone();
    }
    let fingerprint = compute_git_source_fingerprint(&root);
    if let Ok(mut cache) = cache.lock() {
        cache.insert(root, fingerprint.clone());
    }
    fingerprint
}

static GIT_SOURCE_FINGERPRINTS: OnceLock<Mutex<HashMap<PathBuf, Option<String>>>> = OnceLock::new();

fn compute_git_source_fingerprint(root: &Path) -> Option<String> {
    let top_level = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !top_level.status.success() {
        return None;
    }
    let top_level = PathBuf::from(String::from_utf8_lossy(&top_level.stdout).trim());
    if top_level.canonicalize().ok()? != root.canonicalize().ok()? {
        return None;
    }

    let head = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("HEAD")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !head.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();

    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--porcelain=v1")
        .arg("-z")
        .arg("--untracked-files=all")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    if status.stdout.is_empty() {
        return Some(format!("git-clean:{head}"));
    }

    let mut dirty_entries = parse_git_status_z(&status.stdout)
        .into_iter()
        .filter(|entry| language_for_path(Path::new(&entry.path)).is_some())
        .collect::<Vec<_>>();
    dirty_entries.sort();
    if dirty_entries.is_empty() {
        return Some(format!("git-clean:{head}"));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"git-dirty-v1\0");
    hasher.update(head.as_bytes());
    for entry in dirty_entries {
        hasher.update(b"\0entry\0");
        hasher.update(entry.status.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.path.as_bytes());
        let path = root.join(&entry.path);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {
                hasher.update(b"\0file\0");
                hasher.update(metadata.len().to_le_bytes());
                if let Ok(bytes) = fs::read(&path) {
                    hasher.update(Sha256::digest(bytes));
                }
            }
            Ok(metadata) if metadata.is_dir() => {
                hasher.update(b"\0dir\0");
                hasher.update(metadata.len().to_le_bytes());
            }
            Ok(_) => {
                hasher.update(b"\0other\0");
            }
            Err(_) => {
                hasher.update(b"\0missing\0");
            }
        }
    }
    Some(format!("git-dirty:{:x}", hasher.finalize()))
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GitStatusEntry {
    status: String,
    path: String,
}

fn parse_git_status_z(output: &[u8]) -> Vec<GitStatusEntry> {
    let mut entries = Vec::new();
    let mut fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    while let Some(field) = fields.next() {
        let text = String::from_utf8_lossy(field);
        let status = text.get(0..2).unwrap_or("").to_owned();
        let path = text.get(3..).unwrap_or("").to_owned();
        if !path.is_empty() {
            entries.push(GitStatusEntry {
                status: status.clone(),
                path,
            });
        }
        if (status.starts_with('R') || status.starts_with('C'))
            && let Some(previous) = fields.next()
        {
            let previous = String::from_utf8_lossy(previous).to_string();
            if !previous.is_empty() {
                entries.push(GitStatusEntry {
                    status: format!("{status}:from"),
                    path: previous,
                });
            }
        }
    }
    entries
}

fn write_cached_index_payload(index: &SifsIndex) {
    let Some(cache_entry) = index.cache_entry.as_ref() else {
        return;
    };
    let Some(signatures) = index.signatures.as_ref() else {
        return;
    };
    let Some(context) = index.cache_context.as_ref() else {
        return;
    };
    let payload = CachedIndexPayload {
        version: CACHE_VERSION,
        context: context.clone(),
        source_fingerprint: index.cache_source_fingerprint(),
        signatures: signatures.to_vec(),
        chunks: index.chunks.clone(),
        bm25_index: index.bm25_index.clone(),
        file_mapping: Some(index.file_mapping.clone()),
        language_mapping: Some(index.language_mapping.clone()),
        symbol_index: Some(index.symbol_index.clone()),
        warnings: index.warnings.clone(),
    };
    let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
        return;
    };
    let cache_path = sparse_cache_path(cache_entry);
    if let Some(parent) = cache_path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let tmp_path = cache_path.with_extension("bin.tmp");
    if fs::write(&tmp_path, bytes).is_ok() {
        let _ = fs::rename(tmp_path, cache_path);
    }
}

fn write_cached_navigation_payload(index: &SifsIndex) {
    let Some(cache_entry) = index.cache_entry.as_ref() else {
        return;
    };
    let Some(signatures) = index.signatures.as_ref() else {
        return;
    };
    let Some(context) = index.cache_context.as_ref() else {
        return;
    };
    let navigation = NavigationIndex::from_sifs_index(index);
    write_cached_navigation_manifest_and_shards(index, &navigation);
    let payload = CachedNavigationPayload {
        version: CACHE_VERSION,
        context: context.clone(),
        source_fingerprint: index.cache_source_fingerprint(),
        signatures: signatures.to_vec(),
        files: navigation.files.clone(),
        stats: navigation.stats.clone(),
        symbol_postings: navigation.symbol_index.postings(),
        symbol_index: None,
        warnings: navigation.warnings.clone(),
    };
    let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
        return;
    };
    let cache_path = navigation_cache_path(cache_entry);
    if let Some(parent) = cache_path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let tmp_path = cache_path.with_extension("bin.tmp");
    if fs::write(&tmp_path, bytes).is_ok() {
        let _ = fs::rename(tmp_path, cache_path);
    }
}

fn write_cached_navigation_manifest_and_shards(index: &SifsIndex, navigation: &NavigationIndex) {
    let Some(cache_entry) = index.cache_entry.as_ref() else {
        return;
    };
    let Some(signatures) = index.signatures.as_ref() else {
        return;
    };
    let Some(context) = index.cache_context.as_ref() else {
        return;
    };
    write_cached_navigation_manifest_and_shards_from_parts(
        cache_entry,
        context,
        index.cache_source_fingerprint(),
        signatures,
        navigation,
    );
}

fn write_cached_navigation_manifest_and_shards_from_parts(
    cache_entry: &CacheEntry,
    context: &CacheContext,
    source_fingerprint: Option<String>,
    signatures: &[FileSignature],
    navigation: &NavigationIndex,
) {
    if source_fingerprint.is_some()
        && fs::read(navigation_manifest_path(cache_entry))
            .ok()
            .and_then(|bytes| {
                bincode::serde::decode_from_slice::<CachedNavigationManifest, _>(
                    &bytes,
                    bincode::config::standard(),
                )
                .ok()
                .map(|(payload, _)| payload)
            })
            .is_some_and(|payload| {
                payload.version == CACHE_VERSION
                    && payload.context == *context
                    && payload.source_fingerprint == source_fingerprint
            })
    {
        return;
    }
    let manifest = CachedNavigationManifest {
        version: CACHE_VERSION,
        context: context.clone(),
        source_fingerprint: source_fingerprint.clone(),
        signatures: signatures.to_vec(),
        files: navigation.files.clone(),
        stats: navigation.stats.clone(),
        warnings: navigation.warnings.clone(),
    };
    write_bincode_atomically(navigation_manifest_path(cache_entry), &manifest);

    let postings = navigation.symbol_index.postings();
    let mut symbol_shards = BTreeMap::<String, Vec<SymbolPosting>>::new();
    let mut file_symbols = BTreeMap::<String, Vec<SymbolPosting>>::new();
    for posting in postings {
        let exact_key = symbol_shard_key(&posting.name);
        symbol_shards
            .entry(exact_key.clone())
            .or_default()
            .push(posting.clone());
        let folded_key = symbol_shard_key(&symbol_index::fold_symbol_key(&posting.name));
        if folded_key != exact_key {
            symbol_shards
                .entry(folded_key)
                .or_default()
                .push(posting.clone());
        }
        file_symbols
            .entry(posting.file_path.clone())
            .or_default()
            .push(posting);
    }
    for (key, mut postings) in symbol_shards {
        sort_symbol_postings_for_cache(&mut postings);
        let payload = CachedSymbolLookupPayload {
            version: CACHE_VERSION,
            context: context.clone(),
            source_fingerprint: source_fingerprint.clone(),
            key: key.clone(),
            postings,
        };
        write_bincode_atomically(navigation_symbol_shard_path(cache_entry, &key), &payload);
    }
    for (file_path, mut symbols) in file_symbols {
        sort_symbol_postings_for_cache(&mut symbols);
        let Some(outline) = file_symbol_outline_from_postings(file_path.clone(), symbols) else {
            continue;
        };
        let payload = CachedFileOutlinePayload {
            version: CACHE_VERSION,
            context: context.clone(),
            source_fingerprint: source_fingerprint.clone(),
            file_path: file_path.clone(),
            outline,
        };
        write_bincode_atomically(
            navigation_outline_shard_path(cache_entry, &file_path),
            &payload,
        );
    }
}

fn write_bincode_atomically<T: Serialize>(path: PathBuf, value: &T) {
    let Ok(bytes) = bincode::serde::encode_to_vec(value, bincode::config::standard()) else {
        return;
    };
    if let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let tmp_path = path.with_extension("bin.tmp");
    if fs::write(&tmp_path, bytes).is_ok() {
        let _ = fs::rename(tmp_path, path);
    }
}

fn symbol_shard_key(name: &str) -> String {
    name.trim().to_owned()
}

fn sort_symbol_postings_for_cache(postings: &mut [SymbolPosting]) {
    postings.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn file_symbol_outline_from_postings(
    file_path: String,
    mut symbols: Vec<SymbolPosting>,
) -> Option<FileOutline> {
    if symbols.is_empty() {
        return None;
    }
    symbols.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    let start_line = symbols
        .iter()
        .map(|posting| posting.start_line)
        .min()
        .unwrap_or(1);
    let end_line = symbols
        .iter()
        .map(|posting| posting.end_line)
        .max()
        .unwrap_or(start_line);
    let mut language_counts = BTreeMap::<String, usize>::new();
    let mut chunks = HashSet::<(usize, usize)>::new();
    for posting in &symbols {
        if let Some(language) = &posting.language {
            *language_counts.entry(language.clone()).or_default() += 1;
        }
        chunks.insert((posting.start_line, posting.end_line));
    }
    let language = language_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(language, _)| language);
    Some(FileOutline {
        file_path,
        language,
        start_line,
        end_line,
        chunk_count: chunks.len(),
        symbol_count: symbols.len(),
        symbols,
        chunks: Vec::new(),
    })
}

fn write_cached_structural_payload(index: &SifsIndex) {
    let Some(cache_entry) = index.cache_entry.as_ref() else {
        return;
    };
    let Some(signatures) = index.signatures.as_ref() else {
        return;
    };
    let Some(context) = index.cache_context.as_ref() else {
        return;
    };
    let structural = StructuralIndex::from_sifs_index(index);
    let payload = CachedStructuralPayload {
        version: CACHE_VERSION,
        context: context.clone(),
        source_fingerprint: index.cache_source_fingerprint(),
        signatures: signatures.clone(),
        files: structural.files,
        stats: structural.stats,
        symbol_index: structural.symbol_index,
        outlines: structural.outlines,
        warnings: structural.warnings,
    };
    let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
        return;
    };
    let cache_path = structural_cache_path(cache_entry);
    if let Some(parent) = cache_path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let tmp_path = cache_path.with_extension("bin.tmp");
    if fs::write(&tmp_path, bytes).is_ok() {
        let _ = fs::rename(tmp_path, cache_path);
    }
}

fn sparse_cache_path(cache_entry: &CacheEntry) -> PathBuf {
    cache_entry.root.join(SPARSE_CACHE_FILE)
}

fn structural_cache_path(cache_entry: &CacheEntry) -> PathBuf {
    cache_entry.root.join(STRUCTURAL_CACHE_FILE)
}

fn navigation_cache_path(cache_entry: &CacheEntry) -> PathBuf {
    cache_entry.root.join(NAVIGATION_CACHE_FILE)
}

fn navigation_manifest_path(cache_entry: &CacheEntry) -> PathBuf {
    cache_entry.root.join(NAVIGATION_MANIFEST_FILE)
}

fn navigation_symbol_shard_path(cache_entry: &CacheEntry, key: &str) -> PathBuf {
    cache_entry
        .root
        .join(NAVIGATION_SYMBOL_DIR)
        .join(format!("{}.bin", cache_hash(&("symbol-shard-v1", key))))
}

fn navigation_outline_shard_path(cache_entry: &CacheEntry, file_path: &str) -> PathBuf {
    cache_entry.root.join(NAVIGATION_OUTLINE_DIR).join(format!(
        "{}.bin",
        cache_hash(&("outline-shard-v1", file_path))
    ))
}

fn semantic_cache_path(
    cache_entry: &CacheEntry,
    encoder_spec: &EncoderSpec,
    model_fingerprint: &str,
) -> PathBuf {
    cache_entry.root.join(format!(
        "{SEMANTIC_CACHE_PREFIX}-{}-{model_fingerprint}.bin",
        encoder_spec.cache_key(),
    ))
}

fn resolve_cache_entry(
    config: &CacheConfig,
    source_root: &Path,
    context: &CacheContext,
) -> Result<Option<CacheEntry>> {
    let root = match config {
        CacheConfig::Disabled => return Ok(None),
        CacheConfig::Project => source_root.join(CACHE_DIR),
        CacheConfig::Custom(path) => path.join(context.entry_key()),
        CacheConfig::Platform => platform_cache_root()?.join(context.entry_key()),
    };
    Ok(Some(CacheEntry { root }))
}

pub fn platform_cache_root() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        Ok(home_dir()?
            .join("Library")
            .join("Caches")
            .join(PLATFORM_CACHE_DIR))
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        Ok(PathBuf::from(xdg).join(PLATFORM_CACHE_DIR))
    } else {
        Ok(home_dir()?.join(".cache").join(PLATFORM_CACHE_DIR))
    }
}

pub fn cache_summary(root: &Path) -> CacheSummary {
    let mut summary = CacheSummary {
        root: root.to_path_buf(),
        exists: root.exists(),
        entries: 0,
        files: 0,
        bytes: 0,
    };
    visit_cache_files(root, &mut |path, metadata| {
        if path.file_name().and_then(|name| name.to_str()) == Some(SPARSE_CACHE_FILE) {
            summary.entries += 1;
        }
        summary.files += 1;
        summary.bytes += metadata.len();
    });
    summary
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheSummary {
    pub root: PathBuf,
    pub exists: bool,
    pub entries: usize,
    pub files: usize,
    pub bytes: u64,
}

fn visit_cache_files(root: &Path, f: &mut impl FnMut(&Path, &fs::Metadata)) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            visit_cache_files(&path, f);
        } else if metadata.is_file() {
            f(&path, &metadata);
        }
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .context("HOME is not set; pass --cache-dir to choose a cache location")
}

fn normalized_set(values: &Option<HashSet<String>>) -> Vec<String> {
    let mut values: Vec<_> = values
        .as_ref()
        .map(|set| set.iter().cloned().collect())
        .unwrap_or_default();
    values.sort();
    values
}

fn cache_hash<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

fn current_file_signatures(
    root: &Path,
    extensions: Option<&HashSet<String>>,
    ignore: Option<&HashSet<String>>,
    include_text_files: bool,
) -> Result<Vec<FileSignature>> {
    let extensions = filter_extensions(extensions.cloned(), include_text_files);
    let files = walk_files(root, &extensions, ignore);
    files
        .par_iter()
        .map(|path| {
            let metadata = fs::metadata(path)?;
            let rel_path = path.strip_prefix(root).unwrap_or(path);
            let modified_ns = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            Ok(FileSignature {
                path: rel_path.to_string_lossy().to_string(),
                len: metadata.len(),
                modified_ns,
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn encode_chunks_batched(model: &dyn Encoder, chunks: &[Chunk]) -> Array2<f32> {
    let mut embeddings = Array2::<f32>::zeros((chunks.len(), model.dim()));
    for (batch_idx, batch) in chunks.chunks(EMBED_BATCH_SIZE).enumerate() {
        let start = batch_idx * EMBED_BATCH_SIZE;
        let end = start + batch.len();
        let texts: Vec<String> = batch.iter().map(semantic_text).collect();
        let encoded = model.encode(&texts);
        embeddings.slice_mut(s![start..end, ..]).assign(&encoded);
    }
    embeddings
}

fn semantic_text(chunk: &Chunk) -> String {
    let symbols = chunk
        .symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "path: {}\nlanguage: {}\nsymbols: {}\ncontext: {}\n\n{}",
        chunk.file_path,
        chunk.language.as_deref().unwrap_or("unknown"),
        symbols,
        chunk.breadcrumbs.join(" > "),
        chunk.content,
    )
}

fn empty_to_none(values: &[String]) -> Option<&[String]> {
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

pub fn create_chunks_from_path(
    path: &Path,
    extensions: Option<HashSet<String>>,
    ignore: Option<&HashSet<String>>,
    include_text_files: bool,
    display_root: &Path,
) -> Result<Vec<Chunk>> {
    Ok(create_chunks_from_path_with_warnings(
        path,
        extensions,
        ignore,
        include_text_files,
        display_root,
    )?
    .0)
}

fn create_chunks_from_path_with_warnings(
    path: &Path,
    extensions: Option<HashSet<String>>,
    ignore: Option<&HashSet<String>>,
    include_text_files: bool,
    display_root: &Path,
) -> Result<(Vec<Chunk>, Vec<IndexWarning>)> {
    let extensions = filter_extensions(extensions, include_text_files);
    let files = walk_files(path, &extensions, ignore);
    let chunks_by_file: Vec<(Vec<Chunk>, Option<IndexWarning>)> = files
        .par_iter()
        .map(|file_path| {
            let rel_path: PathBuf = file_path
                .strip_prefix(display_root)
                .unwrap_or(file_path)
                .to_path_buf();
            let chunk_path = rel_path.to_string_lossy().to_string();
            let source = match fs::read_to_string(file_path) {
                Ok(source) => source,
                Err(err) => {
                    return Ok((
                        Vec::new(),
                        Some(IndexWarning {
                            path: chunk_path,
                            message: format!("skipped indexed file: {err}"),
                        }),
                    ));
                }
            };
            let language = language_for_path(file_path).map(str::to_owned);
            Ok((chunk_source(&source, &chunk_path, language), None))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut warnings = Vec::new();
    let chunks: Vec<Chunk> = chunks_by_file
        .into_iter()
        .flat_map(|(chunks, warning)| {
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            chunks
        })
        .collect();
    if chunks.is_empty() {
        bail!("No supported files found under {}.", path.display());
    }
    Ok((chunks, warnings))
}

fn query_cache_entry_limit() -> usize {
    std::env::var("SIFS_QUERY_CACHE_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_QUERY_CACHE_ENTRIES)
}

fn normalize_filter_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|segment| *segment != "..") {
                    parts.pop();
                } else {
                    parts.push(part);
                }
            }
            value => parts.push(value),
        }
    }
    parts.join("/")
}

fn populate_mapping(chunks: &[Chunk]) -> IndexMappings {
    let mut file_mapping: HashMap<String, Vec<usize>> = HashMap::new();
    let mut language_mapping: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, chunk) in chunks.iter().enumerate() {
        file_mapping
            .entry(chunk.file_path.clone())
            .or_default()
            .push(idx);
        if let Some(language) = &chunk.language {
            language_mapping
                .entry(language.clone())
                .or_default()
                .push(idx);
        }
    }
    (file_mapping, language_mapping)
}

fn outlines_from_chunks(
    file_mapping: &HashMap<String, Vec<usize>>,
    chunks: &[Chunk],
) -> HashMap<String, FileOutline> {
    file_mapping
        .iter()
        .filter_map(|(file_path, chunk_ids)| {
            symbol_index::file_outline(file_path, chunk_ids, chunks)
                .map(|outline| (file_path.clone(), outline))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chunk(content: &str) -> Chunk {
        Chunk {
            content: content.to_owned(),
            file_path: "src/lib.rs".to_owned(),
            start_line: 1,
            end_line: 1,
            language: Some("rust".to_owned()),
            symbols: Vec::new(),
            breadcrumbs: Vec::new(),
        }
    }

    #[test]
    fn query_cache_is_entry_capped() {
        let index =
            SifsIndex::from_chunks_with_semantic_config(None, vec![test_chunk("fn token() {}")])
                .unwrap();

        for idx in 0..(DEFAULT_QUERY_CACHE_ENTRIES + 10) {
            index
                .search_with(
                    &format!("token {idx}"),
                    &SearchOptions::new(1).with_mode(SearchMode::Bm25),
                )
                .unwrap();
        }

        assert!(index.query_cache_len() <= DEFAULT_QUERY_CACHE_ENTRIES);
    }
}
