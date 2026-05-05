# Agent Integration

SIFS is CLI-first for agents. The MCP server is useful when a client exposes it, but generated SIFS skills and snippets always include shell fallbacks so agents can continue when MCP is not visible in the current session.

## Recommended Path

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
| `codex` | `skill`, `snippet`, `mcp` | Skill default is `~/.codex/skills/sifs-search`; snippet default is `AGENTS.md`. |
| `claude-code` | `skill`, `snippet`, `mcp` | Skill default preserves `.claude/agents/sifs-search.md`; snippet default is `CLAUDE.md`. |
| `openclaw` | `skill`, `snippet` | Local artifact support only; public discovery is not claimed. |
| `hermes` | `skill`, `snippet` | Local artifact support only; public discovery is not claimed. |
| `generic` | `skill`, `snippet` | Portable agent-skill package and generic `AGENTS.md` snippet. |

Use `--target all` for best-effort multi-target checks or dry-runs. Results are reported per target; the operation is not transactional.

## Artifacts

`skill` renders or installs a `sifs-search` skill. Package targets include:

- `SKILL.md`
- `references/commands.md`
- `references/mcp.md`
- `references/troubleshooting.md`
- `scripts/check-setup.sh`

`snippet` inserts a short managed block into `AGENTS.md` or `CLAUDE.md`.

`mcp` prints guidance and redirects mutation to `sifs mcp install`; broad MCP config mutation stays with the existing MCP command family.

## Managed Snippets

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
- User-modified managed blocks require `--force`.
- Uninstall removes only the managed block.

## Doctor States

`sifs agent doctor --target <target> --json` reports a readiness matrix using:

- `pass`
- `fail`
- `unknown`

Checks include binary availability, skill/snippet presence, MCP config, MCP handshake guidance, search smoke guidance, current-session visibility, and CLI fallback readiness.

`unknown` is deliberate. A config file can exist even when the active agent session has no visible MCP tools, so doctor does not overclaim runtime visibility.

## Skill Package Publishing

The canonical portable skill lives at `skills/sifs-search/`. The ClawHub-ready
OpenClaw package lives at `extras/openclaw/sifs-search/` so it can be published
as a self-contained folder with:

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

The check command validates OpenClaw metadata, confirms the package files are
present, runs the bundled setup script, inspects the remote ClawHub slug when
`clawhub` is installed, and prints a changelog preview. It does not publish.

Publishing is intentionally manual:

```bash
clawhub auth login --token "$CLAWHUB_TOKEN" --no-browser
python3 scripts/clawhub_skill_sync.py publish
```

The GitHub Actions workflow `.github/workflows/clawhub-skill.yml` runs checks on
skill-package changes. It only publishes when manually dispatched with
`mode=publish` and a `CLAWHUB_TOKEN` secret is available.
