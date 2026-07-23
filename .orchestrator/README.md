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

## Cooking: parallelize by clash, run unattended (user rule, 2026-07-23)

When the orchestrator is cooking a queue, the default is throughput, not caution:

1. **Parallelize by file-clash, not by fear.** Items whose file/module
   footprints are DISJOINT cook CONCURRENTLY — a few at a time (~3–4; enough to
   fill the machine without thrashing cargo), not one-at-a-time. Assess each
   item's footprint up front; only genuine same-file clashes are sequenced.
   Parallel builds run in ISOLATED worktrees (never the main tree), so
   concurrent builds can't clobber each other or a live session.
2. **Integration stays serial and gated.** Worktree branches merge to main ONE
   at a time through the suite gate (full `cargo test`, both conventions for
   keymap-adjacent work, wasm on the train — the existing merge train). A clash
   on merge is reconciled by a delegated MERGE agent, never by serializing the
   whole queue out of merge-fear and never by the orchestrator hand-editing
   conflict markers. On any red at integration, reset main clean and skip-flag
   the item (below) — main is never left broken.
3. **Aim to cook unattended; never idle with work queued.** While independent
   items remain, something is always cooking. A stuck item — can't reach green,
   or hits an ambiguity that would need a user decision — is REVERTED clean,
   left out of main, and FLAGGED for the user; it never blocks the rest. Only
   genuinely user-gated items (a permission grant, an approval, a taste call the
   user reserved) wait; everything else proceeds. "If you get stuck, do
   everything else before pausing to wait for my say" (user, 2026-07-23).

## Execution hygiene

These are orchestration rules, not live queue state:

- **Durable worktrees only.** Never create a worktree under `/private/tmp`;
  use `.claude/worktrees/` or another durable path. Commit work in progress
  before any pause.
- **Clean up after every landed wave.** Once a worktree is clean and its patch
  is merged (or patch-equivalent on main), remove the worktree and prune stale
  registrations. Leave dirty, unmerged, locked, or differently-owned
  worktrees alone unless their owner explicitly hands them back.
- **Preserve gate truth.** Never pipe a build/test gate through `head`, `tail`,
  or anything else that can hide its exit status. Run the wasm gate on every
  train as required by `AGENTS.md`.
- **Treat suspicious incremental failures as suspect first.** Retry with
  `CARGO_INCREMENTAL=0` before diagnosing product code.
- **Terminate only owned processes.** Never kill `awl` by a bare process name;
  stop only the exact PID created by the current run.
- Background model/effort routing follows the Brew skill and any narrower
  repository rule; record any launcher substitution.

## Blocked items PARK; never stall the queue on them (user rule, 2026-07-23)

When an item hits a blocker only the user can clear — a taste/product fork, a
permission grant, an approval — the orchestrator NOTES it blocked on the board
(a one-line status naming exactly what's needed) and IMMEDIATELY moves on:
every non-blocked item keeps cooking in parallel. The blocker note IS the
channel: WRITE the decision straight into the queue item — the specific fork,
the options, and your recommendation — mark it blocked, and move on. The user
resolves it by EDITING THE QUEUE on their own time (exactly how items 48–52 and
the frost rework landed). Do NOT reach for an interactive prompt that stalls the
turn for a decision that could just be a queue note, and do NOT idle other work
waiting on the answer. The blocked item resumes the moment the user's queue edit
lands. (User's word: "you can just write down the blocking decision in the queue
and move on"; "just note that it's blocked and churn through everything else.")
The failure this kills: asking a question and then idling the whole queue until
it's answered.
