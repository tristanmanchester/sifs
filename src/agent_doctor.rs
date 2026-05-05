use crate::agent_artifacts::{
    AgentArtifact, AgentTarget, MANAGED_BLOCK_BEGIN_PREFIX, render_artifact,
};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug, Serialize)]
pub struct AgentDoctorOutput {
    pub schema_version: u8,
    pub targets: Vec<AgentDoctorTarget>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentDoctorTarget {
    pub target: AgentTarget,
    pub status: String,
    pub checks: Vec<AgentDoctorCheck>,
    pub next_actions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentDoctorCheck {
    pub name: String,
    pub state: String,
    pub evidence: String,
}

pub fn doctor(target: AgentTarget, artifact: AgentArtifact) -> AgentDoctorOutput {
    let mut targets = Vec::new();
    for concrete_target in target.concrete_targets() {
        let artifacts = artifact.concrete_artifacts(concrete_target);
        targets.push(doctor_target(concrete_target, artifacts));
    }
    AgentDoctorOutput {
        schema_version: 1,
        targets,
    }
}

fn doctor_target(target: AgentTarget, artifacts: Vec<AgentArtifact>) -> AgentDoctorTarget {
    let mut checks = Vec::new();
    let binary = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sifs"));
    let binary_ok = Command::new(&binary).arg("--version").output();
    match binary_ok {
        Ok(output) if output.status.success() => checks.push(check(
            "binary_on_path",
            "pass",
            format!(
                "{} ({})",
                binary.display(),
                String::from_utf8_lossy(&output.stdout).trim()
            ),
        )),
        Ok(output) => checks.push(check(
            "binary_on_path",
            "fail",
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        )),
        Err(err) => checks.push(check("binary_on_path", "fail", err.to_string())),
    }

    for artifact in artifacts {
        match artifact {
            AgentArtifact::Skill => checks.push(skill_check(target)),
            AgentArtifact::Snippet => checks.push(snippet_check(target)),
            AgentArtifact::Mcp => checks.extend(mcp_checks(target)),
            AgentArtifact::All => {}
        }
    }
    checks.push(check(
        "visible_to_current_session",
        "unknown",
        "Current-session agent tool visibility cannot be proven from this process.",
    ));
    checks.push(check(
        "cli_fallback_ready",
        if checks
            .iter()
            .any(|check| check.name == "binary_on_path" && check.state == "pass")
        {
            "pass"
        } else {
            "fail"
        },
        "CLI fallback uses shell commands such as `sifs search`.",
    ));
    let status = if checks
        .iter()
        .any(|check| check.name == "binary_on_path" && check.state == "pass")
    {
        "ready_fallback_only"
    } else {
        "not_ready"
    };
    AgentDoctorTarget {
        target,
        status: status.to_owned(),
        checks,
        next_actions: vec![format!(
            "Use CLI fallback: sifs search \"<query>\" --source <project>"
        )],
    }
}

fn skill_check(target: AgentTarget) -> AgentDoctorCheck {
    let Some(path) = target.default_skill_destination() else {
        return check(
            "skill_present",
            "unknown",
            "No default skill destination is known; pass --destination during install.",
        );
    };
    if !path.exists() {
        return check(
            "skill_present",
            "fail",
            format!("{} does not exist", path.display()),
        );
    }
    if path.is_dir() {
        let skill = path.join("SKILL.md");
        return content_check("skill_content_current", skill, target, AgentArtifact::Skill);
    }
    content_check("skill_content_current", path, target, AgentArtifact::Skill)
}

fn snippet_check(target: AgentTarget) -> AgentDoctorCheck {
    let Some(path) = target.default_snippet_file() else {
        return check("snippet_present", "unknown", "No default snippet file.");
    };
    if !path.exists() {
        return check(
            "snippet_present",
            "fail",
            format!("{} does not exist", path.display()),
        );
    }
    match fs::read_to_string(&path) {
        Ok(content) if content.contains(MANAGED_BLOCK_BEGIN_PREFIX) => check(
            "snippet_present",
            "pass",
            format!("{} contains a SIFS managed block", path.display()),
        ),
        Ok(_) => check(
            "snippet_present",
            "fail",
            format!("{} has no SIFS managed block", path.display()),
        ),
        Err(err) => check("snippet_present", "fail", err.to_string()),
    }
}

fn content_check(
    name: &str,
    path: PathBuf,
    target: AgentTarget,
    artifact: AgentArtifact,
) -> AgentDoctorCheck {
    match fs::read_to_string(&path) {
        Ok(content) => match render_artifact(target, artifact, None, None) {
            Ok(rendered)
                if content == rendered.content || content.contains("name: sifs-search") =>
            {
                check(
                    name,
                    "pass",
                    format!("{} is a SIFS skill artifact", path.display()),
                )
            }
            Ok(_) => check(
                name,
                "fail",
                format!("{} is stale or modified", path.display()),
            ),
            Err(err) => check(name, "unknown", err.to_string()),
        },
        Err(err) => check(name, "fail", err.to_string()),
    }
}

fn mcp_checks(target: AgentTarget) -> Vec<AgentDoctorCheck> {
    let config_present = match target {
        AgentTarget::Codex => std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".codex/config.toml"))
            .filter(|path| path.exists())
            .map(|path| {
                fs::read_to_string(&path)
                    .map(|content| content.contains("[mcp_servers.sifs]"))
                    .unwrap_or(false)
            })
            .unwrap_or(false),
        AgentTarget::ClaudeCode if PathBuf::from(".mcp.json").exists() => {
            fs::read_to_string(".mcp.json")
                .map(|content| content.contains("\"sifs\""))
                .unwrap_or(false)
        }
        AgentTarget::ClaudeCode => false,
        _ => false,
    };
    vec![
        check(
            "mcp_config_present",
            if config_present { "pass" } else { "fail" },
            if config_present {
                "SIFS MCP config appears present."
            } else {
                "No SIFS MCP config was found for this target."
            },
        ),
        check(
            "mcp_handshake_ok",
            "unknown",
            "Run `sifs mcp doctor --json` for protocol handshake and BM25 smoke probes.",
        ),
        check(
            "mcp_tools_listed",
            "unknown",
            "Current-session MCP tools cannot be listed from this process.",
        ),
        check(
            "search_smoke_ok",
            "unknown",
            "Run `sifs mcp doctor --source <project> --offline --no-cache --json`.",
        ),
    ]
}

fn check(
    name: impl Into<String>,
    state: impl Into<String>,
    evidence: impl Into<String>,
) -> AgentDoctorCheck {
    AgentDoctorCheck {
        name: name.into(),
        state: state.into(),
        evidence: evidence.into(),
    }
}
