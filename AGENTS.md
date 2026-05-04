# AGENTS.md

This file contains local project instructions for AI agents working in this
checkout.

## Changelog Discipline

`CHANGELOG.md` is the source of truth for release notes.

- For every user-facing code, behavior, documentation, packaging, installer,
  workflow, or compatibility change, update `CHANGELOG.md` in the same turn.
- Add new work under `## Unreleased` first. Use the existing categories:
  `Added`, `Changed`, `Fixed`, `Performance`, `Security`, `Removed`, and
  `Notes`.
- Do not add empty categories. Create a category only when it has at least one
  bullet.
- Keep entries user-facing and specific. Avoid commit-log noise such as
  "bump version," "prepare release," or "apply formatting," unless that is the
  only release-relevant change.
- When preparing a release, move the relevant `Unreleased` entries into a
  `## X.Y.Z - YYYY-MM-DD` section, leave a fresh `Unreleased` section at the
  top, and verify the version matches `Cargo.toml` and the `vX.Y.Z` tag.
- Before tagging a release, confirm that `CHANGELOG.md` includes the version
  section that GitHub release notes will use.
