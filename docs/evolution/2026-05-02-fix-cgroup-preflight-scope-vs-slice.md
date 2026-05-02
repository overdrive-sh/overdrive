# fix-cgroup-preflight-scope-vs-slice — Feature Evolution

**Feature ID**: fix-cgroup-preflight-scope-vs-slice
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-05-02
**Commits**:
- `8011950` — `test(control-plane): RED — preflight oracle for empty-scope parent-slice fallback`
- `3bcf243` — `fix(control-plane): cgroup preflight falls back to parent slice on empty scope subtree_control`

**Status**: Delivered.

---

## Symptom

An interactive-shell `overdrive serve` invocation (TTY, not a systemd unit) returns `DelegationMissing` and refuses to start, even when the operator's `user-1000.slice/` is correctly delegated via `sudo systemctl set-property user-1000.slice Delegate=yes` and exposes both `cpu` and `memory` in its `cgroup.subtree_control`. The misdiagnosis prescribes the very `Delegate=yes` configuration the operator already has, leaving no actionable remediation. `cgroup_preflight::run_preflight_at` reads `/proc/self/cgroup` to discover the enclosing path, but for a non-systemd-unit invocation that path resolves to a *scope* (e.g. `session-3.scope`) whose `subtree_control` is empty under cgroup v2's "no internal processes" rule — controllers live on the parent slice, not on leaf scopes containing processes. The check inspects the empty file at the scope, sees no `cpu` / `memory` listed, and returns the wrong error.

## Root cause (3 compounding causes)

### A. ADR-prescribed empty-fallback was never implemented

ADR-0028 §4 step 4 explicitly specifies "Read `<that_path>/cgroup.subtree_control` (or the parent's if the file is empty)". The prior fix (`docs/evolution/2026-04-29-fix-cgroup-preflight-wrong-slice.md`) implemented the `/proc/self/cgroup` discovery and the `cgroup_root.join(...)` shape but omitted the parenthesised fallback clause. `cgroup_preflight.rs:330-345` (pre-fix) called `read_to_string(&subtree_control)?` once with no consideration of the empty-leaf-scope semantics that cgroup v2 mandates. The doc-comment said "the *enclosing* slice" but the code treated every `/proc/self/cgroup` tail uniformly, regardless of whether it terminated at a slice or a scope.

### B. Mono-shape test fixtures hid both production realities

Every step-4 test fixture pointed `/proc/self/cgroup` at a *slice* path (`user.slice/user-1000.slice`) and wrote `cgroup.subtree_control` directly under that path — a shape congruent with the partially-implemented code. Production has at least two distinct shapes: systemd-unit (slice == discovered path, controllers in `subtree_control`) and interactive shell (scope == discovered path, empty `subtree_control`, controllers in parent). The mono-shape testing satisfied the prior fix's oracle and missed both production realities. The single new oracle (`preflight_reads_enclosing_slice.rs`) was scoped to root-vs-enclosing distinction, not scope-vs-slice — the same Root Cause B shape the prior fix's RCA had named four days earlier.

### C. ADR-implementation drift recurred at the same review boundary

The prior fix's RCA flagged "ADR-implementation drift was not caught at review" as Root Cause C and tagged "ADR↔code traceability tooling" as out-of-scope follow-up. That follow-up was not actioned, and the same class of drift produced the same class of bug — against the same ADR clause, the very next sentence — within four days. Reading an ADR step as one line of prose continues to mask omitted sub-clauses when the omitted code is small (a one-line conditional re-read).

## Fix

**Approved fix shape**: **Option A** — implement the ADR-prescribed fallback verbatim. Read `<discovered>/cgroup.subtree_control`; if its contents contain zero non-whitespace tokens, re-read `<discovered>.parent()/cgroup.subtree_control` and apply the cpu/memory check against the parent. **Rejected: Option B** (always read parent) — fails the systemd-unit case, where the unit's own `subtree_control` is authoritative and the parent slice typically does NOT enable the same controllers; trades one wrong answer for another and diverges from the ADR.

The change in `crates/overdrive-control-plane/src/cgroup_preflight.rs` replaces the single `read_to_string(&subtree_control)?` (lines 343-345 pre-fix) with: read primary; if `split_ascii_whitespace().next().is_none()`, compute `enclosing_abs.parent()`, re-read `<parent>/cgroup.subtree_control`, and update the `slice` value used for any eventual `DelegationMissing` to point at the parent (so the rendered remediation `Delegate=yes` against the right slice is accurate). Edge case: if `parent()` returns `None` (the `/proc/self/cgroup` tail parsed to `/`, i.e. the kernel-root cgroup), surface `SubtreeControlUnreadable` rather than panicking — the kernel-root is structurally an unexpected enclosing path here. Module-top and `run_preflight_at` rustdoc updated to name the fallback; `DelegationMissing.slice` field doc clarified to "the slice whose `subtree_control` was inspected".

## Tests added / updated

- **NEW**: `tests/integration/cgroup_isolation/preflight_falls_back_to_parent_slice_on_empty_scope.rs` — the RED oracle. Seeds `<tmp>/user.slice/user-1000.slice/cgroup.subtree_control` = `cpu memory io`, `<tmp>/user.slice/user-1000.slice/session-3.scope/cgroup.subtree_control` = empty, and `<tmp>/proc-self-cgroup` = `0::/user.slice/user-1000.slice/session-3.scope\n`. Asserts `Ok(())` — fallback finds delegation in parent. FAILS under pre-fix code with `DelegationMissing`; PASSES under fix.
- **NEW**: `tests/integration/cgroup_isolation/preflight_refuses_when_both_scope_and_parent_slice_lack_delegation.rs` — locking test. Scope empty; parent slice carries `io pids` only. Asserts `DelegationMissing` with `slice` = `user.slice/user-1000.slice`. Confirms fallback does NOT silently mask missing delegation.
- **No existing fixture needed updating** — all 4 prior step-4 tests point `/proc/self/cgroup` at a slice with non-empty `subtree_control`, exercising the "no fallback needed" path. They continue to gate the primary read; the new oracles gate the fallback.

## Quality gates

- **Workspace nextest** on Linux via Lima (`cargo xtask lima run -- cargo nextest run --workspace --features integration-tests`): full suite green.
- **DES integrity** — `verify_deliver_integrity` exit 0; all 10 phase entries EXECUTED, 2 SKIPPED with valid `NOT_APPLICABLE` reasons.
- **Mutation gate** — **SKIPPED per user instruction during `/nw-bugfix` Phase 3.** This delivery did not run `cargo xtask mutants` against the patched file. The prior wrong-slice fix landed at 93.8% kill rate on the same file; the new conditional adds a single boolean branch (`is_empty()`) which the new oracle exercises directly, so structural coverage is plausible — but no mutation evidence was collected for this delivery.

## Out of scope (flagged for follow-up)

- **ADR↔code traceability tooling.** Carried forward from the prior fix's evolution doc with **elevated urgency**: two ADR-implementation drift incidents within four days, both against the same ADR-0028 §4 step 4 clause, both producing operator-facing misdiagnoses. The class of bug (a small omitted sub-clause inside a one-paragraph ADR step that no automated check links back to code) will continue to recur until traceability is mechanised. Process improvement; out of scope for this PR but increasingly load-bearing.

## References

- RCA: `docs/feature/fix-cgroup-preflight-scope-vs-slice/bugfix-rca.md`
- ADR: `docs/product/architecture/adr-0028-cgroup-preflight-refusal.md` §4 step 4 (the "or the parent's if the file is empty" clause)
- Prior fix: `docs/evolution/2026-04-29-fix-cgroup-preflight-wrong-slice.md`
- Commits: `8011950` (RED), `3bcf243` (GREEN)
- Test discipline: `.claude/rules/testing.md` §RED scaffolds, §Lima rule for `--features integration-tests`
