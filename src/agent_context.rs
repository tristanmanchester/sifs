use serde_json::{Value, json};

pub const AGENT_CONTEXT_SCHEMA_VERSION: &str = "1";

pub fn agent_context(profile_names: Vec<String>, feedback_enabled: bool) -> Value {
    json!({
        "schema_version": AGENT_CONTEXT_SCHEMA_VERSION,
        "cli": {
            "name": "sifs",
            "version": env!("CARGO_PKG_VERSION"),
            "non_interactive": true,
            "structured_output_flag": "--json",
            "result_stream_flag": "--jsonl",
            "canonical_vocabulary": {
                "source": "Local directory or Git URL to index and search.",
                "filter_path": "Repository-relative indexed file path filter.",
                "limit": "Maximum number of records or ranked chunks returned.",
                "force": "Required for destructive or overwriting mutations.",
                "dry_run": "Preview a mutation without changing state."
            }
        },
        "commands": {
            "agent-context": {
                "summary": "Print this machine-readable contract.",
                "flags": {"--json": {"type": "boolean", "required": true}},
                "mutates": false,
                "output": "object"
            },
            "search": {
                "summary": "Search a local directory or Git URL.",
                "args": {"query": {"type": "string", "required": true}},
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--profile": {"type": "string", "required": false},
                    "--mode": {"type": "enum", "values": ["hybrid", "semantic", "bm25"], "default": "hybrid"},
                    "--limit": {"type": "integer", "default": 5, "minimum": 1},
                    "--language": {"type": "string", "repeatable": true},
                    "--filter-path": {"type": "string", "repeatable": true},
                    "--context-lines": {"type": "integer", "default": 0, "minimum": 0},
                    "--json": {"type": "boolean"},
                    "--jsonl": {"type": "boolean"}
                },
                "mutates": false,
                "output": "search_result_set"
            },
            "find-related": {
                "summary": "Find chunks related to a known file and one-based line number.",
                "args": {
                    "file_path": {"type": "string", "required": true},
                    "line": {"type": "integer", "required": true, "minimum": 1}
                },
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--profile": {"type": "string", "required": false},
                    "--limit": {"type": "integer", "default": 5, "minimum": 1},
                    "--json": {"type": "boolean"},
                    "--jsonl": {"type": "boolean"}
                },
                "mutates": false,
                "output": "search_result_set"
            },
            "list-files": {
                "summary": "List repository-relative file paths included in an index.",
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--profile": {"type": "string", "required": false},
                    "--limit": {"type": "integer", "default": 200, "minimum": 1},
                    "--json": {"type": "boolean"}
                },
                "mutates": false,
                "output": "file_list"
            },
            "get": {
                "summary": "Print the indexed chunk containing a file and one-based line number.",
                "args": {
                    "file_path": {"type": "string", "required": true},
                    "line": {"type": "integer", "required": true, "minimum": 1}
                },
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--profile": {"type": "string", "required": false},
                    "--json": {"type": "boolean"}
                },
                "mutates": false,
                "output": "chunk"
            },
            "status": {
                "summary": "Print index status.",
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--profile": {"type": "string", "required": false},
                    "--json": {"type": "boolean"}
                },
                "mutates": false,
                "output": "index_status"
            },
            "doctor": {
                "summary": "Check model, cache, and source readiness.",
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--json": {"type": "boolean"},
                    "--offline": {"type": "boolean"},
                    "--no-download": {"type": "boolean"}
                },
                "mutates": false,
                "output": "diagnostic_report"
            },
            "profile": {
                "summary": "Manage persistent search contexts.",
                "subcommands": ["save", "list", "show", "delete"],
                "mutates": true,
                "mutation_boundary": "profile delete requires --force"
            },
            "feedback": {
                "summary": "Record local feedback about agent friction.",
                "subcommands": ["create", "list"],
                "mutates": true,
                "mutation_boundary": "local append-only feedback log"
            }
        },
        "mcp": {
            "tools": [
                "agent_context",
                "search",
                "find_related",
                "index_status",
                "refresh_index",
                "clear_index",
                "list_files",
                "get_chunk",
                "profile_list",
                "profile_show",
                "feedback_create",
                "feedback_list",
                "init_agent"
            ],
            "resources": [
                "sifs://server/context",
                "sifs://agent/context",
                "sifs://profiles",
                "sifs://feedback"
            ]
        },
        "profiles": {
            "available": profile_names,
            "precedence": ["explicit_flag", "environment", "profile", "default"]
        },
        "feedback": {
            "enabled": feedback_enabled,
            "local_first": true
        }
    })
}
