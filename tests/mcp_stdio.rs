use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
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
        "pub fn auth_flow() -> bool {\n    token_validation()\n}\n",
    )
    .unwrap();
    dir
}

fn run_mcp(input: &[u8]) -> Output {
    let dir = fixture();
    run_mcp_for_source(input, dir.path(), &[])
}

fn run_mcp_for_source(
    input: &[u8],
    source: &std::path::Path,
    envs: &[(&str, &std::path::Path)],
) -> Output {
    let mut child = sifs()
        .args([
            "mcp",
            "--source",
            source.to_str().unwrap(),
            "--offline",
            "--no-cache",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(envs.iter().map(|(key, value)| (*key, value)))
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(input).unwrap();
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn run_mcp_without_source(input: &[u8], envs: &[(&str, &std::path::Path)]) -> Output {
    let mut child = sifs()
        .args(["mcp", "--offline", "--no-cache"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(envs.iter().map(|(key, value)| (*key, value)))
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(input).unwrap();
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn short_socket_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("sifs-")
        .tempdir_in("/tmp")
        .unwrap()
}

fn socket_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("sifs.sock")
}

fn unix_sockets_available() -> bool {
    let dir = short_socket_tempdir();
    UnixListener::bind(socket_path(&dir)).is_ok()
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for_daemon(socket: &Path) {
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

fn content_length_message(message: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(message).unwrap();
    let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    framed.extend(body);
    framed
}

fn parse_content_length_response(output: &[u8]) -> Value {
    let separator = b"\r\n\r\n";
    let header_end = output
        .windows(separator.len())
        .position(|window| window == separator)
        .expect("missing Content-Length separator");
    let header = std::str::from_utf8(&output[..header_end]).unwrap();
    let length = header
        .strip_prefix("Content-Length: ")
        .expect("missing Content-Length header")
        .parse::<usize>()
        .unwrap();
    let body_start = header_end + separator.len();
    let body_end = body_start + length;
    serde_json::from_slice(&output[body_start..body_end]).unwrap()
}

#[test]
fn content_length_initialize_gets_content_length_response() {
    let output = run_mcp(&content_length_message(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }
    })));

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).starts_with("Content-Length: "),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let response = parse_content_length_response(&output.stdout);
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
    assert!(response["result"]["capabilities"]["tools"].is_object());
}

#[test]
fn newline_initialize_and_tools_list_get_newline_responses() {
    let input = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }
        })
        .to_string(),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        })
        .to_string(),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        })
        .to_string(),
    ]
    .join("\n")
        + "\n";

    let output = run_mcp(input.as_bytes());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("Content-Length:"),
        "newline transport should not emit Content-Length: {stdout}"
    );
    let responses: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        responses.len(),
        2,
        "initialized notification must not produce a response"
    );
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[1]["id"], 2);
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert!(tools.len() >= 5);
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"symbol"));
    assert!(names.contains(&"outline"));
    assert!(names.contains(&"pack"));
}

#[test]
fn unsupported_protocol_version_falls_back_without_startup_failure() {
    let input = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2099-01-01",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }
    })
    .to_string()
        + "\n";

    let output = run_mcp(input.as_bytes());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
}

#[test]
fn empty_stdin_exits_without_stdout_banner() {
    let output = run_mcp(b"");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "stdout must contain only MCP protocol messages"
    );
}

#[test]
fn unknown_method_returns_structured_result_without_corrupting_transport() {
    let input = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "sifs/unknown"
    })
    .to_string()
        + "\n";

    let output = run_mcp(input.as_bytes());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 9);
    assert!(
        response["result"]["error"]
            .as_str()
            .unwrap()
            .contains("Unsupported method")
    );
}

#[test]
fn empty_source_argument_is_rejected_instead_of_indexing_cwd() {
    let input = json!({
        "jsonrpc": "2.0",
        "id": 7,
            "method": "tools/call",
            "params": {
                "name": "index_status",
                "arguments": {"source": ""}
            }
    })
    .to_string()
        + "\n";

    let output = run_mcp(input.as_bytes());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 7);
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("source must not be empty")
    );
}

#[test]
fn invalid_search_mode_returns_actionable_tool_error() {
    let input = json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {
            "name": "search",
            "arguments": {
                "query": "token validation",
                "mode": "lexical"
            }
        }
    })
    .to_string()
        + "\n";

    let output = run_mcp(input.as_bytes());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 8);
    assert_eq!(response["result"]["isError"], true);
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("mode must be one of: hybrid, semantic, bm25"));
    assert!(text.contains("lexical"));
}

#[test]
fn symbol_tool_returns_structured_symbol_postings() {
    let input = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "symbol",
            "arguments": {"name": "token_validation"}
        }
    })
    .to_string()
        + "\n";

    let output = run_mcp(input.as_bytes());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 10);
    let symbols = response["result"]["structuredContent"]["symbols"]
        .as_array()
        .unwrap();
    assert_eq!(symbols[0]["name"], "token_validation");
    assert_eq!(symbols[0]["file_path"], "src/lib.rs");
}

#[test]
fn outline_tool_returns_indexed_file_outline() {
    let input = json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tools/call",
        "params": {
            "name": "outline",
            "arguments": {"file_path": "src/lib.rs"}
        }
    })
    .to_string()
        + "\n";

    let output = run_mcp(input.as_bytes());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 11);
    let outline = &response["result"]["structuredContent"]["outline"];
    assert_eq!(outline["file_path"], "src/lib.rs");
    assert_eq!(outline["symbols"][0]["name"], "token_validation");
}

#[test]
fn list_files_tool_accepts_documented_limit_and_per_call_docs_scope() {
    let repo = fixture();
    fs::write(repo.path().join("notes.md"), "# Token notes\n").unwrap();
    let input = json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "tools/call",
        "params": {
            "name": "list_files",
            "arguments": {"limit": 200, "include_docs": true, "extensions": ["md"]}
        }
    })
    .to_string()
        + "\n";

    let output = run_mcp_for_source(input.as_bytes(), repo.path(), &[]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 13);
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["limit"], 200);
    assert!(
        structured["files"]
            .as_array()
            .unwrap()
            .contains(&json!("notes.md"))
    );
}

#[test]
fn structural_mcp_tools_work_with_running_daemon() {
    if !unix_sockets_available() {
        return;
    }
    let repo = fixture();
    let runtime = short_socket_tempdir();
    let socket = socket_path(&runtime);
    let child = sifs()
        .args(["daemon", "run", "--replace-existing-socket"])
        .env("SIFS_DAEMON_SOCKET", &socket)
        .spawn()
        .unwrap();
    let _guard = ChildGuard(child);
    wait_for_daemon(&socket);

    let input = json!({
        "jsonrpc": "2.0",
        "id": 14,
        "method": "tools/call",
        "params": {
            "name": "pack",
            "arguments": {
                "query": "auth_flow token_validation",
                "mode": "bm25",
                "budget_tokens": 400,
                "include_symbol_definitions": true
            }
        }
    })
    .to_string()
        + "\n";

    let output = run_mcp_for_source(
        input.as_bytes(),
        repo.path(),
        &[("SIFS_DAEMON_SOCKET", &socket)],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 14);
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["query"], "auth_flow token_validation");
    assert!(structured["items"].as_array().unwrap().iter().any(|item| {
        item["kind"] == "symbol_definition"
            && item["file_path"].as_str().unwrap().starts_with("src/")
    }));

    let status = sifs()
        .args(["daemon", "status", "--json"])
        .env("SIFS_DAEMON_SOCKET", &socket)
        .output()
        .unwrap();
    let status_value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status_value["indexes"].as_array().unwrap().len(), 1);
}

#[test]
fn pack_tool_returns_bounded_context_items() {
    let input = json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "tools/call",
        "params": {
            "name": "pack",
            "arguments": {
                "query": "auth_flow token_validation",
                "mode": "bm25",
                "budget_tokens": 400,
                "include_symbol_definitions": true
            }
        }
    })
    .to_string()
        + "\n";

    let output = run_mcp(input.as_bytes());

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 12);
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["query"], "auth_flow token_validation");
    assert_eq!(structured["include_symbol_definitions"], true);
    assert!(structured["estimated_tokens_used"].as_u64().unwrap() <= 400);
    let items = structured["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert!(items.iter().any(|item| item["kind"] == "symbol_definition"));
}

#[test]
fn profile_search_applies_document_and_extension_index_options() {
    let repo = fixture();
    fs::write(
        repo.path().join("release-notes.md"),
        "# Release notes\n\nThe zephyr changelog explains the MCP profile docs contract.\n",
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let save = sifs()
        .args([
            "profile",
            "save",
            "agent-docs",
            "--source",
            repo.path().to_str().unwrap(),
            "--mode",
            "bm25",
            "--include-docs",
            "--extension",
            "MD",
            "--no-cache",
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

    let input = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "tools/call",
        "params": {
            "name": "search",
            "arguments": {
                "profile": "agent-docs",
                "query": "zephyr changelog"
            }
        }
    })
    .to_string()
        + "\n";

    let output = run_mcp_without_source(input.as_bytes(), &[("HOME", home.path())]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value =
        serde_json::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap();
    assert_eq!(response["id"], 9);
    let results = response["result"]["structuredContent"]["results"]
        .as_array()
        .unwrap();
    assert!(!results.is_empty());
    assert!(
        results
            .iter()
            .any(|result| result["file_path"] == "release-notes.md")
    );
}
