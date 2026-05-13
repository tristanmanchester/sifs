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
                    "--include-docs": {"type": "boolean", "default": false},
                    "--extension": {"type": "string", "repeatable": true},
                    "--model": {"type": "string", "required": false},
                    "--encoder": {"type": "enum", "values": ["model2vec", "hashing"], "default": "model2vec"},
                    "--offline": {"type": "boolean", "default": false},
                    "--no-download": {"type": "boolean", "default": false},
                    "--explain": {"type": "boolean", "default": false},
                    "--json": {"type": "boolean"},
                    "--jsonl": {"type": "boolean"}
                },
                "mutates": false,
                "output": "search_result_set"
            },
            "pack": {
                "summary": "Build a deduplicated context pack for a query.",
                "args": {"query": {"type": "string", "required": true}},
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--profile": {"type": "string", "required": false},
                    "--mode": {"type": "enum", "values": ["hybrid", "semantic", "bm25"], "default": "hybrid"},
                    "--budget-tokens": {"type": "integer", "default": 6000, "minimum": 1},
                    "--include-neighbors": {"type": "integer", "default": 0, "minimum": 0},
                    "--include-symbol-definitions": {"type": "boolean", "default": false},
                    "--limit": {"type": "integer", "default": 20, "minimum": 1},
                    "--include-docs": {"type": "boolean", "default": false},
                    "--extension": {"type": "string", "repeatable": true},
                    "--model": {"type": "string", "required": false},
                    "--encoder": {"type": "enum", "values": ["model2vec", "hashing"], "default": "model2vec"},
                    "--offline": {"type": "boolean", "default": false},
                    "--no-download": {"type": "boolean", "default": false},
                    "--json": {"type": "boolean"}
                },
                "mutates": false,
                "output": "context_pack"
            },
            "eval": {
                "summary": "Evaluate search quality against local feedback cases.",
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--from-feedback": {"type": "boolean"},
                    "--mode": {"type": "enum", "values": ["hybrid", "semantic", "bm25"], "default": "bm25"},
                    "--all-modes": {"type": "boolean", "default": false},
                    "--limit": {"type": "integer", "default": 10, "minimum": 1},
                    "--json": {"type": "boolean"}
                },
                "mutates": false,
                "output": "evaluation_report"
            },
            "tune": {
                "summary": "Inspect local search-tuning readiness from feedback cases.",
                "flags": {
                    "--source": {"type": "string", "default": "."},
                    "--from-feedback": {"type": "boolean"},
                    "--limit": {"type": "integer", "default": 10, "minimum": 1},
                    "--encoder": {"type": "enum", "values": ["model2vec", "hashing"], "default": "model2vec"},
                    "--offline": {"type": "boolean", "default": false},
                    "--no-download": {"type": "boolean", "default": false},
                    "--dry-run": {"type": "boolean", "required": true},
                    "--json": {"type": "boolean"}
                },
                "mutates": false,
                "output": "tuning_report"
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
            "update": {
                "summary": "Check for or install the latest SIFS release through the package manager that owns the current binary.",
                "flags": {
                    "--check": {"type": "boolean", "description": "Report update availability without planning or running mutation."},
                    "--dry-run": {"type": "boolean", "description": "Validate install ownership and print planned package-manager commands without mutation."},
                    "--json": {"type": "boolean"},
                    "--update-timeout": {"type": "integer", "default": 600}
                },
                "mutates": true,
                "mutation_boundary": "only runs Cargo or Homebrew when the current executable is proven to be owned by that manager; unsupported/dev/ambiguous installs return next actions instead",
                "output": "update_report"
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
            },
            "agent": {
                "summary": "Print, install, inspect, or remove target-specific SIFS agent artifacts.",
                "subcommands": ["print", "install", "doctor", "uninstall"],
                "targets": ["codex", "claude-code", "openclaw", "hermes", "generic", "all"],
                "artifacts": ["skill", "snippet", "mcp", "all"],
                "flags": {
                    "--target": {"type": "enum", "required": true},
                    "--artifact": {"type": "enum", "required": true},
                    "--destination": {"type": "path", "required": false},
                    "--file": {"type": "path", "required": false},
                    "--source": {"type": "string", "required": false},
                    "--profile": {"type": "string", "required": false},
                    "--dry-run": {"type": "boolean"},
                    "--force": {"type": "boolean"},
                    "--json": {"type": "boolean"}
                },
                "mutates": true,
                "mutation_boundary": "install/uninstall only touches SIFS-managed skill files or managed instruction blocks; MCP mutation stays with `sifs mcp install`"
            }
        },
        "integrations": {
            "schema_version": 1,
            "principle": "CLI-first. MCP is optional and should only be used when visible in the current agent session.",
            "targets": [
                {
                    "name": "codex",
                    "artifacts": ["skill", "snippet", "mcp"],
                    "default_skill_destination": "~/.codex/skills/sifs-search",
                    "default_snippet_file": "AGENTS.md",
                    "visibility_probe": "unknown_from_cli"
                },
                {
                    "name": "claude-code",
                    "artifacts": ["skill", "snippet", "mcp"],
                    "default_skill_destination": ".claude/agents/sifs-search.md",
                    "default_snippet_file": "CLAUDE.md",
                    "visibility_probe": "client_dependent"
                },
                {
                    "name": "openclaw",
                    "artifacts": ["skill", "snippet"],
                    "default_skill_destination": "~/.agents/skills/sifs-search",
                    "default_snippet_file": "AGENTS.md",
                    "visibility_probe": "unknown_from_cli",
                    "support_note": "local artifact install only; no public discovery claim"
                },
                {
                    "name": "hermes",
                    "artifacts": ["skill", "snippet"],
                    "default_skill_destination": "~/.agents/skills/sifs-search",
                    "default_snippet_file": "AGENTS.md",
                    "visibility_probe": "unknown_from_cli",
                    "support_note": "local artifact install only; no public discovery claim"
                },
                {
                    "name": "generic",
                    "artifacts": ["skill", "snippet"],
                    "default_skill_destination": null,
                    "default_snippet_file": "AGENTS.md",
                    "visibility_probe": "unknown_from_cli"
                }
            ],
            "commands": {
                "print": "sifs agent print --target codex --artifact snippet --json",
                "install": "sifs agent install --target codex --artifact snippet --file AGENTS.md --dry-run --json",
                "doctor": "sifs agent doctor --target codex --json",
                "uninstall": "sifs agent uninstall --target codex --artifact snippet --file AGENTS.md --dry-run --json"
            },
            "doctor_states": ["pass", "fail", "unknown"],
            "managed_snippet_markers": {
                "begin": "<!-- BEGIN SIFS AGENT INSTRUCTIONS schema=1 checksum=... -->",
                "end": "<!-- END SIFS AGENT INSTRUCTIONS -->"
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
                "agent_print",
                "agent_doctor",
                "init_agent"
            ],
            "resources": [
                "sifs://server/context",
                "sifs://agent/context",
                "sifs://index/status",
                "sifs://index/files",
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
