#!/usr/bin/env python3
"""Check or publish the OpenClaw SIFS skill package.

This script intentionally supports a dry readiness check so maintainers can
prepare the package without publishing it.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SKILL_DIR = ROOT / "extras" / "openclaw" / "sifs-search"
SKILL_FILE = SKILL_DIR / "SKILL.md"
CHANGELOG = ROOT / "CHANGELOG.md"
SLUG = "sifs-search"


def run(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=ROOT,
        check=check,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def openclaw_metadata() -> dict:
    text = SKILL_FILE.read_text(encoding="utf-8")
    match = re.search(r"^metadata:\s*(\{.*\})\s*$", text, re.MULTILINE)
    if not match:
        raise SystemExit("OpenClaw SKILL.md must contain single-line JSON metadata")
    try:
        data = json.loads(match.group(1))
    except json.JSONDecodeError as exc:
        raise SystemExit(f"Invalid OpenClaw metadata JSON: {exc}") from exc
    metadata = data.get("openclaw")
    if not isinstance(metadata, dict):
        raise SystemExit("OpenClaw metadata must contain an openclaw object")
    version = metadata.get("version")
    if not isinstance(version, str) or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise SystemExit("OpenClaw metadata version must be semver, for example 0.1.0")
    return metadata


def inspect_remote() -> dict | None:
    try:
        proc = run(["clawhub", "inspect", SLUG, "--json"], check=False)
    except FileNotFoundError:
        print("clawhub CLI is not installed; skipping remote slug inspection.")
        return None
    if proc.returncode == 0:
        try:
            return json.loads(proc.stdout)
        except json.JSONDecodeError as exc:
            raise SystemExit(f"clawhub inspect returned invalid JSON: {exc}") from exc
    combined = f"{proc.stdout}\n{proc.stderr}"
    if "not found" in combined.lower() or "skill not found" in combined.lower():
        return None
    raise SystemExit(combined.strip() or f"clawhub inspect failed with {proc.returncode}")


def unreleased_changelog() -> str:
    text = CHANGELOG.read_text(encoding="utf-8")
    match = re.search(r"## Unreleased\n(?P<body>.*?)(?:\n## \d|\Z)", text, re.S)
    if not match:
        return "Prepare sifs-search skill package for ClawHub publishing."
    bullets = []
    current = None
    for line in match.group("body").splitlines():
        if line.startswith("- "):
            if current is not None:
                bullets.append(current)
            current = line[2:].strip()
            continue
        if current is not None and line.startswith("  "):
            current = f"{current} {line.strip()}"
            continue
        if current is not None:
            bullets.append(current)
            current = None
    if current is not None:
        bullets.append(current)

    keywords = ("sifs-search", "skill", "clawhub", "openclaw", "hermes")
    relevant = [
        bullet for bullet in bullets if any(keyword in bullet.lower() for keyword in keywords)
    ]
    return "\n".join(relevant) or "Prepare sifs-search skill package for ClawHub publishing."


def check_package() -> tuple[str, dict | None]:
    metadata = openclaw_metadata()
    for rel in (
        "SKILL.md",
        "references/commands.md",
        "references/mcp.md",
        "references/troubleshooting.md",
        "scripts/check-setup.sh",
    ):
        path = SKILL_DIR / rel
        if not path.exists():
            raise SystemExit(f"Missing package file: {path.relative_to(ROOT)}")
    env = os.environ.copy()
    local_bin = ROOT / "target" / "debug" / "sifs"
    if "SIFS_BIN" not in env and local_bin.exists():
        env["SIFS_BIN"] = str(local_bin)
    setup = subprocess.run(
        [str(SKILL_DIR / "scripts" / "check-setup.sh")],
        cwd=ROOT,
        env=env,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if setup.returncode != 0:
        raise SystemExit(setup.stderr.strip() or setup.stdout.strip() or "setup check failed")
    remote = inspect_remote()
    return metadata["version"], remote


def command_check() -> None:
    version, remote = check_package()
    if remote is None:
        print(f"{SLUG} is not published yet; local version {version} is ready.")
    else:
        remote_version = remote.get("version") or remote.get("metadata", {}).get("version")
        print(f"{SLUG} local version {version}; remote version {remote_version or 'unknown'}.")
    print("Changelog preview:")
    print(unreleased_changelog())


def command_publish() -> None:
    version, remote = check_package()
    remote_version = None
    if remote is not None:
        remote_version = remote.get("version") or remote.get("metadata", {}).get("version")
    if remote_version == version:
        print(f"{SLUG} {version} is already published.")
        return
    cmd = [
        "clawhub",
        "publish",
        str(SKILL_DIR),
        "--slug",
        SLUG,
        "--version",
        version,
        "--changelog",
        unreleased_changelog(),
    ]
    try:
        proc = run(cmd, check=False)
    except FileNotFoundError as exc:
        raise SystemExit("clawhub CLI is required for publish") from exc
    if proc.returncode != 0:
        raise SystemExit(proc.stderr.strip() or proc.stdout.strip() or "clawhub publish failed")
    print(proc.stdout.strip())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "publish"))
    args = parser.parse_args()
    if args.command == "check":
        command_check()
    else:
        command_publish()


if __name__ == "__main__":
    main()
