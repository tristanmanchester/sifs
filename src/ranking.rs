use crate::tokens::split_identifier;
use crate::types::Chunk;
use once_cell::sync::Lazy;
use regex::Regex;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

static SYMBOL_QUERY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?:[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\\|->|\.)[A-Za-z_][A-Za-z0-9_]*)+|_[A-Za-z0-9_]*|[A-Za-z][A-Za-z0-9]*[A-Z_][A-Za-z0-9_]*|[A-Z][A-Za-z0-9]*)$").unwrap()
});
static EMBEDDED_SYMBOL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"\b(?:[A-Z][a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]*|[a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]+)\b",
    )
    .unwrap()
});
static TEST_FILE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:^|/)(?:test_[^/]*\.py|[^/]*_test\.py|[^/]*_test\.go|[^/]*Tests?\.java|[^/]*Test\.php|[^/]*_spec\.rb|[^/]*_test\.rb|[^/]*\.test\.[jt]sx?|[^/]*\.spec\.[jt]sx?|[^/]*Tests?\.kt|[^/]*Spec\.kt|[^/]*Tests?\.swift|[^/]*Spec\.swift|[^/]*Tests?\.cs|test_[^/]*\.cpp|[^/]*_test\.cpp|test_[^/]*\.c|[^/]*_test\.c|[^/]*Spec\.scala|[^/]*Suite\.scala|[^/]*Test\.scala|[^/]*_test\.dart|test_[^/]*\.dart|[^/]*_spec\.lua|[^/]*_test\.lua|test_[^/]*\.lua|test_helpers?[^/]*\.\w+)$").unwrap()
});
static TEST_DIR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:^|/)(?:tests?|__tests__|spec|testing)(?:/|$)").unwrap());
static COMPAT_DIR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:^|/)(?:compat|_compat|legacy)(?:/|$)").unwrap());
static EXAMPLES_DIR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:^|/)(?:_?examples?|docs?_src)(?:/|$)").unwrap());
static TYPE_DEFS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\.d\.ts$").unwrap());
static STOPWORDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    "a an and are as at be by do does for from has have how if in is it not of on or the to was what when where which who why with"
        .split_whitespace()
        .collect()
});

const DEFINITION_KEYWORDS: &[&str] = &[
    "class",
    "module",
    "defmodule",
    "def",
    "interface",
    "struct",
    "enum",
    "trait",
    "type",
    "func",
    "function",
    "object",
    "abstract class",
    "data class",
    "fn",
    "fun",
    "package",
    "namespace",
    "protocol",
    "record",
    "typedef",
];
const SQL_DEFINITION_KEYWORDS: &[&str] = &[
    "CREATE TABLE",
    "CREATE VIEW",
    "CREATE PROCEDURE",
    "CREATE FUNCTION",
];

pub fn resolve_alpha(query: &str, alpha: Option<f32>) -> f32 {
    alpha.unwrap_or_else(|| {
        if is_symbol_query(query) {
            0.3
        } else if is_architecture_query(query) {
            0.55
        } else {
            0.3
        }
    })
}

pub fn is_symbol_query(query: &str) -> bool {
    SYMBOL_QUERY_RE.is_match(query.trim())
}

fn is_architecture_query(query: &str) -> bool {
    let lowered = query.trim().to_lowercase();
    lowered.starts_with("how ")
        || lowered.starts_with("how does ")
        || lowered.starts_with("how are ")
}

pub fn boost_multi_chunk_files(scores: &mut HashMap<usize, f32>, chunks: &[Chunk]) {
    if scores.is_empty() {
        return;
    }
    let max_score = scores.values().copied().fold(0.0f32, f32::max);
    if max_score == 0.0 {
        return;
    }
    let mut file_sum: HashMap<&str, f32> = HashMap::new();
    let mut best_chunk: HashMap<&str, usize> = HashMap::new();
    for (&chunk_id, &score) in scores.iter() {
        let file = chunks[chunk_id].file_path.as_str();
        *file_sum.entry(file).or_default() += score;
        if best_chunk
            .get(file)
            .is_none_or(|&best| score > scores[&best])
        {
            best_chunk.insert(file, chunk_id);
        }
    }
    let max_file_sum = file_sum.values().copied().fold(0.0f32, f32::max);
    let boost_unit = max_score * 0.2;
    for (file, chunk_id) in best_chunk {
        if let Some(score) = scores.get_mut(&chunk_id) {
            *score += boost_unit * file_sum[file] / max_file_sum;
        }
    }
}

pub fn apply_query_boost(
    scores: &HashMap<usize, f32>,
    query: &str,
    chunks: &[Chunk],
) -> HashMap<usize, f32> {
    if scores.is_empty() {
        return scores.clone();
    }
    let max_score = scores.values().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut boosted = scores.clone();
    if is_symbol_query(query) {
        boost_symbol_definitions(&mut boosted, query, max_score, chunks);
    } else {
        boost_stem_matches(&mut boosted, query, max_score, chunks);
        boost_embedded_symbols(&mut boosted, query, max_score, chunks);
    }
    boosted
}

fn boost_symbol_definitions(
    boosted: &mut HashMap<usize, f32>,
    query: &str,
    max_score: f32,
    chunks: &[Chunk],
) {
    let symbol_name = extract_symbol_name(query);
    let mut names = HashSet::from([symbol_name.clone()]);
    if symbol_name != query.trim() {
        names.insert(query.trim().to_owned());
    }
    let matchers = definition_matchers(&names);
    let boost_unit = max_score * 3.0;
    for chunk_id in boosted.keys().copied().collect::<Vec<_>>() {
        let tier = definition_tier(&chunks[chunk_id], &names, &matchers, boost_unit);
        if tier > 0.0 {
            *boosted.get_mut(&chunk_id).unwrap() += tier;
        }
    }
    for (chunk_id, chunk) in chunks.iter().enumerate() {
        if boosted.contains_key(&chunk_id) {
            continue;
        }
        let stem = path_stem_lower(&chunk.file_path);
        if stem_matches(&stem, &symbol_name.to_lowercase()) {
            let tier = definition_tier(chunk, &names, &matchers, boost_unit);
            if tier > 0.0 {
                boosted.insert(chunk_id, tier);
            }
        }
    }
}

fn boost_embedded_symbols(
    boosted: &mut HashMap<usize, f32>,
    query: &str,
    max_score: f32,
    chunks: &[Chunk],
) {
    let names: HashSet<String> = EMBEDDED_SYMBOL_RE
        .find_iter(query)
        .map(|m| m.as_str().to_owned())
        .collect();
    if names.is_empty() {
        return;
    }
    let matchers = definition_matchers(&names);
    let boost_unit = max_score * 3.0 * 0.5;
    for chunk_id in boosted.keys().copied().collect::<Vec<_>>() {
        let tier = definition_tier(&chunks[chunk_id], &names, &matchers, boost_unit);
        if tier > 0.0 {
            *boosted.get_mut(&chunk_id).unwrap() += tier;
        }
    }
    let symbols_lower: Vec<String> = names.iter().map(|s| s.to_lowercase()).collect();
    for (chunk_id, chunk) in chunks.iter().enumerate() {
        if boosted.contains_key(&chunk_id) {
            continue;
        }
        let stem = path_stem_lower(&chunk.file_path);
        let stem_norm = stem.replace('_', "");
        let stem_ok = symbols_lower.iter().any(|symbol| {
            stem == *symbol
                || stem_norm == *symbol
                || (stem.len() >= 4 && symbol.starts_with(&stem))
                || (stem_norm.len() >= 4 && symbol.starts_with(&stem_norm))
        });
        if stem_ok {
            let tier = definition_tier(chunk, &names, &matchers, boost_unit);
            if tier > 0.0 {
                boosted.insert(chunk_id, tier);
            }
        }
    }
}

fn boost_stem_matches(
    boosted: &mut HashMap<usize, f32>,
    query: &str,
    max_score: f32,
    chunks: &[Chunk],
) {
    let query_words: Vec<String> = crate::tokens::tokenize(query)
        .into_iter()
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w.as_str()))
        .collect();
    if query_words.is_empty() {
        return;
    }
    for (&chunk_id, score) in boosted.iter_mut() {
        let stem = path_stem_lower(&chunks[chunk_id].file_path);
        let stem_parts: HashSet<String> = split_identifier(&stem).into_iter().collect();
        let matches = query_words
            .iter()
            .filter(|word| {
                stem_parts.contains(*word)
                    || stem_parts.iter().any(|part| {
                        part.starts_with(word.as_str()) || word.starts_with(part.as_str())
                    })
            })
            .count();
        if matches > 0 {
            *score += max_score * matches as f32 / query_words.len() as f32;
        }
    }
}

fn extract_symbol_name(query: &str) -> String {
    for sep in ["::", "\\", "->", "."] {
        if let Some((_, leaf)) = query.rsplit_once(sep) {
            return leaf.to_owned();
        }
    }
    query.trim().to_owned()
}

struct DefinitionMatcher {
    general: Regex,
    sql: Regex,
}

fn definition_matchers(names: &HashSet<String>) -> Vec<DefinitionMatcher> {
    names
        .iter()
        .filter_map(|name| {
            let escaped = regex::escape(name);
            let general_pattern = definition_pattern(DEFINITION_KEYWORDS, &escaped, "");
            let sql_pattern = definition_pattern(SQL_DEFINITION_KEYWORDS, &escaped, "i");
            Some(DefinitionMatcher {
                general: Regex::new(&general_pattern).ok()?,
                sql: Regex::new(&sql_pattern).ok()?,
            })
        })
        .collect()
}

fn definition_pattern(keywords: &[&str], escaped_name: &str, flags: &str) -> String {
    let prefix = if flags.is_empty() { "(?m)" } else { "(?im)" };
    format!(
        r"{prefix}(?:^|\s)(?:{})\s+(?:[A-Za-z_][A-Za-z0-9_]*(?:\.|::))*{}(?:\s|[<({{\:\[;]|$)",
        keywords
            .iter()
            .map(|k| regex::escape(k))
            .collect::<Vec<_>>()
            .join("|"),
        escaped_name
    )
}

fn chunk_defines_symbol(chunk: &Chunk, matchers: &[DefinitionMatcher]) -> bool {
    matchers.iter().any(|matcher| {
        matcher.general.is_match(&chunk.content) || matcher.sql.is_match(&chunk.content)
    })
}

fn definition_tier(
    chunk: &Chunk,
    names: &HashSet<String>,
    matchers: &[DefinitionMatcher],
    boost_unit: f32,
) -> f32 {
    if !chunk_defines_symbol(chunk, matchers) {
        return 0.0;
    }
    let stem = path_stem_lower(&chunk.file_path);
    if names
        .iter()
        .any(|name| stem_matches(&stem, &name.to_lowercase()))
    {
        boost_unit * 1.5
    } else {
        boost_unit
    }
}

fn stem_matches(stem: &str, name: &str) -> bool {
    let stem_norm = stem.replace('_', "");
    stem == name
        || stem_norm == name
        || stem.trim_end_matches('s') == name
        || stem_norm.trim_end_matches('s') == name
}

fn path_stem_lower(file_path: &str) -> String {
    Path::new(file_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn rerank_topk(
    scores: &HashMap<usize, f32>,
    chunks: &[Chunk],
    top_k: usize,
    penalise_paths: bool,
) -> Vec<(usize, f32)> {
    if scores.is_empty() || top_k == 0 {
        return Vec::new();
    }
    let mut penalized: Vec<(usize, f32)> = scores
        .iter()
        .map(|(&id, &score)| {
            let penalty = if penalise_paths {
                file_path_penalty(&chunks[id].file_path)
            } else {
                1.0
            };
            (id, score * penalty)
        })
        .collect();
    penalized.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    let mut file_selected: HashMap<&str, usize> = HashMap::new();
    let mut selected: Vec<(usize, f32)> = Vec::new();
    let mut min_selected = f32::INFINITY;
    for (chunk_id, score) in penalized {
        if selected.len() >= top_k && score <= min_selected {
            break;
        }
        let file = chunks[chunk_id].file_path.as_str();
        let already = *file_selected.get(file).unwrap_or(&0);
        let mut eff_score = score;
        if already >= 1 {
            let excess = already;
            eff_score *= 0.5f32.powi(excess as i32);
        }
        selected.push((chunk_id, eff_score));
        file_selected.insert(file, already + 1);
        if selected.len() >= top_k {
            min_selected = selected
                .iter()
                .map(|(_, s)| *s)
                .fold(f32::INFINITY, f32::min);
        }
    }
    selected.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    selected.truncate(top_k.min(selected.len()));
    selected
}

fn file_path_penalty(file_path: &str) -> f32 {
    let normalized = file_path.replace('\\', "/");
    let mut penalty = 1.0f32;
    if TEST_FILE_RE.is_match(&normalized) || TEST_DIR_RE.is_match(&normalized) {
        penalty *= 0.3;
    }
    let name = Path::new(file_path)
        .file_name()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    if name == "__init__.py" || name == "package-info.java" {
        penalty *= 0.5;
    }
    if COMPAT_DIR_RE.is_match(&normalized) {
        penalty *= 0.3;
    }
    if EXAMPLES_DIR_RE.is_match(&normalized) {
        penalty *= 0.3;
    }
    if TYPE_DEFS_RE.is_match(&normalized) {
        penalty *= 0.7;
    }
    penalty
}
