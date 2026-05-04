use crate::chunker::chunk_source;
use crate::dense::DenseIndex;
use crate::file_walker::{filter_extensions, language_for_path, walk_files};
use crate::model2vec::{Encoder, ModelOptions, load_model_with_options};
use crate::search::{search_bm25, search_hybrid, search_semantic};
use crate::sparse::Bm25Index;
use crate::types::{
    CacheMode, Chunk, IndexOptions, IndexStats, IndexWarning, SearchMode, SearchOptions,
    SearchResult,
};
use anyhow::{Context, Result, bail};
use ndarray::{Array2, s};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

pub struct SifsIndex {
    bm25_index: Bm25Index,
    semantic_state: Mutex<Option<SemanticState>>,
    model_options: ModelOptions,
    pub chunks: Vec<Chunk>,
    file_mapping: HashMap<String, Vec<usize>>,
    language_mapping: HashMap<String, Vec<usize>>,
    search_cache: Mutex<HashMap<SearchCacheKey, Vec<SearchResult>>>,
    cache_dir: Option<PathBuf>,
    signatures: Option<Vec<FileSignature>>,
    cache_metadata: Option<CacheMetadata>,
    warnings: Vec<IndexWarning>,
}

struct SemanticState {
    model: Box<dyn Encoder>,
    index: DenseIndex,
}

const EMBED_BATCH_SIZE: usize = 1024;
const CACHE_VERSION: u32 = 3;
const CACHE_DIR: &str = ".sifs";
const SPARSE_CACHE_FILE: &str = "index-v3-sparse.bin";
const SEMANTIC_CACHE_PREFIX: &str = "semantic-v3";
const MAX_INDEXED_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SearchCacheKey {
    query: String,
    top_k: usize,
    mode: SearchMode,
    alpha_bits: Option<u32>,
    filter_languages: Option<Vec<String>>,
    filter_paths: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedIndexPayload {
    version: u32,
    metadata: CacheMetadata,
    signatures: Vec<FileSignature>,
    warnings: Vec<IndexWarning>,
    chunks: Vec<Chunk>,
    bm25_index: Bm25Index,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedSemanticPayload {
    version: u32,
    metadata: CacheMetadata,
    embedding_dim: usize,
    signatures: Vec<FileSignature>,
    semantic_index: DenseIndex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileSignature {
    path: String,
    len: u64,
    modified_ns: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CacheMetadata {
    sifs_version: String,
    cache_version: u32,
    root_hash: String,
    model: String,
    model_cache_key: String,
    include_text_files: bool,
    extensions: Vec<String>,
    ignore: Vec<String>,
    walker_options: WalkerCacheOptions,
    chunker: ChunkerCacheOptions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct WalkerCacheOptions {
    hidden: bool,
    parents: bool,
    ignore: bool,
    git_ignore: bool,
    git_exclude: bool,
    git_global: bool,
    require_git: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ChunkerCacheOptions {
    max_indexed_file_bytes: u64,
}

struct ChunkBuildOutput {
    chunks: Vec<Chunk>,
    warnings: Vec<IndexWarning>,
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
        Self::from_path_with_model_options_and_index_options(
            path,
            model_options,
            extensions,
            ignore,
            include_text_files,
            IndexOptions::default(),
        )
    }

    pub fn from_path_with_model_options_and_index_options(
        path: impl AsRef<Path>,
        model_options: ModelOptions,
        extensions: Option<HashSet<String>>,
        ignore: Option<HashSet<String>>,
        include_text_files: bool,
        index_options: IndexOptions,
    ) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("Path is not a directory: {}", path.display());
        }
        let root = path.canonicalize()?;
        let extensions = filter_extensions(extensions, include_text_files);
        let metadata = cache_metadata(
            &root,
            &model_options,
            &extensions,
            ignore.as_ref(),
            include_text_files,
        );
        let cache_dir = cache_dir_for(&root, index_options.cache_mode, &metadata);
        if let Some(cache_dir) = &cache_dir
            && let Some(payload) = load_cached_index_payload(&root, cache_dir, &metadata)
        {
            return Self::from_cached_parts(model_options, payload, Some(cache_dir.clone()));
        }
        let output =
            create_chunks_from_path_with_extensions(&root, extensions, ignore.as_ref(), &root)?;
        let mut index = Self::from_chunks_with_model_options_and_warnings(
            model_options,
            output.chunks,
            output.warnings,
        )?;
        if let Some(cache_dir) = cache_dir {
            index.attach_cache(cache_dir.clone(), &root, metadata);
            write_cached_index_payload(&root, &cache_dir, &index);
        }
        Ok(index)
    }

    pub fn clean_cache(path: impl AsRef<Path>, cache_mode: CacheMode) -> Result<bool> {
        let root = path.as_ref().canonicalize()?;
        match cache_mode {
            CacheMode::Off => Ok(false),
            CacheMode::Local => remove_cache_dir(root.join(CACHE_DIR)),
            CacheMode::Platform => {
                let Some(base) = platform_cache_base() else {
                    return Ok(false);
                };
                let prefix = stable_hash(root.to_string_lossy().as_ref());
                let mut removed = false;
                if let Ok(entries) = fs::read_dir(base.join("sifs")) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with(&prefix) {
                            removed |= remove_cache_dir(entry.path())?;
                        }
                    }
                }
                Ok(removed)
            }
        }
    }

    pub fn from_git(url: &str, ref_name: Option<&str>) -> Result<Self> {
        Self::from_git_with_model_options(url, ref_name, ModelOptions::default())
    }

    pub fn from_git_with_model_options(
        url: &str,
        ref_name: Option<&str>,
        model_options: ModelOptions,
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
        Self::from_path_with_model_options(tmp.path(), model_options, None, None, false)
    }

    pub fn from_chunks(model: Box<dyn Encoder>, chunks: Vec<Chunk>) -> Result<Self> {
        Self::from_chunks_with_semantic_state(ModelOptions::default(), model, chunks)
    }

    pub fn from_chunks_with_model_options(
        model_options: ModelOptions,
        chunks: Vec<Chunk>,
    ) -> Result<Self> {
        Self::from_chunks_with_model_options_and_warnings(model_options, chunks, Vec::new())
    }

    fn from_chunks_with_model_options_and_warnings(
        model_options: ModelOptions,
        chunks: Vec<Chunk>,
        warnings: Vec<IndexWarning>,
    ) -> Result<Self> {
        if chunks.is_empty() {
            bail!("No supported files found.");
        }
        let bm25_index = Bm25Index::build_from_chunks(&chunks);
        let (file_mapping, language_mapping) = populate_mapping(&chunks);
        Ok(Self {
            bm25_index,
            semantic_state: Mutex::new(None),
            model_options,
            chunks,
            file_mapping,
            language_mapping,
            search_cache: Mutex::new(HashMap::new()),
            cache_dir: None,
            signatures: None,
            cache_metadata: None,
            warnings,
        })
    }

    fn from_chunks_with_semantic_state(
        model_options: ModelOptions,
        model: Box<dyn Encoder>,
        chunks: Vec<Chunk>,
    ) -> Result<Self> {
        let index = Self::from_chunks_with_model_options(model_options, chunks)?;
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
        model_options: ModelOptions,
        payload: CachedIndexPayload,
        cache_dir: Option<PathBuf>,
    ) -> Result<Self> {
        if payload.chunks.is_empty() {
            bail!("No supported files found.");
        }
        let signatures = payload.signatures.clone();
        let metadata = payload.metadata.clone();
        let (file_mapping, language_mapping) = populate_mapping(&payload.chunks);
        Ok(Self {
            bm25_index: payload.bm25_index,
            semantic_state: Mutex::new(None),
            model_options,
            chunks: payload.chunks,
            file_mapping,
            language_mapping,
            search_cache: Mutex::new(HashMap::new()),
            cache_dir,
            signatures: Some(signatures),
            cache_metadata: Some(metadata),
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

    pub fn warnings(&self) -> &[IndexWarning] {
        &self.warnings
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
        };
        if let Some(results) = self
            .search_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&cache_key).cloned())
        {
            return Ok(results);
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
                    options.top_k,
                    options.alpha,
                    selector_ref,
                )
            }
        };
        if let Ok(mut cache) = self.search_cache.lock() {
            cache.insert(cache_key, results.clone());
        }
        Ok(results)
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
        );
        results.retain(|r| r.chunk != *source);
        results.truncate(top_k);
        Ok(results)
    }

    pub fn semantic_loaded(&self) -> bool {
        self.semantic_state
            .lock()
            .map(|state| state.is_some())
            .unwrap_or(false)
    }

    fn ensure_semantic(&self) -> Result<std::sync::MutexGuard<'_, Option<SemanticState>>> {
        let mut state = self
            .semantic_state
            .lock()
            .map_err(|_| anyhow::anyhow!("semantic index lock is poisoned"))?;
        if state.is_none() {
            let model = load_model_with_options(&self.model_options)?;
            let semantic_index =
                self.load_cached_semantic_index(model.dim())
                    .unwrap_or_else(|| {
                        let embeddings = encode_chunks_batched(model.as_ref(), &self.chunks);
                        let semantic_index = DenseIndex::new(embeddings);
                        self.write_cached_semantic_index(&semantic_index);
                        semantic_index
                    });
            *state = Some(SemanticState {
                model,
                index: semantic_index,
            });
        }
        Ok(state)
    }

    fn attach_cache(&mut self, cache_dir: PathBuf, root: &Path, metadata: CacheMetadata) {
        self.signatures = current_file_signatures(root, &metadata).ok();
        self.cache_dir = Some(cache_dir);
        self.cache_metadata = Some(metadata);
    }

    fn load_cached_semantic_index(&self, embedding_dim: usize) -> Option<DenseIndex> {
        let cache_dir = self.cache_dir.as_ref()?;
        let signatures = self.signatures.as_ref()?;
        let metadata = self.cache_metadata.as_ref()?;
        let bytes = fs::read(semantic_cache_path(cache_dir, &self.model_options)).ok()?;
        let (payload, _): (CachedSemanticPayload, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
        (payload.version == CACHE_VERSION
            && payload.metadata == *metadata
            && payload.embedding_dim == embedding_dim
            && payload.signatures == *signatures)
            .then_some(payload.semantic_index)
    }

    fn write_cached_semantic_index(&self, semantic_index: &DenseIndex) {
        let Some(cache_dir) = self.cache_dir.as_ref() else {
            return;
        };
        let Some(signatures) = self.signatures.as_ref() else {
            return;
        };
        let Some(metadata) = self.cache_metadata.as_ref() else {
            return;
        };
        let payload = CachedSemanticPayload {
            version: CACHE_VERSION,
            metadata: metadata.clone(),
            embedding_dim: semantic_index.dim(),
            signatures: signatures.clone(),
            semantic_index: semantic_index.clone(),
        };
        let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
            return;
        };
        let cache_path = semantic_cache_path(cache_dir, &self.model_options);
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
        let mut selected: Option<Vec<usize>> = None;

        if let Some(languages) = filter_languages {
            selected = Some(sorted_set(
                languages
                    .iter()
                    .filter_map(|language| self.language_mapping.get(language))
                    .flatten()
                    .copied(),
            ));
        }

        if let Some(paths) = filter_paths {
            let path_ids = sorted_set(
                paths
                    .iter()
                    .filter_map(|path| self.file_mapping.get(path))
                    .flatten()
                    .copied(),
            );
            selected = Some(match selected {
                None => path_ids,
                Some(existing) => {
                    let path_ids: HashSet<_> = path_ids.into_iter().collect();
                    existing
                        .into_iter()
                        .filter(|id| path_ids.contains(id))
                        .collect()
                }
            });
        }

        selected
    }
}

fn sorted_set(ids: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let mut ids: Vec<_> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn load_cached_index_payload(
    root: &Path,
    cache_dir: &Path,
    metadata: &CacheMetadata,
) -> Option<CachedIndexPayload> {
    let signatures = current_file_signatures(root, metadata).ok()?;
    let bytes = fs::read(cache_path(cache_dir)).ok()?;
    let (payload, _): (CachedIndexPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    (payload.version == CACHE_VERSION
        && payload.metadata == *metadata
        && payload.signatures == signatures)
        .then_some(payload)
}

fn write_cached_index_payload(root: &Path, cache_dir: &Path, index: &SifsIndex) {
    let Some(metadata) = index.cache_metadata.as_ref() else {
        return;
    };
    let Ok(signatures) = current_file_signatures(root, metadata) else {
        return;
    };
    let payload = CachedIndexPayload {
        version: CACHE_VERSION,
        metadata: metadata.clone(),
        signatures,
        warnings: index.warnings.clone(),
        chunks: index.chunks.clone(),
        bm25_index: index.bm25_index.clone(),
    };
    let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
        return;
    };
    let cache_path = cache_path(cache_dir);
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

fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(SPARSE_CACHE_FILE)
}

fn semantic_cache_path(cache_dir: &Path, model_options: &ModelOptions) -> PathBuf {
    cache_dir.join(format!(
        "{SEMANTIC_CACHE_PREFIX}-{}.bin",
        model_options.cache_key()
    ))
}

fn cache_dir_for(root: &Path, mode: CacheMode, metadata: &CacheMetadata) -> Option<PathBuf> {
    match mode {
        CacheMode::Off => None,
        CacheMode::Local => Some(root.join(CACHE_DIR)),
        CacheMode::Platform => platform_cache_base().map(|base| {
            base.join("sifs").join(format!(
                "{}-{}",
                metadata.root_hash,
                stable_hash(&metadata_fingerprint(metadata))
            ))
        }),
    }
}

fn platform_cache_base() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Caches"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".cache"))
    }
}

fn remove_cache_dir(path: PathBuf) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(path)?;
    Ok(true)
}

fn cache_metadata(
    root: &Path,
    model_options: &ModelOptions,
    extensions: &HashSet<String>,
    ignore: Option<&HashSet<String>>,
    include_text_files: bool,
) -> CacheMetadata {
    CacheMetadata {
        sifs_version: env!("CARGO_PKG_VERSION").to_owned(),
        cache_version: CACHE_VERSION,
        root_hash: stable_hash(root.to_string_lossy().as_ref()),
        model: model_options.model.clone(),
        model_cache_key: model_options.cache_key(),
        include_text_files,
        extensions: sorted_strings(extensions.iter().cloned()),
        ignore: sorted_strings(ignore.into_iter().flatten().cloned()),
        walker_options: WalkerCacheOptions {
            hidden: true,
            parents: true,
            ignore: true,
            git_ignore: true,
            git_exclude: true,
            git_global: true,
            require_git: false,
        },
        chunker: ChunkerCacheOptions {
            max_indexed_file_bytes: MAX_INDEXED_FILE_BYTES,
        },
    }
}

fn metadata_fingerprint(metadata: &CacheMetadata) -> String {
    serde_json::to_string(metadata).unwrap_or_else(|_| format!("{metadata:?}"))
}

fn sorted_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut values: Vec<_> = values.into_iter().collect();
    values.sort();
    values.dedup();
    values
}

fn stable_hash(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn current_file_signatures(root: &Path, metadata: &CacheMetadata) -> Result<Vec<FileSignature>> {
    let extensions: HashSet<String> = metadata.extensions.iter().cloned().collect();
    let ignore: HashSet<String> = metadata.ignore.iter().cloned().collect();
    let files = walk_files(root, &extensions, Some(&ignore));
    files
        .into_iter()
        .map(|path| {
            let metadata = fs::metadata(&path)?;
            let rel_path = path.strip_prefix(root).unwrap_or(&path);
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
        .collect()
}

fn encode_chunks_batched(model: &dyn Encoder, chunks: &[Chunk]) -> Array2<f32> {
    let mut embeddings = Array2::<f32>::zeros((chunks.len(), model.dim()));
    for (batch_idx, batch) in chunks.chunks(EMBED_BATCH_SIZE).enumerate() {
        let start = batch_idx * EMBED_BATCH_SIZE;
        let end = start + batch.len();
        let texts: Vec<String> = batch.iter().map(|chunk| chunk.content.clone()).collect();
        let encoded = model.encode(&texts);
        embeddings.slice_mut(s![start..end, ..]).assign(&encoded);
    }
    embeddings
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
    let extensions = filter_extensions(extensions, include_text_files);
    Ok(create_chunks_from_path_with_extensions(path, extensions, ignore, display_root)?.chunks)
}

fn create_chunks_from_path_with_extensions(
    path: &Path,
    extensions: HashSet<String>,
    ignore: Option<&HashSet<String>>,
    display_root: &Path,
) -> Result<ChunkBuildOutput> {
    let files = walk_files(path, &extensions, ignore);
    let results: Vec<(Vec<Chunk>, Option<IndexWarning>)> = files
        .par_iter()
        .map(
            |file_path| match read_indexed_source(file_path, display_root) {
                Ok(source) => {
                    let rel_path: PathBuf = file_path
                        .strip_prefix(display_root)
                        .unwrap_or(file_path)
                        .to_path_buf();
                    let chunk_path = rel_path.to_string_lossy().to_string();
                    let language = language_for_path(file_path).map(str::to_owned);
                    (chunk_source(&source, &chunk_path, language), None)
                }
                Err(warning) => (Vec::new(), Some(warning)),
            },
        )
        .collect();
    let mut chunks = Vec::new();
    let mut warnings = Vec::new();
    for (file_chunks, warning) in results {
        chunks.extend(file_chunks);
        if let Some(warning) = warning {
            warnings.push(warning);
        }
    }
    if chunks.is_empty() {
        bail!("No supported files found under {}.", path.display());
    }
    Ok(ChunkBuildOutput { chunks, warnings })
}

fn read_indexed_source(
    file_path: &Path,
    display_root: &Path,
) -> std::result::Result<String, IndexWarning> {
    let warning_path = file_path
        .strip_prefix(display_root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();
    let metadata = fs::metadata(file_path).map_err(|err| IndexWarning {
        path: warning_path.clone(),
        message: format!("skipped unreadable file metadata: {err}"),
    })?;
    if metadata.len() > MAX_INDEXED_FILE_BYTES {
        return Err(IndexWarning {
            path: warning_path,
            message: format!("skipped file larger than {MAX_INDEXED_FILE_BYTES} bytes"),
        });
    }
    fs::read_to_string(file_path).map_err(|err| IndexWarning {
        path: warning_path,
        message: format!("skipped unreadable or non-UTF-8 file: {err}"),
    })
}

fn populate_mapping(
    chunks: &[Chunk],
) -> (HashMap<String, Vec<usize>>, HashMap<String, Vec<usize>>) {
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
