use crate::chunker::chunk_source;
use crate::dense::DenseIndex;
use crate::file_walker::{filter_extensions, language_for_path, walk_files};
use crate::model2vec::{Encoder, ModelOptions, load_model_with_options};
use crate::search::{search_bm25, search_hybrid, search_semantic};
use crate::sparse::Bm25Index;
use crate::types::{Chunk, IndexStats, SearchMode, SearchOptions, SearchResult};
use anyhow::{Context, Result, bail};
use ndarray::{Array2, s};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
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
    cache_root: Option<PathBuf>,
    signatures: Option<Vec<FileSignature>>,
}

struct SemanticState {
    model: Box<dyn Encoder>,
    index: DenseIndex,
}

const EMBED_BATCH_SIZE: usize = 1024;
const CACHE_VERSION: u32 = 2;
const CACHE_DIR: &str = ".sifs";
const SPARSE_CACHE_FILE: &str = "index-v2-sparse.bin";
const SEMANTIC_CACHE_PREFIX: &str = "semantic-v2";

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
    signatures: Vec<FileSignature>,
    chunks: Vec<Chunk>,
    bm25_index: Bm25Index,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedSemanticPayload {
    version: u32,
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
        let path = path.as_ref();
        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("Path is not a directory: {}", path.display());
        }
        let root = path.canonicalize()?;
        let cacheable = extensions.is_none() && ignore.is_none() && !include_text_files;
        if cacheable {
            if let Some(payload) = load_cached_index_payload(&root) {
                return Self::from_cached_parts(model_options, payload, Some(root));
            }
        }
        let chunks = create_chunks_from_path(
            &root,
            extensions,
            ignore.as_ref(),
            include_text_files,
            &root,
        )?;
        let mut index = Self::from_chunks_with_model_options(model_options, chunks)?;
        if cacheable {
            index.attach_cache_root(root.clone());
            write_cached_index_payload(&root, &index);
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
            cache_root: None,
            signatures: None,
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
        cache_root: Option<PathBuf>,
    ) -> Result<Self> {
        if payload.chunks.is_empty() {
            bail!("No supported files found.");
        }
        let signatures = payload.signatures.clone();
        let (file_mapping, language_mapping) = populate_mapping(&payload.chunks);
        Ok(Self {
            bm25_index: payload.bm25_index,
            semantic_state: Mutex::new(None),
            model_options,
            chunks: payload.chunks,
            file_mapping,
            language_mapping,
            search_cache: Mutex::new(HashMap::new()),
            cache_root,
            signatures: Some(signatures),
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
            let semantic_index = self.load_cached_semantic_index().unwrap_or_else(|| {
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

    fn attach_cache_root(&mut self, root: PathBuf) {
        self.signatures = current_file_signatures(&root).ok();
        self.cache_root = Some(root);
    }

    fn load_cached_semantic_index(&self) -> Option<DenseIndex> {
        let root = self.cache_root.as_ref()?;
        let signatures = self.signatures.as_ref()?;
        let bytes = fs::read(semantic_cache_path(root, &self.model_options)).ok()?;
        let (payload, _): (CachedSemanticPayload, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
        (payload.version == CACHE_VERSION && payload.signatures == *signatures)
            .then_some(payload.semantic_index)
    }

    fn write_cached_semantic_index(&self, semantic_index: &DenseIndex) {
        let Some(root) = self.cache_root.as_ref() else {
            return;
        };
        let Some(signatures) = self.signatures.as_ref() else {
            return;
        };
        let payload = CachedSemanticPayload {
            version: CACHE_VERSION,
            signatures: signatures.clone(),
            semantic_index: semantic_index.clone(),
        };
        let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
            return;
        };
        let cache_path = semantic_cache_path(root, &self.model_options);
        if let Some(parent) = cache_path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
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
                if let Some(values) = self.file_mapping.get(path) {
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

fn load_cached_index_payload(root: &Path) -> Option<CachedIndexPayload> {
    let signatures = current_file_signatures(root).ok()?;
    let bytes = fs::read(cache_path(root)).ok()?;
    let (payload, _): (CachedIndexPayload, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    (payload.version == CACHE_VERSION && payload.signatures == signatures).then_some(payload)
}

fn write_cached_index_payload(root: &Path, index: &SifsIndex) {
    let Ok(signatures) = current_file_signatures(root) else {
        return;
    };
    let payload = CachedIndexPayload {
        version: CACHE_VERSION,
        signatures,
        chunks: index.chunks.clone(),
        bm25_index: index.bm25_index.clone(),
    };
    let Ok(bytes) = bincode::serde::encode_to_vec(&payload, bincode::config::standard()) else {
        return;
    };
    let cache_path = cache_path(root);
    if let Some(parent) = cache_path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let tmp_path = cache_path.with_extension("bin.tmp");
    if fs::write(&tmp_path, bytes).is_ok() {
        let _ = fs::rename(tmp_path, cache_path);
    }
}

fn cache_path(root: &Path) -> PathBuf {
    root.join(CACHE_DIR).join(SPARSE_CACHE_FILE)
}

fn semantic_cache_path(root: &Path, model_options: &ModelOptions) -> PathBuf {
    root.join(CACHE_DIR).join(format!(
        "{SEMANTIC_CACHE_PREFIX}-{}.bin",
        model_options.cache_key()
    ))
}

fn current_file_signatures(root: &Path) -> Result<Vec<FileSignature>> {
    let extensions = filter_extensions(None, false);
    let files = walk_files(root, &extensions, None);
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
    let files = walk_files(path, &extensions, ignore);
    let chunks_by_file: Vec<Vec<Chunk>> = files
        .par_iter()
        .map(|file_path| {
            let source = fs::read_to_string(file_path)
                .with_context(|| format!("read indexed file {}", file_path.display()))?;
            let rel_path: PathBuf = file_path
                .strip_prefix(display_root)
                .unwrap_or(file_path)
                .to_path_buf();
            let chunk_path = rel_path.to_string_lossy().to_string();
            let language = language_for_path(file_path).map(str::to_owned);
            Ok(chunk_source(&source, &chunk_path, language))
        })
        .collect::<Result<Vec<_>>>()?;
    let chunks: Vec<Chunk> = chunks_by_file.into_iter().flatten().collect();
    if chunks.is_empty() {
        bail!("No supported files found under {}.", path.display());
    }
    Ok(chunks)
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
