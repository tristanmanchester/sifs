use serde_json::Value;
use std::fs;
use std::process::Command;

fn sifs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sifs"))
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn token_validation() -> bool {\n    true\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/auth.rs"),
        "pub fn auth_flow() {\n    let token = token_validation();\n}\n",
    )
    .unwrap();
    fs::write(dir.path().join("README.md"), "# Auth flow\n").unwrap();
    dir
}

#[test]
fn bare_sifs_prints_help() {
    let output = sifs().output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage: sifs"));
    assert!(stdout.contains("mcp"));
}

#[test]
fn mcp_help_documents_server_options() {
    let output = sifs().args(["mcp", "--help"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("install"));
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("--ref"));
    assert!(stdout.contains("--model"));
    assert!(stdout.contains("--offline"));
    assert!(stdout.contains("--no-download"));
    assert!(stdout.contains("[PATH]"));
}

#[test]
fn mcp_install_dry_run_prints_codex_command_and_config() {
    let dir = fixture();
    let output = sifs()
        .args([
            "mcp",
            "install",
            "--dry-run",
            "--client",
            "codex",
            "--source",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("codex mcp add sifs --"));
    assert!(stdout.contains(" mcp "));
    assert!(stdout.contains(dir.path().canonicalize().unwrap().to_str().unwrap()));
    assert!(stdout.contains("[mcp_servers.sifs]"));
    assert!(stdout.contains("startup_timeout_sec = 20"));
    assert!(stdout.contains("tool_timeout_sec = 60"));
}

#[test]
fn mcp_install_dry_run_prints_claude_command_and_project_json() {
    let dir = fixture();
    let output = sifs()
        .args([
            "mcp",
            "install",
            "--dry-run",
            "--client",
            "claude",
            "--scope",
            "local",
            "--source",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("claude mcp add-json sifs"));
    assert!(stdout.contains("--scope local"));
    assert!(stdout.contains("\"type\": \"stdio\""));
    assert!(stdout.contains("\"command\""));
    assert!(stdout.contains("\"args\": ["));
    assert!(stdout.contains("\"mcp\""));
    assert!(stdout.contains(dir.path().canonicalize().unwrap().to_str().unwrap()));
}

#[test]
fn mcp_install_dry_run_all_includes_offline_for_both_clients() {
    let dir = fixture();
    let output = sifs()
        .args([
            "mcp",
            "install",
            "--dry-run",
            "--source",
            dir.path().to_str().unwrap(),
            "--offline",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Codex MCP:"));
    assert!(stdout.contains("Claude Code MCP:"));
    assert!(stdout.contains("--offline"));
}

#[test]
fn mcp_install_offline_rejects_git_url() {
    let output = sifs()
        .args([
            "mcp",
            "install",
            "--dry-run",
            "--source",
            "https://github.com/owner/repo",
            "--offline",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--offline does not allow remote Git sources"));
}

#[test]
fn search_json_is_structured() {
    let dir = fixture();
    let output = sifs()
        .args([
            "search",
            "token validation",
            dir.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["query"], "token validation");
    assert_eq!(value["mode"], "bm25");
    assert!(value["index_stats"]["indexed_files"].as_u64().unwrap() >= 2);
    assert!(value["warnings"].as_array().unwrap().is_empty());
    assert!(value["results"].as_array().unwrap().len() > 0);
}

#[test]
fn search_jsonl_is_parseable_without_markdown() {
    let dir = fixture();
    let output = sifs()
        .args([
            "search",
            "token validation",
            dir.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--jsonl",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("```"));
    let rows: Vec<_> = stdout.lines().collect();
    assert!(!rows.is_empty());
    for row in rows {
        let value: Value = serde_json::from_str(row).unwrap();
        assert_eq!(value["query"], "token validation");
        assert_eq!(value["mode"], "bm25");
        assert!(value["result"]["file_path"].is_string());
    }
}

#[test]
fn search_filters_by_language_and_path() {
    let dir = fixture();
    let output = sifs()
        .args([
            "search",
            "token",
            dir.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--language",
            "rust",
            "--path",
            "src/lib.rs",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = value["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert!(
        results
            .iter()
            .all(|result| result["file_path"] == "src/lib.rs" && result["language"] == "rust")
    );
}

#[test]
fn find_related_json_is_structured() {
    let dir = fixture();
    let output = sifs()
        .args([
            "find-related",
            "src/lib.rs",
            "1",
            dir.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["file_path"], "src/lib.rs");
    assert_eq!(value["line"], 1);
    assert!(value["results"].as_array().unwrap().len() > 0);
}

#[test]
fn files_status_and_get_work_against_fixture() {
    let dir = fixture();

    let files = sifs()
        .args(["files", dir.path().to_str().unwrap(), "--format", "compact"])
        .output()
        .unwrap();
    assert!(
        files.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&files.stderr)
    );
    let files_stdout = String::from_utf8(files.stdout).unwrap();
    assert!(files_stdout.contains("src/lib.rs"));

    let status = sifs()
        .args(["status", dir.path().to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(
        status_value["index_stats"]["indexed_files"]
            .as_u64()
            .unwrap()
            >= 2
    );

    let get = sifs()
        .args([
            "get",
            "src/lib.rs",
            "1",
            dir.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&get.stderr)
    );
    let get_value: Value = serde_json::from_slice(&get.stdout).unwrap();
    assert_eq!(get_value["chunk"]["file_path"], "src/lib.rs");
}

#[test]
fn json_and_jsonl_conflict() {
    let dir = fixture();
    let output = sifs()
        .args([
            "search",
            "token",
            dir.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--json",
            "--jsonl",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("cannot be used with"));
}
