use crate::tokens::split_identifier;
use crate::types::Chunk;
use once_cell::sync::Lazy;
use regex::Regex;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
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
            0.25
        } else if is_architecture_query(query) || is_natural_language_question(query) {
            0.65
        } else if is_mixed_code_phrase(query) {
            0.45
        } else {
            0.55
        }
    })
}

pub(crate) fn truncate_top_k(scores: &mut Vec<(usize, f32)>, k: usize) {
    if scores.len() <= k {
        scores.sort_unstable_by(desc_score);
        return;
    }
    scores.select_nth_unstable_by(k, desc_score);
    scores.truncate(k);
    scores.sort_unstable_by(desc_score);
}

fn desc_score(a: &(usize, f32), b: &(usize, f32)) -> Ordering {
    b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal)
}

pub fn is_symbol_query(query: &str) -> bool {
    SYMBOL_QUERY_RE.is_match(query.trim())
}

fn is_architecture_query(query: &str) -> bool {
    let lowered = query.trim().to_lowercase();
    lowered.contains(" architecture")
        || lowered.contains(" design")
        || lowered.contains(" flow")
        || lowered.contains(" lifecycle")
        || lowered.contains(" pipeline")
        || lowered.starts_with("how does ")
        || lowered.starts_with("how are ")
}

fn is_natural_language_question(query: &str) -> bool {
    let lowered = query.trim().to_lowercase();
    lowered.ends_with('?')
        || lowered.starts_with("how ")
        || lowered.starts_with("what ")
        || lowered.starts_with("where ")
        || lowered.starts_with("when ")
        || lowered.starts_with("why ")
        || lowered.starts_with("which ")
        || lowered.starts_with("who ")
}

fn is_mixed_code_phrase(query: &str) -> bool {
    EMBEDDED_SYMBOL_RE.is_match(query)
        || query.split_whitespace().any(|part| {
            is_symbol_query(part.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_'))
        })
}

pub fn boost_multi_chunk_files<S: BuildHasher>(
    scores: &mut HashMap<usize, f32, S>,
    chunks: &[Chunk],
) {
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
    apply_query_boost_in_place(scores.clone(), query, chunks, None)
}

pub fn apply_query_boost_in_place<S: BuildHasher>(
    mut boosted: HashMap<usize, f32, S>,
    query: &str,
    chunks: &[Chunk],
    file_mapping: Option<&HashMap<String, Vec<usize>>>,
) -> HashMap<usize, f32, S> {
    if boosted.is_empty() {
        return boosted;
    }
    let max_score = boosted.values().copied().fold(f32::NEG_INFINITY, f32::max);
    if is_symbol_query(query) {
        boost_symbol_definitions(&mut boosted, query, max_score, chunks);
    } else {
        if stem_boost_query(query) {
            boost_stem_matches(&mut boosted, query, max_score, chunks);
        }
        if may_contain_embedded_symbol(query) {
            boost_embedded_symbols(&mut boosted, query, max_score, chunks);
        }
    }
    boost_path_intent(&mut boosted, query, max_score, chunks, file_mapping);
    boosted
}

fn may_contain_embedded_symbol(query: &str) -> bool {
    query
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0].is_ascii_lowercase() && pair[1].is_ascii_uppercase())
}

fn stem_boost_query(query: &str) -> bool {
    let lowered = query.to_lowercase();
    lowered.contains("public")
        || lowered.contains("api")
        || lowered.contains("config")
        || lowered.contains("schema")
        || lowered.contains("builder")
        || lowered.contains("worker")
        || lowered.contains("reporter")
        || lowered.contains("snapshot")
        || lowered.contains("state")
        || lowered.contains("task")
        || lowered.contains("type")
        || lowered.contains("error")
        || lowered.contains("parser")
        || lowered.contains("serializer")
        || lowered.contains("deserializer")
        || lowered.contains("router")
        || lowered.contains("request")
        || lowered.contains("response")
}

fn path_intent_query(lowered: &str) -> bool {
    query_path_terms(lowered).next().is_some()
        || lowered.contains("public api")
        || lowered.contains("test file")
        || lowered.contains("test block")
        || lowered.contains("source file")
        || lowered.contains("example")
        || lowered.contains("examples")
        || lowered.contains("docs")
        || lowered.contains("documentation")
}

fn boost_path_intent<S: BuildHasher>(
    boosted: &mut HashMap<usize, f32, S>,
    query: &str,
    max_score: f32,
    chunks: &[Chunk],
    _file_mapping: Option<&HashMap<String, Vec<usize>>>,
) {
    let lowered = query.to_lowercase();
    if !path_intent_query(&lowered) {
        return;
    }

    let query_terms: HashSet<String> = query_path_terms(&lowered).collect();
    let wants_test_file = lowered.contains("test file")
        || lowered.contains("test block")
        || lowered.contains("describe");
    let wants_docs = lowered.contains("docs") || lowered.contains("documentation");
    let wants_examples = lowered.contains("example") || lowered.contains("examples");
    let wants_public_api = lowered.contains("public api");
    let wants_source_file = lowered.contains("source file");

    for (&chunk_id, score) in boosted.iter_mut() {
        let path = chunks[chunk_id].file_path.replace('\\', "/").to_lowercase();
        let path_terms = path_terms(&path);
        let mut additive = 0.0f32;
        let mut multiplier = 1.0f32;

        let overlap = query_terms
            .iter()
            .filter(|term| path_terms.contains(*term))
            .count();
        if overlap > 0 {
            additive += max_score * (0.35 * overlap as f32).min(1.4);
        }

        if query_terms
            .iter()
            .any(|term| path_stem_matches(&path, term))
        {
            additive += max_score * 0.7;
        }
        if wants_public_api
            && path_terms
                .iter()
                .any(|term| term == "api" || term == "public")
        {
            additive += max_score * 0.5;
        }
        if wants_test_file && (TEST_FILE_RE.is_match(&path) || TEST_DIR_RE.is_match(&path)) {
            additive += max_score * 0.7;
        }
        if wants_docs
            && path_terms
                .iter()
                .any(|term| term == "doc" || term == "docs")
        {
            additive += max_score * 0.6;
        }
        if wants_examples && EXAMPLES_DIR_RE.is_match(&path) {
            additive += max_score * 0.6;
        }
        if wants_source_file && (TEST_FILE_RE.is_match(&path) || EXAMPLES_DIR_RE.is_match(&path)) {
            multiplier *= 0.8;
        }

        *score = *score * multiplier + additive;
    }
}

fn query_path_terms(lowered: &str) -> impl Iterator<Item = String> + '_ {
    lowered
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|term| term.len() >= 3 && !STOPWORDS.contains(*term))
        .flat_map(split_identifier)
}

fn path_terms(path: &str) -> HashSet<String> {
    path.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|term| !term.is_empty())
        .flat_map(split_identifier)
        .collect()
}

fn path_stem_matches(path: &str, query_term: &str) -> bool {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| {
            split_identifier(stem)
                .into_iter()
                .any(|stem_term| stem_matches(&stem_term, query_term))
        })
        .unwrap_or(false)
}

fn boost_symbol_definitions<S: BuildHasher>(
    boosted: &mut HashMap<usize, f32, S>,
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

fn boost_embedded_symbols<S: BuildHasher>(
    boosted: &mut HashMap<usize, f32, S>,
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

fn boost_stem_matches<S: BuildHasher>(
    boosted: &mut HashMap<usize, f32, S>,
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
    let keywords = unique_words(query_words);
    let public_api = public_api_query(&keywords);
    let mut path_matches: HashMap<&str, (usize, bool)> = HashMap::new();
    for (&chunk_id, score) in boosted.iter_mut() {
        let chunk = &chunks[chunk_id];
        let (matches, is_public_api_file) =
            *path_matches.entry(&chunk.file_path).or_insert_with(|| {
                (
                    count_keyword_path_matches(&keywords, &chunk.file_path),
                    public_api_file(&chunk.file_path),
                )
            });
        if matches > 0 {
            *score += max_score * matches as f32 / keywords.len() as f32;
        }
        if public_api && is_public_api_file {
            *score += max_score * 0.8;
        }
    }
}

fn unique_words(words: Vec<String>) -> Vec<String> {
    let mut unique = Vec::with_capacity(words.len());
    for word in words {
        if !unique.contains(&word) {
            unique.push(word);
        }
    }
    unique
}

fn count_keyword_path_matches(keywords: &[String], file_path: &str) -> usize {
    let path = Path::new(file_path);
    let mut parts = path
        .file_stem()
        .map(|stem| split_identifier(&stem.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    if let Some(parent) = path.parent().and_then(|parent| parent.file_name()) {
        let parent = parent.to_string_lossy().to_lowercase();
        if parent != "." && parent != "/" && parent != ".." {
            parts.extend(split_identifier(&parent));
        }
    }
    let mut matches = 0;
    for keyword in keywords {
        if parts.iter().any(|part| {
            if keyword == part {
                return true;
            }
            let (shorter, longer) = if keyword.len() <= part.len() {
                (keyword.as_str(), part.as_str())
            } else {
                (part.as_str(), keyword.as_str())
            };
            shorter.len() >= 3 && longer.starts_with(shorter)
        }) {
            matches += 1;
        }
    }
    matches
}

fn public_api_query(keywords: &[String]) -> bool {
    keywords.iter().any(|keyword| {
        matches!(
            keyword.as_str(),
            "api" | "public" | "function" | "functions" | "builder" | "builders"
        )
    })
}

fn public_api_file(file_path: &str) -> bool {
    matches!(
        path_stem_lower(file_path).as_str(),
        "api" | "public" | "index" | "mod" | "lib"
    )
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
        r"{prefix}(?:^|\s)(?:{})\s+(?:[A-Za-z_][A-Za-z0-9_]*(?:\.|::))*\$?{}(?:\s|[<({{\:\[;]|$)",
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

pub fn rerank_topk<S: BuildHasher>(
    scores: &HashMap<usize, f32, S>,
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

#[cfg(test)]
mod tests {
    use super::{apply_query_boost, boost_path_intent, rerank_topk, resolve_alpha};
    use crate::types::Chunk;
    use std::collections::HashMap;

    fn chunk(content: &str, file_path: &str) -> Chunk {
        Chunk {
            content: content.to_owned(),
            file_path: file_path.to_owned(),
            start_line: 1,
            end_line: 1,
            language: Some("python".to_owned()),
            symbols: Vec::new(),
            breadcrumbs: Vec::new(),
        }
    }

    #[test]
    fn boosts_definitions_and_penalizes_tests() {
        let defining = chunk("class MyService:\n    pass", "src/my_service.py");
        let other = chunk("x = MyService()", "src/utils.py");
        let chunks = vec![defining, other];
        let mut scores = HashMap::new();
        scores.insert(1usize, 0.4);
        let boosted = apply_query_boost(&scores, "MyService", &chunks);
        assert!(boosted[&0] > boosted[&1]);

        let regular = chunk("def impl(): pass", "src/regular.py");
        let test = chunk("def impl(): pass", "tests/test_auth.py");
        let chunks = vec![regular, test];
        let scores = HashMap::from([(0usize, 1.0), (1usize, 1.0)]);
        let ranked = rerank_topk(&scores, &chunks, 2, true);
        assert_eq!(ranked[0].0, 0);
    }

    #[test]
    fn alpha_detection_matches_symbol_vs_natural_language() {
        assert_eq!(resolve_alpha("MyService", None), 0.25);
        assert_eq!(resolve_alpha("parse_json", None), 0.25);
        assert_eq!(resolve_alpha("how does routing work", None), 0.65);
        assert_eq!(resolve_alpha("request lifecycle architecture", None), 0.65);
        assert_eq!(
            resolve_alpha("where is request validation handled?", None),
            0.65
        );
        assert_eq!(resolve_alpha("useSession hook", None), 0.45);
        assert_eq!(
            resolve_alpha("request validation and error handling", None),
            0.55
        );
        assert_eq!(resolve_alpha("MyService", Some(0.7)), 0.7);
    }

    #[test]
    fn query_path_intent_prefers_requested_version_and_surface() {
        let chunks = vec![
            chunk("pub struct Runtime {}", "v3/legacy/old_runtime.rs"),
            chunk("pub struct Runtime {}", "v4/core/runtime_builder.rs"),
            chunk("pub struct Runtime {}", "v4/classic/runtime_builder.rs"),
        ];
        let mut scores = HashMap::from([(0usize, 1.0), (1usize, 1.0), (2usize, 1.0)]);

        boost_path_intent(&mut scores, "core v4 runtime builder", 1.0, &chunks, None);

        assert!(scores[&1] > scores[&2]);
        assert!(scores[&2] > scores[&0]);
    }

    #[test]
    fn query_path_intent_uses_generic_path_terms_without_injecting_non_candidates() {
        let chunks = vec![
            chunk("fn parse() {}", "src/transfer.c"),
            chunk("fn parse() {}", "src/retry/backoff.rs"),
            chunk("fn parse() {}", "src/http/client.rs"),
        ];
        let mut scores = HashMap::from([(0usize, 1.0), (2usize, 1.0)]);

        boost_path_intent(
            &mut scores,
            "where is retry backoff implemented",
            1.0,
            &chunks,
            None,
        );

        assert!(scores[&2] <= scores[&0]);
        assert!(!scores.contains_key(&1));
    }
}
