# Review: Codex data-driven keymap dispatch (2026-07-16, claude adversarial review)

## Verdict: sound — no confirmed majors. Three small pre-merge fixes recommended.

**The circularity question — REFUTED empirically.** The exhaustive agreement tests
(catalog_and_keymap_agree_on_every_default_chord + the both-conventions dispatch law)
DID become round-trip tautologies on the chord-VALUE axis (dispatch and expectation now
read one parse). BUT the pre-existing hand-written literal snapshots
(mac_convention_is_byte_identical_to_the_pre_round_table @ keymap.rs:2463,
linux_collision_table_matches_the_documented_displaced_list @ :2506) are TOML-immutable
oracles. PROOF: editing save=["Cmd-J",""] in the TOML fails 8 keymap tests + 2 label
tests. A silent chord rewrite IS caught. The exhaustive tests still genuinely cross-check
the hand-written Linux POLICY layer (displacement/collision/keep).

**Behavior preservation: green.** Full suite 2205/0 both conventions on the branch base.
Tripwires verified via real --keys replay: C-k kill-line on Linux both flavors;
[keys] insert_link=C-k reclaims over the keep-floor; prefix sequences fire; Cmd-Z/Cmd-Shift-Z
split survives shift-fill; unknown [keys] action ignored with note. Fail-fast on malformed
embedded TOML is STRENGTHENED and #[should_panic]-tested.

## Pre-merge fixes (all small)
1. **TOML header comment is now FALSE (real doc/behavior drift):** assets/keymap-defaults.toml
   still says an absent command is "unbound out of the box" — the new code PANICS at startup
   on an absent command (proven live). Update the header.
2. **Re-scope the exhaustive tests' doc comments** — they overclaim ("pins the two together")
   now that the value axis is tautological; point readers at the literal snapshots as the
   value oracle.
3. **Optional hardening (recommended):** ONE frozen slug→resolved-chord-string snapshot table,
   regenerated deliberately — restores structural per-command value pinning so a NEW command's
   typo'd chord can't slip past the (now manual) literal tests.

## Notes
- hermetic_canary flaked once under a full parallel Linux-convention run, passes alone —
  a DIFFERENT flake mode than the fixed fontconfig one (concurrent test writes into the
  canary home); fold into the flake chore, unrelated to this patch.
- shift-fill/super-variant seeding creates some dead map entries — harmless, or_insert
  protects explicit entries (redo split verified live).
- Integration: base 928f830 is fresh; input-core files untouched by the in-flight rounds.
