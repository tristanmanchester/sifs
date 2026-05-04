use crate::types::Chunk;
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
            Some(Chunk {
                content,
                file_path: file_path.to_owned(),
                start_line: line_number_at_byte(source, range.start),
                end_line: line_number_at_byte(source, end_index),
                language: Some(language.clone()),
            })
        })
        .collect();
    (!chunks.is_empty()).then_some(chunks)
}

fn group_child_nodes(
    source: &str,
    node: tree_sitter::Node<'_>,
    chunk_size: usize,
) -> (Vec<Vec<Range<usize>>>, Vec<usize>) {
    let child_count = node.child_count();
    if child_count == 0 {
        let range = node.byte_range();
        return (vec![vec![range.clone()]], vec![token_count(source, &range)]);
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
}
