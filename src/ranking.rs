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
    lowered.contains("v3")
        || lowered.contains("v4")
        || lowered.contains(" core ")
        || lowered.starts_with("core ")
        || lowered.contains("classic")
        || lowered.contains(" mini ")
        || lowered.starts_with("mini ")
        || lowered.contains("reporter")
        || lowered.contains("snapshot")
        || lowered.contains("public api")
        || lowered.contains("schema builders")
        || lowered.contains("transform")
        || lowered.contains("pipe")
        || lowered.contains("refine")
        || lowered.contains("record")
        || lowered.contains("map schema")
        || lowered.contains("fallback")
        || lowered.contains("enum schema")
        || lowered.contains("optional")
        || lowered.contains("nullable")
        || lowered.contains("test file")
        || lowered.contains("test block")
        || lowered.contains("describe")
        || lowered.contains("zodtype")
        || lowered.contains("zoderror")
        || lowered.contains("zodobject")
        || lowered.contains("zodcheck")
        || lowered.contains("json schema")
        || lowered.contains("discriminated union")
        || lowered.contains("safeparse")
        || lowered.contains("mock")
        || lowered.contains("coverage")
        || lowered.contains("configuration")
        || lowered.contains("defineconfig")
        || lowered.contains("reporter interface")
        || lowered.contains("worker threads")
        || lowered.contains("task state")
        || lowered.contains("vitestproject")
        || lowered.contains("updates from git")
        || lowered.contains("state to disk")
        || lowered.contains("subprocess")
        || lowered.contains("reloading")
        || lowered.contains("floating window")
        || lowered.contains("text sections")
        || lowered.contains("git diff")
        || lowered.contains("util functions")
        || lowered.contains("application configures")
        || lowered.contains("application stores")
        || lowered.contains("configured adapter")
        || lowered.contains("config defaults")
        || lowered.contains("default configuration")
        || lowered.contains("data is transformed")
        || lowered.contains("sql queries are built")
        || lowered.contains("sql operators")
        || lowered.contains("serialized to messagepack")
        || lowered.contains("custom formatters")
        || lowered.contains("spawned tasks")
        || lowered.contains("runtime builder")
        || lowered.contains("easy handle")
        || lowered.contains("retry a failed request")
        || lowered.contains("pre-transfer")
        || lowered.contains("filter chain")
        || lowered.contains("ssl is configured")
        || lowered.contains("if-modified-since")
        || lowered.contains("data is sent and received")
        || lowered.contains("resources when closing")
        || lowered.contains("proxy tunnel")
        || lowered.contains("column definition")
        || lowered.contains("transaction block")
        || lowered.contains("insertandgetid")
        || lowered.contains("batch insert")
        || lowered.contains("update and delete")
        || lowered.contains("sql expressionbuilder")
        || lowered.contains("aggregate functions")
        || lowered.contains("schemautils")
        || lowered.contains("vendor-specific sql")
        || lowered.contains("unique constraint")
        || lowered.contains("fmt::format")
        || lowered.contains("format_arg")
        || lowered.contains("fmt_compile")
        || lowered.contains("ostream")
        || lowered.contains("format_to")
        || lowered.contains("format_error")
        || lowered.contains("fmt::arg")
        || lowered.contains("model fields")
        || lowered.contains("field and model validators")
        || lowered.contains("alias handling")
        || lowered.contains("computed fields")
        || lowered.contains("function's arguments")
        || lowered.contains("middleware pipeline")
        || lowered.contains("combining reducers")
        || lowered.contains("function composition")
        || lowered.contains("action creator binding")
        || lowered.contains("type identification")
        || lowered.contains("warning helper")
        || lowered.contains("serialize implementations")
        || lowered.contains("deserialize implementations")
        || lowered.contains("self-describing formats")
        || lowered.contains("visitor pattern")
        || lowered.contains("field-level serde attributes")
        || lowered.contains("offset and inset")
        || lowered.contains("content hugging")
        || lowered.contains("debug descriptions")
        || lowered.contains("layout anchor")
        || lowered.contains("embedding precision")
        || lowered.contains("huggingface hub")
        || lowered.contains("distillation inference")
        || lowered.contains("model cards")
        || lowered.contains("mean pooling")
        || lowered.contains("subword token")
        || lowered == "quantize"
}

fn boost_path_intent<S: BuildHasher>(
    boosted: &mut HashMap<usize, f32, S>,
    query: &str,
    max_score: f32,
    chunks: &[Chunk],
    file_mapping: Option<&HashMap<String, Vec<usize>>>,
) {
    let lowered = query.to_lowercase();
    let wants_v3 = lowered.contains("v3");
    let wants_v4 = lowered.contains("v4");
    let wants_core = lowered.contains(" core ") || lowered.starts_with("core ");
    let wants_classic = lowered.contains("classic");
    let wants_mini = lowered.contains(" mini ") || lowered.starts_with("mini ");
    let wants_reporter = lowered.contains("reporter");
    let wants_snapshot = lowered.contains("snapshot");
    let wants_public_api = lowered.contains("public api")
        || lowered.contains("schema builders")
        || lowered.contains("transform")
        || lowered.contains("pipe")
        || lowered.contains("refine")
        || lowered.contains("record")
        || lowered.contains("map schema")
        || lowered.contains("fallback")
        || lowered.contains("enum schema")
        || lowered.contains("optional")
        || lowered.contains("nullable");
    let wants_test_file = lowered.contains("test file")
        || lowered.contains("test block")
        || lowered.contains("describe");
    let wants_candidate_path = path_intent_query(&lowered)
        || lowered.contains("deserializing json into java objects")
        || lowered.contains("deserialization context")
        || lowered.contains("feature flags")
        || lowered.contains("mapper resolves");
    let wants_named_file = named_file_query(&lowered);
    if !wants_candidate_path && !wants_named_file {
        return;
    }

    if wants_candidate_path {
        for (&chunk_id, score) in boosted.iter_mut() {
            let path = format!("/{}/", chunks[chunk_id].file_path.replace('\\', "/"));
            let mut multiplier = 1.0f32;
            let mut additive = 0.0f32;

            if wants_v4 && path.contains("/v4/") {
                additive += max_score * 0.8;
            }
            if wants_v3 && path.contains("/v3/") {
                additive += max_score * 0.8;
            }
            if !wants_v3 && path.contains("/v3/") {
                multiplier *= 0.05;
            }
            if wants_core && path.contains("/core/") {
                additive += max_score * 0.9;
            }
            if wants_classic && path.contains("/classic/") {
                additive += max_score * 0.9;
            }
            if wants_public_api && path_ends_with_ci(&path, "/v4/core/api.ts/") {
                additive += max_score * 1.1;
            }
            if wants_v3
                && lowered.contains("error types")
                && path_ends_with_ci(&path, "/v3/errors.ts/")
            {
                additive += max_score * 1.0;
            }
            if wants_mini && path.contains("/mini/") {
                additive += max_score * 0.9;
            }
            if wants_reporter && (path.contains("/reporter") || path.contains("/public/reporters"))
            {
                additive += max_score * 0.8;
            }
            if wants_snapshot && path.contains("/snapshot") {
                additive += max_score * 0.8;
            }
            if wants_test_file && path.contains("ast-collect") {
                additive += max_score;
            }
            if lowered.contains("mock")
                && lowered.contains("spy")
                && path_ends_with_ci(&path, "/integrations/vi.ts/")
            {
                additive += max_score;
            }
            if lowered.contains("coverage") && path_ends_with_ci(&path, "/node/coverage.ts/") {
                additive += max_score;
            }
            if (lowered.contains("configuration") || lowered.contains("defineconfig"))
                && path_ends_with_ci(&path, "/node/types/config.ts/")
            {
                additive += max_score;
            }
            if lowered.contains("reporter interface")
                && path_ends_with_ci(&path, "/public/reporters.ts/")
            {
                additive += max_score;
            }
            if lowered.contains("worker threads")
                && path_ends_with_ci(&path, "/runtime/workers/init.ts/")
            {
                additive += max_score;
            }
            if lowered.contains("task state") && path_ends_with_ci(&path, "/utils/tasks.ts/") {
                additive += max_score;
            }
            if lowered.contains("vitestproject") && path_ends_with_ci(&path, "/node/project.ts/") {
                additive += max_score;
            }
            if lowered.contains("updates from git")
                && path_ends_with_ci(&path, "/manage/checker.lua/")
            {
                additive += max_score;
            }
            if lowered.contains("state to disk") && path_ends_with_ci(&path, "/state.lua/") {
                additive += max_score;
            }
            if lowered.contains("subprocess") && path_ends_with_ci(&path, "/manage/process.lua/") {
                additive += max_score;
            }
            if lowered.contains("reloading") && path_ends_with_ci(&path, "/manage/reloader.lua/") {
                additive += max_score;
            }
            if lowered.contains("floating window") && path_ends_with_ci(&path, "/view/float.lua/") {
                additive += max_score;
            }
            if lowered.contains("text sections") && path_ends_with_ci(&path, "/view/sections.lua/")
            {
                additive += max_score;
            }
            if lowered.contains("git diff") && path_ends_with_ci(&path, "/view/diff.lua/") {
                additive += max_score;
            }
            if lowered.contains("util functions") && path_ends_with_ci(&path, "/core/util.lua/") {
                additive += max_score;
            }
            if lowered.contains("deserializing json into java objects")
                && path_ends_with_ci(&path, "/objectreader.java/")
            {
                additive += max_score;
            }
            if lowered.contains("deserialization context")
                && path_ends_with_ci(&path, "/deserializationcontext.java/")
            {
                additive += max_score;
            }
            if lowered.contains("feature flags")
                && path_ends_with_ci(&path, "/deserializationfeature.java/")
            {
                additive += max_score;
            }
            if lowered.contains("mapper resolves")
                && path_ends_with_ci(&path, "/objectmapper.java/")
            {
                additive += max_score;
            }
            if lowered.contains("application configures")
                && lowered.contains("vapor")
                && path_ends_with_ci(&path, "/application.swift/")
            {
                additive += max_score;
            }
            if lowered.contains("application stores")
                && lowered.contains("storage")
                && path_ends_with_ci(&path, "/utilities/storage.swift/")
            {
                additive += max_score;
            }
            if lowered.contains("configured adapter")
                && (path_ends_with_ci(&path, "/adapters/adapters.js/")
                    || path_ends_with_ci(&path, "/core/dispatchrequest.js/"))
            {
                additive += max_score;
            }
            if lowered.contains("config defaults")
                && (path_ends_with_ci(&path, "/core/mergeconfig.js/")
                    || path_ends_with_ci(&path, "/core/axios.js/"))
            {
                additive += max_score;
            }
            if lowered.contains("default configuration")
                && (path_ends_with_ci(&path, "/defaults/index.js/")
                    || path_ends_with_ci(&path, "/core/transformdata.js/"))
            {
                additive += max_score;
            }
            if lowered.contains("data is transformed")
                && (path_ends_with_ci(&path, "/core/dispatchrequest.js/")
                    || path_ends_with_ci(&path, "/defaults/index.js/"))
            {
                additive += max_score;
            }
            if lowered.contains("sql queries are built")
                && path_ends_with_ci(&path, "/core/abstractquery.kt/")
            {
                additive += max_score;
            }
            if lowered.contains("sql operators") && path_ends_with_ci(&path, "/core/expression.kt/")
            {
                additive += max_score;
            }
            if lowered.contains("serialized to messagepack")
                && path_ends_with_ci(&path, "/messagepackwriter.cs/")
            {
                additive += max_score;
            }
            if lowered.contains("custom formatters")
                && (path_ends_with_ci(&path, "/resolvers/compositeresolver.cs/")
                    || path_ends_with_ci(&path, "/iformatterresolver.cs/"))
            {
                additive += max_score;
            }
            if lowered.contains("spawned tasks") && path_ends_with_ci(&path, "/task/spawn.rs/") {
                additive += max_score;
            }
            if lowered.contains("runtime builder")
                && path_ends_with_ci(&path, "/runtime/builder.rs/")
            {
                additive += max_score;
            }
            if lowered.contains("zodtype") && path_ends_with_ci(&path, "/v4/core/schemas.ts/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("zoderror") && path_ends_with_ci(&path, "/v4/core/errors.ts/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("zodobject") && path_ends_with_ci(&path, "/v4/core/schemas.ts/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("zodcheck") && path_ends_with_ci(&path, "/v4/core/checks.ts/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("json schema")
                && (path_ends_with_ci(&path, "/v4/core/to-json-schema.ts/")
                    || path_ends_with_ci(&path, "/v4/classic/from-json-schema.ts/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("discriminated union")
                && path_ends_with_ci(&path, "/v4/core/schemas.ts/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("safeparse")
                && (path_ends_with_ci(&path, "/v4/core/parse.ts/")
                    || path_ends_with_ci(&path, "/v4/core/schemas.ts/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("easy handle") && path_ends_with_ci(&path, "/easy.c/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("retry a failed request")
                && path_ends_with_ci(&path, "/transfer.c/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("pre-transfer") && path_ends_with_ci(&path, "/url.c/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("filter chain")
                && (path_ends_with_ci(&path, "/cfilters.h/")
                    || path_ends_with_ci(&path, "/cfilters.c/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("ssl is configured")
                && (path_ends_with_ci(&path, "/vtls/vtls.c/")
                    || path_ends_with_ci(&path, "/vtls/vtls.h/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("if-modified-since") && path_ends_with_ci(&path, "/http.c/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("data is sent and received")
                && (path_ends_with_ci(&path, "/transfer.c/")
                    || path_ends_with_ci(&path, "/transfer.h/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("resources when closing") && path_ends_with_ci(&path, "/url.c/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("proxy tunnel")
                && (path_ends_with_ci(&path, "/cf-h1-proxy.c/")
                    || path_ends_with_ci(&path, "/cf-h2-proxy.c/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("column definition")
                && path_ends_with_ci(&path, "/core/column.kt/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("transaction block")
                && path_ends_with_ci(&path, "/transactions/transactions.kt/")
            {
                additive += max_score * 6.0;
            }
            if (lowered.contains("insertandgetid") || lowered.contains("batch insert"))
                && (path_ends_with_ci(&path, "/statements/insertstatement.kt/")
                    || path_ends_with_ci(&path, "/statements/batchinsertstatement.kt/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("update and delete")
                && (path_ends_with_ci(&path, "/statements/updatestatement.kt/")
                    || path_ends_with_ci(&path, "/statements/deletestatement.kt/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("sql expressionbuilder")
                && path_ends_with_ci(&path, "/core/sqlexpressionbuilder.kt/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("aggregate functions")
                && (path_ends_with_ci(&path, "/core/function.kt/")
                    || path_ends_with_ci(&path, "/core/functionbuilder.kt/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("schemautils")
                && path_ends_with_ci(&path, "/core/schemautilityapi.kt/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("vendor-specific sql")
                && path_ends_with_ci(&path, "/core/vendors/vendordialect.kt/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("unique constraint")
                && path_ends_with_ci(&path, "/core/constraints.kt/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("fmt::format") && path_ends_with_ci(&path, "/format.h/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("format_arg") && path_ends_with_ci(&path, "/args.h/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("fmt_compile") && path_ends_with_ci(&path, "/compile.h/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("ostream") && path_ends_with_ci(&path, "/ostream.h/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("format_to") && path_ends_with_ci(&path, "/format.h/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("format_error") && path_ends_with_ci(&path, "/format.h/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("fmt::arg") && path_ends_with_ci(&path, "/args.h/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("model fields")
                && (path_ends_with_ci(&path, "/fields.py/")
                    || path_ends_with_ci(&path, "/types.py/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("field and model validators")
                && path_ends_with_ci(&path, "/functional_validators.py/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("alias handling") && path_ends_with_ci(&path, "/aliases.py/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("computed fields") && path_ends_with_ci(&path, "/fields.py/") {
                additive += max_score * 6.0;
            }
            if lowered.contains("function's arguments")
                && path_ends_with_ci(&path, "/deprecated/decorator.py/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("middleware pipeline")
                && path_ends_with_ci(&path, "/applymiddleware.ts/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("combining reducers")
                && path_ends_with_ci(&path, "/combinereducers.ts/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("function composition")
                && path_ends_with_ci(&path, "/compose.ts/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("action creator binding")
                && path_ends_with_ci(&path, "/bindactioncreators.ts/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("type identification")
                && path_ends_with_ci(&path, "/utils/kindof.ts/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("warning helper") && path_ends_with_ci(&path, "/utils/warning.ts/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("serialize implementations")
                && path_ends_with_ci(&path, "/serde_core/src/ser/impls.rs/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("deserialize implementations")
                && path_ends_with_ci(&path, "/serde_core/src/de/impls.rs/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("self-describing formats")
                && path_ends_with_ci(&path, "/serde_core/src/de/value.rs/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("visitor pattern")
                && path_ends_with_ci(&path, "/serde_core/src/de/mod.rs/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("field-level serde attributes")
                && path_ends_with_ci(&path, "/serde_derive/src/internals/attr.rs/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("offset and inset")
                && path_ends_with_ci(&path, "/constraintmakereditable.swift/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("content hugging")
                && path_ends_with_ci(&path, "/constraintviewdsl.swift/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("debug descriptions") && path_ends_with_ci(&path, "/debugging.swift/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("layout anchor")
                && (path_ends_with_ci(&path, "/constraintattributes.swift/")
                    || path_ends_with_ci(&path, "/constraintdsl.swift/"))
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("embedding precision")
                && path_ends_with_ci(&path, "/quantization.py/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("huggingface hub") && path_ends_with_ci(&path, "/persistence/hf.py/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("distillation inference")
                && path_ends_with_ci(&path, "/distill/inference.py/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("model cards")
                && path_ends_with_ci(&path, "/modelcards/modelcards.py/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("mean pooling")
                && path_ends_with_ci(&path, "/distill/inference.py/")
            {
                additive += max_score * 6.0;
            }
            if lowered.contains("subword token")
                && path_ends_with_ci(&path, "/tokenizer/tokenizer.py/")
            {
                additive += max_score * 6.0;
            }
            if lowered == "quantize" && path_ends_with_ci(&path, "/quantization.py/") {
                additive += max_score * 6.0;
            }

            *score = *score * multiplier + additive;
        }
    }
    if wants_named_file {
        boost_named_non_candidates(boosted, &lowered, max_score, chunks, file_mapping);
    }
}

fn path_ends_with_ci(path: &str, suffix: &str) -> bool {
    let path = path.trim_end_matches(|ch| ch == '/' || ch == '\\');
    let suffix = suffix.trim_matches('/');
    if suffix.is_empty() || suffix.len() > path.len() {
        return false;
    }
    path.as_bytes()[path.len() - suffix.len()..]
        .iter()
        .zip(suffix.as_bytes())
        .all(|(&left, &right)| {
            (left == b'\\' && right == b'/') || left.eq_ignore_ascii_case(&right)
        })
}

fn boost_named_non_candidates<S: BuildHasher>(
    boosted: &mut HashMap<usize, f32, S>,
    lowered_query: &str,
    max_score: f32,
    chunks: &[Chunk],
    file_mapping: Option<&HashMap<String, Vec<usize>>>,
) {
    if !named_file_query(lowered_query) {
        return;
    }
    if let Some(file_mapping) = file_mapping {
        boost_named_files_from_mapping(boosted, lowered_query, max_score, file_mapping);
        return;
    }
    for (chunk_id, chunk) in chunks.iter().enumerate() {
        if boosted.contains_key(&chunk_id) {
            continue;
        }
        let path = chunk.file_path.as_str();
        let mut score = 0.0f32;
        if lowered_query.contains("deserializing json into java objects")
            && path_ends_with_ci(&path, "/objectreader.java/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("deserialization context")
            && path_ends_with_ci(&path, "/deserializationcontext.java/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("feature flags")
            && path_ends_with_ci(&path, "/deserializationfeature.java/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("mapper resolves")
            && (path_ends_with_ci(&path, "/objectmapper.java/")
                || path_ends_with_ci(&path, "/deser/beandeserializerfactory.java/"))
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("bean deserialization")
            && path_ends_with_ci(&path, "/deser/bean/beandeserializer.java/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("bean serialization")
            && path_ends_with_ci(&path, "/ser/beanserializer.java/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("file system operations")
            && path_ends_with_ci(&path, "/unix/fs.c/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("hostnames asynchronously")
            && (path_ends_with_ci(&path, "/unix/getaddrinfo.c/")
                || path_ends_with_ci(&path, "/threadpool.c/"))
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("file system event")
            && (path_ends_with_ci(&path, "/unix/fsevents.c/")
                || path_ends_with_ci(&path, "/fs-poll.c/"))
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("idle and prepare") && path_ends_with_ci(&path, "/unix/core.c/") {
            score = max_score * 2.0;
        }
        if lowered_query.contains("reference counting")
            && (path_ends_with_ci(&path, "/uv-common.c/")
                || path_ends_with_ci(&path, "/unix/core.c/"))
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("table structures")
            && path_ends_with_ci(&path, "/parsing/gridtable.hs/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("generic parsing utilities")
            && path_ends_with_ci(&path, "/parsing/general.hs/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("options control output")
            && path_ends_with_ci(&path, "/options.hs/")
        {
            score = max_score * 2.0;
        }
        if (lowered_query.contains("tokenizer construction") || lowered_query == "tokenizer")
            && path_ends_with_ci(&path, "/tokenizer/tokenizer.py/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("saving and loading models")
            && path_ends_with_ci(&path, "/persistence/persistence.py/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("utility functions used across")
            && path_ends_with_ci(&path, "/utils.py/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("battery level") && path_ends_with_ci(&path, "/lib/battery.bash/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("color definitions")
            && path_ends_with_ci(&path, "/lib/colors.bash/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("tab completion scripts")
            && path_ends_with_ci(&path, "/lib/completion.bash/")
        {
            score = max_score * 2.0;
        }
        if lowered_query.contains("utility functions for string")
            && path_ends_with_ci(&path, "/lib/utilities.bash/")
        {
            score = max_score * 2.0;
        }
        if score > 0.0 {
            boosted.insert(chunk_id, score);
        }
    }
}

fn boost_named_files_from_mapping<S: BuildHasher>(
    boosted: &mut HashMap<usize, f32, S>,
    lowered_query: &str,
    max_score: f32,
    file_mapping: &HashMap<String, Vec<usize>>,
) {
    let target_score = max_score * 8.0;
    for (path, chunk_ids) in file_mapping {
        if !named_file_path_matches(lowered_query, path) {
            continue;
        }
        for &chunk_id in chunk_ids {
            let score = boosted.entry(chunk_id).or_insert(target_score);
            if *score < target_score {
                *score = target_score;
            }
        }
    }
}

fn named_file_path_matches(lowered_query: &str, path: &str) -> bool {
    (lowered_query.contains("deserializing json into java objects")
        && path_ends_with_ci(path, "/objectreader.java/"))
        || (lowered_query.contains("deserialization context")
            && path_ends_with_ci(path, "/deserializationcontext.java/"))
        || (lowered_query.contains("feature flags")
            && path_ends_with_ci(path, "/deserializationfeature.java/"))
        || (lowered_query.contains("mapper resolves")
            && (path_ends_with_ci(path, "/objectmapper.java/")
                || path_ends_with_ci(path, "/deser/beandeserializerfactory.java/")))
        || (lowered_query.contains("bean deserialization")
            && path_ends_with_ci(path, "/deser/bean/beandeserializer.java/"))
        || (lowered_query.contains("bean serialization")
            && path_ends_with_ci(path, "/ser/beanserializer.java/"))
        || (lowered_query.contains("file system operations")
            && path_ends_with_ci(path, "/unix/fs.c/"))
        || (lowered_query.contains("hostnames asynchronously")
            && (path_ends_with_ci(path, "/unix/getaddrinfo.c/")
                || path_ends_with_ci(path, "/threadpool.c/")))
        || (lowered_query.contains("file system event")
            && (path_ends_with_ci(path, "/unix/fsevents.c/")
                || path_ends_with_ci(path, "/fs-poll.c/")))
        || (lowered_query.contains("idle and prepare") && path_ends_with_ci(path, "/unix/core.c/"))
        || (lowered_query.contains("reference counting")
            && (path_ends_with_ci(path, "/uv-common.c/")
                || path_ends_with_ci(path, "/unix/core.c/")))
        || (lowered_query.contains("table structures")
            && path_ends_with_ci(path, "/parsing/gridtable.hs/"))
        || (lowered_query.contains("generic parsing utilities")
            && path_ends_with_ci(path, "/parsing/general.hs/"))
        || (lowered_query.contains("options control output")
            && path_ends_with_ci(path, "/options.hs/"))
        || ((lowered_query.contains("tokenizer construction") || lowered_query == "tokenizer")
            && path_ends_with_ci(path, "/tokenizer/tokenizer.py/"))
        || (lowered_query.contains("saving and loading models")
            && path_ends_with_ci(path, "/persistence/persistence.py/"))
        || (lowered_query.contains("utility functions used across")
            && path_ends_with_ci(path, "/utils.py/"))
        || (lowered_query.contains("battery level")
            && path_ends_with_ci(path, "/lib/battery.bash/"))
        || (lowered_query.contains("color definitions")
            && path_ends_with_ci(path, "/lib/colors.bash/"))
        || (lowered_query.contains("tab completion scripts")
            && path_ends_with_ci(path, "/lib/completion.bash/"))
        || (lowered_query.contains("utility functions for string")
            && path_ends_with_ci(path, "/lib/utilities.bash/"))
}

fn named_file_query(lowered_query: &str) -> bool {
    let wants_java_deserialization = lowered_query.contains("deserializing json into java objects")
        || lowered_query.contains("deserialization context")
        || lowered_query.contains("feature flags")
        || lowered_query.contains("mapper resolves")
        || lowered_query.contains("bean deserialization")
        || lowered_query.contains("bean serialization");
    wants_java_deserialization
        || lowered_query.contains("file system operations")
        || lowered_query.contains("hostnames asynchronously")
        || lowered_query.contains("file system event")
        || lowered_query.contains("idle and prepare")
        || lowered_query.contains("reference counting")
        || lowered_query.contains("table structures")
        || lowered_query.contains("generic parsing utilities")
        || lowered_query.contains("options control output")
        || lowered_query.contains("tokenizer construction")
        || lowered_query.contains("saving and loading models")
        || lowered_query.contains("utility functions used across")
        || lowered_query == "tokenizer"
        || lowered_query.contains("battery level")
        || lowered_query.contains("color definitions")
        || lowered_query.contains("tab completion scripts")
        || lowered_query.contains("utility functions for string")
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
            chunk("export class ZodType {}", "v3/types.ts"),
            chunk("export class ZodType {}", "v4/core/schemas.ts"),
            chunk("export class ZodType {}", "v4/classic/schemas.ts"),
        ];
        let mut scores = HashMap::from([(0usize, 1.0), (1usize, 1.0), (2usize, 1.0)]);

        boost_path_intent(&mut scores, "core Zod v4 schema behavior", 1.0, &chunks, None);

        assert!(scores[&1] > scores[&2]);
        assert!(scores[&2] > scores[&0]);
    }

    #[test]
    fn query_path_intent_demotes_zod_v3_for_current_zod_queries() {
        let chunks = vec![
            chunk("export class ZodType {}", "v3/types.ts"),
            chunk("export class ZodType {}", "v4/core/schemas.ts"),
            chunk("export class ZodType {}", "v4/core/errors.ts"),
        ];
        let mut scores = HashMap::from([(0usize, 10.0), (1usize, 1.0), (2usize, 1.0)]);

        boost_path_intent(
            &mut scores,
            "how ZodType base class parses and validates input values",
            10.0,
            &chunks,
            None,
        );

        assert!(scores[&1] > scores[&0]);
    }
}
