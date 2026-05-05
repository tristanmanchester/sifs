use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackEntry {
    pub id: String,
    pub created_at_unix_ms: u128,
    pub version: String,
    pub message: String,
    pub command_context: Option<String>,
}

pub fn feedback_log_path(cache_root: &Path) -> PathBuf {
    cache_root.join("feedback.jsonl")
}

pub fn create_feedback(
    cache_root: &Path,
    message: &str,
    command_context: Option<String>,
) -> Result<FeedbackEntry> {
    if message.trim().is_empty() {
        bail!("feedback message must not be empty");
    }
    fs::create_dir_all(cache_root)
        .with_context(|| format!("create cache root {}", cache_root.display()))?;
    let created_at_unix_ms = unix_ms();
    let entry = FeedbackEntry {
        id: format!("fbk-{created_at_unix_ms}"),
        created_at_unix_ms,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        message: message.trim().to_owned(),
        command_context,
    };
    let path = feedback_log_path(cache_root);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open feedback log {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)
        .with_context(|| format!("write feedback log {}", path.display()))?;
    Ok(entry)
}

pub fn list_feedback(cache_root: &Path, limit: usize) -> Result<(Vec<FeedbackEntry>, usize)> {
    let path = feedback_log_path(cache_root);
    if !path.exists() {
        return Ok((Vec::new(), 0));
    }
    let file =
        fs::File::open(&path).with_context(|| format!("open feedback log {}", path.display()))?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        entries.push(serde_json::from_str::<FeedbackEntry>(&line)?);
    }
    let total = entries.len();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at_unix_ms));
    entries.truncate(limit);
    Ok((entries, total))
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
