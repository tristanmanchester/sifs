#!/usr/bin/env python3
"""Validate release metadata that is easy to drift in CI."""

from __future__ import annotations

import argparse
import os
import re
import sys
import tomllib
from pathlib import Path


CHANGELOG_CATEGORIES = {
    "Added",
    "Changed",
    "Fixed",
    "Performance",
    "Security",
    "Removed",
    "Notes",
}


def read_package_version(root: Path) -> str:
    cargo_toml = root / "Cargo.toml"
    with cargo_toml.open("rb") as file:
        data = tomllib.load(file)
    return data["package"]["version"]


def changelog_sections(changelog: str) -> dict[str, str]:
    matches = list(re.finditer(r"^## (?P<title>.+)$", changelog, re.MULTILINE))
    sections: dict[str, str] = {}
    for index, match in enumerate(matches):
        start = match.end()
        end = matches[index + 1].start() if index + 1 < len(matches) else len(changelog)
        sections[match.group("title").strip()] = changelog[start:end]
    return sections


def validate_unreleased(changelog: str) -> list[str]:
    errors: list[str] = []
    sections = changelog_sections(changelog)
    unreleased = sections.get("Unreleased")
    if unreleased is None:
        return ["CHANGELOG.md must contain a '## Unreleased' section"]

    category_matches = list(
        re.finditer(r"^### (?P<category>.+)$", unreleased, re.MULTILINE)
    )
    for index, match in enumerate(category_matches):
        category = match.group("category").strip()
        if category not in CHANGELOG_CATEGORIES:
            errors.append(
                f"CHANGELOG.md uses unknown Unreleased category '{category}'"
            )
        start = match.end()
        end = (
            category_matches[index + 1].start()
            if index + 1 < len(category_matches)
            else len(unreleased)
        )
        body = unreleased[start:end].strip()
        if not re.search(r"^- ", body, re.MULTILINE):
            errors.append(f"CHANGELOG.md Unreleased category '{category}' is empty")

    return errors


def validate_tag(changelog: str, version: str, tag: str | None) -> list[str]:
    if not tag:
        return []

    expected = f"v{version}"
    if tag != expected:
        return [f"tag '{tag}' does not match Cargo.toml version '{expected}'"]

    section_pattern = rf"^## {re.escape(version)} - \d{{4}}-\d{{2}}-\d{{2}}$"
    if not re.search(section_pattern, changelog, re.MULTILINE):
        return [
            f"CHANGELOG.md must contain a dated '## {version} - YYYY-MM-DD' section"
        ]

    return []


def github_tag_from_env() -> str | None:
    if os.environ.get("GITHUB_REF_TYPE") == "tag":
        return os.environ.get("GITHUB_REF_NAME")
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument(
        "--tag",
        default=github_tag_from_env(),
        help="release tag to validate, defaults to GitHub tag context",
    )
    args = parser.parse_args()

    root = Path(args.root).resolve()
    version = read_package_version(root)
    changelog = (root / "CHANGELOG.md").read_text()

    errors = []
    errors.extend(validate_unreleased(changelog))
    errors.extend(validate_tag(changelog, version, args.tag))

    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1

    print("release metadata checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
