use anyhow::{Context, Result, anyhow, bail};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const UPDATE_SCHEMA_VERSION: u8 = 1;
const CRATES_IO_CRATE_URL: &str = "https://crates.io/api/v1/crates/sifs";
const USER_AGENT: &str = concat!(
    "sifs/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/tristanmanchester/sifs)"
);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    Check,
    DryRun,
    Execute,
}

#[derive(Clone, Debug)]
pub struct UpdateOptions {
    pub mode: UpdateMode,
    pub timeout: Duration,
    pub update_timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallOwner {
    Cargo,
    Homebrew,
    Development,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionInfo {
    pub current_version: String,
    pub actionable_latest_version: Option<String>,
    pub manager_available_version: Option<String>,
    pub upstream_latest_version: Option<String>,
    pub latest_version_source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnershipInfo {
    pub install_owner: InstallOwner,
    pub current_exe: String,
    pub canonical_exe: String,
    pub path_sifs: Option<String>,
    pub planned_target: Option<String>,
    pub target_matches_current: bool,
    pub mutation_supported: bool,
    pub owner_evidence: Vec<String>,
    pub blocking_conditions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlannedCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerOutput {
    pub status: String,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateReport {
    pub schema_version: u8,
    pub mode: UpdateMode,
    pub status: String,
    pub changed: bool,
    pub update_available: Option<bool>,
    pub actionable_update_available: Option<bool>,
    pub ownership: OwnershipInfo,
    pub versions: VersionInfo,
    pub planned_commands: Vec<PlannedCommand>,
    pub runner: Option<RunnerOutput>,
    pub verified_target_path: Option<String>,
    pub verification_status: Option<String>,
    pub warnings: Vec<String>,
    pub next_actions: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct UpdateEnvironment {
    pub current_exe: PathBuf,
    pub path_sifs: Option<PathBuf>,
    pub cargo_home: Option<PathBuf>,
    pub cargo_install_root: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub cargo_path: Option<PathBuf>,
    pub brew_path: Option<PathBuf>,
    pub latest_version_override: Option<String>,
    pub homebrew_manager_version_override: Option<String>,
    pub runner_status_override: Option<i32>,
    pub runner_log: Option<PathBuf>,
}

impl UpdateEnvironment {
    pub fn detect() -> Result<Self> {
        let current_exe = env_path_override("SIFS_UPDATE_CURRENT_EXE")
            .map(Ok)
            .unwrap_or_else(env::current_exe)
            .context("resolve current sifs executable")?;
        Ok(Self {
            current_exe,
            path_sifs: env_path_override("SIFS_UPDATE_PATH_SIFS").or_else(|| command_path("sifs")),
            cargo_home: env_path_override("CARGO_HOME"),
            cargo_install_root: env_path_override("CARGO_INSTALL_ROOT"),
            home: env_path_override("HOME"),
            cargo_path: env_path_override("SIFS_UPDATE_CARGO_PATH")
                .or_else(|| command_path("cargo")),
            brew_path: env_path_override("SIFS_UPDATE_BREW_PATH").or_else(|| command_path("brew")),
            latest_version_override: env::var("SIFS_UPDATE_LATEST_VERSION").ok(),
            homebrew_manager_version_override: env::var("SIFS_UPDATE_HOMEBREW_MANAGER_VERSION")
                .ok(),
            runner_status_override: env::var("SIFS_UPDATE_RUNNER_STATUS")
                .ok()
                .and_then(|value| value.parse::<i32>().ok()),
            runner_log: env_path_override("SIFS_UPDATE_RUNNER_LOG"),
        })
    }
}

pub fn run_update(options: &UpdateOptions) -> Result<UpdateReport> {
    let env = UpdateEnvironment::detect()?;
    run_update_with_env(options, &env)
}

pub fn run_update_with_env(
    options: &UpdateOptions,
    environment: &UpdateEnvironment,
) -> Result<UpdateReport> {
    let ownership = detect_ownership(environment);
    let versions = resolve_versions(&ownership, environment, options.timeout)?;
    let actionable_update_available = actionable_update_available(&versions)?;
    let planned_commands = if options.mode != UpdateMode::Check
        && ownership.mutation_supported
        && actionable_update_available == Some(true)
    {
        plan_commands(&ownership, environment)?
    } else {
        Vec::new()
    };

    let mut report = UpdateReport {
        schema_version: UPDATE_SCHEMA_VERSION,
        mode: options.mode,
        status: status_for(
            options.mode,
            &ownership,
            actionable_update_available,
            &planned_commands,
        ),
        changed: false,
        update_available: actionable_update_available,
        actionable_update_available,
        ownership,
        versions,
        planned_commands,
        runner: None,
        verified_target_path: None,
        verification_status: None,
        warnings: Vec::new(),
        next_actions: Vec::new(),
    };
    add_next_actions(&mut report);

    if options.mode != UpdateMode::Execute {
        return Ok(report);
    }
    if !report.ownership.mutation_supported {
        return Ok(report);
    }
    if report.actionable_update_available != Some(true) {
        return Ok(report);
    }
    if report.planned_commands.is_empty() {
        bail!("no safe package-manager command could be planned");
    }

    let mut changed = false;
    for command in report.planned_commands.clone() {
        let output = run_planned_command(&command, environment, options.update_timeout)?;
        let ok = output.status == "success";
        report.runner = Some(output);
        if !ok {
            report.status = "failed".to_owned();
            return Ok(report);
        }
        changed = true;
    }
    report.changed = changed;
    report.status = if changed { "updated" } else { "unchanged" }.to_owned();
    if let Some(target) = &report.ownership.planned_target {
        report.verified_target_path = Some(target.clone());
        report.verification_status = Some("not_checked_in_process".to_owned());
        report.warnings.push(
            "rerun `sifs --version` after update to confirm the new process version".to_owned(),
        );
    }
    Ok(report)
}

fn detect_ownership(environment: &UpdateEnvironment) -> OwnershipInfo {
    let current_exe = environment.current_exe.clone();
    let canonical_exe = canonical_or_original(&current_exe);
    let path_sifs = environment
        .path_sifs
        .as_ref()
        .map(|path| canonical_or_original(path));
    let mut evidence = Vec::new();
    let mut blocking = Vec::new();

    if is_development_path(&canonical_exe) {
        blocking.push("development_binary".to_owned());
        evidence.push("current executable is under a Cargo target/debug/release path".to_owned());
        return ownership(
            InstallOwner::Development,
            current_exe,
            canonical_exe,
            path_sifs,
            None,
            false,
            evidence,
            blocking,
        );
    }

    if let Some(target) = cargo_target(environment) {
        let target = canonical_or_original(&target);
        if same_path(&canonical_exe, &target) {
            evidence.push("current executable matches Cargo install root target".to_owned());
            return ownership(
                InstallOwner::Cargo,
                current_exe,
                canonical_exe,
                path_sifs,
                Some(target),
                true,
                evidence,
                blocking,
            );
        }
        if looks_cargo_path(&canonical_exe) {
            blocking.push("cargo_target_mismatch".to_owned());
            evidence.push(format!("expected Cargo target {}", target.display()));
            return ownership(
                InstallOwner::Cargo,
                current_exe,
                canonical_exe,
                path_sifs,
                Some(target),
                false,
                evidence,
                blocking,
            );
        }
    }

    if looks_homebrew_path(&canonical_exe) {
        let target = canonical_exe.clone();
        evidence.push("current executable is under a Homebrew prefix or Cellar path".to_owned());
        return ownership(
            InstallOwner::Homebrew,
            current_exe,
            canonical_exe,
            path_sifs,
            Some(target),
            true,
            evidence,
            blocking,
        );
    }

    blocking.push("unknown_install_owner".to_owned());
    ownership(
        InstallOwner::Unknown,
        current_exe,
        canonical_exe,
        path_sifs,
        None,
        false,
        evidence,
        blocking,
    )
}

#[allow(clippy::too_many_arguments)]
fn ownership(
    install_owner: InstallOwner,
    current_exe: PathBuf,
    canonical_exe: PathBuf,
    path_sifs: Option<PathBuf>,
    planned_target: Option<PathBuf>,
    mutation_supported: bool,
    owner_evidence: Vec<String>,
    blocking_conditions: Vec<String>,
) -> OwnershipInfo {
    let target_matches_current = planned_target
        .as_ref()
        .map(|target| same_path(&canonical_exe, target))
        .unwrap_or(false);
    OwnershipInfo {
        install_owner,
        current_exe: current_exe.display().to_string(),
        canonical_exe: canonical_exe.display().to_string(),
        path_sifs: path_sifs.map(|path| path.display().to_string()),
        planned_target: planned_target.map(|path| path.display().to_string()),
        target_matches_current,
        mutation_supported: mutation_supported && target_matches_current,
        owner_evidence,
        blocking_conditions,
    }
}

fn resolve_versions(
    ownership: &OwnershipInfo,
    environment: &UpdateEnvironment,
    timeout: Duration,
) -> Result<VersionInfo> {
    let current_version = env!("CARGO_PKG_VERSION").to_owned();
    match ownership.install_owner {
        InstallOwner::Cargo => {
            let latest = latest_crates_version(environment, timeout).ok();
            Ok(VersionInfo {
                current_version,
                actionable_latest_version: latest.clone(),
                manager_available_version: latest.clone(),
                upstream_latest_version: latest,
                latest_version_source: Some("crates_io".to_owned()),
            })
        }
        InstallOwner::Homebrew => {
            let manager = homebrew_manager_version(environment, timeout).ok();
            let upstream = latest_crates_version(environment, timeout).ok();
            Ok(VersionInfo {
                current_version,
                actionable_latest_version: manager.clone(),
                manager_available_version: manager,
                upstream_latest_version: upstream,
                latest_version_source: Some("homebrew".to_owned()),
            })
        }
        InstallOwner::Development | InstallOwner::Unknown => {
            let latest = latest_crates_version(environment, timeout).ok();
            Ok(VersionInfo {
                current_version,
                actionable_latest_version: latest.clone(),
                manager_available_version: None,
                upstream_latest_version: latest,
                latest_version_source: Some("crates_io".to_owned()),
            })
        }
    }
}

fn latest_crates_version(environment: &UpdateEnvironment, timeout: Duration) -> Result<String> {
    if let Some(version) = &environment.latest_version_override {
        validate_version(version)?;
        return Ok(version.clone());
    }
    let response: Value = ureq::AgentBuilder::new()
        .timeout(timeout)
        .build()
        .get(CRATES_IO_CRATE_URL)
        .set("User-Agent", USER_AGENT)
        .call()
        .context("fetch latest sifs version from crates.io")?
        .into_json()
        .context("parse crates.io response")?;
    let version = response
        .get("crate")
        .and_then(|krate| {
            krate
                .get("max_stable_version")
                .or_else(|| krate.get("max_version"))
        })
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("crates.io response did not include a latest version"))?;
    validate_version(version)?;
    Ok(version.to_owned())
}

fn homebrew_manager_version(environment: &UpdateEnvironment, timeout: Duration) -> Result<String> {
    if let Some(version) = &environment.homebrew_manager_version_override {
        validate_version(version)?;
        return Ok(version.clone());
    }
    let brew = trusted_program(environment.brew_path.as_deref(), "brew")?;
    let output = run_metadata_command(
        &PlannedCommand {
            program: brew.display().to_string(),
            args: vec![
                "info".to_owned(),
                "--json=v2".to_owned(),
                "tristanmanchester/tap/sifs".to_owned(),
            ],
        },
        timeout,
    )?;
    if output.status != "success" {
        bail!("brew info failed: {}", output.stderr.trim());
    }
    let value: Value = serde_json::from_str(&output.stdout).context("parse brew info JSON")?;
    let version = value
        .get("formulae")
        .and_then(Value::as_array)
        .and_then(|formulae| formulae.first())
        .and_then(|formula| formula.get("versions"))
        .and_then(|versions| versions.get("stable"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("brew info JSON did not include a stable version"))?;
    validate_version(version)?;
    Ok(version.to_owned())
}

fn actionable_update_available(versions: &VersionInfo) -> Result<Option<bool>> {
    let Some(latest) = &versions.actionable_latest_version else {
        return Ok(None);
    };
    let current = validate_version(&versions.current_version)?;
    let latest = validate_version(latest)?;
    Ok(Some(latest > current))
}

fn validate_version(version: &str) -> Result<Version> {
    let parsed = Version::parse(version).with_context(|| format!("parse version {version}"))?;
    if !parsed.pre.is_empty() {
        bail!("pre-release versions are not supported for update checks: {version}");
    }
    Ok(parsed)
}

fn plan_commands(
    ownership: &OwnershipInfo,
    environment: &UpdateEnvironment,
) -> Result<Vec<PlannedCommand>> {
    match ownership.install_owner {
        InstallOwner::Cargo => {
            let cargo = trusted_program(environment.cargo_path.as_deref(), "cargo")?;
            Ok(vec![PlannedCommand {
                program: cargo.display().to_string(),
                args: vec![
                    "install".to_owned(),
                    "--locked".to_owned(),
                    "sifs".to_owned(),
                    "--force".to_owned(),
                ],
            }])
        }
        InstallOwner::Homebrew => {
            let brew = trusted_program(environment.brew_path.as_deref(), "brew")?;
            Ok(vec![
                PlannedCommand {
                    program: brew.display().to_string(),
                    args: vec!["update".to_owned()],
                },
                PlannedCommand {
                    program: brew.display().to_string(),
                    args: vec![
                        "upgrade".to_owned(),
                        "tristanmanchester/tap/sifs".to_owned(),
                    ],
                },
            ])
        }
        InstallOwner::Development | InstallOwner::Unknown => Ok(Vec::new()),
    }
}

fn trusted_program(path: Option<&Path>, name: &str) -> Result<PathBuf> {
    let path = path
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("{name} was not found on PATH"))?;
    if !path.is_absolute() {
        bail!("{name} resolved to a non-absolute path: {}", path.display());
    }
    let canonical = canonical_or_original(&path);
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if canonical.starts_with(&cwd) || canonical.starts_with(env::temp_dir()) {
        bail!(
            "{name} resolved to an unsafe path for package-manager mutation: {}",
            canonical.display()
        );
    }
    Ok(canonical)
}

fn run_planned_command(
    command: &PlannedCommand,
    environment: &UpdateEnvironment,
    timeout: Duration,
) -> Result<RunnerOutput> {
    if let Some(log) = &environment.runner_log {
        let line = format!("{} {}\n", command.program, command.args.join(" "));
        fs::write(log, line).with_context(|| format!("write runner log {}", log.display()))?;
    }
    if let Some(code) = environment.runner_status_override {
        return Ok(RunnerOutput {
            status: if code == 0 { "success" } else { "failed" }.to_owned(),
            code: Some(code),
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            elapsed_ms: 0,
        });
    }
    let started = Instant::now();
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("run {}", command.program))?;

    loop {
        if let Some(_status) = child.try_wait()? {
            let output = child.wait_with_output()?;
            let elapsed_ms = started.elapsed().as_millis();
            let (stdout, stdout_truncated) = capped_text(&output.stdout);
            let (stderr, stderr_truncated) = capped_text(&output.stderr);
            return Ok(RunnerOutput {
                status: if output.status.success() {
                    "success"
                } else {
                    "failed"
                }
                .to_owned(),
                code: output.status.code(),
                stdout,
                stderr,
                stdout_truncated,
                stderr_truncated,
                elapsed_ms,
            });
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(RunnerOutput {
                status: "timeout".to_owned(),
                code: None,
                stdout: String::new(),
                stderr: format!("timed out after {} seconds", timeout.as_secs()),
                stdout_truncated: false,
                stderr_truncated: false,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run_metadata_command(command: &PlannedCommand, timeout: Duration) -> Result<RunnerOutput> {
    let environment = UpdateEnvironment {
        current_exe: PathBuf::new(),
        path_sifs: None,
        cargo_home: None,
        cargo_install_root: None,
        home: None,
        cargo_path: None,
        brew_path: None,
        latest_version_override: None,
        homebrew_manager_version_override: None,
        runner_status_override: None,
        runner_log: None,
    };
    run_planned_command(command, &environment, timeout)
}

fn capped_text(bytes: &[u8]) -> (String, bool) {
    const MAX: usize = 16 * 1024;
    let truncated = bytes.len() > MAX;
    let slice = if truncated { &bytes[..MAX] } else { bytes };
    (String::from_utf8_lossy(slice).into_owned(), truncated)
}

fn status_for(
    mode: UpdateMode,
    ownership: &OwnershipInfo,
    actionable_update_available: Option<bool>,
    planned_commands: &[PlannedCommand],
) -> String {
    if !ownership.mutation_supported && mode != UpdateMode::Check {
        return "unsupported".to_owned();
    }
    match actionable_update_available {
        Some(false) => "unchanged".to_owned(),
        Some(true) if mode == UpdateMode::Check => "update_available".to_owned(),
        Some(true) if planned_commands.is_empty() => "blocked".to_owned(),
        Some(true) => "planned".to_owned(),
        None => "unknown".to_owned(),
    }
}

fn add_next_actions(report: &mut UpdateReport) {
    if !report.ownership.mutation_supported {
        report.next_actions.push(match report.ownership.install_owner {
            InstallOwner::Development => {
                "Install SIFS with `cargo install --locked sifs` or Homebrew, then rerun `sifs update`.".to_owned()
            }
            InstallOwner::Cargo => {
                "Resolve the Cargo install-root mismatch, or run `cargo install --locked sifs --force` manually.".to_owned()
            }
            InstallOwner::Homebrew => {
                "Resolve the Homebrew ownership mismatch, or run `brew upgrade tristanmanchester/tap/sifs` manually.".to_owned()
            }
            InstallOwner::Unknown => {
                "Update manually with `cargo install --locked sifs --force` or `brew upgrade tristanmanchester/tap/sifs`.".to_owned()
            }
        });
    } else if report.actionable_update_available == Some(true) {
        report.next_actions.push(
            "Run `sifs update --dry-run` to preview mutation, or `sifs update` to update."
                .to_owned(),
        );
    }
}

fn cargo_target(environment: &UpdateEnvironment) -> Option<PathBuf> {
    if let Some(root) = &environment.cargo_install_root {
        return Some(root.join("bin/sifs"));
    }
    environment
        .cargo_home
        .clone()
        .or_else(|| environment.home.as_ref().map(|home| home.join(".cargo")))
        .map(|root| root.join("bin/sifs"))
}

fn command_path(command: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

fn env_path_override(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn canonical_or_original(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn same_path(left: &Path, right: &Path) -> bool {
    canonical_or_original(left) == canonical_or_original(right)
}

fn is_development_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("target" | "debug" | "release")
        )
    })
}

fn looks_cargo_path(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str().to_str() == Some(".cargo"))
}

fn looks_homebrew_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains("/Homebrew/")
        || text.contains("/Cellar/sifs/")
        || text.contains("/opt/homebrew/")
        || text.contains("/usr/local/Cellar/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_for(current: &str) -> UpdateEnvironment {
        UpdateEnvironment {
            current_exe: PathBuf::from(current),
            path_sifs: None,
            cargo_home: Some(PathBuf::from("/home/me/.cargo")),
            cargo_install_root: None,
            home: Some(PathBuf::from("/home/me")),
            cargo_path: Some(PathBuf::from("/usr/bin/cargo")),
            brew_path: Some(PathBuf::from("/opt/homebrew/bin/brew")),
            latest_version_override: Some("9.9.9".to_owned()),
            homebrew_manager_version_override: Some("9.9.9".to_owned()),
            runner_status_override: Some(0),
            runner_log: None,
        }
    }

    #[test]
    fn cargo_install_owned_binary_plans_cargo_update() {
        let env = env_for("/home/me/.cargo/bin/sifs");
        let report = run_update_with_env(
            &UpdateOptions {
                mode: UpdateMode::DryRun,
                timeout: Duration::from_secs(1),
                update_timeout: Duration::from_secs(1),
            },
            &env,
        )
        .unwrap();
        assert_eq!(report.ownership.install_owner, InstallOwner::Cargo);
        assert!(report.ownership.mutation_supported);
        assert_eq!(report.actionable_update_available, Some(true));
        assert_eq!(
            report.planned_commands[0].args,
            ["install", "--locked", "sifs", "--force"]
        );
    }

    #[test]
    fn cargo_target_mismatch_blocks_mutation() {
        let env = env_for("/tmp/copied/.cargo/bin/sifs");
        let report = run_update_with_env(
            &UpdateOptions {
                mode: UpdateMode::DryRun,
                timeout: Duration::from_secs(1),
                update_timeout: Duration::from_secs(1),
            },
            &env,
        )
        .unwrap();
        assert!(!report.ownership.mutation_supported);
        assert!(
            report
                .ownership
                .blocking_conditions
                .contains(&"development_binary".to_owned())
                || report
                    .ownership
                    .blocking_conditions
                    .contains(&"unknown_install_owner".to_owned())
                || report
                    .ownership
                    .blocking_conditions
                    .contains(&"cargo_target_mismatch".to_owned())
        );
    }

    #[test]
    fn homebrew_uses_manager_version_as_actionable_latest() {
        let mut env = env_for("/opt/homebrew/Cellar/sifs/0.3.0/bin/sifs");
        env.latest_version_override = Some("9.9.9".to_owned());
        env.homebrew_manager_version_override = Some(env!("CARGO_PKG_VERSION").to_owned());
        let report = run_update_with_env(
            &UpdateOptions {
                mode: UpdateMode::Check,
                timeout: Duration::from_secs(1),
                update_timeout: Duration::from_secs(1),
            },
            &env,
        )
        .unwrap();
        assert_eq!(report.ownership.install_owner, InstallOwner::Homebrew);
        assert_eq!(
            report.versions.upstream_latest_version.as_deref(),
            Some("9.9.9")
        );
        assert_eq!(report.actionable_update_available, Some(false));
    }

    #[test]
    fn development_binary_is_unsupported() {
        let env = env_for("/repo/target/debug/sifs");
        let report = run_update_with_env(
            &UpdateOptions {
                mode: UpdateMode::DryRun,
                timeout: Duration::from_secs(1),
                update_timeout: Duration::from_secs(1),
            },
            &env,
        )
        .unwrap();
        assert_eq!(report.ownership.install_owner, InstallOwner::Development);
        assert_eq!(report.status, "unsupported");
        assert!(report.planned_commands.is_empty());
    }
}
