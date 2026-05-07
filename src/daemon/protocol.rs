use crate::index::CacheConfig;
use crate::model2vec::{EncoderSpec, ModelLoadPolicy, ModelOptions};
use crate::types::{Chunk, IndexStats, IndexWarning, SearchMode, SearchOptions, SearchResult};
use crate::utils::is_git_url;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const DAEMON_PROTOCOL_VERSION: u32 = 1;

pub fn daemon_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    LocalPath,
    GitUrl,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceSpec {
    pub kind: SourceKind,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_name: Option<String>,
}

impl SourceSpec {
    pub fn resolve(
        source: impl AsRef<str>,
        ref_name: Option<String>,
        offline: bool,
    ) -> Result<Self> {
        let source = source.as_ref();
        if is_git_url(source) {
            if offline {
                bail!("--offline does not allow remote Git sources");
            }
            return Ok(Self {
                kind: SourceKind::GitUrl,
                source: source.to_owned(),
                ref_name,
            });
        }

        let path = PathBuf::from(source);
        if !path.exists() {
            bail!("local source does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("local source is not a directory: {}", path.display());
        }
        Ok(Self {
            kind: SourceKind::LocalPath,
            source: path
                .canonicalize()
                .with_context(|| format!("canonicalize source {}", path.display()))?
                .to_string_lossy()
                .into_owned(),
            ref_name: None,
        })
    }

    pub fn current_dir(offline: bool) -> Result<Self> {
        let cwd = std::env::current_dir().context("resolve current directory")?;
        Self::resolve(cwd.to_string_lossy(), None, offline)
    }

    pub fn cache_key(&self) -> String {
        match (&self.kind, &self.ref_name) {
            (SourceKind::LocalPath, _) => format!("path:{}", self.source),
            (SourceKind::GitUrl, Some(ref_name)) => format!("git:{}@{}", self.source, ref_name),
            (SourceKind::GitUrl, None) => format!("git:{}", self.source),
        }
    }

    pub fn display(&self) -> String {
        match &self.ref_name {
            Some(ref_name) => format!("{}@{}", self.source, ref_name),
            None => self.source.clone(),
        }
    }

    pub fn as_path(&self) -> Option<&Path> {
        matches!(self.kind, SourceKind::LocalPath).then(|| Path::new(&self.source))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheConfigSpec {
    Platform,
    Project,
    Custom { path: PathBuf },
    Disabled,
}

impl From<&CacheConfig> for CacheConfigSpec {
    fn from(value: &CacheConfig) -> Self {
        match value {
            CacheConfig::Platform => Self::Platform,
            CacheConfig::Project => Self::Project,
            CacheConfig::Custom(path) => Self::Custom { path: path.clone() },
            CacheConfig::Disabled => Self::Disabled,
        }
    }
}

impl From<CacheConfigSpec> for CacheConfig {
    fn from(value: CacheConfigSpec) -> Self {
        match value {
            CacheConfigSpec::Platform => Self::Platform,
            CacheConfigSpec::Project => Self::Project,
            CacheConfigSpec::Custom { path } => Self::Custom(path),
            CacheConfigSpec::Disabled => Self::Disabled,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderSpecWire {
    Model2Vec {
        model: String,
        policy: ModelLoadPolicyWire,
    },
    Hashing {
        dim: usize,
    },
    Sparse,
}

impl EncoderSpecWire {
    pub fn from_encoder_spec(spec: Option<&EncoderSpec>) -> Self {
        match spec {
            Some(EncoderSpec::Model2Vec(options)) => Self::Model2Vec {
                model: options.model.clone(),
                policy: ModelLoadPolicyWire::from(options.policy),
            },
            Some(EncoderSpec::Hashing { dim }) => Self::Hashing { dim: *dim },
            None => Self::Sparse,
        }
    }

    pub fn into_encoder_spec(self) -> Option<EncoderSpec> {
        match self {
            Self::Model2Vec { model, policy } => Some(EncoderSpec::Model2Vec(ModelOptions {
                model,
                policy: policy.into(),
            })),
            Self::Hashing { dim } => Some(EncoderSpec::Hashing { dim }),
            Self::Sparse => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelLoadPolicyWire {
    AllowDownload,
    NoDownload,
    Offline,
}

impl From<ModelLoadPolicy> for ModelLoadPolicyWire {
    fn from(value: ModelLoadPolicy) -> Self {
        match value {
            ModelLoadPolicy::AllowDownload => Self::AllowDownload,
            ModelLoadPolicy::NoDownload => Self::NoDownload,
            ModelLoadPolicy::Offline => Self::Offline,
        }
    }
}

impl From<ModelLoadPolicyWire> for ModelLoadPolicy {
    fn from(value: ModelLoadPolicyWire) -> Self {
        match value {
            ModelLoadPolicyWire::AllowDownload => Self::AllowDownload,
            ModelLoadPolicyWire::NoDownload => Self::NoDownload,
            ModelLoadPolicyWire::Offline => Self::Offline,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRuntimeOptions {
    pub encoder: EncoderSpecWire,
    pub cache: CacheConfigSpec,
    pub extensions: Option<Vec<String>>,
    pub ignore: Option<Vec<String>>,
    pub include_text_files: bool,
}

impl Default for IndexRuntimeOptions {
    fn default() -> Self {
        Self {
            encoder: EncoderSpecWire::Model2Vec {
                model: ModelOptions::default().model,
                policy: ModelLoadPolicyWire::AllowDownload,
            },
            cache: CacheConfigSpec::Platform,
            extensions: None,
            ignore: None,
            include_text_files: false,
        }
    }
}

impl IndexRuntimeOptions {
    pub fn sparse(cache: CacheConfig) -> Self {
        Self {
            encoder: EncoderSpecWire::Sparse,
            cache: CacheConfigSpec::from(&cache),
            extensions: None,
            ignore: None,
            include_text_files: false,
        }
    }

    pub fn with_encoder(encoder: EncoderSpec, cache: CacheConfig) -> Self {
        Self {
            encoder: EncoderSpecWire::from_encoder_spec(Some(&encoder)),
            cache: CacheConfigSpec::from(&cache),
            extensions: None,
            ignore: None,
            include_text_files: false,
        }
    }

    pub fn extensions_set(&self) -> Option<HashSet<String>> {
        self.extensions
            .as_ref()
            .map(|items| items.iter().cloned().collect())
    }

    pub fn ignore_set(&self) -> Option<HashSet<String>> {
        self.ignore
            .as_ref()
            .map(|items| items.iter().cloned().collect())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IndexIdentity {
    pub source: SourceSpec,
    pub encoder_key: String,
    pub cache_key: String,
    pub extensions: Option<Vec<String>>,
    pub ignore: Option<Vec<String>>,
    pub include_text_files: bool,
}

impl IndexIdentity {
    pub fn new(source: SourceSpec, options: &IndexRuntimeOptions) -> Self {
        Self {
            source,
            encoder_key: encoder_key(&options.encoder),
            cache_key: cache_key(&options.cache),
            extensions: normalized_vec(options.extensions.clone()),
            ignore: normalized_vec(options.ignore.clone()),
            include_text_files: options.include_text_files,
        }
    }

    pub fn key(&self) -> String {
        serde_json::to_string(self).expect("index identity is serializable")
    }
}

fn encoder_key(encoder: &EncoderSpecWire) -> String {
    match encoder {
        EncoderSpecWire::Model2Vec { model, policy } => {
            format!("model2vec:{model}:{}", policy_key(*policy))
        }
        EncoderSpecWire::Hashing { dim } => format!("hashing:{dim}"),
        EncoderSpecWire::Sparse => "sparse".to_owned(),
    }
}

fn policy_key(policy: ModelLoadPolicyWire) -> &'static str {
    match policy {
        ModelLoadPolicyWire::AllowDownload => "allow-download",
        ModelLoadPolicyWire::NoDownload => "no-download",
        ModelLoadPolicyWire::Offline => "offline",
    }
}

fn cache_key(cache: &CacheConfigSpec) -> String {
    match cache {
        CacheConfigSpec::Platform => "platform".to_owned(),
        CacheConfigSpec::Project => "project".to_owned(),
        CacheConfigSpec::Custom { path } => format!("custom:{}", path.display()),
        CacheConfigSpec::Disabled => "disabled".to_owned(),
    }
}

fn normalized_vec(items: Option<Vec<String>>) -> Option<Vec<String>> {
    let mut items = items?;
    items.sort();
    items.dedup();
    Some(items)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DaemonRequestEnvelope {
    pub protocol_version: u32,
    pub request_id: String,
    pub request: DaemonRequest,
}

impl DaemonRequestEnvelope {
    pub fn new(request_id: impl Into<String>, request: DaemonRequest) -> Self {
        Self {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            request_id: request_id.into(),
            request,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonRequest {
    Ping,
    Status,
    IndexStatus {
        source: SourceSpec,
        options: IndexRuntimeOptions,
    },
    Search {
        source: SourceSpec,
        options: IndexRuntimeOptions,
        query: String,
        search: SearchOptionsWire,
    },
    FindRelated {
        source: SourceSpec,
        options: IndexRuntimeOptions,
        file_path: String,
        line: usize,
        top_k: usize,
    },
    ListFiles {
        source: SourceSpec,
        options: IndexRuntimeOptions,
        limit: usize,
    },
    GetChunk {
        source: SourceSpec,
        options: IndexRuntimeOptions,
        file_path: String,
        line: usize,
    },
    Refresh {
        source: SourceSpec,
        options: IndexRuntimeOptions,
    },
    Clear {
        source: SourceSpec,
        options: IndexRuntimeOptions,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchOptionsWire {
    pub top_k: usize,
    pub mode: SearchMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f32>,
    pub filter_languages: Vec<String>,
    pub filter_paths: Vec<String>,
    pub use_query_cache: bool,
    #[serde(default)]
    pub explain: bool,
}

impl From<SearchOptions> for SearchOptionsWire {
    fn from(value: SearchOptions) -> Self {
        Self {
            top_k: value.top_k,
            mode: value.mode,
            alpha: value.alpha,
            filter_languages: value.filter_languages,
            filter_paths: value.filter_paths,
            use_query_cache: value.use_query_cache,
            explain: value.explain,
        }
    }
}

impl From<SearchOptionsWire> for SearchOptions {
    fn from(value: SearchOptionsWire) -> Self {
        Self {
            top_k: value.top_k,
            mode: value.mode,
            alpha: value.alpha,
            filter_languages: value.filter_languages,
            filter_paths: value.filter_paths,
            use_query_cache: value.use_query_cache,
            explain: value.explain,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DaemonResponseEnvelope {
    pub protocol_version: u32,
    pub request_id: String,
    #[serde(flatten)]
    pub result: ResultEnvelope,
}

impl DaemonResponseEnvelope {
    pub fn ok(request_id: impl Into<String>, result: DaemonResult) -> Self {
        Self {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            request_id: request_id.into(),
            result: ResultEnvelope::Ok { result },
        }
    }

    pub fn error(request_id: impl Into<String>, error: DaemonError) -> Self {
        Self {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            request_id: request_id.into(),
            result: ResultEnvelope::Error { error },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultEnvelope {
    Ok { result: DaemonResult },
    Error { error: DaemonError },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonResult {
    Pong {
        version: String,
    },
    Status(DaemonStatus),
    IndexStatus(IndexStatusResult),
    Search(SearchResultSet),
    FindRelated(SearchResultSet),
    ListFiles {
        source: SourceSpec,
        total: usize,
        files: Vec<String>,
    },
    GetChunk {
        source: SourceSpec,
        chunk: Chunk,
    },
    Refresh(IndexStatusResult),
    Clear {
        source: SourceSpec,
        removed: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub version: String,
    pub protocol_version: u32,
    pub pid: u32,
    pub indexes: Vec<CachedIndexStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedIndexStatus {
    pub source: SourceSpec,
    pub stats: IndexStats,
    pub semantic_loaded: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IndexStatusResult {
    pub source: SourceSpec,
    pub stats: IndexStats,
    pub semantic_loaded: bool,
    pub warnings: Vec<IndexWarning>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResultSet {
    pub source: SourceSpec,
    pub query: String,
    pub mode: SearchMode,
    pub stats: IndexStats,
    pub elapsed_ms: u64,
    pub results: Vec<SearchResult>,
    pub warnings: Vec<IndexWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonError {
    pub code: String,
    pub message: String,
}

impl DaemonError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
