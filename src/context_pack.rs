use crate::SifsIndex;
use crate::types::{Chunk, SearchMode, SearchResult};
use serde_json::{Value, json};
use std::collections::HashSet;

#[allow(clippy::too_many_arguments)]
pub fn context_pack_payload(
    index: &SifsIndex,
    source: &str,
    query: &str,
    mode: SearchMode,
    limit: usize,
    budget_tokens: usize,
    include_neighbors: usize,
    include_symbol_definitions: bool,
    results: &[SearchResult],
) -> Value {
    let mut remaining_chars = budget_tokens.saturating_mul(4);
    let mut seen_files = HashSet::new();
    let mut seen_chunks = HashSet::new();
    let mut packed = Vec::new();
    let mut state = PackState {
        packed: &mut packed,
        seen_chunks: &mut seen_chunks,
        remaining_chars: &mut remaining_chars,
    };
    let chunks = &index.chunks;
    let symbol_definition_chunks = if include_symbol_definitions {
        query_symbol_definition_chunks(query, chunks, results)
    } else {
        Vec::new()
    };
    let mut symbol_definition_terms = symbol_definition_terms(query, results)
        .into_iter()
        .collect::<Vec<_>>();
    symbol_definition_terms.sort();

    for result in results {
        if *state.remaining_chars == 0 || !seen_files.insert(result.chunk.file_path.clone()) {
            continue;
        }
        push_pack_chunk(
            &mut state,
            &result.chunk,
            Some(result.score),
            result.source.to_string(),
            "primary",
            format!("ranked by {mode} for the pack query"),
        );
        if let Some(header) = file_header_chunk(chunks, &result.chunk) {
            push_pack_chunk(
                &mut state,
                header,
                None,
                "file_header".to_owned(),
                "file_header",
                format!("file header context for {}", result.chunk.file_path),
            );
        }
        if include_neighbors > 0 {
            for neighbor in adjacent_chunks(chunks, &result.chunk, include_neighbors) {
                if *state.remaining_chars == 0 {
                    break;
                }
                push_pack_chunk(
                    &mut state,
                    neighbor,
                    None,
                    "adjacent".to_owned(),
                    "neighbor",
                    format!(
                        "adjacent context for {}:{}-{}",
                        result.chunk.file_path, result.chunk.start_line, result.chunk.end_line
                    ),
                );
            }
        }
        if include_symbol_definitions {
            for definition in &symbol_definition_chunks {
                if *state.remaining_chars == 0 {
                    break;
                }
                push_pack_chunk(
                    &mut state,
                    definition,
                    None,
                    "symbol".to_owned(),
                    "symbol_definition",
                    "defines a symbol named in the pack query".to_owned(),
                );
            }
        }
    }

    json!({
        "query": query,
        "source": source,
        "mode": mode.to_string(),
        "limit": limit,
        "budget_tokens": budget_tokens,
        "include_neighbors": include_neighbors,
        "include_symbol_definitions": include_symbol_definitions,
        "symbol_definition_terms": symbol_definition_terms,
        "estimated_tokens_used": budget_tokens.saturating_mul(4).saturating_sub(remaining_chars).div_ceil(4),
        "stats": index.stats(),
        "warnings": index.warnings(),
        "items": packed,
    })
}

fn file_header_chunk<'a>(chunks: &'a [Chunk], chunk: &Chunk) -> Option<&'a Chunk> {
    if chunk.start_line == 1 {
        return None;
    }
    chunks
        .iter()
        .find(|candidate| candidate.file_path == chunk.file_path && candidate.start_line == 1)
}

struct PackState<'a> {
    packed: &'a mut Vec<Value>,
    seen_chunks: &'a mut HashSet<(String, usize, usize)>,
    remaining_chars: &'a mut usize,
}

fn push_pack_chunk(
    state: &mut PackState,
    chunk: &Chunk,
    score: Option<f32>,
    source: String,
    kind: &str,
    why: String,
) {
    if *state.remaining_chars == 0
        || !state
            .seen_chunks
            .insert((chunk.file_path.clone(), chunk.start_line, chunk.end_line))
    {
        return;
    }
    let content = if chunk.content.len() > *state.remaining_chars {
        chunk
            .content
            .chars()
            .take(*state.remaining_chars)
            .collect::<String>()
    } else {
        chunk.content.clone()
    };
    *state.remaining_chars = state.remaining_chars.saturating_sub(content.len());
    state.packed.push(json!({
        "kind": kind,
        "file_path": chunk.file_path,
        "start_line": chunk.start_line,
        "end_line": chunk.end_line,
        "score": score,
        "source": source,
        "why": why,
        "symbols": chunk.symbols,
        "breadcrumbs": chunk.breadcrumbs,
        "content": content,
    }));
}

fn adjacent_chunks<'a>(
    chunks: &'a [Chunk],
    target: &Chunk,
    include_neighbors: usize,
) -> Vec<&'a Chunk> {
    let mut before: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| {
            chunk.file_path == target.file_path
                && chunk.start_line < target.start_line
                && !(chunk.start_line == target.start_line && chunk.end_line == target.end_line)
        })
        .collect();
    before.sort_by_key(|chunk| std::cmp::Reverse(chunk.end_line));
    let mut after: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| {
            chunk.file_path == target.file_path
                && chunk.start_line > target.start_line
                && !(chunk.start_line == target.start_line && chunk.end_line == target.end_line)
        })
        .collect();
    after.sort_by_key(|chunk| chunk.start_line);
    before
        .into_iter()
        .take(include_neighbors)
        .chain(after.into_iter().take(include_neighbors))
        .collect()
}

fn query_symbol_definition_chunks<'a>(
    query: &str,
    chunks: &'a [Chunk],
    results: &[SearchResult],
) -> Vec<&'a Chunk> {
    let terms = symbol_definition_terms(query, results);
    if terms.is_empty() {
        return Vec::new();
    }
    chunks
        .iter()
        .filter(|chunk| {
            chunk.symbols.iter().any(|symbol| {
                terms.contains(&symbol.name.to_ascii_lowercase())
                    && matches!(
                        symbol.kind.as_str(),
                        "class"
                            | "const"
                            | "def"
                            | "enum"
                            | "fn"
                            | "function"
                            | "impl"
                            | "interface"
                            | "struct"
                            | "trait"
                            | "type"
                    )
            })
        })
        .collect()
}

fn symbol_definition_terms(query: &str, results: &[SearchResult]) -> HashSet<String> {
    let mut terms = query
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .filter(|term| is_identifier_like_query_term(term))
        .map(|term| term.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if terms.is_empty() {
        return terms;
    }
    let explicit_terms = terms.clone();
    for result in results.iter().take(5) {
        for symbol in &result.chunk.symbols {
            let folded = symbol.name.to_ascii_lowercase();
            if explicit_terms.iter().any(|term| folded.contains(term)) {
                terms.insert(folded);
            }
        }
    }
    terms
}

fn is_identifier_like_query_term(term: &str) -> bool {
    let term = term.trim();
    term.len() >= 3
        && (term.contains('_')
            || term.contains('$')
            || term.chars().any(|c| c.is_ascii_uppercase())
            || term.chars().any(|c| c.is_ascii_digit()))
}
