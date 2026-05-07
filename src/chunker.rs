use crate::types::{Chunk, Symbol};
use std::ops::Range;

pub fn chunk_source(source: &str, file_path: &str, language: Option<String>) -> Vec<Chunk> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    chunk_code_aware(source, file_path, language.clone())
        .unwrap_or_else(|| chunk_lines(source, file_path, language, 50, 5))
}

pub fn chunk_lines(
    source: &str,
    file_path: &str,
    language: Option<String>,
    max_lines: usize,
    overlap_lines: usize,
) -> Vec<Chunk> {
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    if lines.is_empty() || max_lines == 0 {
        return Vec::new();
    }
    let overlap_lines = overlap_lines.min(max_lines.saturating_sub(1));
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + max_lines).min(lines.len());
        let content = lines[start..end].concat();
        if !content.trim().is_empty() {
            chunks.push(Chunk {
                content,
                file_path: file_path.to_owned(),
                start_line: start + 1,
                end_line: end,
                language: language.clone(),
                symbols: Vec::new(),
                breadcrumbs: Vec::new(),
            });
        }
        start = if end < lines.len() {
            end.saturating_sub(overlap_lines)
        } else {
            end
        };
    }
    chunks
}

fn chunk_code_aware(source: &str, file_path: &str, language: Option<String>) -> Option<Vec<Chunk>> {
    let language = language?;
    let mut parser = tree_sitter_language_pack::get_parser(&language).ok()?;
    let tree = parser.parse(source.as_bytes(), None)?;
    let root = tree.root_node();
    let (node_groups, _) = group_child_nodes(source, root, 1500);
    let ranges = text_ranges_from_node_groups(source, &node_groups);
    let chunks: Vec<Chunk> = ranges
        .into_iter()
        .filter_map(|range| {
            let content = source.get(range.clone())?.to_owned();
            if content.trim().is_empty() {
                return None;
            }
            let end_index = range.end.saturating_sub(1).max(range.start);
            let start_line = line_number_at_byte(source, range.start);
            let end_line = line_number_at_byte(source, end_index);
            let symbols = extract_symbols(&content, start_line);
            let breadcrumbs = symbols
                .iter()
                .map(|symbol| format!("{} {}", symbol.kind, symbol.name))
                .collect();
            Some(Chunk {
                content,
                file_path: file_path.to_owned(),
                start_line,
                end_line,
                language: Some(language.clone()),
                symbols,
                breadcrumbs,
            })
        })
        .collect();
    (!chunks.is_empty()).then_some(chunks)
}

fn extract_symbols(content: &str, start_line: usize) -> Vec<Symbol> {
    content
        .lines()
        .enumerate()
        .filter_map(|(offset, line)| extract_symbol(line, start_line + offset))
        .collect()
}

fn extract_symbol(line: &str, line_number: usize) -> Option<Symbol> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("#define ") {
        return Some(Symbol {
            name: symbol_name(rest)?,
            kind: "macro".to_owned(),
            line: line_number,
        });
    }
    let trimmed = trimmed
        .strip_prefix("export default ")
        .or_else(|| trimmed.strip_prefix("export "))
        .unwrap_or(trimmed);
    let trimmed = strip_declaration_modifiers(trimmed);
    let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("typedef struct ") {
        ("struct", rest)
    } else if let Some(rest) = trimmed.strip_prefix("typedef enum ") {
        ("enum", rest)
    } else if let Some(rest) = c_like_function_name(trimmed) {
        ("function", rest)
    } else if let Some(rest) = trimmed.strip_prefix("pub async fn ") {
        ("fn", rest)
    } else if let Some(rest) = trimmed.strip_prefix("pub fn ") {
        ("fn", rest)
    } else if let Some(rest) = trimmed.strip_prefix("async fn ") {
        ("fn", rest)
    } else if let Some(rest) = trimmed.strip_prefix("fn ") {
        ("fn", rest)
    } else if let Some(rest) = trimmed.strip_prefix("async function ") {
        ("function", rest)
    } else if let Some(rest) = trimmed.strip_prefix("function ") {
        ("function", rest)
    } else if let Some(rest) = trimmed.strip_prefix("async def ") {
        ("def", rest)
    } else if let Some(rest) = trimmed.strip_prefix("def ") {
        ("def", rest)
    } else if let Some(rest) = trimmed.strip_prefix("class ") {
        ("class", rest)
    } else if let Some(rest) = trimmed.strip_prefix("struct ") {
        ("struct", rest)
    } else if let Some(rest) = trimmed.strip_prefix("pub struct ") {
        ("struct", rest)
    } else if let Some(rest) = trimmed.strip_prefix("enum ") {
        ("enum", rest)
    } else if let Some(rest) = trimmed.strip_prefix("pub enum ") {
        ("enum", rest)
    } else if let Some(rest) = trimmed.strip_prefix("trait ") {
        ("trait", rest)
    } else if let Some(rest) = trimmed.strip_prefix("pub trait ") {
        ("trait", rest)
    } else if let Some(rest) = trimmed.strip_prefix("interface ") {
        ("interface", rest)
    } else if let Some(rest) = trimmed.strip_prefix("impl ") {
        ("impl", rest)
    } else if let Some(rest) = trimmed.strip_prefix("type ") {
        ("type", rest)
    } else if let Some(rest) = trimmed.strip_prefix("const ") {
        ("const", rest)
    } else if let Some(rest) = trimmed.strip_prefix("let ") {
        ("let", rest)
    } else {
        return None;
    };
    let name = symbol_name(rest)?;
    Some(Symbol {
        name,
        kind: kind.to_owned(),
        line: line_number,
    })
}

fn strip_declaration_modifiers(mut trimmed: &str) -> &str {
    loop {
        let next = trimmed
            .strip_prefix("public ")
            .or_else(|| trimmed.strip_prefix("private "))
            .or_else(|| trimmed.strip_prefix("protected "))
            .or_else(|| trimmed.strip_prefix("internal "))
            .or_else(|| trimmed.strip_prefix("static "))
            .or_else(|| trimmed.strip_prefix("final "))
            .or_else(|| trimmed.strip_prefix("abstract "))
            .or_else(|| trimmed.strip_prefix("open "))
            .or_else(|| trimmed.strip_prefix("override "))
            .or_else(|| trimmed.strip_prefix("inline "))
            .or_else(|| trimmed.strip_prefix("extern "))
            .or_else(|| trimmed.strip_prefix("virtual "))
            .or_else(|| trimmed.strip_prefix("async "));
        let Some(next) = next else {
            return trimmed;
        };
        trimmed = next;
    }
}

fn c_like_function_name(trimmed: &str) -> Option<&str> {
    const CONTROL_KEYWORDS: &[&str] = &["if", "for", "while", "switch", "catch", "return"];
    let before_paren = trimmed.split_once('(')?.0.trim_end();
    let mut parts = before_paren.split_whitespace();
    let _return_or_keyword = parts.next()?;
    let name = parts
        .last()
        .filter(|name| {
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        })
        .filter(|name| !CONTROL_KEYWORDS.contains(name))?;
    trimmed.contains(')').then_some(name)
}

fn symbol_name(rest: &str) -> Option<String> {
    Some(
        rest.trim_start()
            .trim_start_matches("async ")
            .trim_matches('{')
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
            .next()
            .filter(|name| !name.is_empty())?
            .to_owned(),
    )
}

fn group_child_nodes(
    source: &str,
    node: tree_sitter::Node<'_>,
    chunk_size: usize,
) -> (Vec<Vec<Range<usize>>>, Vec<usize>) {
    let child_count = node.child_count();
    if child_count == 0 {
        let range = node.byte_range();
        let count = token_count(source, &range);
        if count > chunk_size {
            let ranges = split_range_by_chars(source, range, chunk_size);
            let counts = ranges
                .iter()
                .map(|range| token_count(source, range))
                .collect();
            return (
                ranges.into_iter().map(|range| vec![range]).collect(),
                counts,
            );
        }
        return (vec![vec![range.clone()]], vec![count]);
    }

    let mut node_groups: Vec<Vec<Range<usize>>> = Vec::new();
    let mut group_token_counts = Vec::new();
    let mut current_group = Vec::new();
    let mut current_count = 0usize;

    for idx in 0..child_count {
        let Some(child) = node.child(idx as u32) else {
            continue;
        };
        let range = child.byte_range();
        let count = token_count(source, &range);
        if count > chunk_size {
            if !current_group.is_empty() {
                node_groups.push(std::mem::take(&mut current_group));
                group_token_counts.push(current_count);
                current_count = 0;
            }
            let (child_groups, child_counts) = group_child_nodes(source, child, chunk_size);
            node_groups.extend(child_groups);
            group_token_counts.extend(child_counts);
        } else if current_count + count > chunk_size {
            node_groups.push(std::mem::take(&mut current_group));
            group_token_counts.push(current_count);
            current_group.push(range);
            current_count = count;
        } else {
            current_group.push(range);
            current_count += count;
        }
    }
    if !current_group.is_empty() {
        node_groups.push(current_group);
        group_token_counts.push(current_count);
    }

    merge_node_groups(node_groups, group_token_counts, chunk_size)
}

fn merge_node_groups(
    node_groups: Vec<Vec<Range<usize>>>,
    group_token_counts: Vec<usize>,
    chunk_size: usize,
) -> (Vec<Vec<Range<usize>>>, Vec<usize>) {
    let mut cumulative = Vec::with_capacity(group_token_counts.len() + 1);
    cumulative.push(0usize);
    for count in &group_token_counts {
        cumulative.push(cumulative.last().copied().unwrap_or(0) + count);
    }

    let mut merged_groups = Vec::new();
    let mut merged_counts = Vec::new();
    let mut pos = 0usize;
    while pos < node_groups.len() {
        let target = cumulative[pos] + chunk_size;
        let mut index = cumulative[pos..]
            .iter()
            .position(|count| *count >= target)
            .map(|offset| pos + offset)
            .unwrap_or(cumulative.len() - 1)
            .saturating_sub(1);
        if index == pos {
            index = pos + 1;
        }
        index = index.min(node_groups.len());
        if index <= pos {
            index = pos + 1;
        }
        let mut merged = Vec::new();
        for group in &node_groups[pos..index] {
            merged.extend(group.iter().cloned());
        }
        merged_groups.push(merged);
        merged_counts.push(cumulative[index] - cumulative[pos]);
        pos = index;
    }
    (merged_groups, merged_counts)
}

fn text_ranges_from_node_groups(
    source: &str,
    node_groups: &[Vec<Range<usize>>],
) -> Vec<Range<usize>> {
    if source.is_empty() || node_groups.is_empty() {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    for (idx, group) in node_groups.iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        let start = group.first().unwrap().start;
        let mut end = group.last().unwrap().end;
        if idx < node_groups.len() - 1
            && let Some(next_start) = node_groups[idx + 1].first().map(|range| range.start)
        {
            end = next_start;
        }
        ranges.push(start..end);
    }
    if let Some(first) = ranges.first_mut() {
        first.start = 0;
    }
    if let Some(last) = ranges.last_mut() {
        last.end = source.len();
    }
    ranges
}

fn token_count(source: &str, range: &Range<usize>) -> usize {
    source
        .get(range.clone())
        .map(|text| text.chars().count())
        .unwrap_or(0)
}

fn split_range_by_chars(source: &str, range: Range<usize>, chunk_size: usize) -> Vec<Range<usize>> {
    if chunk_size == 0 {
        return Vec::new();
    }
    let Some(text) = source.get(range.clone()) else {
        return Vec::new();
    };
    let mut ranges = Vec::new();
    let mut chunk_start = range.start;
    let mut count = 0usize;
    for (offset, _) in text.char_indices() {
        if count == chunk_size {
            let chunk_end = range.start + offset;
            ranges.push(chunk_start..chunk_end);
            chunk_start = chunk_end;
            count = 0;
        }
        count += 1;
    }
    if chunk_start < range.end {
        ranges.push(chunk_start..range.end);
    }
    ranges
}

fn line_number_at_byte(source: &str, byte_index: usize) -> usize {
    let index = previous_char_boundary(source, byte_index.min(source.len()));
    source[..index].matches('\n').count() + 1
}

fn previous_char_boundary(source: &str, mut byte_index: usize) -> usize {
    while byte_index > 0 && !source.is_char_boundary(byte_index) {
        byte_index -= 1;
    }
    byte_index
}

#[cfg(test)]
mod tests {
    use super::{chunk_lines, chunk_source};

    #[test]
    fn chunk_lines_uses_overlap_and_locations() {
        let source = (1..=60).map(|i| format!("line {i}\n")).collect::<String>();
        let chunks = chunk_lines(&source, "src/foo.py", Some("python".to_owned()), 50, 5);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 50);
        assert_eq!(chunks[1].start_line, 46);
    }

    #[test]
    fn chunk_lines_clamps_overlap_to_keep_progress() {
        let source = (1..=5).map(|i| format!("line {i}\n")).collect::<String>();
        let chunks = chunk_lines(&source, "src/foo.py", None, 2, 2);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks.last().unwrap().start_line, 4);
        assert!(chunk_lines(&source, "src/foo.py", None, 0, 10).is_empty());
    }

    #[test]
    fn code_chunker_uses_tree_sitter_boundaries() {
        let source = "def alpha():\n    return 1\n\n".repeat(80);
        let chunks = chunk_source(&source, "many.py", Some("python".to_owned()));
        assert!(chunks.len() > 1);
        assert_eq!(chunks.first().unwrap().start_line, 1);
        assert!(chunks.windows(2).all(|w| w[0].end_line <= w[1].start_line));
        assert!(chunks.iter().all(|chunk| chunk.content.len() <= 1800));
    }

    #[test]
    fn code_chunker_splits_long_leaf_nodes() {
        let source = format!("VALUE = \"{}\"\n", "x".repeat(2500));
        let chunks = chunk_source(&source, "long.py", Some("python".to_owned()));

        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| chunk.content.len() <= 1800));
        assert_eq!(chunks.first().unwrap().start_line, 1);
        assert_eq!(chunks.last().unwrap().end_line, 1);
    }

    #[test]
    fn code_chunker_handles_multibyte_boundaries_in_line_numbers() {
        let source = format!(
            "{}\nconst marker = \"{}\";\n{}",
            "export const alpha = 1;\n".repeat(100),
            '\u{e007f}',
            "export const beta = 2;\n".repeat(100)
        );
        let chunks = chunk_source(&source, "many.ts", Some("typescript".to_owned()));

        assert!(!chunks.is_empty());
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.start_line <= chunk.end_line)
        );
        assert!(
            chunks
                .iter()
                .any(|chunk| chunk.content.contains('\u{e007f}'))
        );
    }

    #[test]
    fn code_chunker_extracts_symbols_and_breadcrumbs() {
        let source = "class SessionStore:\n    pass\n\ndef load_session():\n    return None\n";
        let chunks = chunk_source(source, "session.py", Some("python".to_owned()));
        let symbols = chunks
            .iter()
            .flat_map(|chunk| chunk.symbols.iter().map(|symbol| symbol.name.as_str()))
            .collect::<Vec<_>>();

        assert!(symbols.contains(&"SessionStore"));
        assert!(symbols.contains(&"load_session"));
        assert!(
            chunks
                .iter()
                .any(|chunk| chunk.breadcrumbs.iter().any(|b| b == "class SessionStore"))
        );
    }

    #[test]
    fn code_chunker_extracts_common_export_async_and_impl_symbols() {
        let source = [
            "export function fetchUser() { return null; }",
            "export class UserCard {}",
            "const useThing = () => null;",
            "pub async fn load_token() {}",
            "impl SessionStore {}",
            "async def fetch_user():",
            "    return None",
            "manager.validate();",
            "public class AccountController {}",
            "private static final class InnerThing {}",
            "#define CURL_MAX_WRITE_SIZE 16384",
            "static inline int Curl_retry_request(struct Curl_easy *data) { return 0; }",
            "typedef struct json_object json_object;",
        ]
        .join("\n");
        let chunks = chunk_source(&source, "symbols.ts", Some("typescript".to_owned()));
        let symbols = chunks
            .iter()
            .flat_map(|chunk| chunk.symbols.iter().map(|symbol| symbol.name.as_str()))
            .collect::<Vec<_>>();

        assert!(symbols.contains(&"fetchUser"));
        assert!(symbols.contains(&"UserCard"));
        assert!(symbols.contains(&"useThing"));
        assert!(symbols.contains(&"load_token"));
        assert!(symbols.contains(&"SessionStore"));
        assert!(symbols.contains(&"fetch_user"));
        assert!(symbols.contains(&"AccountController"));
        assert!(symbols.contains(&"InnerThing"));
        assert!(symbols.contains(&"CURL_MAX_WRITE_SIZE"));
        assert!(symbols.contains(&"Curl_retry_request"));
        assert!(symbols.contains(&"json_object"));
        assert!(!symbols.contains(&"manager"));
    }
}
