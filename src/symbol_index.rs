use crate::types::{Chunk, Symbol};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolPosting {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub file_path: String,
    #[serde(skip)]
    pub chunk_id: usize,
    pub role: String,
    pub confidence: String,
    pub origin: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub breadcrumbs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChunkOutline {
    pub chunk_id: usize,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    #[serde(default)]
    pub breadcrumbs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileOutline {
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub chunk_count: usize,
    pub symbol_count: usize,
    #[serde(default)]
    pub symbols: Vec<SymbolPosting>,
    #[serde(default)]
    pub chunks: Vec<FileChunkOutline>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SymbolIndex {
    exact: BTreeMap<String, Vec<SymbolPosting>>,
    folded: BTreeMap<String, Vec<SymbolPosting>>,
    chunk_mapping: HashMap<String, Vec<usize>>,
}

impl SymbolIndex {
    pub fn from_chunks(chunks: &[Chunk]) -> Self {
        let mut index = Self::default();
        for (chunk_id, chunk) in chunks.iter().enumerate() {
            for symbol in &chunk.symbols {
                let Some(posting) = posting_for(chunk_id, chunk, symbol) else {
                    continue;
                };
                index
                    .chunk_mapping
                    .entry(fold_symbol_key(&posting.name))
                    .or_default()
                    .push(chunk_id);
                index
                    .exact
                    .entry(posting.name.clone())
                    .or_default()
                    .push(posting.clone());
                index
                    .folded
                    .entry(fold_symbol_key(&posting.name))
                    .or_default()
                    .push(posting);
            }
        }
        for ids in index.chunk_mapping.values_mut() {
            ids.sort_unstable();
            ids.dedup();
        }
        for postings in index.exact.values_mut() {
            sort_postings(postings);
        }
        for postings in index.folded.values_mut() {
            sort_postings(postings);
        }
        index
    }

    pub fn from_postings(postings: Vec<SymbolPosting>) -> Self {
        let mut index = Self::default();
        for posting in postings {
            index
                .chunk_mapping
                .entry(fold_symbol_key(&posting.name))
                .or_default()
                .push(posting.chunk_id);
            index
                .exact
                .entry(posting.name.clone())
                .or_default()
                .push(posting.clone());
            index
                .folded
                .entry(fold_symbol_key(&posting.name))
                .or_default()
                .push(posting);
        }
        for ids in index.chunk_mapping.values_mut() {
            ids.sort_unstable();
            ids.dedup();
        }
        for postings in index.exact.values_mut() {
            sort_postings(postings);
        }
        for postings in index.folded.values_mut() {
            sort_postings(postings);
        }
        index
    }

    pub fn postings(&self) -> Vec<SymbolPosting> {
        self.exact
            .values()
            .flat_map(|postings| postings.iter().cloned())
            .collect()
    }

    pub fn lookup(&self, name: &str, limit: usize) -> Vec<SymbolPosting> {
        if name.trim().is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut postings = self.postings_for(name).to_vec();
        postings.truncate(limit);
        postings
    }

    pub fn lookup_total(&self, name: &str) -> usize {
        if name.trim().is_empty() {
            return 0;
        }
        self.postings_for(name).len()
    }

    pub fn chunk_mapping(&self) -> &HashMap<String, Vec<usize>> {
        &self.chunk_mapping
    }

    fn postings_for(&self, name: &str) -> &[SymbolPosting] {
        self.exact
            .get(name)
            .filter(|postings| !postings.is_empty())
            .or_else(|| self.folded.get(&fold_symbol_key(name)))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

pub fn file_outline(file_path: &str, chunk_ids: &[usize], chunks: &[Chunk]) -> Option<FileOutline> {
    if chunk_ids.is_empty() {
        return None;
    }
    let mut language_counts = BTreeMap::<String, usize>::new();
    let mut start_line = usize::MAX;
    let mut end_line = 0usize;
    let mut symbols = Vec::new();
    let mut outlined_chunks = Vec::new();
    for chunk_id in chunk_ids {
        let Some(chunk) = chunks.get(*chunk_id) else {
            continue;
        };
        if let Some(language) = &chunk.language {
            *language_counts.entry(language.clone()).or_default() += 1;
        }
        start_line = start_line.min(chunk.start_line);
        end_line = end_line.max(chunk.end_line);
        for symbol in &chunk.symbols {
            if let Some(posting) = posting_for(*chunk_id, chunk, symbol) {
                symbols.push(posting);
            }
        }
        outlined_chunks.push(FileChunkOutline {
            chunk_id: *chunk_id,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            symbols: chunk.symbols.clone(),
            breadcrumbs: chunk.breadcrumbs.clone(),
        });
    }
    if outlined_chunks.is_empty() {
        return None;
    }
    symbols.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    outlined_chunks.sort_by_key(|chunk| (chunk.start_line, chunk.end_line));
    let symbol_count = symbols.len();
    let language = language_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(language, _)| language);
    Some(FileOutline {
        file_path: file_path.to_owned(),
        language,
        start_line,
        end_line,
        chunk_count: outlined_chunks.len(),
        symbol_count,
        symbols,
        chunks: outlined_chunks,
    })
}

pub fn fold_symbol_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn posting_for(chunk_id: usize, chunk: &Chunk, symbol: &Symbol) -> Option<SymbolPosting> {
    let name = symbol.name.trim();
    if name.is_empty() {
        return None;
    }
    Some(SymbolPosting {
        name: name.to_owned(),
        kind: symbol.kind.clone(),
        line: symbol.line,
        file_path: chunk.file_path.clone(),
        chunk_id,
        role: symbol.role.clone(),
        confidence: symbol.confidence.clone(),
        origin: symbol.origin.clone(),
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        language: chunk.language.clone(),
        breadcrumbs: chunk.breadcrumbs.clone(),
    })
}

fn sort_postings(postings: &mut [SymbolPosting]) {
    postings.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
    });
}

#[cfg(test)]
mod tests {
    use super::SymbolIndex;
    use crate::types::{Chunk, Symbol};

    fn chunk(path: &str, start: usize, symbols: Vec<Symbol>) -> Chunk {
        Chunk {
            content: "content".to_owned(),
            file_path: path.to_owned(),
            start_line: start,
            end_line: start + 10,
            language: Some("rust".to_owned()),
            symbols,
            breadcrumbs: vec!["module".to_owned()],
        }
    }

    #[test]
    fn lookup_supports_exact_and_folded_symbol_names() {
        let index = SymbolIndex::from_chunks(&[chunk(
            "src/lib.rs",
            1,
            vec![Symbol::definition("TokenManager", "struct", 3)],
        )]);

        assert_eq!(index.lookup("TokenManager", 10)[0].name, "TokenManager");
        assert_eq!(index.lookup("tokenmanager", 10)[0].name, "TokenManager");
    }

    #[test]
    fn chunk_mapping_deduplicates_repeated_symbols_per_chunk() {
        let index = SymbolIndex::from_chunks(&[chunk(
            "src/lib.rs",
            1,
            vec![
                Symbol::definition("TokenManager", "struct", 3),
                Symbol::definition("TokenManager", "impl", 9),
            ],
        )]);

        assert_eq!(
            index.chunk_mapping().get("tokenmanager").unwrap(),
            &vec![0usize]
        );
        assert_eq!(index.lookup("TokenManager", 10).len(), 2);
    }
}
