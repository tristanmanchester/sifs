use crate::chunker::chunk_source;
use crate::dense::DenseIndex;
use crate::file_walker::{filter_extensions, language_for_path, walk_files};
use crate::model2vec::{Encoder, load_model};
use crate::search::{search_bm25, search_hybrid, search_semantic};
use crate::sparse::Bm25Index;
use crate::types::{Chunk, IndexStats, SearchMode, SearchResult};
use anyhow::{Context, Result, bail};
use ndarray::{Array2, s};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

pub struct SifsIndex {
    model: Box<dyn Encoder>,
    bm25_index: Bm25Index,
    semantic_index: DenseIndex,
    pub chunks: Vec<Chunk>,
    file_mapping: HashMap<String, Vec<usize>>,
    language_mapping: HashMap<String, Vec<usize>>,
    search_cache: Mutex<HashMap<SearchCacheKey, Vec<SearchResult>>>,
}

const EMBED_BATCH_SIZE: usize = 1024;
const CACHE_VERSION: u32 = 1;
const CACHE_DIR: &str = ".sifs";
const CACHE_FILE: &str = "index-v1.bin";

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
        let path = path.as_ref();
        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("Path is not a directory: {}", path.display());
        }
        let root = path.canonicalize()?;
        let cacheable =
            model_path.is_none() && extensions.is_none() && ignore.is_none() && !include_text_files;
        if cacheable {
            let (model, cached) = rayon::join(
                || load_model(model_path),
                || load_cached_index_payload(&root),
            );
            if let Some(payload) = cached {
                return Self::from_cached_parts(model?, payload);
            }
        }
        let (model, chunks) = rayon::join(
            || load_model(model_path),
            || {
                create_chunks_from_path(
                    &root,
                    extensions,
                    ignore.as_ref(),
                    include_text_files,
                    &root,
                )
            },
        );
        let model = model?;
        let chunks = chunks?;
        let index = Self::from_chunks(model, chunks)?;
        if cacheable {
            write_cached_index_payload(&root, &index);
        }
        Ok(index)
    }

    pub fn from_git(url: &str, ref_name: Option<&str>) -> Result<Self> {
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
        Self::from_path(tmp.path())
    }

    pub fn from_chunks(model: Box<dyn Encoder>, chunks: Vec<Chunk>) -> Result<Self> {
        if chunks.is_empty() {
            bail!("No supported files found.");
        }
        let bm25_index = Bm25Index::build_from_chunks(&chunks);
        let embeddings = encode_chunks_batched(model.as_ref(), &chunks);
        let semantic_index = DenseIndex::new(embeddings);
        let (file_mapping, language_mapping) = populate_mapping(&chunks);
        Ok(Self {
            model,
            bm25_index,
            semantic_index,
            chunks,
            file_mapping,
            language_mapping,
            search_cache: Mutex::new(HashMap::new()),
        })
    }

    fn from_cached_parts(model: Box<dyn Encoder>, payload: CachedIndexPayload) -> Result<Self> {
        if payload.chunks.is_empty() {
            bail!("No supported files found.");
        }
        let (file_mapping, language_mapping) = populate_mapping(&payload.chunks);
        Ok(Self {
            model,
            bm25_index: payload.bm25_index,
            semantic_index: payload.semantic_index,
            chunks: payload.chunks,
            file_mapping,
            language_mapping,
            search_cache: Mutex::new(HashMap::new()),
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

    pub fn search(
        &self,
        query: &str,
        top_k: usize,
        mode: SearchMode,
        alpha: Option<f32>,
        filter_languages: Option<&[String]>,
        filter_paths: Option<&[String]>,
    ) -> Vec<SearchResult> {
        if self.chunks.is_empty() || query.trim().is_empty() {
            return Vec::new();
        }
        let cache_key = SearchCacheKey {
            query: query.to_owned(),
            top_k,
            mode,
            alpha_bits: alpha.map(f32::to_bits),
            filter_languages: filter_languages.map(<[String]>::to_vec),
            filter_paths: filter_paths.map(<[String]>::to_vec),
        };
        if let Some(results) = self
            .search_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&cache_key).cloned())
        {
            return results;
        }
        let selector = self.selector(filter_languages, filter_paths);
        let selector_ref = selector.as_deref();
        let results = match mode {
            SearchMode::Bm25 => {
                search_bm25(query, &self.bm25_index, &self.chunks, top_k, selector_ref)
            }
            SearchMode::Semantic => search_semantic(
                query,
                self.model.as_ref(),
                &self.semantic_index,
                &self.chunks,
                top_k,
                selector_ref,
            ),
            SearchMode::Hybrid => search_hybrid(
                query,
                self.model.as_ref(),
                &self.semantic_index,
                &self.bm25_index,
                &self.chunks,
                top_k,
                alpha,
                selector_ref,
            ),
        };
        if let Ok(mut cache) = self.search_cache.lock() {
            cache.insert(cache_key, results.clone());
        }
        results
    }

    pub fn find_related(&self, source: &Chunk, top_k: usize) -> Vec<SearchResult> {
        let filter_languages = source.language.as_ref().map(|l| vec![l.clone()]);
        let selector = self.selector(filter_languages.as_deref(), None);
        let mut results = search_semantic(
            &source.content,
            self.model.as_ref(),
            &self.semantic_index,
            &self.chunks,
            top_k + 1,
            selector.as_deref(),
        );
        results.retain(|r| r.chunk != *source);
        results.truncate(top_k);
        results
    }

    fn selector(
        &self,
        filter_languages: Option<&[String]>,
        filter_paths: Option<&[String]>,
    ) -> Option<Vec<usize>> {
        let mut ids = Vec::new();
        if let Some(languages) = filter_languages {
            for language in languages {
                if let Some(values) = self.language_mapping.get(language) {
                    ids.extend(values);
                }
            }
        }
        if let Some(paths) = filter_paths {
            for path in paths {
                if let Some(values) = self.file_mapping.get(path) {
                    ids.extend(values);
                }
            }
        }
        if ids.is_empty() {
            None
        } else {
            ids.sort_unstable();
            ids.dedup();
            Some(ids)
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
        semantic_index: index.semantic_index.clone(),
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
    root.join(CACHE_DIR).join(CACHE_FILE)
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

pub fn create_chunks_from_path(
    path: &Path,
    extensions: Option<HashSet<String>>,
    ignore: Option<&HashSet<String>>,
    include_text_files: bool,
    display_root: &Path,
) -> Result<Vec<Chunk>> {
    let extensions = filter_extensions(extensions, include_text_files);
    let files = walk_files(path, &extensions, ignore);
    let chunks: Vec<Chunk> = files
        .par_iter()
        .map(|file_path| {
            let Ok(source) = fs::read_to_string(file_path) else {
                return Vec::new();
            };
            let rel_path: PathBuf = file_path
                .strip_prefix(display_root)
                .unwrap_or(file_path)
                .to_path_buf();
            let chunk_path = rel_path.to_string_lossy().to_string();
            let language = language_for_path(file_path).map(str::to_owned);
            chunk_source(&source, &chunk_path, language)
        })
        .flatten()
        .collect();
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
