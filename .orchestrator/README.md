# Shared orchestration board

This directory is awl's tool-neutral execution board. Codex, Claude Code, and a
human collaborator all read and update the same files here.

- `queue.md` is the one source of truth for concrete work, dependencies, and status.
- `ROADMAP.md` remains the product-direction document; do not duplicate it here.
- Tool-specific paths may point here for compatibility, but must not carry a
  second writable copy of the queue.
- Preserve active entries and handoff reports when changing tools or worktrees.

**Layout**
- `queue.md` — the canonical execution queue. Siblings support it; never carry a second writable copy.
- `handoffs/` — self-contained round handoffs an agent can cook straight from (e.g. `handoffs/2026-07-14-live-world.md`).
- `reports/` — archived reports + superseded queues (`polish-queue.md`, the dated `*-REPORT` files), kept for reference, not active.

**Compat symlinks** so every tool's path resolves to this one dir: `.claude/orchestrator` and `.codex/orchestrator` both → `../.orchestrator`. `CLAUDE.md` and `AGENTS.md` point at `.orchestrator/queue.md`.
