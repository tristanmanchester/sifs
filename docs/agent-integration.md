# Agent Integration

SIFS is CLI-first for agents. The MCP server adds value when a client exposes
it, and generated skills and snippets always include shell fallbacks so agents
keep working when MCP isn't visible in the current session.

## Recommended path

Install an instruction snippet into a project:

```bash
sifs agent print --target codex --artifact snippet
sifs agent install --target codex --artifact snippet --file AGENTS.md --dry-run --json
sifs agent install --target codex --artifact snippet --file AGENTS.md
sifs agent doctor --target codex --json
```

Install a local skill package:

```bash
sifs agent print --target generic --artifact skill
sifs agent install --target codex --artifact skill --dry-run --json
sifs agent install --target openclaw --artifact skill --destination ~/.agents/skills/sifs-search
```

Use MCP as an optional extra:

```bash
sifs mcp install --client codex --dry-run --json
sifs mcp doctor --source /path/to/project --offline --no-cache --json
```

## Targets

| Target | Artifacts | Notes |
| --- | --- | --- |
| `codex` | `skill`, `snippet`, `mcp` | Skill default: `~/.codex/skills/sifs-search`. Snippet default: `AGENTS.md`. |
| `claude-code` | `skill`, `snippet`, `mcp` | Skill default: `.claude/agents/sifs-search.md`. Snippet default: `CLAUDE.md`. |
| `openclaw` | `skill`, `snippet` | Local artifact files only. |
| `hermes` | `skill`, `snippet` | Local artifact files only. |
| `generic` | `skill`, `snippet` | Portable skill package and generic `AGENTS.md` snippet. |

Use `--target all` for best-effort multi-target checks or dry-runs. Each
target is processed independently; the operation isn't transactional.

## Artifacts

`skill` renders or installs a `sifs-search` skill. Package targets include:

- `SKILL.md`
- `references/commands.md`
- `references/mcp.md`
- `references/troubleshooting.md`
- `scripts/check-setup.sh`

`snippet` inserts a short managed block into `AGENTS.md` or `CLAUDE.md`.

`mcp` prints guidance and redirects mutation to `sifs mcp install`. MCP config
changes happen through that command family, not `sifs agent`.

## Managed snippets

Snippet installs use stable markers:

```markdown
<!-- BEGIN SIFS AGENT INSTRUCTIONS schema=1 checksum=... -->
...
<!-- END SIFS AGENT INSTRUCTIONS -->
```

Rules:

- Existing user content outside the block is preserved.
- Re-running the same install is a no-op.
- Stale generated blocks are updated in place.
- User-modified managed blocks and skill directories missing `SKILL.md` require `--force`.
- Uninstall removes only the managed block.

## Doctor states

`sifs agent doctor --target <target> --json` reports a readiness matrix with
three states:

- `pass`
- `fail`
- `unknown`

Checks cover binary availability, skill and snippet presence, MCP config, MCP
handshake guidance, search smoke guidance, current-session visibility, and CLI
fallback readiness.

`unknown` covers signals doctor can't verify from outside the agent session.
A configured MCP server doesn't guarantee that the running agent sees the
tools, so doctor reports `unknown` rather than `pass` for runtime visibility.

## Skill package publishing

The canonical portable skill lives at `skills/sifs-search/`. The ClawHub-ready
OpenClaw package at `extras/openclaw/sifs-search/` is a self-contained folder
with:

- `SKILL.md`
- `references/commands.md`
- `references/mcp.md`
- `references/troubleshooting.md`
- `scripts/check-setup.sh`

Before publishing, run the local readiness checks:

```bash
cargo test --locked --test skill_parity
python3 scripts/clawhub_skill_sync.py check
```

The `check` command validates OpenClaw metadata, confirms the package files
are present, runs the bundled setup script, inspects the remote ClawHub slug
when `clawhub` is installed, and prints a changelog preview. It doesn't
publish.

Publishing is a manual step:

```bash
clawhub auth login --token "$CLAWHUB_TOKEN" --no-browser
python3 scripts/clawhub_skill_sync.py publish
```

The GitHub Actions workflow `.github/workflows/clawhub-skill.yml` runs checks
on skill-package changes. It publishes only when manually dispatched with
`mode=publish` and a `CLAWHUB_TOKEN` secret available.
