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

## Claiming protocol (multi-tool coordination, added 2026-07-15)

The board only prevents double-work if claims are visible BEFORE work starts. Any tool (Codex, Claude Code, human) picking up an item:

1. **Claim first, work second.** Edit the item's status line in `queue.md` to `🟡 IN PROGRESS — <owner> (codex|claude|frank), <date>, branch <name>` and COMMIT that board edit to main before writing any code. An uncommitted claim is invisible to the other tool.
2. **Work in a worktree, never the main tree.** Branch off local `main`, name the branch on the claim line. The main working tree belongs to merge gates and the human's live session.
3. **Re-read the board before firing.** A claim may have landed since you last looked. `git pull`-equivalent for us is just re-reading `queue.md` at HEAD.
4. **Land = suite-gated merge to local main** (full `cargo test`, both conventions for keymap-adjacent work) + flip the board line to `✅ LANDED @ <sha>` in the same session. Push per the push policy (public repo — push after green trains).
5. **Conflicts are normal, not a coordination failure.** If two branches collide on merge, reconcile via a merge pass (Claude dispatches a merge agent; Codex resolves inline) — never serialize the whole queue out of fear.
6. **Stale claims:** an IN PROGRESS line older than ~a day with no branch activity may be reclaimed — note the takeover on the line.
