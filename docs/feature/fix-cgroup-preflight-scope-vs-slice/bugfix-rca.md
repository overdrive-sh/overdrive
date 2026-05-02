# RCA — cgroup preflight reads scope's empty `subtree_control` instead of parent slice

**Feature ID**: `fix-cgroup-preflight-scope-vs-slice`
**Type**: Bug fix (`/nw-bugfix`)
**Date**: 2026-05-02
**Author**: Rex (nw-troubleshooter)
**Related**:
- ADR-0028 §4 step 4 (`docs/product/architecture/adr-0028-cgroup-preflight-refusal.md:172-177`)
- Prior fix: `docs/evolution/2026-04-29-fix-cgroup-preflight-wrong-slice.md`
- Implementation: `crates/overdrive-control-plane/src/cgroup_preflight.rs:308-345`

---

## Problem statement

`run_preflight_at` joins the path returned by `parse_cgroup_v2_path(/proc/self/cgroup)`
to `cgroup_root` and reads `<that>/cgroup.subtree_control` as if it were authoritative
for delegation. For a non-systemd-unit invocation (interactive shell:
`overdrive serve` from a TTY), `/proc/self/cgroup` resolves to a *scope* (e.g.
`session-3.scope`) whose `subtree_control` is empty under cgroup v2's "no internal
processes" rule. The check returns `DelegationMissing` despite `user-1000.slice/`
having both `cpu` and `memory` correctly delegated via `Delegate=yes`.

The misdiagnosis prescribes `sudo systemctl set-property user-1000.slice
Delegate=yes` — which the operator already has. The actual fix (none required) is
hidden by an error that contradicts reality.

---

## 5 Whys chain

```
WHY 1  Preflight returns DelegationMissing for an operator whose user slice IS
       delegated.
       Evidence: cgroup_preflight.rs:330-359 — code reads
       `<enclosing_abs>/cgroup.subtree_control` where `enclosing_abs` is the
       scope path; missing-controller branch fires when the file is empty.

  WHY 2  The scope's `cgroup.subtree_control` is empty even though the parent
         slice's is correct.
         Evidence: cgroup-v2.rst "no internal processes" rule — a non-root
         cgroup containing processes cannot have child cgroups with controllers,
         so leaf scopes do not enable controllers in `subtree_control`. systemd
         writes the delegated controller set to the parent slice
         (`user-1000.slice/cgroup.subtree_control`), not to per-session scopes.

    WHY 3  Implementation reads the discovered path verbatim with no fallback
           to the parent.
           Evidence: cgroup_preflight.rs:330-331 —
             `let enclosing_abs = cgroup_root.join(enclosing_rel.trim_start_matches('/'));`
             `let subtree_control = enclosing_abs.join("cgroup.subtree_control");`
           No conditional re-read of `enclosing_abs.parent()` when contents
           are empty. Doc-comment at :308-312 says "the *enclosing* slice"
           but the code treats every `/proc/self/cgroup` tail uniformly,
           regardless of whether the tail is a slice or a scope.

      WHY 4  The ADR-prescribed fallback ("or the parent's if the file is
             empty") was never implemented when the prior fix landed.
             Evidence: ADR-0028:172-177 step 4 explicitly states
             "Read <that_path>/cgroup.subtree_control (or the parent's if the
             file is empty)". The prior fix evolution doc
             (`2026-04-29-fix-cgroup-preflight-wrong-slice.md:54-62`) added
             `parse_cgroup_v2_path` and the join-to-`cgroup_root` shape but
             did not implement the parenthesised clause. The RCA for that fix
             flagged "Root Cause C: ADR-implementation drift was not caught
             at review" — and reproduced the same drift for the very next
             clause of the same step.

        WHY 5  No test fixture exercises the scope-vs-slice distinction. Every
               step-4 test points `/proc/self/cgroup` at a *slice* path
               (`user.slice/user-1000.slice`) and writes
               `cgroup.subtree_control` directly under that path. No fixture
               models the production reality where the discovered path is a
               *scope* with an empty `subtree_control` and the parent slice
               carries the delegation.
               Evidence:
               - preflight_reads_enclosing_slice.rs:53-61 — fixture writes
                 controllers at `user.slice/user-1000.slice/...` AND points
                 `/proc/self/cgroup` at the same path. Slice == discovered
                 path; no scope layer.
               - preflight_no_delegation.rs:42-50 — same shape; slice ==
                 discovered path.
               - preflight_missing_cpu.rs (per evolution doc:67-68) — same
                 shape.
               - The single RED oracle (preflight_reads_enclosing_slice.rs)
                 was designed to catch root-cgroup-vs-enclosing-path drift,
                 not scope-vs-slice drift. Its fixture is congruent with the
                 partially-implemented code, the same failure mode the prior
                 fix's Root Cause B identified.

          ROOT CAUSE A (primary): The ADR-0028 §4 step 4 fallback ("or the
            parent's if the file is empty") is not implemented in
            `run_preflight_at`. `enclosing_abs` is read once with no
            consideration of cgroup v2's leaf-scope semantics.

          ROOT CAUSE B (process): No test fixture distinguishes "discovered
            path == slice with controllers" from "discovered path == scope
            whose parent has controllers". Both prior-fix tests and the new
            oracle collapse the two into one shape, leaving the bug invisible.

          ROOT CAUSE C (process): ADR-implementation drift recurred at the
            same review boundary that the prior fix's Root Cause C named.
            No automated traceability check links ADR clauses to code; the
            prior RCA's "out of scope" item ("ADR↔code traceability tooling")
            was not actioned and the same class of drift produced the same
            class of bug within four days.
```

---

## Contributing factors

1. **Fix-locality bias.** The prior fix focused on the kernel-root-vs-discovered-
   path defect. The "or the parent's if the file is empty" sub-clause sat in the
   same ADR sentence but was treated as cosmetic — its code shape (a one-line
   conditional re-read) is small enough to be omitted without a reviewer
   noticing.

2. **Test-fixture mono-shape.** All step-4 fixtures use `user.slice/user-N.slice`
   as the `/proc/self/cgroup` tail. Production has at least two real shapes:
   systemd unit (`system.slice/overdrive.service`, slice == discovered path,
   controllers in `subtree_control`) and interactive shell
   (`user.slice/user-N.slice/session-M.scope`, scope == discovered path, empty
   `subtree_control`, controllers in parent). The mono-shape testing satisfied
   the prior fix's oracle and missed both production realities.

3. **Lima/CI runs as root.** `xtask/src/main.rs:240-247` invokes the test suite
   under `sudo`; UID 0 short-circuits at step 3 (`uid == 0 → return Ok(())` at
   `cgroup_preflight.rs:304-306`). Real-kernel CI never enters step 4. Only
   integration tests with synthetic `proc_self_cgroup` files exercise the path,
   and those fixtures are mono-shaped (above).

4. **No production-shaped manual smoke test.** The interactive-shell shape is
   the most natural way a developer first runs `overdrive serve` on a Linux
   dev VM; the bug fires on the very first attempt for that operator. Yet no
   process step has anyone do that — Lima inner-loop runs as root, CI runs as
   root.

---

## Proposed fix shape — recommendation: **Option A**

### Option A — implement the ADR-prescribed fallback verbatim

Read the discovered path's `cgroup.subtree_control`. If the file is empty
(zero non-whitespace tokens), re-read the **parent**'s
`cgroup.subtree_control` and apply the cpu/memory check against that.

Rationale:
- Matches ADR-0028 §4 step 4 exactly (no implementation/spec drift).
- Covers both production shapes:
  - systemd unit: discovered path's file is non-empty → use it directly.
    Same behaviour as today.
  - interactive shell: discovered path's file is empty → fall back to
    parent slice. Bug fixed.
- Empty-file detection is unambiguous: `contents.split_ascii_whitespace().next().is_none()`.
- Preserves the existing `DelegationMissing.slice` field semantics — it
  names the slice whose `subtree_control` was *actually inspected*, so the
  rendered error message remains accurate (`Delegate=yes` against
  `user-1000.slice` is the correct remediation when the parent is what was
  read).

### Option B — always read the parent

Drop `<discovered>/cgroup.subtree_control` entirely; always read
`<discovered>.parent()/cgroup.subtree_control`.

**Rejected.** Fails the systemd-unit case: `system.slice/overdrive.service`
is a slice-shaped delegation target, the unit's own `subtree_control` is
authoritative, and the parent (`system.slice`) typically does NOT enable
the same controllers. Always-read-parent would re-introduce the same class
of misdiagnosis in production-shaped deployments — trading one wrong
answer for another. Also diverges from the ADR, which would then need
amending.

The `/proc/self/cgroup` tail can be a slice OR a scope; only inspecting
the file content distinguishes them. Option A is the only fix that gets
both shapes right.

---

## Files affected

### Production code
- `crates/overdrive-control-plane/src/cgroup_preflight.rs` — replace the
  single `read_to_string(&subtree_control)?` (lines 343-345) with:
  read; if empty, compute `enclosing_abs.parent()`, re-read
  `<parent>/cgroup.subtree_control`, update `slice` for the eventual
  `DelegationMissing` to point at the parent. Edge case: if `parent()`
  returns `None` (discovered path is `/`, i.e. the kernel-root cgroup),
  surface `SubtreeControlUnreadable` rather than panicking — the kernel-
  root is structurally an unexpected enclosing path here. Doc-comment at
  module top (lines 13-17) and `run_preflight_at` (lines 308-312) update
  to name the fallback.

### New test fixture (RED → GREEN oracle)
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_falls_back_to_parent_slice_on_empty_scope.rs`
  — seeds:
  - `<tmp>/cgroup.controllers` (step 2 pass)
  - `<tmp>/user.slice/user-1000.slice/cgroup.subtree_control` = `cpu memory io`
  - `<tmp>/user.slice/user-1000.slice/session-3.scope/cgroup.subtree_control` = empty
  - `<tmp>/proc-self-cgroup` = `0::/user.slice/user-1000.slice/session-3.scope\n`
  Asserts `Ok(())` — fallback finds delegation in parent. FAILS under current
  code with `DelegationMissing`; PASSES under fix.

### Updated fixtures
- None of the existing 4 step-4 tests need fixture changes — they all point
  `/proc/self/cgroup` at a slice with non-empty `subtree_control`, which is
  the "no fallback needed" path. They continue to exercise the primary
  read; the new oracle exercises the fallback.

- Add a second new fixture
  `preflight_refuses_when_both_scope_and_parent_slice_lack_delegation.rs`
  to confirm fallback does not silently mask missing delegation: scope
  empty, parent slice carries `io pids` only → `DelegationMissing` with
  `slice` = `user.slice/user-1000.slice`.

### Wiring
- `crates/overdrive-control-plane/tests/integration.rs` — register both
  new modules under `mod cgroup_isolation { ... }`.

### Documentation
- `docs/evolution/2026-05-02-fix-cgroup-preflight-scope-vs-slice.md` — at
  delivery time, per the evolution-doc convention used by the prior fix.

---

## Risk assessment

### Low-risk: fallback is opt-in by file-emptiness
The fallback only fires when the discovered path's `subtree_control` is
empty. Every existing passing-shape test seeds non-empty content, so
behaviour for those is unchanged. The only behavioural change is:
"empty-file at discovered path → re-read parent" — previously this
produced `DelegationMissing` (wrong); now it produces either `Ok(())`
(parent has cpu+memory) or `DelegationMissing` against the parent slice
(parent does not). Both new outcomes are strict improvements over the
status quo.

### Medium-risk: `DelegationMissing.slice` field semantics
The error variant currently always carries the discovered path. Under
Option A, when the fallback fires and the parent ALSO lacks delegation,
the field carries the *parent*'s path. This is the correct operator-
facing value (the `Delegate=yes` command targets the parent, not the
scope) but a downstream consumer that reads `.slice` programmatically
expecting "the path from `/proc/self/cgroup`" would observe a
behavioural change.

Mitigation: grep audit confirms `DelegationMissing.slice` is only read
by the `Display` template in the same file (line 92, "subtree_control of
{slice}") — there are no programmatic consumers in the codebase. The
rustdoc on the `slice` field (lines 115-117) needs updating to reflect
"the slice whose `subtree_control` was inspected" rather than "the
enclosing slice (per `/proc/self/cgroup`)".

### Low-risk: parent of `/`
If `enclosing_rel` parses to `/` (cgroup-root case under a misconfigured
host), `Path::parent()` returns `None`. The fix surfaces this as
`SubtreeControlUnreadable` with a synthetic `io::Error` rather than
panicking. This case is unreachable under any documented production
shape but defends against future surprise.

### No silent dependency on broken behaviour
No call site in the repo treats `DelegationMissing` as anything other
than a hard refusal at boot. The CLI prints `Display` and exits non-zero
(per `cgroup_preflight.rs` module doc at lines 1-11). No reconciler,
test, or downstream module branches on the `slice` field's path shape.

### Mutation coverage
The new conditional adds a branch that mutation testing can exercise
(`if subtree_control_contents.is_empty()` mutated to `false` produces a
test failure on the new oracle). Expect the per-file kill rate to remain
above the 80% project gate; the prior fix landed at 93.8%.

---

## Cross-validation

- ROOT CAUSE A → symptom: `enclosing_abs/cgroup.subtree_control` is read
  and is empty → cpu/memory missing → `DelegationMissing` returned. Forward
  trace verified against `cgroup_preflight.rs:343-359`.
- ROOT CAUSE B → why the bug ships: no fixture seeds the empty-scope shape;
  oracle test was scoped to root-vs-enclosing distinction. Forward trace
  verified against `preflight_reads_enclosing_slice.rs:53-61` and the
  prior evolution doc:65-68.
- ROOT CAUSE C → why it slipped review: ADR clause read as one line in
  prose; implementation omitted half the clause; no automated check
  flagged the divergence. Same shape as prior fix's Root Cause C.

All three root causes consistent; no contradictions. Together they
explain both *why the bug exists in code* (A), *why it survived the
prior fix's test additions* (B), and *why it survived review of the
prior fix* (C). Solution Option A addresses A directly; the new test
fixtures address B; the recurring C is flagged for the same follow-up
the prior RCA flagged ("ADR↔code traceability tooling") with elevated
urgency given the recurrence.
