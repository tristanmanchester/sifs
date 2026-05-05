---
date: 2026-05-05
topic: agent-native-integrations
focus: OpenClaw/Hermes agent plugins, agent skills, Codex/Claude Code plugins, and MCP
mode: repo-grounded
---

# Ideation: Agent-Native Integrations

> Status: implemented/superseded by `sifs agent print|install|doctor|uninstall`
> and the CLI-first agent integration docs. This document is retained as the
> original ideation record; statements about the "current" surface describe the
> repository before the agent artifact implementation.

## Grounding Context

SIFS is already a Rust CLI, crate, daemon, and stdio MCP server for local and Git
code search. Its current agent-facing surfaces include `sifs agent-context
--json`, MCP `agent_context`, `tools/list`, `resources/list`, generated
`.claude/agents/sifs-search.md` guidance, profiles, feedback, `sifs mcp
install`, and `sifs mcp doctor`.

The strongest local evidence is that SIFS is already good at the first layer of
agent nativeness: non-interactive CLI behavior, structured JSON, strict
`source` / `limit` vocabulary, bounded results, dry-run/force mutation
boundaries, and MCP tools for search, related code, index status, file listing,
chunk fetch, profiles, feedback, and agent init. The gap is the second layer:
native packaging and visibility inside each agent runtime.

Several repo findings shaped the ranking:

- `src/agent_context.rs` is a useful machine-readable contract, but it does not
  currently advertise every MCP resource documented in `docs/mcp.md` and
  exposed by `src/mcp.rs`, including `sifs://index/status` and
  `sifs://index/files`.
- `src/agents/sifs-search.md` is still Claude-agent-shaped. It is conservative
  and useful, but SIFS does not yet generate target-native Codex, OpenClaw, or
  Hermes artifacts.
- `sifs mcp doctor` already separates stdio handshake readiness from BM25 search
  smoke. That separation should become a broader agent-surface diagnostic,
  because a configured MCP server is not the same as a visible live tool.
- The repo’s own `docs/agent-native-scorecard.md` is static. The next quality
  step is making agent-readiness testable across install, discovery, invocation,
  result schema, and task completion.

External research converged on the same architecture. Claude Code plugins package
skills, agents, hooks, and MCP servers; OpenClaw separates typed tools, skills
that teach when/how, and plugins that package capabilities; Codex and OpenAI MCP
guidance emphasize pairing MCP configuration with agent instructions; MCP
guidance emphasizes small toolsets, progressive discovery, JSON-schema contracts,
and machine-recoverable errors.

Sources used for external grounding:

- Claude Code plugin reference: <https://code.claude.com/docs/en/plugins-reference>
- OpenClaw tools/plugins docs: <https://docs.openclaw.ai/tools>
- MCP server concepts: <https://modelcontextprotocol.io/docs/learn/server-concepts>
- MCP client best practices: <https://modelcontextprotocol.io/docs/develop/clients/client-best-practices>
- MCP tool-description research: <https://arxiv.org/abs/2602.14878>
- MCP production-pattern research: <https://arxiv.org/abs/2603.13417>
- Hermes skills/MCP secondary guide: <https://openclawlaunch.com/guides/hermes-agent-skills>

## Ranked Ideas

### 1. Agent Surface Doctor Matrix

**Description:** Add `sifs agent doctor --target codex|claude-code|openclaw|hermes|all --json` to verify the full path from installed binary to visible agent capability. It should report binary path, version, daemon status, MCP config presence, stdio handshake, `tools/list`, BM25 smoke, generated skill/plugin presence, known host-specific restart requirements, and exact next actions. This should explicitly distinguish `configured`, `handshake_ok`, `search_ok`, and `visible_to_current_session`.

**Warrant:** `direct:` SIFS already has `sifs mcp doctor`, daemon status/install flows, and local memory from Codex testing showed that an enabled `[mcp_servers.sifs]` entry did not necessarily expose a live SIFS tool namespace.

**Rationale:** The highest-friction agent-native failure is not ranking quality; it is agents believing SIFS is available when the runtime cannot actually see or invoke it. A surface-aware doctor lets agents self-correct: use MCP when visible, use CLI fallback when not, and tell the user exactly when a true restart or config repair is needed.

**Downsides:** Some hosts may not expose a reliable way for an external binary to prove live in-session visibility. The command should model those states as `unknown` rather than over-claiming.

**Confidence:** 95%

**Complexity:** Medium

**Status:** Unexplored

### 2. Contract-Driven Agent Pack Exporter

**Description:** Add `sifs agent export --target codex|claude-code|openclaw|hermes|generic|all` to emit native integration artifacts from the same underlying SIFS contract. Outputs could include Codex `AGENTS.md` snippets and MCP TOML, a Claude Code plugin folder with `.mcp.json` plus `skills/sifs-search/SKILL.md`, OpenClaw skill/plugin metadata, Hermes `config.yaml` MCP snippets, and generic MCP instructions. The exporter should support `--dry-run --json`, destination selection, and force-gated writes.

**Warrant:** `direct:` SIFS already embeds `src/agents/sifs-search.md`, MCP instructions, tool descriptions, `agent-context --json`, and `sifs mcp install`; external Claude Code and OpenClaw docs treat plugins/skills as the native packaging layer for reusable capability.

**Rationale:** SIFS should not require every agent ecosystem to reverse-engineer README snippets. A contract-driven exporter makes one canonical vocabulary propagate into all host-native surfaces and reduces drift when CLI flags, MCP tools, or recovery guidance change.

**Downsides:** Exact OpenClaw/Hermes public-discovery formats may be unstable or secondary-source-documented. Start with generated local skill/config artifacts before claiming marketplace support.

**Confidence:** 92%

**Complexity:** Medium

**Status:** Unexplored

### 3. MCP Capability Graph And Context Declaration

**Description:** Extend `agent_context` and MCP resources with a versioned capability graph: workflows, not just tool names. It should describe sequences such as `search -> get_chunk -> find_related -> feedback_create`, when to choose BM25/semantic/hybrid, fallback behavior, mutation boundaries, output schemas, timeout expectations, and recovery actions. Every MCP response should include a compact `context_declaration` with source identity, index freshness, filter scope, cache/offline/model status, truncation state, and warnings.

**Warrant:** `direct:` SIFS already exposes `agent_context`, structured search result metadata, warnings, stats, MCP resources, and generated prompt-native guidance. External MCP guidance distinguishes tools/resources/prompts and emphasizes discoverable schemas and progressive tool loading.

**Rationale:** Better MCP is not simply adding more tools. Agents need to understand the operating envelope of the tool they already have: whether results are fresh, partial, filtered, BM25-only, or semantic-enabled, and what follow-up call is appropriate.

**Downsides:** More metadata can bloat responses if not budgeted carefully. Keep the declaration compact, stable, and suppressible for humans.

**Confidence:** 90%

**Complexity:** Medium

**Status:** Unexplored

### 4. Task-Shaped Context Packs

**Description:** Add a context-pack API that accepts a task, intent, and budget, then returns a bounded bundle of likely files, chunks, tests, definitions, related code, confidence notes, and next actions. CLI and MCP shapes could be `sifs context-pack "fix MCP install"` and MCP `get_context_for_task`, with options like `intent=bugfix|review|onboarding|docs`, `budget_tokens`, `max_files`, `include_tests`, and `source`.

**Warrant:** `reasoned:` Agents usually do not want a ranked list as the final product; they want enough grounded context to safely edit, review, or explain code. SIFS already has the primitives needed to compose this: search, list files, get chunk, find related, profiles, index status, and result metadata.

**Rationale:** This is the strongest product-level reframing. It moves SIFS from “fast search” to “local context assembly for agents” without abandoning the current engine.

**Downsides:** It risks becoming an LLM-like summarizer if scoped poorly. The first version should be deterministic retrieval composition, not generated narrative.

**Confidence:** 84%

**Complexity:** High

**Status:** Unexplored

### 5. Search Session Memory And Handoff Artifacts

**Description:** Introduce local search sessions that capture source, profile, query history, filters, result IDs, selected/opened chunks, misses, feedback, and final evidence. Expose `session_start`, `session_note`, `session_summary`, and `session_export` through CLI and MCP, plus `sifs handoff create/resume` for cross-agent handoffs. Keep it local-first and redaction/export aware.

**Warrant:** `direct:` SIFS already has profiles and local feedback, but those are static defaults and raw friction records rather than task-level investigation state.

**Rationale:** Multi-agent workflows lose value at compaction, restart, and handoff boundaries. A local session ledger lets Codex, Claude Code, OpenClaw, and Hermes share the search trail without redoing the first 10 minutes of discovery.

**Downsides:** This creates a new persistent data model and privacy surface. It should be opt-in, inspectable, redactable, and clearly stored outside the repo unless requested.

**Confidence:** 78%

**Complexity:** High

**Status:** Unexplored

### 6. Agent-Native MCP Eval Harness

**Description:** Turn `docs/agent-native-scorecard.md` into a regression target. Build an eval harness that checks whether real or simulated agent clients can discover SIFS, inspect the contract, choose the right search mode, retrieve target files, respect budgets, handle no-results recovery, and use CLI fallback when MCP is unavailable. Run it across CLI, MCP stdio, generated Claude/Codex/OpenClaw/Hermes artifacts, and packaged plugin surfaces where available.

**Warrant:** `direct:` The repo already maintains an agent-native scorecard and benchmark tooling, but current verification focuses on CLI/MCP commands rather than end-to-end agent discoverability and task success.

**Rationale:** If “agent-native” is the product promise, regressions should include discoverability, setup, tool choice, result schema quality, and recovery behavior. This also guards against generated skill/plugin drift.

**Downsides:** Full real-agent tests are expensive and flaky. Start with deterministic contract/smoke tests, then add optional live-host checks.

**Confidence:** 80%

**Complexity:** Medium-High

**Status:** Unexplored

### 7. Structured Feedback To Profiles

**Description:** Upgrade feedback from free-text friction logs to structured local events: `bad_result`, `missing_file`, `tool_confusion`, `timeout`, `install_failure`, `schema_gap`, `wrong_guidance`, `opened_result`, `edited_after_result`, and `validation_passed`. Add `sifs feedback summary --json` and `sifs profile suggest --from-feedback --dry-run --json` so agents can propose repo-specific ranking/profile changes without silently mutating defaults.

**Warrant:** `direct:` SIFS already has local-first feedback, profiles, and strict mutation boundaries. External MCP production research highlights observability and structured recovery as real gaps in deployed agent tooling.

**Rationale:** Feedback compounds only if it becomes a usable signal. This gives SIFS a privacy-preserving path toward repo-specific relevance and better generated guidance across repeated agent sessions.

**Downsides:** Implicit event collection can feel creepy if automatic. The initial version should require explicit agent calls or generated skill hooks that are easy to inspect and disable.

**Confidence:** 74%

**Complexity:** Medium

**Status:** Unexplored

## Rejection Summary

| # | Idea | Reason Rejected |
|---|------|-----------------|
| 1 | Publish streamable HTTP MCP immediately | Interesting but premature for a local code-search tool; stdio is the right default until multi-user/hosted identity is real. |
| 2 | Split MCP into many specialized tools like `semantic_search`, `bm25_search`, and `test_search` | Tool-bloat risk; stronger idea is a capability graph plus a small set of task-shaped recipes. |
| 3 | Fully automatic ranking autopilot from edited files | Valuable but too implicit for a first pass; structured feedback-to-profiles is safer and more inspectable. |
| 4 | Live workspace index broker for multi-agent sessions | Strong but heavier than the current need; should follow agent doctor and context declaration/freshness work. |
| 5 | `llms.txt` / `AGENTS.md` snippets as the main roadmap | Useful tactical work, but better as part of the agent pack exporter rather than a standalone strategic direction. |
| 6 | Repo search router across all local workspaces | Potentially useful, but source ambiguity and privacy boundaries make it less urgent than making per-repo integration reliable. |
| 7 | Hosted/upstream feedback delivery | Not aligned with SIFS' local-first privacy posture; local structured feedback is enough for now. |
| 8 | Prompt-only OpenClaw/Hermes guidance | Too weak; native local skill/config artifacts are a better fit for those ecosystems. |

## Suggested Next Brainstorm

The best seed for `ce-brainstorm` is **Contract-Driven Agent Pack Exporter**.
It is high-leverage, grounded in the current repo, and can absorb smaller wins:
Codex snippets, Claude Code plugin packaging, OpenClaw/Hermes local skills,
doctor integration, and contract tests.
