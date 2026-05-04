use crate::types::{Chunk, SearchResult};

const GIT_URL_SCHEMES: &[&str] = &[
    "https://",
    "http://",
    "ssh://",
    "git://",
    "git+ssh://",
    "file://",
];

pub fn is_git_url(path: &str) -> bool {
    if GIT_URL_SCHEMES
        .iter()
        .any(|scheme| path.starts_with(scheme))
    {
        return true;
    }
    let Some((left, right)) = path.split_once(':') else {
        return false;
    };
    !right.starts_with('/')
        && left.contains('@')
        && left
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-@_".contains(c))
}

pub fn resolve_chunk(chunks: &[Chunk], file_path: &str, line: usize) -> Option<Chunk> {
    let mut fallback = None;
    for chunk in chunks {
        if chunk.file_path == file_path && chunk.start_line <= line && line <= chunk.end_line {
            if line < chunk.end_line {
                return Some(chunk.clone());
            }
            if fallback.is_none() {
                fallback = Some(chunk.clone());
            }
        }
    }
    fallback
}

pub fn format_results(header: &str, results: &[SearchResult]) -> String {
    let mut lines = vec![header.to_owned(), String::new()];
    for (i, result) in results.iter().enumerate() {
        lines.push(format!(
            "## {}. {}  [score={:.3}]",
            i + 1,
            result.chunk.location(),
            result.score
        ));
        lines.push("```".to_owned());
        lines.push(result.chunk.content.trim().to_owned());
        lines.push("```".to_owned());
        lines.push(String::new());
    }
    lines.join("\n")
}
