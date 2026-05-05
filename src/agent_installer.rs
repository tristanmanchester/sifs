use crate::agent_artifacts::{
    AgentArtifact, AgentTarget, MANAGED_BLOCK_BEGIN_PREFIX, MANAGED_BLOCK_END, RenderedArtifact,
    checksum, managed_snippet, skill_package_files,
};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub enum AgentOperation {
    Install,
    Uninstall,
}

#[derive(Clone, Debug)]
pub struct AgentMutationOptions {
    pub target: AgentTarget,
    pub artifact: AgentArtifact,
    pub destination: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub dry_run: bool,
    pub force: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentMutationReport {
    pub target: AgentTarget,
    pub artifact: AgentArtifact,
    pub status: String,
    pub changed: bool,
    pub destination: Option<String>,
    pub checksum: Option<String>,
    pub warnings: Vec<String>,
    pub next_actions: Vec<String>,
}

pub fn apply_mutation(
    operation: AgentOperation,
    rendered: &RenderedArtifact,
    options: &AgentMutationOptions,
) -> Result<AgentMutationReport> {
    match (operation, rendered.artifact) {
        (AgentOperation::Install, AgentArtifact::Snippet) => install_snippet(rendered, options),
        (AgentOperation::Uninstall, AgentArtifact::Snippet) => uninstall_snippet(rendered, options),
        (AgentOperation::Install, AgentArtifact::Skill) => install_skill(rendered, options),
        (AgentOperation::Uninstall, AgentArtifact::Skill) => uninstall_skill(rendered, options),
        (AgentOperation::Install, AgentArtifact::Mcp) => Ok(mcp_redirect_report(rendered)),
        (AgentOperation::Uninstall, AgentArtifact::Mcp) => Ok(mcp_redirect_report(rendered)),
        _ => bail!("unsupported mutation"),
    }
}

fn install_snippet(
    rendered: &RenderedArtifact,
    options: &AgentMutationOptions,
) -> Result<AgentMutationReport> {
    let path = options
        .file
        .clone()
        .or_else(|| rendered.destination_hint.clone())
        .context("snippet install requires --file or a known default file")?;
    let snippet = managed_snippet(&rendered.content);
    let existing = fs::read_to_string(&path).ok();
    let (status, changed, new_content) = match existing.as_deref() {
        None => ("installed", true, ensure_trailing_newline(&snippet)),
        Some(text) => {
            let block = find_managed_block(text)?;
            match block {
                ManagedBlock::Absent => {
                    let mut content = ensure_trailing_newline(text);
                    content.push('\n');
                    content.push_str(&snippet);
                    ("installed", true, content)
                }
                ManagedBlock::Present {
                    start,
                    end,
                    current,
                } => {
                    if current == snippet {
                        ("unchanged", false, text.to_owned())
                    } else if generated_block_checksum_matches(current) || options.force {
                        let mut content = String::new();
                        content.push_str(&text[..start]);
                        content.push_str(&snippet);
                        content.push_str(&text[end..]);
                        ("updated", true, content)
                    } else {
                        bail!(
                            "{} contains a user-modified SIFS block. Re-run with --force to replace only the managed block.",
                            path.display()
                        );
                    }
                }
            }
        }
    };
    if changed && !options.dry_run {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, new_content)?;
    }
    Ok(report(
        rendered,
        status,
        changed && !options.dry_run,
        Some(path),
        if options.dry_run && changed {
            vec!["dry-run: no files were changed".to_owned()]
        } else {
            Vec::new()
        },
    ))
}

fn uninstall_snippet(
    rendered: &RenderedArtifact,
    options: &AgentMutationOptions,
) -> Result<AgentMutationReport> {
    let path = options
        .file
        .clone()
        .or_else(|| rendered.destination_hint.clone())
        .context("snippet uninstall requires --file or a known default file")?;
    let Some(existing) = fs::read_to_string(&path).ok() else {
        return Ok(report(rendered, "unchanged", false, Some(path), Vec::new()));
    };
    let ManagedBlock::Present {
        start,
        end,
        current,
    } = find_managed_block(&existing)?
    else {
        return Ok(report(rendered, "unchanged", false, Some(path), Vec::new()));
    };
    let expected = managed_snippet(&rendered.content);
    if current != expected && !generated_block_checksum_matches(current) && !options.force {
        bail!(
            "{} contains a user-modified SIFS block. Re-run with --force to remove it.",
            path.display()
        );
    }
    let mut content = String::new();
    content.push_str(&existing[..start]);
    content.push_str(&existing[end..]);
    if !options.dry_run {
        fs::write(&path, content)?;
    }
    Ok(report(
        rendered,
        "removed",
        !options.dry_run,
        Some(path),
        if options.dry_run {
            vec!["dry-run: no files were changed".to_owned()]
        } else {
            Vec::new()
        },
    ))
}

fn install_skill(
    rendered: &RenderedArtifact,
    options: &AgentMutationOptions,
) -> Result<AgentMutationReport> {
    let destination = options
        .destination
        .clone()
        .or_else(|| rendered.destination_hint.clone())
        .context("skill install requires --destination for this target")?;
    if rendered.target == AgentTarget::ClaudeCode {
        return install_single_skill_file(rendered, options, destination);
    }
    let mut changed = false;
    for file in skill_package_files() {
        let target = destination.join(file.relative_path);
        if target.exists() {
            let current = fs::read_to_string(&target)?;
            if current == file.content {
                continue;
            }
            if !current.contains("name: sifs-search") && !options.force {
                bail!(
                    "{} exists and is not a SIFS-managed skill file. Re-run with --force to replace it.",
                    target.display()
                );
            }
        }
        changed = true;
        if !options.dry_run {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, file.content)?;
            #[cfg(unix)]
            if file.executable {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&target)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&target, perms)?;
            }
        }
    }
    Ok(report(
        rendered,
        if changed { "installed" } else { "unchanged" },
        changed && !options.dry_run,
        Some(destination),
        if options.dry_run && changed {
            vec!["dry-run: no files were changed".to_owned()]
        } else {
            Vec::new()
        },
    ))
}

fn install_single_skill_file(
    rendered: &RenderedArtifact,
    options: &AgentMutationOptions,
    destination: PathBuf,
) -> Result<AgentMutationReport> {
    let changed = if destination.exists() {
        let current = fs::read_to_string(&destination)?;
        if current == rendered.content {
            false
        } else if current.contains("name: sifs-search") || options.force {
            true
        } else {
            bail!(
                "{} exists and is not a SIFS-managed skill file. Re-run with --force to replace it.",
                destination.display()
            );
        }
    } else {
        true
    };
    if changed && !options.dry_run {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &rendered.content)?;
    }
    Ok(report(
        rendered,
        if changed { "installed" } else { "unchanged" },
        changed && !options.dry_run,
        Some(destination),
        if options.dry_run && changed {
            vec!["dry-run: no files were changed".to_owned()]
        } else {
            Vec::new()
        },
    ))
}

fn uninstall_skill(
    rendered: &RenderedArtifact,
    options: &AgentMutationOptions,
) -> Result<AgentMutationReport> {
    let destination = options
        .destination
        .clone()
        .or_else(|| rendered.destination_hint.clone())
        .context("skill uninstall requires --destination for this target")?;
    if !destination.exists() {
        return Ok(report(
            rendered,
            "unchanged",
            false,
            Some(destination),
            Vec::new(),
        ));
    }
    if destination.is_dir() {
        let skill = destination.join("SKILL.md");
        if skill.exists() {
            let current = fs::read_to_string(&skill)?;
            if !current.contains("name: sifs-search") && !options.force {
                bail!(
                    "{} is not a SIFS-managed skill package. Re-run with --force to remove it.",
                    destination.display()
                );
            }
        }
        if !options.dry_run {
            fs::remove_dir_all(&destination)?;
        }
    } else {
        let current = fs::read_to_string(&destination)?;
        if !current.contains("name: sifs-search")
            && checksum(&current) != rendered.checksum
            && !options.force
        {
            bail!(
                "{} is not a SIFS-managed skill file. Re-run with --force to remove it.",
                destination.display()
            );
        }
        if !options.dry_run {
            fs::remove_file(&destination)?;
        }
    }
    Ok(report(
        rendered,
        "removed",
        !options.dry_run,
        Some(destination),
        if options.dry_run {
            vec!["dry-run: no files were changed".to_owned()]
        } else {
            Vec::new()
        },
    ))
}

fn mcp_redirect_report(rendered: &RenderedArtifact) -> AgentMutationReport {
    report(
        rendered,
        "unsupported",
        false,
        None,
        vec!["Use `sifs mcp install` or `sifs mcp doctor` for MCP configuration.".to_owned()],
    )
}

fn report(
    rendered: &RenderedArtifact,
    status: &str,
    changed: bool,
    destination: Option<PathBuf>,
    warnings: Vec<String>,
) -> AgentMutationReport {
    AgentMutationReport {
        target: rendered.target,
        artifact: rendered.artifact,
        status: status.to_owned(),
        changed,
        destination: destination.map(|path| path.display().to_string()),
        checksum: Some(rendered.checksum.clone()),
        warnings,
        next_actions: rendered.next_actions.clone(),
    }
}

enum ManagedBlock<'a> {
    Absent,
    Present {
        start: usize,
        end: usize,
        current: &'a str,
    },
}

fn find_managed_block(content: &str) -> Result<ManagedBlock<'_>> {
    let Some(start) = content.find(MANAGED_BLOCK_BEGIN_PREFIX) else {
        return Ok(ManagedBlock::Absent);
    };
    let after_start = &content[start..];
    let Some(relative_end) = after_start.find(MANAGED_BLOCK_END) else {
        bail!("found a SIFS managed block start marker without an end marker");
    };
    let end = start + relative_end + MANAGED_BLOCK_END.len();
    let end = if content[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    if content[end..].contains(MANAGED_BLOCK_BEGIN_PREFIX) {
        bail!("found multiple SIFS managed blocks");
    }
    Ok(ManagedBlock::Present {
        start,
        end,
        current: &content[start..end],
    })
}

fn generated_block_checksum_matches(block: &str) -> bool {
    let Some(first_line_end) = block.find('\n') else {
        return false;
    };
    let header = &block[..first_line_end];
    let Some(checksum_start) = header.find("checksum=") else {
        return false;
    };
    let checksum_value = &header[checksum_start + "checksum=".len()..];
    let checksum_value = checksum_value
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches("-->")
        .trim();
    let body_start = first_line_end + 1;
    let Some(end_start) = block[body_start..].find(MANAGED_BLOCK_END) else {
        return false;
    };
    let body = &block[body_start..body_start + end_start];
    let body = body.strip_suffix('\n').unwrap_or(body);
    checksum(body) == checksum_value
}

fn ensure_trailing_newline(content: &str) -> String {
    if content.ends_with('\n') {
        content.to_owned()
    } else {
        format!("{content}\n")
    }
}

#[allow(dead_code)]
fn _is_within(path: &Path, parent: &Path) -> bool {
    path.starts_with(parent)
}
