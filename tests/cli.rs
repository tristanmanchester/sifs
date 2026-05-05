use serde_json::Value;
use std::fs;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

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

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
fn version_flag_prints_package_version() {
    for flag in ["--version", "-V"] {
        let output = sifs().arg(flag).output().unwrap();

        assert!(
            output.status.success(),
            "stderr for {flag}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stdout.trim(), format!("sifs {}", env!("CARGO_PKG_VERSION")));
    }
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
    assert!(stdout.contains("--source"));
}

#[test]
fn daemon_run_ping_and_status_work_over_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("sifs.sock");
    let child = sifs()
        .args(["daemon", "run", "--replace-existing-socket"])
        .env("SIFS_DAEMON_SOCKET", &socket)
        .spawn()
        .unwrap();
    let _guard = ChildGuard(child);

    let deadline = Instant::now() + Duration::from_secs(5);
    let ping = loop {
        let output = sifs()
            .args(["daemon", "ping"])
            .env("SIFS_DAEMON_SOCKET", &socket)
            .output()
            .unwrap();
        if output.status.success() {
            break output;
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not become ready: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        thread::sleep(Duration::from_millis(50));
    };

    let stdout = String::from_utf8(ping.stdout).unwrap();
    assert!(stdout.contains("SIFS daemon is running"));

    let status = sifs()
        .args(["daemon", "status", "--json"])
        .env("SIFS_DAEMON_SOCKET", &socket)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["indexes"].as_array().unwrap().len(), 0);
}

#[test]
fn daemon_install_agent_dry_run_prints_launch_agent() {
    let output = sifs()
        .args(["daemon", "install-agent", "--dry-run"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("<string>dev.sifs.daemon</string>"));
    assert!(stdout.contains("<string>daemon</string>"));
    assert!(stdout.contains("<string>run</string>"));
    assert!(stdout.contains("--replace-existing-socket"));
}

#[test]
fn search_uses_running_daemon_and_populates_status() {
    let repo = fixture();
    let runtime = tempfile::tempdir().unwrap();
    let socket = runtime.path().join("sifs.sock");
    let child = sifs()
        .args(["daemon", "run", "--replace-existing-socket"])
        .env("SIFS_DAEMON_SOCKET", &socket)
        .spawn()
        .unwrap();
    let _guard = ChildGuard(child);
    wait_for_daemon(&socket);

    let output = sifs()
        .args([
            "search",
            "token validation",
            "--source",
            repo.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--json",
        ])
        .env("SIFS_DAEMON_SOCKET", &socket)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let status = sifs()
        .args(["daemon", "status", "--json"])
        .env("SIFS_DAEMON_SOCKET", &socket)
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["indexes"].as_array().unwrap().len(), 1);
}

fn wait_for_daemon(socket: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = sifs()
            .args(["daemon", "ping"])
            .env("SIFS_DAEMON_SOCKET", socket)
            .output()
            .unwrap();
        if output.status.success() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not become ready: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        thread::sleep(Duration::from_millis(50));
    }
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
fn mcp_install_dry_run_can_install_without_pinned_source() {
    let output = sifs()
        .args(["mcp", "install", "--dry-run", "--client", "codex"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("codex mcp add sifs --"));
    assert!(stdout.contains(" mcp"));
    assert!(stdout.contains("args = [\"mcp\"]"));
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
fn mcp_doctor_reports_handshake_smoke_separately_from_search() {
    let dir = fixture();
    let output = sifs()
        .args([
            "mcp",
            "doctor",
            "--source",
            dir.path().to_str().unwrap(),
            "--offline",
            "--no-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("SIFS MCP doctor"));
    assert!(stdout.contains("MCP command:"));
    assert!(stdout.contains("MCP handshake (newline): passed"));
    assert!(stdout.contains("MCP handshake (Content-Length): passed"));
    assert!(stdout.contains("BM25 smoke: passed"));
}

#[test]
fn search_json_is_structured() {
    let dir = fixture();
    let output = sifs()
        .args([
            "search",
            "token validation",
            "--source",
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
    assert!(!value["results"].as_array().unwrap().is_empty());
}

#[test]
fn agent_context_json_describes_agent_native_contract() {
    let output = sifs().args(["agent-context", "--json"]).output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], "1");
    assert_eq!(value["cli"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(value["commands"]["search"]["flags"]["--source"].is_object());
    assert!(value["commands"]["search"]["flags"]["--limit"].is_object());
    assert!(value["commands"]["list-files"].is_object());
    assert!(
        value["mcp"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "list_files")
    );
    assert!(value["integrations"]["targets"].is_array());
    assert_eq!(
        value["integrations"]["commands"]["install"],
        "sifs agent install --target codex --artifact snippet --file AGENTS.md --dry-run --json"
    );
    assert!(
        value["mcp"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "agent_print")
    );
}

#[test]
fn agent_print_snippet_json_is_cli_first_and_mcp_optional() {
    let output = sifs()
        .args([
            "agent",
            "print",
            "--target",
            "codex",
            "--artifact",
            "snippet",
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
    assert_eq!(value["target"], "codex");
    assert_eq!(value["artifact"], "snippet");
    assert_eq!(value["mcp_optional"], true);
    assert_eq!(value["mcp_required"], false);
    assert!(value["checksum"].as_str().unwrap().starts_with("sha256:"));
    let content = value["content"].as_str().unwrap();
    assert!(content.contains("sifs agent-context --json"));
    assert!(content.contains("sifs search"));
    assert!(content.contains("fall back to the CLI"));
}

#[test]
fn agent_print_raw_skill_has_no_json_wrapper() {
    let output = sifs()
        .args([
            "agent",
            "print",
            "--target",
            "generic",
            "--artifact",
            "skill",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("---\nname: sifs-search"));
    assert!(stdout.contains("sifs list-files"));
    assert!(!stdout.trim_start().starts_with('{'));
}

#[test]
fn agent_snippet_install_is_dry_run_idempotent_and_uninstall_safe() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("AGENTS.md");
    fs::write(
        &agents,
        "# Project Instructions\n\nKeep existing guidance.\n",
    )
    .unwrap();

    let dry_run = sifs()
        .args([
            "agent",
            "install",
            "--target",
            "codex",
            "--artifact",
            "snippet",
            "--file",
            agents.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        dry_run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let planned: Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(planned["dry_run"], true);
    assert_eq!(planned["results"][0]["status"], "installed");
    assert_eq!(
        fs::read_to_string(&agents).unwrap(),
        "# Project Instructions\n\nKeep existing guidance.\n"
    );

    let install = sifs()
        .args([
            "agent",
            "install",
            "--target",
            "codex",
            "--artifact",
            "snippet",
            "--file",
            agents.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        install.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&install.stderr)
    );
    let content = fs::read_to_string(&agents).unwrap();
    assert!(content.contains("# Project Instructions"));
    assert!(content.contains("<!-- BEGIN SIFS AGENT INSTRUCTIONS"));
    assert_eq!(content.matches("BEGIN SIFS AGENT INSTRUCTIONS").count(), 1);

    let second = sifs()
        .args([
            "agent",
            "install",
            "--target",
            "codex",
            "--artifact",
            "snippet",
            "--file",
            agents.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_json: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_json["results"][0]["status"], "unchanged");
    assert_eq!(
        fs::read_to_string(&agents)
            .unwrap()
            .matches("BEGIN SIFS AGENT INSTRUCTIONS")
            .count(),
        1
    );

    let uninstall = sifs()
        .args([
            "agent",
            "uninstall",
            "--target",
            "codex",
            "--artifact",
            "snippet",
            "--file",
            agents.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        uninstall.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&uninstall.stderr)
    );
    let final_content = fs::read_to_string(&agents).unwrap();
    assert!(final_content.contains("Keep existing guidance."));
    assert!(!final_content.contains("SIFS AGENT INSTRUCTIONS"));
}

#[test]
fn agent_install_refuses_user_modified_managed_block_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let agents = dir.path().join("AGENTS.md");
    fs::write(
        &agents,
        "<!-- BEGIN SIFS AGENT INSTRUCTIONS -->\nuser changed this\n<!-- END SIFS AGENT INSTRUCTIONS -->\n",
    )
    .unwrap();

    let output = sifs()
        .args([
            "agent",
            "install",
            "--target",
            "codex",
            "--artifact",
            "snippet",
            "--file",
            agents.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("user-modified SIFS block"));
}

#[test]
fn agent_skill_install_writes_package_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("sifs-search");

    let output = sifs()
        .args([
            "agent",
            "install",
            "--target",
            "generic",
            "--artifact",
            "skill",
            "--destination",
            destination.to_str().unwrap(),
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
    assert_eq!(value["results"][0]["status"], "installed");
    assert!(destination.join("SKILL.md").exists());
    assert!(destination.join("references/commands.md").exists());
    assert!(destination.join("scripts/check-setup.sh").exists());
    let skill = fs::read_to_string(destination.join("SKILL.md")).unwrap();
    assert!(skill.contains("name: sifs-search"));
    assert!(skill.contains("sifs agent-context --json"));

    let second = sifs()
        .args([
            "agent",
            "install",
            "--target",
            "generic",
            "--artifact",
            "skill",
            "--destination",
            destination.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_json: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_json["results"][0]["status"], "unchanged");
}

#[test]
fn agent_doctor_json_reports_readiness_matrix() {
    let output = sifs()
        .args([
            "agent",
            "doctor",
            "--target",
            "codex",
            "--artifact",
            "all",
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
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["targets"][0]["target"], "codex");
    let checks = value["targets"][0]["checks"].as_array().unwrap();
    assert!(checks.iter().any(|check| check["name"] == "binary_on_path"));
    assert!(
        checks
            .iter()
            .any(|check| check["name"] == "visible_to_current_session"
                && check["state"] == "unknown")
    );
}

#[test]
fn profiles_and_feedback_are_json_capable_and_isolated_by_home() {
    let dir = fixture();
    let home = tempfile::tempdir().unwrap();

    let save = sifs()
        .args([
            "profile",
            "save",
            "agent-test",
            "--source",
            dir.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--limit",
            "3",
            "--offline",
            "--json",
        ])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        save.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&save.stderr)
    );
    let saved: Value = serde_json::from_slice(&save.stdout).unwrap();
    assert_eq!(saved["profile"]["name"], "agent-test");
    assert_eq!(saved["changed"], true);

    let list = sifs()
        .args(["profile", "list", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&list.stderr)
    );
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(listed["profiles"][0]["name"], "agent-test");

    let search = sifs()
        .args([
            "search",
            "token validation",
            "--profile",
            "agent-test",
            "--json",
        ])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        search.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&search.stderr)
    );
    let searched: Value = serde_json::from_slice(&search.stdout).unwrap();
    assert_eq!(searched["source"], dir.path().to_str().unwrap());
    assert_eq!(searched["mode"], "bm25");
    assert_eq!(searched["limit"], 3);

    let feedback = sifs()
        .args([
            "feedback",
            "create",
            "invalid mode error was useful",
            "--command-context",
            "search",
            "--json",
        ])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        feedback.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&feedback.stderr)
    );
    let created: Value = serde_json::from_slice(&feedback.stdout).unwrap();
    assert_eq!(created["changed"], true);
    assert_eq!(
        created["feedback"]["message"],
        "invalid mode error was useful"
    );

    let feedback_list = sifs()
        .args(["feedback", "list", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        feedback_list.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&feedback_list.stderr)
    );
    let entries: Value = serde_json::from_slice(&feedback_list.stdout).unwrap();
    assert_eq!(entries["total"], 1);
    assert_eq!(entries["feedback"][0]["command_context"], "search");
}

#[test]
fn search_jsonl_is_parseable_without_markdown() {
    let dir = fixture();
    let output = sifs()
        .args([
            "search",
            "token validation",
            "--source",
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
            "--source",
            dir.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--language",
            "rust",
            "--filter-path",
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
            "--source",
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
    assert!(!value["results"].as_array().unwrap().is_empty());
}

#[test]
fn files_status_and_get_work_against_fixture() {
    let dir = fixture();

    let files = sifs()
        .args([
            "list-files",
            "--source",
            dir.path().to_str().unwrap(),
            "--format",
            "compact",
        ])
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
        .args(["status", "--source", dir.path().to_str().unwrap(), "--json"])
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
            "--source",
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
            "--source",
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
