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

1. **Claim first, work second.** Edit the item's status line in `queue.md` to `🟡 IN PROGRESS — <owner> (codex|claude|human), <date>, branch <name>` and COMMIT that board edit to main before writing any code. An uncommitted claim is invisible to the other tool.
2. **Work in a worktree, never the main tree.** Branch off local `main`, name the branch on the claim line. The main working tree belongs to merge gates and the human's live session.
3. **Re-read the board before firing.** A claim may have landed since you last looked. `git pull`-equivalent for us is just re-reading `queue.md` at HEAD.
4. **Land = suite-gated merge to local main** (full `cargo test`, both conventions for keymap-adjacent work) + flip the board line to `✅ LANDED @ <sha>` in the same session. Push per the push policy (public repo — push after green trains).
5. **Conflicts are normal, not a coordination failure.** If two branches collide on merge, reconcile via a merge pass (Claude dispatches a merge agent; Codex resolves inline) — never serialize the whole queue out of fear.
6. **Stale claims:** an IN PROGRESS line older than ~a day with no branch activity may be reclaimed — note the takeover on the line.

## Board writes are ORCHESTRATOR-ONLY (added 2026-07-15, user rule)

Within each tool, the top-level ORCHESTRATOR session is the board's only
writer. Delegated subagents / workflow workers NEVER edit anything under
`.orchestrator/` — they return structured results, and the orchestrator
translates those into board edits:

- **Claims** are committed by the orchestrator BEFORE dispatching build work
  (claim-first still holds; it just isn't delegated).
- **Status flips** (`✅ LANDED @ sha`, defect notes, morning-review entries)
  happen when the orchestrator processes the workers' results — a worker only
  knows its own slice, so letting it flip status invites premature or
  wrong-altitude entries; the orchestrator holds the cross-workstream truth.
- **Why:** one writer serializes the shared file — no same-file races between
  concurrent workers and the live session (the exact race dodged 2026-07-15:
  a spec log held back because two merge-train agents had queue.md edits in
  flight); and the board keeps one consistent voice and altitude.
- **Corollary:** board edits happen only in the MAIN working tree, never in a
  worktree — a worktree's `queue.md` edit dumps a guaranteed conflict on the
  merge train.
- Worker briefs must therefore NOT include "edit the board / flip the claim"
  steps; they report shas + outcomes instead.

## Design sessions → decisions → the board (added 2026-07-22, user rule)

How a brainstorm/interview session ("awl design"-type) turns talk into work:

1. **Brainstorm read-only.** During discussion the orchestrator changes nothing.
   **Interview ruthlessly (user rule 2026-07-23):** when a note or an answer is
   ambiguous, the designer asks until the intent is unambiguous — a guess never
   gets built into a queue item.
2. **Decisions land as queue items.** Each crystallized decision becomes a
   self-contained item (or a `DECIDED <date>` line folded into an existing
   item) — a worker must receive the decided thing, never the open question.
3. **The commit message is the session record.** Board-decision edits are
   committed to main with a subject starting `orchestrator: decisions` —
   `git log --grep=decisions` replays every design session in order. There is
   deliberately NO decisions log file: append-only logs rot into noise, and
   git already keeps the full history (the CLAUDE.md philosophy).
4. **Retention tiers (existing law, restated):** build decisions live in queue
   items → git history when archived. A standing constraint a future agent
   would re-litigate ("no locale sniffing") gets ONE line in CLAUDE.md's
   "Open decisions & known divergences". Taste/product-level decisions amend
   PHILOSOPHY/DESIGN/SCOPE/THEMES in the landing round.
5. **The user's notes (private, outside the repo) are the user's space.**
   Agents READ questionnaires and notes there when directed; they NEVER write
   there. The machine-side record lives in this repo.
