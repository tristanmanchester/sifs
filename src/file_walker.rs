use ignore::gitignore::{Gitignore, GitignoreBuilder};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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
    let gitignore = RootGitignore::load(root);
    let files: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            if DEFAULT_IGNORED_DIRS.contains(name.as_ref()) || ignore_owned.contains(name.as_ref()) {
                return false;
            }
            entry.path() == root || !gitignore.is_match(entry.path(), entry.file_type().is_dir())
        })
        .filter_map(Result::ok)
        .filter(|entry| {
            if entry.path() == root {
                return true;
            }
            !gitignore.is_match(entry.path(), entry.file_type().is_dir())
        })
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
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

struct RootGitignore {
    matcher: Gitignore,
}

impl RootGitignore {
    fn load(root: &Path) -> Self {
        let mut builder = GitignoreBuilder::new(root);
        let gitignore = root.join(".gitignore");
        if gitignore.is_file() {
            let _ = builder.add(gitignore);
        }
        Self {
            matcher: builder.build().unwrap_or_else(|_| Gitignore::empty()),
        }
    }

    fn is_match(&self, rel: &Path, is_dir: bool) -> bool {
        self.matcher.matched_path_or_any_parents(rel, is_dir).is_ignore()
    }
}
