use crate::SifsIndex;
use crate::agent_artifacts::{AgentArtifact, AgentTarget, render_artifact};
use crate::daemon::{
    DaemonClient, DaemonRequest, DaemonResult, IndexRuntimeOptions, SearchOptionsWire, SourceSpec,
    default_daemon_paths,
};
use crate::index::{CacheConfig, IndexOptions};
use crate::model2vec::{EncoderSpec, ModelOptions};
use crate::types::{SearchMode, SearchOptions};
use crate::utils::{fenced_code_block, format_results, is_git_url, resolve_chunk};
use crate::{agent_context, feedback, platform_cache_root, profiles};
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
const MCP_MAX_LIMIT: usize = 50;
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[DEFAULT_PROTOCOL_VERSION];

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
    let default_source = match default_source {
        Some(source) => Some(source),
        None => std::env::current_dir()
            .ok()
            .filter(|path| path.is_dir())
            .map(|path| path.to_string_lossy().into_owned()),
    };
    let mut cache = IndexCache::new(model_options, cache_config, offline);
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();

    while let Some(incoming) = read_message(&mut reader)? {
        let message = incoming.value;
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = match method {
            "initialize" => json!({
                "protocolVersion": negotiated_protocol_version(&message),
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
            incoming.framing,
        )?;
    }
    Ok(())
}

fn negotiated_protocol_version(message: &Value) -> &'static str {
    let requested = message
        .get("params")
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    requested
        .and_then(|version| {
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|supported| *supported == version)
        })
        .unwrap_or(DEFAULT_PROTOCOL_VERSION)
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
        } else if self
            .indexes
            .get(&key)
            .and_then(SifsIndex::is_fresh)
            .is_some_and(|fresh| !fresh)
        {
            self.refresh(source, ref_name)?;
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

    fn daemon_search(
        &self,
        source: &str,
        ref_name: Option<&str>,
        query: &str,
        options: &SearchOptions,
    ) -> Option<crate::daemon::protocol::SearchResultSet> {
        let paths = default_daemon_paths().ok()?;
        if !paths.socket.exists() {
            return None;
        }
        let source = SourceSpec::resolve(source, ref_name.map(str::to_owned), self.offline).ok()?;
        let runtime_options = match options.mode {
            SearchMode::Bm25 => IndexRuntimeOptions::sparse(self.cache_config.clone()),
            SearchMode::Semantic | SearchMode::Hybrid => IndexRuntimeOptions::with_encoder(
                EncoderSpec::Model2Vec(self.model_options.clone()),
                self.cache_config.clone(),
            ),
        };
        match DaemonClient::new(paths).send(DaemonRequest::Search {
            source,
            options: runtime_options,
            query: query.to_owned(),
            search: SearchOptionsWire::from(options.clone()),
        }) {
            Ok(DaemonResult::Search(result)) => Some(result),
            _ => None,
        }
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
        "list_files" => tool_list_files(args, cache, default_source, ref_name),
        "get_chunk" => tool_get_chunk(args, cache, default_source, ref_name),
        "agent_context" => tool_agent_context(),
        "profile_list" => tool_profile_list(),
        "profile_show" => tool_profile_show(args),
        "feedback_create" => tool_feedback_create(args),
        "feedback_list" => tool_feedback_list(args),
        "agent_print" => tool_agent_print(args),
        "agent_doctor" => tool_agent_doctor(args),
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
    let profile = match selected_profile(&args) {
        Ok(profile) => profile,
        Err(message) => return ToolText::error(message),
    };
    let mode = match parse_mcp_mode(&args, profile.as_ref()) {
        Ok(mode) => mode,
        Err(message) => return ToolText::error(message),
    };
    let top_k = match parse_mcp_limit(&args, "limit", profile.as_ref().and_then(|p| p.limit), 5) {
        Ok(limit) => limit,
        Err(message) => return ToolText::error(message),
    };
    let options = match search_options_from_args(&args, top_k, mode) {
        Ok(options) => options,
        Err(message) => return ToolText::error(message),
    };
    if let Some(result) = cache.daemon_search(&source, ref_name, query, &options) {
        return mcp_search_result(McpSearchPresentation {
            source: &source,
            query,
            mode,
            top_k,
            options: &options,
            stats: result.stats,
            warnings: json!([]),
            fresh: None,
            results: result.results,
        });
    }
    match cache.get(&source, ref_name) {
        Ok(index) => {
            let results = match index.search_with(query, &options) {
                Ok(results) => results,
                Err(err) => return ToolText::error(format!("Search failed: {err}")),
            };
            mcp_search_result(McpSearchPresentation {
                source: &source,
                query,
                mode,
                top_k,
                options: &options,
                stats: index.stats(),
                warnings: search_warnings_json(index, &options),
                fresh: index.is_fresh(),
                results,
            })
        }
        Err(err) => ToolText::error(format!("Failed to index {source:?}: {err}")),
    }
}

struct McpSearchPresentation<'a> {
    source: &'a str,
    query: &'a str,
    mode: SearchMode,
    top_k: usize,
    options: &'a SearchOptions,
    stats: crate::IndexStats,
    warnings: Value,
    fresh: Option<bool>,
    results: Vec<crate::SearchResult>,
}

fn mcp_search_result(presentation: McpSearchPresentation<'_>) -> ToolText {
    let structured = json!({
        "source": presentation.source,
        "mode": presentation.mode.to_string(),
        "limit": presentation.top_k,
        "alpha": presentation.options.alpha,
        "filter_languages": presentation.options.filter_languages,
        "filter_paths": presentation.options.filter_paths,
        "stats": presentation.stats,
        "fresh": presentation.fresh,
        "warnings": presentation.warnings,
        "truncated": presentation.results.len() >= presentation.top_k,
        "hint": if presentation.results.len() >= presentation.top_k { Some("Increase limit or add filter_languages/filter_paths to narrow the search.") } else { None },
        "results": structured_results(&presentation.results),
    });
    if presentation.results.is_empty() {
        ToolText::ok_structured(no_results_message(), structured)
    } else {
        ToolText::ok_structured(
            format_results(
                &format!(
                    "Search results for: {:?} (mode={})",
                    presentation.query, presentation.mode
                ),
                &presentation.results,
            ),
            structured,
        )
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
    let profile = match selected_profile(&args) {
        Ok(profile) => profile,
        Err(message) => return ToolText::error(message),
    };
    let top_k = match parse_mcp_limit(&args, "limit", profile.as_ref().and_then(|p| p.limit), 5) {
        Ok(limit) => limit,
        Err(message) => return ToolText::error(message),
    };
    match cache.get(&source, ref_name) {
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
    let was_cached = cache.contains_source(&source, ref_name);
    match cache.get(&source, ref_name) {
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
    match cache.refresh(&source, ref_name) {
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
    match cache.remove(&source, ref_name) {
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

fn tool_list_files(
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
    let limit = match parse_mcp_limit(&args, "limit", None, 200) {
        Ok(limit) => limit,
        Err(message) => return ToolText::error(message),
    };
    match cache.get(&source, ref_name) {
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
                json!({"source": source, "total": files.len(), "limit": limit, "truncated": files.len() > shown.len(), "hint": if files.len() > shown.len() { Some("Increase limit to inspect more indexed files.") } else { None }, "warnings": index_warnings(index), "files": shown}),
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
    match cache.get(&source, ref_name) {
        Ok(index) => {
            let Some(chunk) = resolve_chunk(&index.chunks, file_path, line) else {
                return ToolText::ok(format!(
                    "No chunk found at {file_path}:{line}. Use list_files to check indexed paths."
                ));
            };
            ToolText::ok_structured(
                format!(
                    "{}\n\n{}",
                    chunk.location(),
                    fenced_code_block(chunk.language.as_deref(), &chunk.content)
                ),
                json!({"source": source, "warnings": index_warnings(index), "chunk": chunk}),
            )
        }
        Err(err) => ToolText::error(format!("Failed to index {source:?}: {err}")),
    }
}

fn tool_agent_print(args: Value) -> ToolText {
    let target = match parse_agent_target(&args, "target", false) {
        Ok(target) => target,
        Err(message) => return ToolText::error(message),
    };
    let artifact = match parse_agent_artifact(&args, "artifact", false) {
        Ok(artifact) => artifact,
        Err(message) => return ToolText::error(message),
    };
    let source = args.get("source").and_then(Value::as_str);
    let profile = args.get("profile").and_then(Value::as_str);
    match render_artifact(target, artifact, source, profile) {
        Ok(rendered) => {
            ToolText::ok_structured(rendered.content.clone(), json!(rendered.print_output()))
        }
        Err(err) => ToolText::error(err.to_string()),
    }
}

fn tool_agent_doctor(args: Value) -> ToolText {
    let target = match parse_agent_target(&args, "target", true) {
        Ok(target) => target,
        Err(message) => return ToolText::error(message),
    };
    let artifact = match parse_agent_artifact(&args, "artifact", true) {
        Ok(artifact) => artifact,
        Err(message) => return ToolText::error(message),
    };
    let report = crate::agent_doctor::doctor(target, artifact);
    ToolText::ok_structured(
        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "agent doctor failed".to_owned()),
        json!(report),
    )
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
    let rendered = match render_artifact(AgentTarget::ClaudeCode, AgentArtifact::Skill, None, None)
    {
        Ok(rendered) => rendered,
        Err(err) => return ToolText::error(format!("Failed to render Claude agent file: {err}")),
    };
    if let Err(err) = fs::write(&dest, &rendered.content) {
        return ToolText::error(format!("Failed to write {}: {err}", dest.display()));
    }
    ToolText::ok_structured(
        format!("Created {}", dest.display()),
        json!({
            "destination": dest,
            "force": force,
            "created": true,
            "checksum": rendered.checksum,
            "next_actions": ["sifs agent install --target claude-code --artifact skill --destination .claude/agents/sifs-search.md"]
        }),
    )
}

fn parse_agent_target(
    args: &Value,
    field: &str,
    allow_all_default: bool,
) -> std::result::Result<AgentTarget, String> {
    let value = args.get(field).and_then(Value::as_str);
    match value {
        Some("codex") => Ok(AgentTarget::Codex),
        Some("claude-code") | Some("claude") => Ok(AgentTarget::ClaudeCode),
        Some("openclaw") => Ok(AgentTarget::Openclaw),
        Some("hermes") => Ok(AgentTarget::Hermes),
        Some("generic") => Ok(AgentTarget::Generic),
        Some("all") if allow_all_default => Ok(AgentTarget::All),
        None if allow_all_default => Ok(AgentTarget::All),
        None => Err(format!("{field} is required")),
        Some(other) => Err(format!("unsupported agent target: {other}")),
    }
}

fn parse_agent_artifact(
    args: &Value,
    field: &str,
    allow_all_default: bool,
) -> std::result::Result<AgentArtifact, String> {
    let value = args.get(field).and_then(Value::as_str);
    match value {
        Some("skill") => Ok(AgentArtifact::Skill),
        Some("snippet") => Ok(AgentArtifact::Snippet),
        Some("mcp") => Ok(AgentArtifact::Mcp),
        Some("all") if allow_all_default => Ok(AgentArtifact::All),
        None if allow_all_default => Ok(AgentArtifact::All),
        None => Err(format!("{field} is required")),
        Some(other) => Err(format!("unsupported agent artifact: {other}")),
    }
}

fn selected_source(
    args: &Value,
    default_source: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    if args.get("repo").is_some() {
        return Err("repo is no longer a canonical MCP argument; use source instead".to_owned());
    }
    let requested = args.get("source").and_then(Value::as_str);
    if requested.is_some_and(str::is_empty) {
        return Err("source must not be empty; omit it to use the default source".to_owned());
    }
    if let Some(requested) = requested {
        return Ok(Some(requested.to_owned()));
    }
    if let Some(profile_name) = args.get("profile").and_then(Value::as_str) {
        if profile_name.is_empty() {
            return Err("profile must not be empty".to_owned());
        }
        let root = platform_cache_root().map_err(|err| err.to_string())?;
        let profile = profiles::get_profile(&root, profile_name).map_err(|err| err.to_string())?;
        if let Some(source) = profile.source {
            return Ok(Some(source));
        }
        return Err(format!("profile {profile_name:?} does not define a source"));
    }
    Ok(default_source.map(str::to_owned))
}

fn search_options_from_args(
    args: &Value,
    top_k: usize,
    mode: SearchMode,
) -> std::result::Result<SearchOptions, String> {
    let mut options = SearchOptions::new(top_k).with_mode(mode);
    options.alpha = match args.get("alpha") {
        Some(Value::Null) | None => None,
        Some(value) => {
            let Some(alpha) = value.as_f64() else {
                return Err("alpha must be a number between 0 and 1".to_owned());
            };
            if !(0.0..=1.0).contains(&alpha) {
                return Err("alpha must be between 0 and 1".to_owned());
            }
            Some(alpha as f32)
        }
    };
    options.filter_languages = string_array_arg(args, "filter_languages")?;
    options.filter_paths = string_array_arg(args, "filter_paths")?;
    options.explain = args
        .get("explain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(options)
}

fn selected_profile(args: &Value) -> std::result::Result<Option<profiles::Profile>, String> {
    let Some(profile_name) = args.get("profile").and_then(Value::as_str) else {
        return Ok(None);
    };
    if profile_name.is_empty() {
        return Err("profile must not be empty".to_owned());
    }
    let root = platform_cache_root().map_err(|err| err.to_string())?;
    profiles::get_profile(&root, profile_name)
        .map(Some)
        .map_err(|err| err.to_string())
}

fn parse_mcp_mode(
    args: &Value,
    profile: Option<&profiles::Profile>,
) -> std::result::Result<SearchMode, String> {
    match args.get("mode").and_then(Value::as_str) {
        Some(value) => value
            .parse::<SearchMode>()
            .map_err(|_| format!("mode must be one of: hybrid, semantic, bm25 (got {value:?})")),
        None => Ok(profile
            .and_then(|profile| profile.mode)
            .unwrap_or(SearchMode::Hybrid)),
    }
}

fn parse_mcp_limit(
    args: &Value,
    key: &str,
    profile_default: Option<usize>,
    default: usize,
) -> std::result::Result<usize, String> {
    let Some(value) = args.get(key) else {
        return Ok(profile_default.unwrap_or(default));
    };
    let Some(limit) = value.as_u64() else {
        return Err(format!("{key} must be an integer >= 1"));
    };
    if limit == 0 {
        return Err(format!("{key} must be at least 1"));
    }
    if limit as usize > MCP_MAX_LIMIT {
        return Err(format!("{key} must be at most {MCP_MAX_LIMIT}"));
    }
    Ok(limit as usize)
}

fn string_array_arg(args: &Value, key: &str) -> std::result::Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        return Err(format!("{key} must be an array of strings"));
    };
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} must contain only strings"))
        })
        .collect()
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
                "symbols": result.chunk.symbols,
                "breadcrumbs": result.chunk.breadcrumbs,
                "explanation": result.explanation,
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

fn search_warnings_json(index: &SifsIndex, options: &SearchOptions) -> Value {
    let mut warnings = index
        .warnings()
        .iter()
        .map(|warning| {
            json!({
                "kind": "index_warning",
                "path": warning.path,
                "message": warning.message,
            })
        })
        .collect::<Vec<_>>();
    let indexed_files = index.indexed_files();
    for path in &options.filter_paths {
        if !indexed_files.iter().any(|indexed| indexed == path) {
            let normalized = path.strip_prefix("./").unwrap_or(path);
            let suggestion = indexed_files
                .iter()
                .find(|indexed| indexed.as_str() == normalized)
                .cloned();
            warnings.push(json!({
                "kind": "path_filter_no_match",
                "message": if let Some(suggestion) = &suggestion {
                    format!("No indexed file exactly matched {path:?}. Did you mean {suggestion:?}?")
                } else {
                    format!("No indexed file matched {path:?}.")
                },
                "suggestions": suggestion.into_iter().collect::<Vec<_>>(),
            }));
        }
    }
    let languages = index.stats().languages;
    for language in &options.filter_languages {
        if !languages.contains_key(language) {
            warnings.push(json!({
                "kind": "language_filter_no_match",
                "message": format!("No indexed chunks matched language {language:?}."),
                "valid_languages": languages.keys().cloned().collect::<Vec<_>>(),
            }));
        }
    }
    json!(warnings)
}

fn tool_agent_context() -> ToolText {
    let names = platform_cache_root()
        .ok()
        .and_then(|root| profiles::profile_names(&root).ok())
        .unwrap_or_default();
    let structured = agent_context::agent_context(names, true);
    ToolText::ok_structured(
        serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "{}".to_owned()),
        structured,
    )
}

fn tool_profile_list() -> ToolText {
    match platform_cache_root().and_then(|root| {
        let profiles = profiles::load_profiles(&root)?;
        Ok(json!({"profiles": profiles, "total": profiles.len(), "path": profiles::profile_store_path(&root)}))
    }) {
        Ok(structured) => ToolText::ok_structured(
            serde_json::to_string_pretty(&structured).unwrap_or_default(),
            structured,
        ),
        Err(err) => ToolText::error(format!("Failed to list profiles: {err}")),
    }
}

fn tool_profile_show(args: Value) -> ToolText {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    match platform_cache_root().and_then(|root| {
        let profile = profiles::get_profile(&root, name)?;
        Ok(json!({"profile": profile, "path": profiles::profile_store_path(&root)}))
    }) {
        Ok(structured) => ToolText::ok_structured(
            serde_json::to_string_pretty(&structured).unwrap_or_default(),
            structured,
        ),
        Err(err) => ToolText::error(format!("Failed to show profile: {err}")),
    }
}

fn tool_feedback_create(args: Value) -> ToolText {
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let command_context = args
        .get("command_context")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let query = args.get("query").and_then(Value::as_str).map(str::to_owned);
    let expected = args
        .get("expected")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match platform_cache_root().and_then(|root| {
        feedback::create_feedback_case(&root, message, command_context, query, expected)
            .map(|entry| (root, entry))
    }) {
        Ok((root, entry)) => ToolText::ok_structured(
            format!("Feedback recorded locally: {}", entry.id),
            json!({"changed": true, "feedback": entry, "path": feedback::feedback_log_path(&root)}),
        ),
        Err(err) => ToolText::error(format!("Failed to record feedback: {err}")),
    }
}

fn tool_feedback_list(args: Value) -> ToolText {
    let limit = match parse_mcp_limit(&args, "limit", None, 20) {
        Ok(limit) => limit,
        Err(message) => return ToolText::error(message),
    };
    match platform_cache_root().and_then(|root| {
        let (entries, total) = feedback::list_feedback(&root, limit)?;
        Ok(json!({
            "feedback": entries,
            "total": total,
            "limit": limit,
            "truncated": total > limit,
            "path": feedback::feedback_log_path(&root),
        }))
    }) {
        Ok(structured) => ToolText::ok_structured(
            serde_json::to_string_pretty(&structured).unwrap_or_default(),
            structured,
        ),
        Err(err) => ToolText::error(format!("Failed to list feedback: {err}")),
    }
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
            "\n\nCurrent server context:\n- Default source: {source}\n- Git ref: {}\n- Source selection policy: tool calls may omit `source` to use the default source, pass `source` to search another local path or Git URL, or pass `profile` to use a saved profile.\n- Long-lived sessions cache indexes in memory; call `refresh_index` after files change.",
            ref_name.unwrap_or("none")
        ),
        None => "\n\nCurrent server context:\n- No default source is configured. Tool calls must pass `source` as a local path or Git URL, or pass a saved `profile`.".to_owned(),
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
            "uri": "sifs://agent/context",
            "name": "SIFS agent context",
            "description": "Versioned CLI and MCP contract for agents.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "sifs://profiles",
            "name": "SIFS profiles",
            "description": "Saved source and search profiles.",
            "mimeType": "application/json"
        }),
        json!({
            "uri": "sifs://feedback",
            "name": "SIFS feedback",
            "description": "Local feedback entries recorded by agents.",
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
        "sifs://agent/context" => {
            let names = platform_cache_root()
                .ok()
                .and_then(|root| profiles::profile_names(&root).ok())
                .unwrap_or_default();
            agent_context::agent_context(names, true)
        }
        "sifs://profiles" => {
            match platform_cache_root().and_then(|root| {
                let profiles = profiles::load_profiles(&root)?;
                Ok(json!({"profiles": profiles, "path": profiles::profile_store_path(&root)}))
            }) {
                Ok(value) => value,
                Err(err) => json!({"error": err.to_string()}),
            }
        }
        "sifs://feedback" => {
            match platform_cache_root().and_then(|root| {
                let (entries, total) = feedback::list_feedback(&root, 20)?;
                Ok(json!({"feedback": entries, "total": total, "limit": 20, "path": feedback::feedback_log_path(&root)}))
            }) {
                Ok(value) => value,
                Err(err) => json!({"error": err.to_string()}),
            }
        }
        "sifs://index/status" => json!({
            "message": "Call the index_status tool to build or inspect the default index.",
            "default_source": default_source,
            "ref": ref_name,
            "memory_cached": default_source.map(|source| cache.contains_source(source, ref_name)).unwrap_or(false),
        }),
        "sifs://index/files" => json!({
            "message": "Call the list_files tool to build or inspect the default index file list.",
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
        "list_files",
        "get_chunk",
        "agent_context",
        "profile_list",
        "profile_show",
        "feedback_create",
        "feedback_list",
        "agent_print",
        "agent_doctor",
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
            "name": "agent_context",
            "description": "Return the versioned SIFS CLI/MCP contract for agents.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "search",
            "description": SEARCH_DESCRIPTION.trim(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Natural language or code query."},
                    "source": {"type": ["string", "null"], "description": "Git URL or local path to index and search."},
                    "profile": {"type": ["string", "null"], "description": "Saved profile to use for source and search defaults."},
                    "mode": {"type": "string", "enum": ["hybrid", "semantic", "bm25"], "default": "hybrid", "description": "Use hybrid by default, bm25 for exact symbols/literals, and semantic for conceptual queries."},
                    "limit": {"type": "integer", "minimum": 1, "default": 5, "description": "Maximum number of ranked chunks to return."},
                    "alpha": {"type": ["number", "null"], "minimum": 0, "maximum": 1, "description": "Optional hybrid semantic weight. Omit to let SIFS choose from query shape."},
                    "filter_languages": {"type": "array", "items": {"type": "string"}, "description": "Optional exact language labels to search, such as rust or typescript."},
                    "filter_paths": {"type": "array", "items": {"type": "string"}, "description": "Optional repository-relative file paths to search."},
                    "explain": {"type": "boolean", "default": false, "description": "Include per-result ranking evidence such as BM25 rank, semantic rank, alpha, and boosted score."}
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
                    "source": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."},
                    "profile": {"type": ["string", "null"], "description": "Saved profile to use for source defaults."},
                    "limit": {"type": "integer", "minimum": 1, "default": 5, "description": "Maximum number of related chunks to return."}
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
                    "source": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."},
                    "profile": {"type": ["string", "null"], "description": "Saved profile to use for source defaults."}
                }
            }
        }),
        json!({
            "name": "refresh_index",
            "description": "Rebuild the selected index and replace the in-memory MCP cache. Use after files change in a long-lived session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."},
                    "profile": {"type": ["string", "null"], "description": "Saved profile to use for source defaults."}
                }
            }
        }),
        json!({
            "name": "clear_index",
            "description": "Remove the selected source from the in-memory MCP cache. The next search or status call rebuilds or reloads it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."},
                    "profile": {"type": ["string", "null"], "description": "Saved profile to use for source defaults."}
                }
            }
        }),
        json!({
            "name": "list_files",
            "description": "List repository-relative file paths included in the selected index.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."},
                    "profile": {"type": ["string", "null"], "description": "Saved profile to use for source defaults."},
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
                    "file_path": {"type": "string", "description": "Repository-relative file path exactly as shown in a search result or list_files."},
                    "line": {"type": "integer", "minimum": 1, "description": "One-based line number inside the desired chunk."},
                    "source": {"type": ["string", "null"], "description": "Git URL or local path. Omit only when the server has a default source."},
                    "profile": {"type": ["string", "null"], "description": "Saved profile to use for source defaults."}
                },
                "required": ["file_path", "line"]
            }
        }),
        json!({
            "name": "profile_list",
            "description": "List saved SIFS profiles.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "profile_show",
            "description": "Show one saved SIFS profile.",
            "inputSchema": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"]
            }
        }),
        json!({
            "name": "feedback_create",
            "description": "Record local feedback about SIFS agent friction.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message": {"type": "string"},
                    "command_context": {"type": ["string", "null"]},
                    "query": {"type": ["string", "null"], "description": "Optional search query for local eval."},
                    "expected": {"type": ["string", "null"], "description": "Optional expected file path or location prefix for local eval."}
                },
                "required": ["message"]
            }
        }),
        json!({
            "name": "feedback_list",
            "description": "List local SIFS feedback entries.",
            "inputSchema": {
                "type": "object",
                "properties": {"limit": {"type": "integer", "minimum": 1, "default": 20}}
            }
        }),
        json!({
            "name": "agent_print",
            "description": "Render a SIFS agent skill, instruction snippet, or MCP guidance artifact without writing files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {"type": "string", "enum": ["codex", "claude-code", "openclaw", "hermes", "generic"]},
                    "artifact": {"type": "string", "enum": ["skill", "snippet", "mcp"]},
                    "source": {"type": ["string", "null"]},
                    "profile": {"type": ["string", "null"]}
                },
                "required": ["target", "artifact"]
            }
        }),
        json!({
            "name": "agent_doctor",
            "description": "Inspect SIFS agent artifact readiness. This is read-only and reports unknown for current-session visibility when it cannot be proven.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {"type": "string", "enum": ["codex", "claude-code", "openclaw", "hermes", "generic", "all"], "default": "all"},
                    "artifact": {"type": "string", "enum": ["skill", "snippet", "mcp", "all"], "default": "all"}
                }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MessageFraming {
    ContentLength,
    LineDelimited,
}

#[derive(Debug)]
struct IncomingMessage {
    value: Value,
    framing: MessageFraming,
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<IncomingMessage>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let trimmed = trim_line_end(&line);
    if trimmed.is_empty() {
        return Ok(None);
    }

    if let Some(value) = trimmed.strip_prefix("Content-Length:") {
        let mut content_length = Some(
            value
                .trim()
                .parse::<usize>()
                .context("parse Content-Length")?,
        );
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                anyhow::bail!("unexpected EOF while reading MCP headers");
            }
            let trimmed = trim_line_end(&line);
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
        let length = content_length.context("missing Content-Length")?;
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body)?;
        return Ok(Some(IncomingMessage {
            value: serde_json::from_slice(&body)?,
            framing: MessageFraming::ContentLength,
        }));
    }

    Ok(Some(IncomingMessage {
        value: serde_json::from_str(trimmed)?,
        framing: MessageFraming::LineDelimited,
    }))
}

fn trim_line_end(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

fn write_message(writer: &mut impl Write, message: &Value, framing: MessageFraming) -> Result<()> {
    let body = serde_json::to_vec(message)?;
    match framing {
        MessageFraming::ContentLength => {
            write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
            writer.write_all(&body)?;
        }
        MessageFraming::LineDelimited => {
            writer.write_all(&body)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        IndexCache, MessageFraming, build_instructions, handle_resource_read, handle_tool_call,
        negotiated_protocol_version, parse_mcp_limit, parse_mcp_mode, read_message,
        search_options_from_args, selected_source, tool_schemas, write_message,
    };
    use crate::profiles::Profile;
    use crate::types::SearchMode;
    use serde_json::json;
    use std::io::{BufReader, Cursor};

    #[test]
    fn default_source_allows_source_override() {
        let args = json!({"source": "/other/repo"});

        assert_eq!(
            selected_source(&args, Some("/default/repo")).unwrap(),
            Some("/other/repo".to_owned())
        );
    }

    #[test]
    fn default_source_is_fallback_when_repo_is_omitted() {
        assert_eq!(
            selected_source(&json!({}), Some("/default/repo")).unwrap(),
            Some("/default/repo".to_owned())
        );
    }

    #[test]
    fn reads_content_length_framed_message() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let input = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = BufReader::new(Cursor::new(input));

        let message = read_message(&mut reader).unwrap().unwrap();

        assert_eq!(message.framing, MessageFraming::ContentLength);
        assert_eq!(message.value["method"], "initialize");
    }

    #[test]
    fn reads_line_delimited_message() {
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#.to_owned() + "\n";
        let mut reader = BufReader::new(Cursor::new(input));

        let message = read_message(&mut reader).unwrap().unwrap();

        assert_eq!(message.framing, MessageFraming::LineDelimited);
        assert_eq!(message.value["method"], "initialize");
    }

    #[test]
    fn writes_content_length_framed_message() {
        let mut output = Vec::new();

        write_message(
            &mut output,
            &json!({"jsonrpc": "2.0", "id": 1}),
            MessageFraming::ContentLength,
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("Content-Length: "));
        assert!(output.contains("\r\n\r\n"));
        assert!(output.ends_with(r#"{"id":1,"jsonrpc":"2.0"}"#));
    }

    #[test]
    fn writes_line_delimited_message() {
        let mut output = Vec::new();

        write_message(
            &mut output,
            &json!({"jsonrpc": "2.0", "id": 1}),
            MessageFraming::LineDelimited,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "{\"id\":1,\"jsonrpc\":\"2.0\"}\n"
        );
    }

    #[test]
    fn negotiates_supported_protocol_version() {
        let message = json!({
            "params": {"protocolVersion": "2024-11-05"}
        });

        assert_eq!(negotiated_protocol_version(&message), "2024-11-05");
    }

    #[test]
    fn unsupported_protocol_version_falls_back_to_default() {
        let message = json!({
            "params": {"protocolVersion": "2099-01-01"}
        });

        assert_eq!(negotiated_protocol_version(&message), "2024-11-05");
    }

    #[test]
    fn default_source_allows_matching_source() {
        assert_eq!(
            selected_source(&json!({"source": "/default/repo"}), Some("/default/repo")).unwrap(),
            Some("/default/repo".to_owned())
        );
    }

    #[test]
    fn default_source_allows_equivalent_local_path_spellings() {
        let temp = tempfile::tempdir().unwrap();
        let default = temp.path().to_string_lossy().to_string();
        let requested = temp.path().join(".").to_string_lossy().to_string();
        assert_eq!(
            selected_source(&json!({"source": requested}), Some(&default)).unwrap(),
            Some(requested)
        );
    }

    #[test]
    fn missing_default_uses_requested_source() {
        assert_eq!(
            selected_source(&json!({"source": "/requested/repo"}), None).unwrap(),
            Some("/requested/repo".to_owned())
        );
        assert_eq!(selected_source(&json!({}), None).unwrap(), None);
    }

    #[test]
    fn repo_argument_is_rejected_in_favor_of_source() {
        let error = selected_source(&json!({"repo": "/requested/repo"}), None).unwrap_err();
        assert!(error.contains("use source instead"));
    }

    #[test]
    fn mcp_search_options_fall_back_to_profile_defaults() {
        let profile = Profile {
            name: "agent".to_owned(),
            mode: Some(SearchMode::Bm25),
            limit: Some(20),
            ..Profile::default()
        };

        assert_eq!(
            parse_mcp_mode(&json!({}), Some(&profile)).unwrap(),
            SearchMode::Bm25
        );
        assert_eq!(
            parse_mcp_limit(&json!({}), "limit", profile.limit, 5).unwrap(),
            20
        );
        assert_eq!(
            parse_mcp_mode(&json!({"mode": "semantic"}), Some(&profile)).unwrap(),
            SearchMode::Semantic
        );
        assert_eq!(
            parse_mcp_limit(&json!({"limit": 3}), "limit", profile.limit, 5).unwrap(),
            3
        );
        assert!(parse_mcp_limit(&json!({"limit": 51}), "limit", None, 5).is_err());
        assert!(search_options_from_args(&json!({"alpha": 1.2}), 5, SearchMode::Hybrid).is_err());
        assert!(
            search_options_from_args(
                &json!({"filter_languages": ["rust", 3]}),
                5,
                SearchMode::Hybrid
            )
            .is_err()
        );
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
        assert!(names.contains(&"list_files"));
        assert!(names.contains(&"get_chunk"));
        assert!(names.contains(&"agent_context"));
        assert!(names.contains(&"profile_list"));
        assert!(names.contains(&"feedback_create"));
        assert!(names.contains(&"init_agent"));

        let search = schemas
            .iter()
            .find(|schema| schema["name"] == "search")
            .unwrap();
        let props = &search["inputSchema"]["properties"];
        assert!(props.get("alpha").is_some());
        assert!(props.get("source").is_some());
        assert!(props.get("limit").is_some());
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
