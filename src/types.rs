use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CacheMode {
    #[default]
    Platform,
    Local,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexWarning {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chunk {
    pub content: String,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub language: Option<String>,
}

impl Chunk {
    pub fn location(&self) -> String {
        format!("{}:{}-{}", self.file_path, self.start_line, self.end_line)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchMode {
    Hybrid,
    Semantic,
    Bm25,
}

impl std::str::FromStr for SearchMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "hybrid" => Ok(Self::Hybrid),
            "semantic" => Ok(Self::Semantic),
            "bm25" => Ok(Self::Bm25),
            _ => Err(format!("Unknown search mode: {s}")),
        }
    }
}

impl std::fmt::Display for SearchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hybrid => write!(f, "hybrid"),
            Self::Semantic => write!(f, "semantic"),
            Self::Bm25 => write!(f, "bm25"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchOptions {
    pub top_k: usize,
    pub mode: SearchMode,
    pub alpha: Option<f32>,
    pub filter_languages: Vec<String>,
    pub filter_paths: Vec<String>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            top_k: 5,
            mode: SearchMode::Hybrid,
            alpha: None,
            filter_languages: Vec::new(),
            filter_paths: Vec::new(),
        }
    }
}

impl SearchOptions {
    pub fn new(top_k: usize) -> Self {
        Self {
            top_k,
            ..Self::default()
        }
    }

    pub fn with_mode(mut self, mode: SearchMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = Some(alpha);
        self
    }

    pub fn with_languages(mut self, languages: impl IntoIterator<Item = String>) -> Self {
        self.filter_languages = languages.into_iter().collect();
        self
    }

    pub fn with_paths(mut self, paths: impl IntoIterator<Item = String>) -> Self {
        self.filter_paths = paths.into_iter().collect();
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub chunk: Chunk,
    pub score: f32,
    pub source: SearchMode,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexStats {
    pub indexed_files: usize,
    pub total_chunks: usize,
    pub languages: std::collections::BTreeMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexOptions {
    pub cache_mode: CacheMode,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            cache_mode: CacheMode::Platform,
        }
    }
}
