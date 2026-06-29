# Adversarial Review — Step 03-01

**Step**: 03-01 — "Empty-candidate honesty / NXDOMAIN end-to-end (US-DBN-4)"
**Artifact**: `docs/feature/dial-by-name-responder/deliver/execution-log.json` (step 03-01) + as-landed commits `5b44b59d..2a9d44cd`
**Reviewer**: adversarial (`/nw-review`), Opus, with full repo context
**Date**: 2026-06-28
**Verdict**: **NEEDS_REVISION**

---

## Summary

Step 03-01 was scoped as a **test-only** Tier-3 slice (roadmap: *"this step adds
NO new production type"*). In execution it surfaced a genuine production defect
(#251: a stopped-but-declared dial-by-name Service keeps resolving its stale
frontend `F` forever, never NXDOMAIN), and the crafter — correctly — surfaced it
as a blocker, then landed a focused production fix, an ADR-0049 amendment, an
RCA, and a test-serialization fix.

**The code that landed is correct, well-reasoned, and better than the RCA's own
recommended fix.** The fix is verified end-to-end by a now-live Tier-3 test and
by honest unit + integration coverage. A prior internal adversarial pass already
caught and remediated a "testing theater" problem; that remediation is real, not
cosmetic.

**The blocking issues are all in the artifact/process layer**, not the code:
the RCA document misdescribes the fix that shipped, the DES execution log + the
roadmap do not reflect 03-01's as-landed production scope, and the mutation-gate
evidence the step's own AC requires for an added production helper is missing.
Per this repo's CLAUDE.md these are load-bearing ("the DES log is a contract";
"No aspirational docs"), so they block — but each is low-effort to clear.

---

## Blocking issues

### issue (blocking): RCA-251 misdescribes the fix that actually shipped

`docs/analysis/rca-251-withhold-on-stop.md` was committed in `aeaee91a` — *after*
the fix `88679ed8` — yet it still prescribes a fix that was **not taken**:

- **Pinned fix-site (lines 15, 108, 137):** `reconciler_runtime.rs::hydrate_bridge_desired_listeners`
  (≈L2080–2154). **This function was not touched by the fix.**
- **Recommended fix (line 124, "Option A — recommended"):** make the bridge
  fall back to the last `service_backends` row when the VIP memo is absent —
  i.e. **keep release-on-terminal** and make the bridge robust to the eviction.
- **Line 126, "Option B … NOT recommended":** gating `service_vip_release_emission`.
  The landed fix is a variant of the *rejected* direction (it changes
  `service_vip_release_emission` to release-on-deletion / retain-on-stop).

What actually landed (`workload_lifecycle.rs:901-933`) is a **third option**:
retain the VIP entirely until intent withdrawal (`if desired.job.is_some() {
return None; }`, line 927), so the memo is never evicted on stop and the bridge's
existing dependency is satisfied unchanged ("No bridge edit needed", per the
commit). There is **no as-landed / superseded note** anywhere in the 176-line
doc (verified: no `as-landed`, `superseded`, `instead`, `chose`, `amend` marker
in the prescription sections).

Why this is blocking, not a nitpick:

1. **It will misdirect the next debugger.** Someone who opens RCA-251 to
   understand the #251 fix is pointed at an untouched function and a
   non-implemented option — exactly the "stale measurement becomes a
   load-bearing premise" failure `debugging.md` §6 warns against.
2. **It is internally self-contradictory within one commit.** `aeaee91a`
   amends ADR-0049 to *retain* the VIP (withhold-not-release, symmetric with
   `F`), while the RCA it commits recommends Option A, which **keeps
   release-on-terminal** (RCA line 157: *"ReleaseServiceVip still runs and
   returns the VIP to the pool"*) — the opposite of the amendment in the same
   commit. A reader cannot reconcile the two.
3. CLAUDE.md: *"No aspirational docs. Never document behaviour that is not
   implemented."*

**Note — the diagnosis is sound; only the prescription is stale.** The root
cause (mechanism 3: the `ReleaseServiceVip` executor evicts the VIP memo that
`hydrate_bridge_desired_listeners` depends on, racing the zero-backend
retraction) is correct and *is* what the landed fix addresses (by never evicting
on stop). The population-diff evidence (RCA §"Evidence") is excellent. **Fix:**
add an as-landed addendum reconciling Option A → the shipped gate-swap, and state
*why* Option A was rejected (it retains release-on-terminal, contradicting the
withhold-not-release amendment). Update the "Pinned fix-site" to
`workload_lifecycle.rs:901-933`.

### issue (blocking): the DES log and roadmap do not reflect 03-01's as-landed production scope

The execution log records `03-01 COMMIT EXECUTED PASS` at `2026-06-28T03:29:10Z`
— but that marker corresponds only to the **first, test-only batch** (`5b44b59d`
tests + `479495f1` docs, the version with NXDOMAIN-02 `#[ignore]`'d). The entire
#251 production fix landed *after* the "done" marker and was **never logged**:

- `88679ed8` — production change to a core reconciler (`workload_lifecycle.rs`,
  the `service_vip_release_emission` gate + View-field rename + un-ignore of the
  Tier-3 oracle).
- `aeaee91a` — ADR-0049 amendment + RCA + prior-art research.
- `2a9d44cd` — nextest serialization + testing-theater remediation.

Two concrete corruptions of the contract:

1. **Stale RED_UNIT rationale.** The logged RED_UNIT SKIP (03:14Z) reads
   *"Tier-3 step adds NO new below-port mutable production source … this Tier-3
   step adds no new mutable in-process source."* The landed gate-swap is exactly
   new mutable reconciler logic — the rationale is now false and was never
   corrected.
2. **Roadmap drift.** Roadmap step 03-01 still asserts *"this step adds NO new
   production type"* and *"No API beyond the pinned 01-03/01-04/02-00/02-01
   surface."* Both are now false (the `released_for_terminal` → `released_for_deletion`
   View field + gate-signature change `(desired, actual, view)` → `(desired,
   view)` is new surface). A `grep` for `#251` / `release-on-deletion` /
   `released_for_deletion` across `feature-delta.md` and `roadmap.json` returns
   **nothing** — 03-01 never got the as-landed reconciliation that 02-02
   received in `b8b7a7ad` ("reconcile roadmap + feature-delta with as-landed").

CLAUDE.md: *"The DES log is a contract; partial completions corrupt it"* — here
the inverse, a *complete* marker over scope that kept growing. **Fix:** append
the post-fix phases to the execution log (the production fix is its own
GREEN/COMMIT arc), correct the RED_UNIT rationale, and reconcile roadmap +
feature-delta to the as-landed scope (mirroring `b8b7a7ad` for 02-02).

### issue (blocking): mutation evidence missing for the added production gate (the step's own AC requires it)

Roadmap 03-01's mutation AC: *"If a small production helper IS added here, it
carries its own per-step diff-scoped mutants run at kill-rate >= 80%."* A
production helper **was** effectively added/modified (`service_vip_release_emission`,
a private fn in a `core` reconciler — a *mandatory* mutation target per
`testing.md` §"Mandatory targets → Reconciler logic"). There is **no
`mutants-03-01.md`** in `deliver/` (only `mutants-02-00.md`, `mutants-02-01.md`),
and no mutation phase in the execution log for 03-01.

The kill coverage very likely *exists* — `declared_service_retains_vip`
(`workload_lifecycle.rs:1446`) kills "delete the gate" / "invert to `is_none()`"
mutants, and the two withdrawn-* tests cover the other direction — so this is
"run it and record the evidence," not "coverage is absent." But the step's own
AC mandates the per-step run, and it is currently unsatisfied. **Fix:**
`cargo xtask mutants --diff origin/main --features integration-tests --package
overdrive-core --file crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`
(Lima-wrapped), record ≥80% as `mutants-03-01.md`.

---

## Suggestions / questions (non-blocking)

### question (non-blocking): was the in-scope production fix + ADR amendment user-authorized under 03-01?

The execution log GREEN-FAIL correctly records *"COMMIT withheld pending user
decision"* (03:20Z) — good discipline. The fix then landed, so a decision was
made. But folding a core-reconciler semantics reversal + an ADR amendment into a
step the roadmap declared test-only is a material scope expansion. Per CLAUDE.md
this needed explicit user approval (and #251 must not have been created
unilaterally — `gh` confirms #251 exists, OPEN, scope-matching). Please confirm
the user approved (a) folding the #251 fix into 03-01 vs. a new step, and (b)
the #251 issue creation. If approved, this is consistent with "fix the surfaced
bug in-scope"; flagging only because the authorization isn't visible in the
artifacts.

### question (non-blocking): was the ADR-0049 amendment authored via the architect agent?

Repo convention (and prior feedback): ADR edits go through the architect agent,
not inline. `aeaee91a` amends ADR-0049 §6. Confirm the amendment went through the
architect path rather than an inline crafter edit.

### suggestion (non-blocking): make the release-on-deletion dead-path note operationally explicit

Verified at source: `read_job` returns `(None, None, …)` on absent intent
(`reconciler_runtime.rs:2309`) → `service_spec_digest = None`
(`:1688-1689`) → the gate's `desired.service_spec_digest?` short-circuits before
the `desired.job.is_none()` branch. So **release-on-deletion is unreachable on
the v1 convergence path** and every stopped-but-declared Service **permanently
holds its 10.96.x VIP for the process lifetime** until #211 wires deletion. This
is honestly documented (ADR-0049 D3; the inert companion test
`withdrawn_service_without_digest_emits_no_release` at `workload_lifecycle.rs:1518`;
the inline #211 note at `vip_allocator_lifecycle.rs:733-748`). Acceptable for
single-node Phase 1, but the *operational* consequence (unbounded VIP retention
across stop/restart churn until process exit) deserves one explicit line in
ADR-0049 D3 so it isn't a surprise under #249 churn before #211 lands.

### thought (non-blocking): NXDOMAIN-02 AC is partially met by design

S-DBN-NXDOMAIN-02's recovery leg (re-deploy → same `F`) stays `#[ignore]`'d to
#249 (`dns_responder_nxdomain.rs:1068`). The ignore reason is thorough, cites a
verified real issue, and matches the 02-02 dependency. The retained-F invariant
is Tier-1 mutation-gated at 01-04, so only the Tier-3 *observable* is deferred —
reasonable. Just noting the AC is satisfied in two halves (withhold leg live;
recovery leg deferred), which the roadmap reconciliation above should make
explicit.

### question (non-blocking): final combined state proven Lima-green?

The original GREEN-FAIL diagnosis came from a real Lima run, and `88679ed8`
states the un-ignored Tier-3 oracle is "now GREEN." But the execution log has no
post-fix GREEN entry covering the production change + the serialization fix
(`2a9d44cd`). Per CLAUDE.md "a green compile-check plus a green Lima *run* is the
honest signal; either alone is not" — confirm the full integration suite
(un-ignored `after_backend_stops_…` + new `convergence_tick_retains_vip_…` +
the serialized `dns_responder_nxdomain` group) was *run* green under Lima at HEAD
(`2a9d44cd`), not just compile-checked.

---

## Praise (genuine)

- **praise:** Exemplary debugging discipline. The RCA falsified the two
  originally-framed mechanisms and pinned mechanism 3 with a live Lima
  *population-diff* probe (server-only writes the retraction; server+client does
  not) — textbook `debugging.md` §5/§11, probe reverted after. The first
  on-the-spot diagnosis ("bridge never writes a zero-backend row") was
  confidently wrong; the population diff corrected it rather than rationalizing
  it.

- **praise:** Honest blocker-first handling. The crafter did **not** improvise
  past the gap on first contact — surfaced it, withheld COMMIT pending a user
  decision, filed #251, and committed the test-only slice with `#[ignore]`s
  before later landing the fix. This is precisely the CLAUDE.md "STOP and surface
  the gap" behavior.

- **praise:** The shipped fix is *better than the RCA's own recommendation*.
  Option A would have kept release-on-terminal — contradicting the
  withhold-not-release contract (ADR-0072, symmetric with `F`). The gate-swap
  aligns the Service VIP with `F` (both deletion-only) and needs no bridge edit.
  Good judgment to diverge from the written recommendation for the right reason.

- **praise:** The "testing theater" remediation is real, not cosmetic. The
  un-producible `(job=None, digest=Some)` expectation was **deleted** at the
  integration level and replaced with a genuine production-path test
  (`convergence_tick_retains_vip_on_stopped_but_declared_service`,
  `vip_allocator_lifecycle.rs:682`), an explicit "release-on-deletion is unwired,
  #211" note (`:733-748`), and an inert companion unit test that "flips red the
  moment a withdrawn intent starts carrying a digest on the v1 path"
  (`workload_lifecycle.rs:1518`). The integration test was made *honest*, not
  made to pass.

- **praise:** Correct serialization mechanism. `dns_responder_nxdomain` joins the
  by-module `host-kernel-shared` single-writer nextest group
  (`.config/nextest.toml`), not `serial_test` — which (per the file docstring and
  prior project learning) does not cross nextest's per-test process boundary.
  The comment explicitly frames it as "required discipline, not scope creep."

- **praise:** Persist-inputs-not-derived-state discipline maintained through the
  rename: `released_for_deletion` records the *input* "already emitted release
  for this digest," with `#[serde(alias = "released_for_terminal")]` keeping
  pre-rename CBOR blobs readable (additive evolution). All three cited issues
  (#211, #249, #251) verified real, OPEN, and scope-matching.

---

## Verdict: NEEDS_REVISION

The implementation is sound and the fix is correct — but three artifact/process
integrity gaps block sign-off, all low-effort:

1. Reconcile RCA-251 to the as-landed fix (add an addendum; re-point the fix-site;
   explain why Option A was rejected).
2. Reconcile the DES log + roadmap + feature-delta to 03-01's as-landed
   production scope (append post-fix phases; correct the stale RED_UNIT rationale;
   drop the "adds NO new production type" claim).
3. Run + record the diff-scoped mutation gate on `workload_lifecycle.rs` per the
   step's own AC (`mutants-03-01.md`, ≥80%).

Confirm the two open questions (user authorization of the in-scope fix + #251
creation; ADR amendment via the architect agent) and the Lima-green status of the
final combined state.
