use ignore::WalkBuilder;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const DEFAULT_MAX_FILE_BYTES: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileCategory {
    Code,
    Document,
}

#[derive(Clone, Copy, Debug)]
pub struct FileType {
    pub language: &'static str,
    pub category: FileCategory,
}

pub static FILE_TYPES: Lazy<HashMap<&'static str, FileType>> = Lazy::new(|| {
    HashMap::from([
        (
            ".py",
            FileType {
                language: "python",
                category: FileCategory::Code,
            },
        ),
        (
            ".js",
            FileType {
                language: "javascript",
                category: FileCategory::Code,
            },
        ),
        (
            ".jsx",
            FileType {
                language: "javascript",
                category: FileCategory::Code,
            },
        ),
        (
            ".ts",
            FileType {
                language: "typescript",
                category: FileCategory::Code,
            },
        ),
        (
            ".tsx",
            FileType {
                language: "typescript",
                category: FileCategory::Code,
            },
        ),
        (
            ".go",
            FileType {
                language: "go",
                category: FileCategory::Code,
            },
        ),
        (
            ".rs",
            FileType {
                language: "rust",
                category: FileCategory::Code,
            },
        ),
        (
            ".java",
            FileType {
                language: "java",
                category: FileCategory::Code,
            },
        ),
        (
            ".kt",
            FileType {
                language: "kotlin",
                category: FileCategory::Code,
            },
        ),
        (
            ".kts",
            FileType {
                language: "kotlin",
                category: FileCategory::Code,
            },
        ),
        (
            ".rb",
            FileType {
                language: "ruby",
                category: FileCategory::Code,
            },
        ),
        (
            ".php",
            FileType {
                language: "php",
                category: FileCategory::Code,
            },
        ),
        (
            ".c",
            FileType {
                language: "c",
                category: FileCategory::Code,
            },
        ),
        (
            ".h",
            FileType {
                language: "c",
                category: FileCategory::Code,
            },
        ),
        (
            ".cpp",
            FileType {
                language: "cpp",
                category: FileCategory::Code,
            },
        ),
        (
            ".hpp",
            FileType {
                language: "cpp",
                category: FileCategory::Code,
            },
        ),
        (
            ".cs",
            FileType {
                language: "csharp",
                category: FileCategory::Code,
            },
        ),
        (
            ".swift",
            FileType {
                language: "swift",
                category: FileCategory::Code,
            },
        ),
        (
            ".scala",
            FileType {
                language: "scala",
                category: FileCategory::Code,
            },
        ),
        (
            ".sbt",
            FileType {
                language: "scala",
                category: FileCategory::Code,
            },
        ),
        (
            ".ex",
            FileType {
                language: "elixir",
                category: FileCategory::Code,
            },
        ),
        (
            ".exs",
            FileType {
                language: "elixir",
                category: FileCategory::Code,
            },
        ),
        (
            ".dart",
            FileType {
                language: "dart",
                category: FileCategory::Code,
            },
        ),
        (
            ".lua",
            FileType {
                language: "lua",
                category: FileCategory::Code,
            },
        ),
        (
            ".sql",
            FileType {
                language: "sql",
                category: FileCategory::Code,
            },
        ),
        (
            ".sh",
            FileType {
                language: "bash",
                category: FileCategory::Code,
            },
        ),
        (
            ".bash",
            FileType {
                language: "bash",
                category: FileCategory::Code,
            },
        ),
        (
            ".zig",
            FileType {
                language: "zig",
                category: FileCategory::Code,
            },
        ),
        (
            ".hs",
            FileType {
                language: "haskell",
                category: FileCategory::Code,
            },
        ),
        (
            ".md",
            FileType {
                language: "markdown",
                category: FileCategory::Document,
            },
        ),
        (
            ".yaml",
            FileType {
                language: "yaml",
                category: FileCategory::Document,
            },
        ),
        (
            ".yml",
            FileType {
                language: "yaml",
                category: FileCategory::Document,
            },
        ),
        (
            ".toml",
            FileType {
                language: "toml",
                category: FileCategory::Document,
            },
        ),
        (
            ".json",
            FileType {
                language: "json",
                category: FileCategory::Document,
            },
        ),
    ])
});

static DEFAULT_IGNORED_DIRS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    HashSet::from([
        ".git",
        ".hg",
        ".svn",
        "__pycache__",
        "node_modules",
        ".venv",
        "venv",
        ".tox",
        ".mypy_cache",
        ".pytest_cache",
        ".ruff_cache",
        ".cache",
        ".sifs",
        "dist",
        "build",
        ".eggs",
    ])
});

static DEFAULT_IGNORED_FILES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    HashSet::from([
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lockb",
        "Cargo.lock",
        "composer.lock",
        "poetry.lock",
        "Pipfile.lock",
        "Gemfile.lock",
        "coverage.json",
    ])
});

pub fn language_for_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    FILE_TYPES
        .get(format!(".{ext}").as_str())
        .map(|ft| ft.language)
}

pub fn filter_extensions(
    extensions: Option<HashSet<String>>,
    include_text_files: bool,
) -> HashSet<String> {
    if let Some(extensions) = extensions {
        return extensions;
    }
    FILE_TYPES
        .iter()
        .filter(|(_, ft)| ft.category == FileCategory::Code || include_text_files)
        .map(|(ext, _)| (*ext).to_owned())
        .collect()
}

pub fn walk_files(
    root: &Path,
    extensions: &HashSet<String>,
    ignore: Option<&HashSet<String>>,
) -> Vec<PathBuf> {
    let ignore_owned = ignore.cloned().unwrap_or_default();
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .parents(true)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .filter_entry(move |entry| {
            let name = entry.file_name().to_string_lossy();
            !DEFAULT_IGNORED_DIRS.contains(name.as_ref()) && !ignore_owned.contains(name.as_ref())
        });
    let files: Vec<PathBuf> = builder
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|ft| ft.is_file()))
        .map(|entry| entry.into_path())
        .filter(|path| {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| DEFAULT_IGNORED_FILES.contains(name))
            {
                return false;
            }
            if path
                .metadata()
                .map(|metadata| metadata.len() > DEFAULT_MAX_FILE_BYTES)
                .unwrap_or(false)
            {
                return false;
            }
            path.extension()
                .map(|ext| {
                    extensions
                        .contains(format!(".{}", ext.to_string_lossy().to_lowercase()).as_str())
                })
                .unwrap_or(false)
        })
        .collect();
    let mut files = files;
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::{filter_extensions, walk_files};
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn filter_extensions_can_include_document_files() {
        let code_only = filter_extensions(None, false);
        let with_docs = filter_extensions(None, true);

        assert!(code_only.contains(".rs"));
        assert!(!code_only.contains(".md"));
        assert!(with_docs.contains(".md"));
        assert!(with_docs.contains(".json"));
    }

    #[test]
    fn walk_files_applies_explicit_ignored_directory_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join("generated")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.path().join("generated/lib.rs"), "fn generated() {}\n").unwrap();
        let extensions = HashSet::from([".rs".to_owned()]);
        let ignored = HashSet::from(["generated".to_owned()]);

        let files = walk_files(dir.path(), &extensions, Some(&ignored));

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("src/lib.rs"));
    }

    #[test]
    fn walk_files_respects_nested_gitignore_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/generated")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.path().join("src/.gitignore"), "generated/\n").unwrap();
        fs::write(
            dir.path().join("src/generated/lib.rs"),
            "fn generated() {}\n",
        )
        .unwrap();
        let extensions = HashSet::from([".rs".to_owned()]);

        let files = walk_files(dir.path(), &extensions, None);

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("src/lib.rs"));
    }

    #[test]
    fn walk_files_respects_git_info_exclude() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git/info")).unwrap();
        fs::write(dir.path().join(".git/info/exclude"), "excluded.rs\n").unwrap();
        fs::write(dir.path().join("included.rs"), "fn included() {}\n").unwrap();
        fs::write(dir.path().join("excluded.rs"), "fn excluded() {}\n").unwrap();
        let extensions = HashSet::from([".rs".to_owned()]);

        let files = walk_files(dir.path(), &extensions, None);

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("included.rs"));
    }

    #[test]
    fn walk_files_ignores_hidden_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".hidden")).unwrap();
        fs::write(dir.path().join("visible.rs"), "fn visible() {}\n").unwrap();
        fs::write(dir.path().join(".hidden.rs"), "fn hidden_file() {}\n").unwrap();
        fs::write(dir.path().join(".hidden/lib.rs"), "fn hidden_dir() {}\n").unwrap();
        let extensions = HashSet::from([".rs".to_owned()]);

        let files = walk_files(dir.path(), &extensions, None);

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("visible.rs"));
    }

    #[test]
    fn walk_files_skips_generated_lockfiles_when_docs_are_enabled() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "# docs\n").unwrap();
        fs::write(dir.path().join("package-lock.json"), "{}\n").unwrap();
        fs::write(dir.path().join("config.json"), "{}\n").unwrap();
        let extensions = filter_extensions(None, true);

        let files = walk_files(dir.path(), &extensions, None);

        assert!(files.iter().any(|path| path.ends_with("README.md")));
        assert!(files.iter().any(|path| path.ends_with("config.json")));
        assert!(!files.iter().any(|path| path.ends_with("package-lock.json")));
    }

    #[test]
    fn walk_files_skips_large_files_by_default() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("small.json"), "{}\n").unwrap();
        fs::write(dir.path().join("large.json"), vec![b'a'; 1_000_001]).unwrap();
        let extensions = filter_extensions(None, true);

        let files = walk_files(dir.path(), &extensions, None);

        assert!(files.iter().any(|path| path.ends_with("small.json")));
        assert!(!files.iter().any(|path| path.ends_with("large.json")));
    }
}
