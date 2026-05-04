use crate::SifsIndex;
use crate::types::SearchMode;
use crate::utils::{format_results, is_git_url, resolve_chunk};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

pub fn serve(default_source: Option<String>, ref_name: Option<String>) -> Result<()> {
    let mut cache = IndexCache::default();
    if let Some(source) = &default_source {
        let _ = cache.get(source, ref_name.as_deref());
    }
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
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "sifs", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "SIFS Is Fast Search: instant code search for any local or GitHub repository. Call search to find relevant code; call find_related on a result to discover similar code elsewhere."
            }),
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

#[derive(Default)]
struct IndexCache {
    indexes: HashMap<String, SifsIndex>,
}

impl IndexCache {
    fn get(&mut self, source: &str, ref_name: Option<&str>) -> Result<&SifsIndex> {
        let key = if is_git_url(source) {
            ref_name
                .map(|r| format!("{source}@{r}"))
                .unwrap_or_else(|| source.to_owned())
        } else {
            Path::new(source)
                .canonicalize()?
                .to_string_lossy()
                .to_string()
        };
        if !self.indexes.contains_key(&key) {
            let index = if is_git_url(source) {
                SifsIndex::from_git(source, ref_name)?
            } else {
                SifsIndex::from_path(&key)?
            };
            self.indexes.insert(key.clone(), index);
        }
        Ok(self.indexes.get(&key).unwrap())
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
    let text = match name {
        "search" => tool_search(args, cache, default_source, ref_name),
        "find_related" => tool_find_related(args, cache, default_source, ref_name),
        _ => format!("Unknown tool: {name}"),
    };
    json!({"content": [{"type": "text", "text": text}]})
}

fn tool_search(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> String {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source = args.get("repo").and_then(Value::as_str).or(default_source);
    let Some(source) = source else {
        return "No repo specified and no default index. Pass a git URL (https://github.com/...) or local path as `repo`.".to_owned();
    };
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<SearchMode>().ok())
        .unwrap_or(SearchMode::Hybrid);
    let top_k = args.get("top_k").and_then(Value::as_u64).unwrap_or(5) as usize;
    match cache.get(source, ref_name) {
        Ok(index) => {
            let results = index.search(query, top_k, mode, None, None, None);
            if results.is_empty() {
                "No results found.".to_owned()
            } else {
                format_results(
                    &format!("Search results for: {query:?} (mode={mode})"),
                    &results,
                )
            }
        }
        Err(err) => format!("Failed to index {source:?}: {err}"),
    }
}

fn tool_find_related(
    args: Value,
    cache: &mut IndexCache,
    default_source: Option<&str>,
    ref_name: Option<&str>,
) -> String {
    let file_path = args
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let line = args.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let source = args.get("repo").and_then(Value::as_str).or(default_source);
    let Some(source) = source else {
        return "No repo specified and no default index. Pass a git URL (https://github.com/...) or local path as `repo`.".to_owned();
    };
    let top_k = args.get("top_k").and_then(Value::as_u64).unwrap_or(5) as usize;
    match cache.get(source, ref_name) {
        Ok(index) => {
            let Some(chunk) = resolve_chunk(&index.chunks, file_path, line) else {
                return format!(
                    "No chunk found at {file_path}:{line}. Make sure the file is indexed and the line number is within a known chunk."
                );
            };
            let results = index.find_related(&chunk, top_k);
            if results.is_empty() {
                format!("No related chunks found for {file_path}:{line}.")
            } else {
                format_results(&format!("Chunks related to {file_path}:{line}"), &results)
            }
        }
        Err(err) => format!("Failed to index {source:?}: {err}"),
    }
}

fn tool_schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "search",
            "description": "Search a codebase with a natural-language or code query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Natural language or code query."},
                    "repo": {"type": ["string", "null"], "description": "Git URL or local path to index and search."},
                    "mode": {"type": "string", "enum": ["hybrid", "semantic", "bm25"], "default": "hybrid"},
                    "top_k": {"type": "integer", "minimum": 1, "default": 5}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "find_related",
            "description": "Find code chunks semantically similar to a specific file and line.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "line": {"type": "integer", "minimum": 1},
                    "repo": {"type": ["string", "null"]},
                    "top_k": {"type": "integer", "minimum": 1, "default": 5}
                },
                "required": ["file_path", "line"]
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
