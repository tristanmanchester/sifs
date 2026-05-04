use crate::chunker::chunk_source;
use crate::dense::DenseIndex;
use crate::file_walker::{filter_extensions, language_for_path, walk_files};
use crate::model2vec::{Encoder, load_model};
use crate::search::{search_bm25, search_hybrid, search_semantic};
use crate::sparse::{Bm25Index, enrich_for_bm25};
use crate::types::{Chunk, IndexStats, SearchMode, SearchResult};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct SifsIndex {
    model: Box<dyn Encoder>,
    bm25_index: Bm25Index,
    semantic_index: DenseIndex,
    pub chunks: Vec<Chunk>,
    file_mapping: HashMap<String, Vec<usize>>,
    language_mapping: HashMap<String, Vec<usize>>,
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
        let model = load_model(model_path)?;
        let chunks = create_chunks_from_path(
            &root,
            extensions,
            ignore.as_ref(),
            include_text_files,
            &root,
        )?;
        Self::from_chunks(model, chunks)
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
        let docs: Vec<String> = chunks.iter().map(enrich_for_bm25).collect();
        let bm25_index = Bm25Index::build(&docs);
        let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
        let embeddings = model.encode(&texts);
        let semantic_index = DenseIndex::new(embeddings);
        let (file_mapping, language_mapping) = populate_mapping(&chunks);
        Ok(Self {
            model,
            bm25_index,
            semantic_index,
            chunks,
            file_mapping,
            language_mapping,
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
        let selector = self.selector(filter_languages, filter_paths);
        let selector_ref = selector.as_deref();
        match mode {
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
        }
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

pub fn create_chunks_from_path(
    path: &Path,
    extensions: Option<HashSet<String>>,
    ignore: Option<&HashSet<String>>,
    include_text_files: bool,
    display_root: &Path,
) -> Result<Vec<Chunk>> {
    let extensions = filter_extensions(extensions, include_text_files);
    let files = walk_files(path, &extensions, ignore);
    let mut chunks = Vec::new();
    for file_path in files {
        let Ok(source) = fs::read_to_string(&file_path) else {
            continue;
        };
        let rel_path: PathBuf = file_path
            .strip_prefix(display_root)
            .unwrap_or(&file_path)
            .to_path_buf();
        let chunk_path = rel_path.to_string_lossy().to_string();
        let language = language_for_path(&file_path).map(str::to_owned);
        chunks.extend(chunk_source(&source, &chunk_path, language));
    }
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
