use anyhow::{Result, bail};
use clap::ValueEnum;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::PathBuf;

pub const AGENT_ARTIFACT_SCHEMA_VERSION: u8 = 1;
pub const MANAGED_BLOCK_BEGIN_PREFIX: &str = "<!-- BEGIN SIFS AGENT INSTRUCTIONS";
pub const MANAGED_BLOCK_END: &str = "<!-- END SIFS AGENT INSTRUCTIONS -->";

pub const CANONICAL_SKILL: &str = include_str!("../skills/sifs-search/SKILL.md");
pub const COMMANDS_REFERENCE: &str = include_str!("../skills/sifs-search/references/commands.md");
pub const MCP_REFERENCE: &str = include_str!("../skills/sifs-search/references/mcp.md");
pub const TROUBLESHOOTING_REFERENCE: &str =
    include_str!("../skills/sifs-search/references/troubleshooting.md");
pub const CHECK_SETUP_SCRIPT: &str = include_str!("../skills/sifs-search/scripts/check-setup.sh");

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum AgentTarget {
    Codex,
    ClaudeCode,
    Openclaw,
    Hermes,
    Generic,
    All,
}

impl AgentTarget {
    pub fn concrete_targets(self) -> Vec<Self> {
        match self {
            Self::All => vec![
                Self::Codex,
                Self::ClaudeCode,
                Self::Openclaw,
                Self::Hermes,
                Self::Generic,
            ],
            target => vec![target],
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::Openclaw => "openclaw",
            Self::Hermes => "hermes",
            Self::Generic => "generic",
            Self::All => "all",
        }
    }

    pub fn default_snippet_file(self) -> Option<PathBuf> {
        match self {
            Self::Codex | Self::Openclaw | Self::Hermes | Self::Generic => {
                Some(PathBuf::from("AGENTS.md"))
            }
            Self::ClaudeCode => Some(PathBuf::from("CLAUDE.md")),
            Self::All => None,
        }
    }

    pub fn default_skill_destination(self) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match self {
            Self::Codex => home.map(|home| home.join(".codex").join("skills").join("sifs-search")),
            Self::ClaudeCode => Some(PathBuf::from(".claude/agents/sifs-search.md")),
            Self::Openclaw | Self::Hermes => {
                home.map(|home| home.join(".agents").join("skills").join("sifs-search"))
            }
            Self::Generic | Self::All => None,
        }
    }

    pub fn supports_artifact(self, artifact: AgentArtifact) -> bool {
        match artifact {
            AgentArtifact::Skill | AgentArtifact::Snippet => !matches!(self, Self::All),
            AgentArtifact::Mcp => matches!(self, Self::Codex | Self::ClaudeCode),
            AgentArtifact::All => !matches!(self, Self::All),
        }
    }
}

impl fmt::Display for AgentTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum AgentArtifact {
    Skill,
    Snippet,
    Mcp,
    All,
}

impl AgentArtifact {
    pub fn concrete_artifacts(self, target: AgentTarget) -> Vec<Self> {
        let artifacts: Vec<Self> = match self {
            Self::All => [Self::Skill, Self::Snippet, Self::Mcp]
                .into_iter()
                .collect(),
            artifact => vec![artifact],
        };
        artifacts
            .into_iter()
            .filter(|artifact| target.supports_artifact(*artifact))
            .collect()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Snippet => "snippet",
            Self::Mcp => "mcp",
            Self::All => "all",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentArtifact, AgentTarget};

    #[test]
    fn specific_artifacts_are_filtered_by_target_support() {
        assert_eq!(
            AgentArtifact::Mcp.concrete_artifacts(AgentTarget::Openclaw),
            Vec::<AgentArtifact>::new()
        );
        assert_eq!(
            AgentArtifact::Mcp.concrete_artifacts(AgentTarget::Codex),
            vec![AgentArtifact::Mcp]
        );
    }
}

impl fmt::Display for AgentArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentPrintOutput {
    pub schema_version: u8,
    pub target: AgentTarget,
    pub artifact: AgentArtifact,
    pub destination_hint: Option<String>,
    pub content: String,
    pub checksum: String,
    pub mcp_optional: bool,
    pub mcp_required: bool,
    pub warnings: Vec<String>,
    pub next_actions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillPackageFile {
    pub relative_path: &'static str,
    pub content: &'static str,
    pub executable: bool,
}

#[derive(Clone, Debug)]
pub struct RenderedArtifact {
    pub target: AgentTarget,
    pub artifact: AgentArtifact,
    pub content: String,
    pub checksum: String,
    pub destination_hint: Option<PathBuf>,
    pub mcp_optional: bool,
    pub mcp_required: bool,
    pub warnings: Vec<String>,
    pub next_actions: Vec<String>,
}

impl RenderedArtifact {
    pub fn print_output(&self) -> AgentPrintOutput {
        AgentPrintOutput {
            schema_version: AGENT_ARTIFACT_SCHEMA_VERSION,
            target: self.target,
            artifact: self.artifact,
            destination_hint: self
                .destination_hint
                .as_ref()
                .map(|path| path.display().to_string()),
            content: self.content.clone(),
            checksum: self.checksum.clone(),
            mcp_optional: self.mcp_optional,
            mcp_required: self.mcp_required,
            warnings: self.warnings.clone(),
            next_actions: self.next_actions.clone(),
        }
    }
}

pub fn render_artifact(
    target: AgentTarget,
    artifact: AgentArtifact,
    source: Option<&str>,
    profile: Option<&str>,
) -> Result<RenderedArtifact> {
    if matches!(target, AgentTarget::All) || matches!(artifact, AgentArtifact::All) {
        bail!("render_artifact requires concrete target and artifact");
    }
    if !target.supports_artifact(artifact) {
        bail!("{target} does not support {artifact} artifacts");
    }
    let content = match artifact {
        AgentArtifact::Skill => render_skill(target, source, profile),
        AgentArtifact::Snippet => render_snippet(target, source, profile),
        AgentArtifact::Mcp => render_mcp_guidance(target),
        AgentArtifact::All => unreachable!(),
    };
    let checksum = checksum(&content);
    let destination_hint = match artifact {
        AgentArtifact::Skill => target.default_skill_destination(),
        AgentArtifact::Snippet => target.default_snippet_file(),
        AgentArtifact::Mcp | AgentArtifact::All => None,
    };
    let mut warnings = Vec::new();
    if artifact == AgentArtifact::Skill && destination_hint.is_none() {
        warnings.push("No default skill destination is known; pass --destination.".to_owned());
    }
    let next_actions = next_actions(target, artifact, destination_hint.as_ref());
    Ok(RenderedArtifact {
        target,
        artifact,
        content,
        checksum,
        destination_hint,
        mcp_optional: true,
        mcp_required: false,
        warnings,
        next_actions,
    })
}

pub fn skill_package_files() -> Vec<SkillPackageFile> {
    vec![
        SkillPackageFile {
            relative_path: "SKILL.md",
            content: CANONICAL_SKILL,
            executable: false,
        },
        SkillPackageFile {
            relative_path: "references/commands.md",
            content: COMMANDS_REFERENCE,
            executable: false,
        },
        SkillPackageFile {
            relative_path: "references/mcp.md",
            content: MCP_REFERENCE,
            executable: false,
        },
        SkillPackageFile {
            relative_path: "references/troubleshooting.md",
            content: TROUBLESHOOTING_REFERENCE,
            executable: false,
        },
        SkillPackageFile {
            relative_path: "scripts/check-setup.sh",
            content: CHECK_SETUP_SCRIPT,
            executable: true,
        },
    ]
}

pub fn checksum(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

pub fn managed_snippet(content: &str) -> String {
    let checksum = checksum(content);
    format!(
        "{MANAGED_BLOCK_BEGIN_PREFIX} schema={AGENT_ARTIFACT_SCHEMA_VERSION} checksum={checksum} -->\n{content}\n{MANAGED_BLOCK_END}\n"
    )
}

fn render_skill(target: AgentTarget, source: Option<&str>, profile: Option<&str>) -> String {
    match target {
        AgentTarget::ClaudeCode => {
            let source_note = source
                .map(|source| format!("\nProject source hint: use `--source {source}` when searching this checkout.\n"))
                .unwrap_or_default();
            let profile_note = profile
                .map(|profile| {
                    format!("\nProfile hint: use `--profile {profile}` when appropriate.\n")
                })
                .unwrap_or_default();
            format!(
                "{}\n{source_note}{profile_note}",
                include_str!("agents/sifs-search.md")
            )
        }
        _ => {
            let source_note = source
                .map(|source| format!("\nWhen working in the target project, prefer `--source {source}` if the current directory is ambiguous.\n"))
                .unwrap_or_default();
            let profile_note = profile
                .map(|profile| format!("\nA SIFS profile named `{profile}` was provided; use it when it matches the task.\n"))
                .unwrap_or_default();
            format!("{CANONICAL_SKILL}\n{source_note}{profile_note}")
        }
    }
}

fn render_snippet(target: AgentTarget, source: Option<&str>, profile: Option<&str>) -> String {
    let file_name = target
        .default_snippet_file()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "agent instructions".to_owned());
    let source_arg = source
        .map(|source| format!(" --source {source}"))
        .unwrap_or_else(|| " --source <project>".to_owned());
    let profile_line = profile
        .map(|profile| format!("\n- A SIFS profile named `{profile}` is available; use `--profile {profile}` when it matches this task."))
        .unwrap_or_default();
    format!(
        "## SIFS Code Search\n\nUse SIFS for codebase search before broad file reads when you need to find behavior, symbols, related implementations, or relevant files.\n\n- Discover the current contract with `sifs agent-context --json`.\n- Search with `sifs search \"<query>\"{source_arg} --limit 10`.\n- Narrow by path with `--filter-path <repo-relative-path>` and use `--mode bm25` for exact symbols.\n- Inspect results with `sifs get <file_path> <line>{source_arg}` and `sifs find-related <file_path> <line>{source_arg}`.\n- If SIFS MCP tools are visible in the current session, they may be used; if not, fall back to the CLI immediately.{profile_line}\n\nThis block is intended for `{file_name}` and is managed by `sifs agent install`."
    )
}

fn render_mcp_guidance(target: AgentTarget) -> String {
    let client = match target {
        AgentTarget::Codex => "codex",
        AgentTarget::ClaudeCode => "claude",
        _ => "all",
    };
    format!(
        "SIFS MCP is optional. Configure it with:\n\n```bash\nsifs mcp install --client {client} --dry-run\nsifs mcp doctor --offline --no-cache\n```\n\nOnly use MCP tools when they are visible in the current agent session. Otherwise use `sifs search`, `sifs list-files`, `sifs get`, and `sifs agent-context --json` from the shell.\n"
    )
}

fn next_actions(
    target: AgentTarget,
    artifact: AgentArtifact,
    destination_hint: Option<&PathBuf>,
) -> Vec<String> {
    match artifact {
        AgentArtifact::Skill => {
            let mut command = format!("sifs agent install --target {target} --artifact skill");
            if let Some(destination) = destination_hint {
                command.push_str(&format!(" --destination {}", destination.display()));
            } else {
                command.push_str(" --destination <path>");
            }
            vec![command]
        }
        AgentArtifact::Snippet => {
            let file = destination_hint
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "AGENTS.md".to_owned());
            vec![format!(
                "sifs agent install --target {target} --artifact snippet --file {file}"
            )]
        }
        AgentArtifact::Mcp => vec![format!(
            "sifs mcp install --client {} --dry-run",
            if target == AgentTarget::ClaudeCode {
                "claude"
            } else {
                target.as_str()
            }
        )],
        AgentArtifact::All => Vec::new(),
    }
}
