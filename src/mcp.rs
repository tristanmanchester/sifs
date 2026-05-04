use crate::SifsIndex;
use crate::index::{CacheConfig, IndexOptions};
use crate::model2vec::ModelOptions;
use crate::types::{SearchMode, SearchOptions};
use crate::utils::{format_results, is_git_url, resolve_chunk};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const MCP_INSTRUCTIONS: &str = include_str!("agents/mcp-instructions.md");
const SEARCH_DESCRIPTION: &str = include_str!("agents/tools/search.md");
const FIND_RELATED_DESCRIPTION: &str = include_str!("agents/tools/find-related.md");
const INDEX_STATUS_DESCRIPTION: &str = include_str!("agents/tools/index-status.md");
const NO_RESULTS_MESSAGE: &str = include_str!("agents/messages/no-results.md");
const NO_REPO_MESSAGE: &str = include_str!("agents/messages/no-repo.md");

pub fn serve(default_source: Option<String>, ref_name: Option<String>) -> Result<()> {
    serve_with_options(
        default_source,
        ref_name,
        ModelOptions::default(),
        CacheConfig::default(),
        false,
    )
}

pub fn serve_with_options(
    default_source: Option<String>,
    ref_name: Option<String>,
    model_options: ModelOptions,
    cache_config: CacheConfig,
    offline: bool,
) -> Result<()> {
    let mut cache = IndexCache::new(model_options, cache_config, offline);
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();

    while let Some(message) = read_message(&mut reader)? {
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}, "resources": {}},
                "serverInfo": {"name": "sifs", "version": env!("CARGO_PKG_VERSION")},
                "instructions": build_instructions(default_source.as_deref(), ref_name.as_deref())
            }),
            "resources/list" => json!({"resources": resource_schemas()}),
            "resources/read" => handle_resource_read(
                &message,
                &cache,
                default_source.as_deref(),
                ref_name.as_deref(),
            ),
            "tools/list" => json!({"tools": tool_schemas()}),
            "tools/call" => handle_tool_call(
                &message,
                &mut cache,
                default_source.as_deref(),
                ref_name.as_deref(),
            ),
            _ => json!({"error": format!("Unsupported method: {method}")}),
        };
        write_message(
            &mut stdout,
            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
        )?;
    }
    Ok(())
}

struct IndexCache {
    indexes: HashMap<String, SifsIndex>,
    model_options: ModelOptions,
    cache_config: CacheConfig,
    offline: bool,
}

impl Default for IndexCache {
    fn default() -> Self {
        Self::new(ModelOptions::default(), CacheConfig::default(), false)
    }
}

impl IndexCache {
    fn new(model_options: ModelOptions, cache_config: CacheConfig, offline: bool) -> Self {
        Self {
            indexes: HashMap::new(),
            model_options,
            cache_config,
            offline,
        }
    }

    fn key_for(source: &str, ref_name: Option<&str>) -> Result<String> {
        if is_git_url(source) {
            Ok(ref_name
                .map(|r| format!("{source}@{r}"))
                .unwrap_or_else(|| source.to_owned()))
        } else {
            Ok(Path::new(source)
                .canonicalize()?
                .to_string_lossy()
                .to_string())
        }
    }

    fn get(&mut self, source: &str, ref_name: Option<&str>) -> Result<&SifsIndex> {
        let key = Self::key_for(source, ref_name)?;
        if !self.indexes.contains_key(&key) {
            let index = if is_git_url(source) {
                if self.offline {
                    anyhow::bail!("--offline does not allow remote Git sources");
                }
                SifsIndex::from_git_with_index_options(
                    source,
                    ref_name,
                    IndexOptions::new(self.model_options.clone())
                        .with_cache(self.cache_config.clone()),
                )?
            } else {
                SifsIndex::from_path_with_index_options(
                    &key,
                    IndexOptions::new(self.model_options.clone())
                        .with_cache(self.cache_config.clone()),
                )?
            };
            self.indexes.insert(key.clone(), index);
        }
        Ok(self.indexes.get(&key).unwrap())
    }

    fn refresh(&mut self, source: &str, ref_name: Option<&str>) -> Result<&SifsIndex> {
        let key = Self::key_for(source, ref_name)?;
        let index = if is_git_url(source) {
            if self.offline {
                anyhow::bail!("--offline does not allow remote Git sources");
            }
            SifsIndex::from_git_with_index_options(
                source,
                ref_name,
                IndexOptions::new(self.model_options.clone()).with_cache(self.cache_config.clone()),
            )?
        } else {
            SifsIndex::from_path_with_index_options(
                &key,
                IndexOptions::new(self.model_options.clone()).with_cache(self.cache_config.clone()),
            )?
        };
        self.indexes.insert(key.clone(), index);
        Ok(self.indexes.get(&key).unwrap())
    }

    fn remove(&mut self, source: &str, ref_name: Option<&str>) -> Result<bool> {
        let key = Self::key_for(source, ref_name)?;
        Ok(self.indexes.remove(&key).is_some())
    }

    fn contains_source(&self, source: &str, ref_name: Option<&str>) -> bool {
        Self::key_for(source, ref_name)
            .map(|key| self.indexes.contains_key(&key))
            .unwrap_or(false)
    }

    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<_> = self.indexes.keys().cloned().collect();
        keys.sort();
        keys
    }
}

fn handle_tool_call(
    message: &Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> Value {
    let params = message.get("params").cloned().unwrap_or_default();
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let tool_result = match name {
        "search" => tool_search(args, cache, default_source, ref_name),
        "find_related" => tool_find_related(args, cache, default_source, ref_name),
        "index_status" => tool_index_status(args, cache, default_source, ref_name),
        "refresh_index" => tool_refresh_index(args, cache, default_source, ref_name),
        "clear_index" => tool_clear_index(args, cache, default_source, ref_name),
        "list_indexed_files" => tool_list_indexed_files(args, cache, default_source, ref_name),
        "get_chunk" => tool_get_chunk(args, cache, default_source, ref_name),
        "init_agent" => tool_init_agent(args),
        _ => ToolText::error(format!("Unknown tool: {name}")),
    };
    let mut result = json!({"content": [{"type": "text", "text": tool_result.text}]});
    if let Some(structured) = tool_result.structured {
        result["structuredContent"] = structured;
    }
    if tool_result.is_error {
        result["isError"] = json!(true);
    }
    result
}

struct ToolText {
    text: String,
    structured: Option<Value>,
    is_error: bool,
}

impl ToolText {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            structured: None,
            is_error: false,
        }
    }

    fn ok_structured(text: impl Into<String>, structured: Value) -> Self {
        Self {
            text: text.into(),
            structured: Some(structured),
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            structured: None,
            is_error: true,
        }
    }
}

fn tool_search(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> ToolText {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source = match selected_source(&args, default_source) {
        Ok(Some(source)) => source,
        Ok(None) => return ToolText::error(no_repo_message()),
        Err(message) => return ToolText::error(message),
    };
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<SearchMode>().ok())
        .unwrap_or(SearchMode::Hybrid);
    let top_k = args.get("top_k").and_then(Value::as_u64).unwrap_or(5) as usize;
    match cache.get(source, ref_name) {
        Ok(index) => {
            let options = search_options_from_args(&args, top_k, mode);
            let results = match index.search_with(query, &options) {
                Ok(results) => results,
                Err(err) => return ToolText::error(format!("Search failed: {err}")),
            };
            let structured = json!({
                "source": source,
                "mode": mode.to_string(),
                "top_k": top_k,
                "alpha": options.alpha,
                "filter_languages": options.filter_languages,
                "filter_paths": options.filter_paths,
                "stats": index.stats(),
                "warnings": index_warnings(index),
                "results": structured_results(&results),
            });
            if results.is_empty() {
                ToolText::ok_structured(no_results_message(), structured)
            } else {
                ToolText::ok_structured(
                    format_results(
                        &format!("Search results for: {query:?} (mode={mode})"),
                        &results,
                    ),
                    structured,
                )
            }
        }
        Err(err) => ToolText::error(format!("Failed to index {source:?}: {err}")),
    }
}

fn tool_find_related(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> ToolText {
    let file_path = args
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let line = args.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let source = match selected_source(&args, default_source) {
        Ok(Some(source)) => source,
        Ok(None) => return ToolText::error(no_repo_message()),
        Err(message) => return ToolText::error(message),
    };
    let top_k = args.get("top_k").and_then(Value::as_u64).unwrap_or(5) as usize;
    match cache.get(source, ref_name) {
        Ok(index) => {
            let Some(chunk) = resolve_chunk(&index.chunks, file_path, line) else {
                return ToolText::ok(format!(
                    "No chunk found at {file_path}:{line}. Make sure the file is indexed and the line number is within a known chunk."
                ));
            };
            let results = match index.find_related(&chunk, top_k) {
                Ok(results) => results,
                Err(err) => return ToolText::error(format!("find_related failed: {err}")),
            };
            let structured = json!({
                "source": source,
                "file_path": file_path,
                "line": line,
                "stats": index.stats(),
                "warnings": index_warnings(index),
                "results": structured_results(&results),
            });
            if results.is_empty() {
                ToolText::ok_structured(
                    format!("No related chunks found for {file_path}:{line}."),
                    structured,
                )
            } else {
                ToolText::ok_structured(
                    format_results(&format!("Chunks related to {file_path}:{line}"), &results),
                    structured,
                )
            }
        }
        Err(err) => ToolText::error(format!("Failed to index {source:?}: {err}")),
    }
}

fn tool_index_status(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> ToolText {
    let source = match selected_source(&args, default_source) {
        Ok(Some(source)) => source,
        Ok(None) => {
            return ToolText::ok_structured(
                server_context(cache, default_source, ref_name),
                server_context_json(cache, default_source, ref_name),
            );
        }
        Err(message) => return ToolText::error(message),
    };
    let was_cached = cache.contains_source(source, ref_name);
    match cache.get(source, ref_name) {
        Ok(index) => {
            let stats = index.stats();
            let structured = json!({
                "source": source,
                "ref": ref_name,
                "memory_cached": was_cached,
                "stats": stats,
                "warnings": index_warnings(index),
                "semantic_loaded": index.semantic_loaded(),
                "tools": tool_names(),
            });
            ToolText::ok_structured(
                format!(
                    "Index status for {source:?}: {} files, {} chunks, languages: {}. Warnings: {}. Memory cache: {}. Semantic index: {}.",
                    stats.indexed_files,
                    stats.total_chunks,
                    format_languages(&stats.languages),
                    index.warnings().len(),
                    if was_cached { "hit" } else { "built or loaded" },
                    if index.semantic_loaded() {
                        "loaded"
                    } else {
                        "not loaded"
                    }
                ),
                structured,
            )
        }
        Err(err) => ToolText::error(format!("Failed to index {source:?}: {err}")),
    }
}

fn tool_refresh_index(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> ToolText {
    let source = match selected_source(&args, default_source) {
        Ok(Some(source)) => source,
        Ok(None) => return ToolText::error(no_repo_message()),
        Err(message) => return ToolText::error(message),
    };
    match cache.refresh(source, ref_name) {
        Ok(index) => {
            let stats = index.stats();
            ToolText::ok_structured(
                format!(
                    "Refreshed index for {source:?}: {} files, {} chunks, {} warnings.",
                    stats.indexed_files,
                    stats.total_chunks,
                    index.warnings().len()
                ),
                json!({"source": source, "ref": ref_name, "stats": stats, "warnings": index_warnings(index), "refreshed": true}),
            )
        }
        Err(err) => ToolText::error(format!("Failed to refresh {source:?}: {err}")),
    }
}

fn tool_clear_index(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> ToolText {
    let source = match selected_source(&args, default_source) {
        Ok(Some(source)) => source,
        Ok(None) => return ToolText::error(no_repo_message()),
        Err(message) => return ToolText::error(message),
    };
    match cache.remove(source, ref_name) {
        Ok(removed) => ToolText::ok_structured(
            if removed {
                format!("Cleared in-memory index for {source:?}.")
            } else {
                format!("No in-memory index was cached for {source:?}.")
            },
            json!({"source": source, "ref": ref_name, "removed": removed}),
        ),
        Err(err) => ToolText::error(format!("Failed to clear {source:?}: {err}")),
    }
}

fn tool_list_indexed_files(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> ToolText {
    let source = match selected_source(&args, default_source) {
        Ok(Some(source)) => source,
        Ok(None) => return ToolText::error(no_repo_message()),
        Err(message) => return ToolText::error(message),
    };
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
    match cache.get(source, ref_name) {
        Ok(index) => {
            let files = index.indexed_files();
            let shown: Vec<_> = files.iter().take(limit).cloned().collect();
            ToolText::ok_structured(
                format!(
                    "Indexed files for {source:?} (showing {} of {}):\n{}",
                    shown.len(),
                    files.len(),
                    shown.join("\n")
                ),
                json!({"source": source, "total": files.len(), "limit": limit, "warnings": index_warnings(index), "files": shown}),
            )
        }
        Err(err) => ToolText::error(format!("Failed to index {source:?}: {err}")),
    }
}

fn tool_get_chunk(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> ToolText {
    let file_path = args
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let line = args.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let source = match selected_source(&args, default_source) {
        Ok(Some(source)) => source,
        Ok(None) => return ToolText::error(no_repo_message()),
        Err(message) => return ToolText::error(message),
    };
    match cache.get(source, ref_name) {
        Ok(index) => {
            let Some(chunk) = resolve_chunk(&index.chunks, file_path, line) else {
                return ToolText::ok(format!(
                    "No chunk found at {file_path}:{line}. Use list_indexed_files to check indexed paths."
                ));
            };
            ToolText::ok_structured(
                format!(
                    "{}\n\n```{}\n{}\n```",
                    chunk.location(),
                    chunk.language.clone().unwrap_or_default(),
                    chunk.content
                ),
                json!({"source": source, "warnings": index_warnings(index), "chunk": chunk}),
            )
        }
        Err(err) => ToolText::error(format!("Failed to index {source:?}: {err}")),
    }
}

fn tool_init_agent(args: Value) -> ToolText {
    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
    let dest = args
        .get("destination")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(".claude")
                .join("agents")
                .join("sifs-search.md")
        });
    if dest.exists() && !force {
        return ToolText::error(format!(
            "{} already exists. Call init_agent with force=true to overwrite.",
            dest.display()
        ));
    }
    if let Some(parent) = dest.parent()
        && let Err(err) = fs::create_dir_all(parent)
    {
        return ToolText::error(format!("Failed to create {}: {err}", parent.display()));
    }
    if let Err(err) = fs::write(&dest, include_str!("agents/sifs-search.md")) {
        return ToolText::error(format!("Failed to write {}: {err}", dest.display()));
    }
    ToolText::ok_structured(
        format!("Created {}", dest.display()),
        json!({"destination": dest, "force": force, "created": true}),
    )
}

fn selected_source<'a>(
    args: &'a Value,
    default_source: Option<&'a str>,
) -> std::result::Result<Option<&'a str>, String> {
    let requested = args.get("repo").and_then(Value::as_str);
    if let Some(default_source) = default_source {
        if let Some(requested) = requested
            && !sources_match(requested, default_source)
        {
            return Err(format!(
                "This MCP server is scoped to {default_source:?}; refusing repo override {requested:?}."
            ));
        }
        return Ok(Some(default_source));
    }
    Ok(requested)
}

fn sources_match(requested: &str, default_source: &str) -> bool {
    if requested == default_source {
        return true;
    }
    if is_git_url(requested) || is_git_url(default_source) {
        return false;
    }
    let Ok(requested) = Path::new(requested).canonicalize() else {
        return false;
    };
    let Ok(default_source) = Path::new(default_source).canonicalize() else {
        return false;
    };
    requested == default_source
}

fn search_options_from_args(args: &Value, top_k: usize, mode: SearchMode) -> SearchOptions {
    let mut options = SearchOptions::new(top_k).with_mode(mode);
    options.alpha = args.get("alpha").and_then(Value::as_f64).map(|v| v as f32);
    options.filter_languages = string_array_arg(args, "filter_languages");
    options.filter_paths = string_array_arg(args, "filter_paths");
    options
}

fn string_array_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn structured_results(results: &[crate::types::SearchResult]) -> Value {
    json!(
        results
            .iter()
            .map(|result| json!({
                "file_path": result.chunk.file_path,
                "start_line": result.chunk.start_line,
                "end_line": result.chunk.end_line,
                "language": result.chunk.language,
                "score": result.score,
                "source": result.source.to_string(),
                "content": result.chunk.content,
            }))
            .collect::<Vec<_>>()
    )
}

fn index_warnings(index: &SifsIndex) -> Value {
    json!(
        index
            .warnings()
            .iter()
            .map(|warning| json!({
                "path": warning.path,
                "message": warning.message,
            }))
            .collect::<Vec<_>>()
    )
}

fn no_results_message() -> &'static str {
    NO_RESULTS_MESSAGE.trim()
}

fn no_repo_message() -> &'static str {
    NO_REPO_MESSAGE.trim()
}

fn build_instructions(default_source: Option<&str>, ref_name: Option<&str>) -> String {
    let source_context = match default_source {
        Some(source) => format!(
            "\n\nCurrent server context:\n- Default source: {source}\n- Git ref: {}\n- Repo override policy: tool calls may omit `repo`; if provided it must match the default source.\n- Long-lived sessions cache indexes in memory; call `refresh_index` after files change.",
            ref_name.unwrap_or("none")
        ),
        None => "\n\nCurrent server context:\n- No default source is configured. Tool calls must pass `repo` as a local path or Git URL.".to_owned(),
    };
    format!("{}{}", MCP_INSTRUCTIONS.trim(), source_context)
}

fn resource_schemas() -> Vec<Value> {
    vec![
        json!({
            "uri": "sifs://server/context",
            "name": "SIFS server context",
            "description": "Default source, ref, cache keys, and available tools.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "sifs://index/status",
            "name": "SIFS index status",
            "description": "Stats for the default source, when one is configured.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "sifs://index/files",
            "name": "SIFS indexed files",
            "description": "Indexed file paths for the default source, when one is configured.",
            "mimeType": "application/json"
        }),
    ]
}

fn handle_resource_read(
    message: &Value,
    cache: &IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> Value {
    let uri = message
        .get("params")
        .and_then(|params| params.get("uri"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let payload = match uri {
        "sifs://server/context" => server_context_json(cache, default_source, ref_name),
        "sifs://index/status" => json!({
            "message": "Call the index_status tool to build or inspect the default index.",
            "default_source": default_source,
            "ref": ref_name,
            "memory_cached": default_source.map(|source| cache.contains_source(source, ref_name)).unwrap_or(false),
        }),
        "sifs://index/files" => json!({
            "message": "Call the list_indexed_files tool to build or inspect the default index file list.",
            "default_source": default_source,
            "ref": ref_name,
        }),
        _ => json!({"error": format!("Unknown resource: {uri}")}),
    };
    json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": serde_json::to_string_pretty(&payload).unwrap()
        }]
    })
}

fn server_context(
    cache: &IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> String {
    serde_json::to_string_pretty(&server_context_json(cache, default_source, ref_name)).unwrap()
}

fn server_context_json(
    cache: &IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> Value {
    json!({
        "server": "sifs",
        "version": env!("CARGO_PKG_VERSION"),
        "default_source": default_source,
        "ref": ref_name,
        "cache_keys": cache.keys(),
        "tools": tool_names(),
        "resources": resource_schemas().into_iter().map(|resource| resource["uri"].clone()).collect::<Vec<_>>(),
    })
}

fn tool_names() -> Vec<&'static str> {
    vec![
        "search",
        "find_related",
        "index_status",
        "refresh_index",
        "clear_index",
        "list_indexed_files",
        "get_chunk",
        "init_agent",
    ]
}

fn format_languages(languages: &std::collections::BTreeMap<String, usize>) -> String {
    if languages.is_empty() {
        return "none".to_owned();
    }
    languages
        .iter()
        .map(|(language, count)| format!("{language}={count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn tool_schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "search",
            "description": SEARCH_DESCRIPTION.trim(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Natural language or code query."},
                    "repo": {"type": ["string", "null"], "description": "Git URL or local path to index and search."},
                    "mode": {"type": "string", "enum": ["hybrid", "semantic", "bm25"], "default": "hybrid", "description": "Use hybrid by default, bm25 for exact symbols/literals, and semantic for conceptual queries."},
                    "top_k": {"type": "integer", "minimum": 1, "default": 5, "description": "Maximum number of ranked chunks to return."},
                    "alpha": {"type": ["number", "null"], "minimum": 0, "maximum": 1, "description": "Optional hybrid semantic weight. Omit to let SIFS choose from query shape."},
                    "filter_languages": {"type": "array", "items": {"type": "string"}, "description": "Optional exact language labels to search, such as rust or typescript."},
                    "filter_paths": {"type": "array", "items": {"type": "string"}, "description": "Optional repository-relative file paths to search."}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "find_related",
            "description": FIND_RELATED_DESCRIPTION.trim(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {"type": "string", "description": "Repository-relative file path exactly as shown in a search result."},
                    "line": {"type": "integer", "minimum": 1, "description": "One-based line number inside the known chunk."},
                    "repo": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."},
                    "top_k": {"type": "integer", "minimum": 1, "default": 5, "description": "Maximum number of related chunks to return."}
                },
                "required": ["file_path", "line"]
            }
        }),
        json!({
            "name": "index_status",
            "description": INDEX_STATUS_DESCRIPTION.trim(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."}
                }
            }
        }),
        json!({
            "name": "refresh_index",
            "description": "Rebuild the selected index and replace the in-memory MCP cache. Use after files change in a long-lived session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."}
                }
            }
        }),
        json!({
            "name": "clear_index",
            "description": "Remove the selected source from the in-memory MCP cache. The next search or status call rebuilds or reloads it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."}
                }
            }
        }),
        json!({
            "name": "list_indexed_files",
            "description": "List repository-relative file paths included in the selected index.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."},
                    "limit": {"type": "integer", "minimum": 1, "default": 200, "description": "Maximum number of file paths to return."}
                }
            }
        }),
        json!({
            "name": "get_chunk",
            "description": "Read the indexed chunk containing a repository-relative file path and one-based line.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {"type": "string", "description": "Repository-relative file path exactly as shown in a search result or list_indexed_files."},
                    "line": {"type": "integer", "minimum": 1, "description": "One-based line number inside the desired chunk."},
                    "repo": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."}
                },
                "required": ["file_path", "line"]
            }
        }),
        json!({
            "name": "init_agent",
            "description": "Create the SIFS Claude agent file in the shared workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "destination": {"type": ["string", "null"], "description": "Optional path for the generated agent file. Defaults to .claude/agents/sifs-search.md."},
                    "force": {"type": "boolean", "default": false, "description": "Overwrite an existing file when true."}
                }
            }
        }),
    ]
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .context("parse Content-Length")?,
            );
        }
    }
    let Some(length) = content_length else {
        return Ok(None);
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

fn write_message(writer: &mut impl Write, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        IndexCache, build_instructions, handle_resource_read, handle_tool_call, selected_source,
        tool_schemas,
    };
    use serde_json::json;

    #[test]
    fn default_source_rejects_repo_override() {
        let args = json!({"repo": "/other/repo"});
        let err = selected_source(&args, Some("/default/repo")).unwrap_err();

        assert!(err.contains("refusing repo override"));
        assert!(err.contains("/default/repo"));
        assert!(err.contains("/other/repo"));
    }

    #[test]
    fn default_source_allows_omitted_or_matching_repo() {
        assert_eq!(
            selected_source(&json!({}), Some("/default/repo")).unwrap(),
            Some("/default/repo")
        );
        assert_eq!(
            selected_source(&json!({"repo": "/default/repo"}), Some("/default/repo")).unwrap(),
            Some("/default/repo")
        );
    }

    #[test]
    fn default_source_allows_equivalent_local_path_spellings() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().to_string_lossy().to_string();
        let requested = temp.path().join(".").to_string_lossy().to_string();
        assert_eq!(
            selected_source(&json!({"repo": requested}), Some(&default)).unwrap(),
            Some(default.as_str())
        );
    }

    #[test]
    fn missing_default_uses_requested_repo() {
        assert_eq!(
            selected_source(&json!({"repo": "/requested/repo"}), None).unwrap(),
            Some("/requested/repo")
        );
        assert_eq!(selected_source(&json!({}), None).unwrap(), None);
    }

    #[test]
    fn unknown_tool_result_is_marked_as_error() {
        let response = handle_tool_call(
            &json!({"params": {"name": "missing_tool", "arguments": {}}}),
            &mut IndexCache::default(),
            None,
            None,
        );

        assert_eq!(response["isError"], true);
        assert!(
            response["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Unknown tool")
        );
    }

    #[test]
    fn initialize_instructions_include_dynamic_default_source() {
        let instructions = build_instructions(Some("/default/repo"), Some("main"));

        assert!(instructions.contains("Default source: /default/repo"));
        assert!(instructions.contains("Git ref: main"));
        assert!(instructions.contains("refresh_index"));
    }

    #[test]
    fn tool_schemas_expose_agent_native_index_tools_and_full_search_options() {
        let schemas = tool_schemas();
        let names: Vec<_> = schemas
            .iter()
            .filter_map(|schema| schema["name"].as_str())
            .collect();

        assert!(names.contains(&"index_status"));
        assert!(names.contains(&"refresh_index"));
        assert!(names.contains(&"clear_index"));
        assert!(names.contains(&"list_indexed_files"));
        assert!(names.contains(&"get_chunk"));
        assert!(names.contains(&"init_agent"));

        let search = schemas
            .iter()
            .find(|schema| schema["name"] == "search")
            .unwrap();
        let props = &search["inputSchema"]["properties"];
        assert!(props.get("alpha").is_some());
        assert!(props.get("filter_languages").is_some());
        assert!(props.get("filter_paths").is_some());
    }

    #[test]
    fn server_context_resource_is_readable() {
        let response = handle_resource_read(
            &json!({"params": {"uri": "sifs://server/context"}}),
            &IndexCache::default(),
            Some("/default/repo"),
            None,
        );

        let text = response["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("/default/repo"));
        assert!(text.contains("index_status"));
    }

    #[test]
    fn init_agent_tool_writes_generated_agent_file() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("sifs-search.md");
        let response = handle_tool_call(
            &json!({"params": {"name": "init_agent", "arguments": {"destination": destination}}}),
            &mut IndexCache::default(),
            None,
            None,
        );

        assert!(response.get("isError").is_none());
        let content = std::fs::read_to_string(temp.path().join("sifs-search.md")).unwrap();
        assert!(content.contains("name: sifs-search"));
        assert!(content.contains("## Capabilities"));
    }
}
