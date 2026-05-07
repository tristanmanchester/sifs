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
            "## {}. {}  [score={:.3}, source={}]",
            i + 1,
            result.chunk.location(),
            result.score,
            result.source
        ));
        lines.push(fenced_code_block(None, result.chunk.content.trim()));
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn fenced_code_block(language: Option<&str>, content: &str) -> String {
    let fence = markdown_code_fence(content);
    let language = language.unwrap_or_default();
    format!("{fence}{language}\n{content}\n{fence}")
}

fn markdown_code_fence(content: &str) -> String {
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in content.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

#[cfg(test)]
mod tests {
    use super::{fenced_code_block, format_results, is_git_url, resolve_chunk};
    use crate::types::{Chunk, SearchMode, SearchResult};

    fn chunk(file_path: &str, start_line: usize, end_line: usize) -> Chunk {
        Chunk {
            content: "fn example() {}".to_owned(),
            file_path: file_path.to_owned(),
            start_line,
            end_line,
            language: Some("rust".to_owned()),
            symbols: Vec::new(),
            breadcrumbs: Vec::new(),
        }
    }

    #[test]
    fn resolves_line_at_chunk_boundaries() {
        let chunks = vec![chunk("src/lib.rs", 1, 10), chunk("src/lib.rs", 10, 20)];

        assert_eq!(
            resolve_chunk(&chunks, "src/lib.rs", 10).unwrap().start_line,
            10
        );
        assert!(resolve_chunk(&chunks, "src/lib.rs", 21).is_none());
    }

    #[test]
    fn git_url_detection_covers_scheme_and_scp_forms() {
        assert!(is_git_url("https://github.com/org/repo"));
        assert!(is_git_url("git@github.com:org/repo.git"));
        assert!(!is_git_url("/tmp/local:repo"));
    }

    #[test]
    fn formatted_results_show_source_mode() {
        let result = SearchResult {
            chunk: chunk("src/lib.rs", 1, 1),
            score: 0.5,
            source: SearchMode::Bm25,
            explanation: None,
        };

        let output = format_results("Header", &[result]);

        assert!(output.contains("[score=0.500, source=bm25]"));
    }

    #[test]
    fn fenced_code_block_uses_standard_fence_for_plain_content() {
        let block = fenced_code_block(Some("rust"), "fn main() {}");

        assert_eq!(block, "```rust\nfn main() {}\n```");
    }

    #[test]
    fn fenced_code_block_uses_longer_fence_than_content_backticks() {
        let block = fenced_code_block(
            Some("markdown"),
            "before\n```rust\nfn main() {}\n```\nafter",
        );

        assert!(block.starts_with("````markdown\n"));
        assert!(block.ends_with("\n````"));
    }
}
