# Implementation Review — Step 03-02 (S-WS Keystone)

**Feature:** canonical-workload-address-inbound-tproxy (GH #241)
**Step:** `03-02` — Keystone Tier-3 bidirectional mesh e2e on the production inbound rule (S-WS)
**Review type:** Adversarial implementation review (`/nw-review @nw-software-crafter implementation`)
**Reviewed at:** 2026-06-24
**Commits under review:**
- `61edf95d` — `canonical_address_inbound_walking_skeleton.rs` keystone (1082 LOC) + E04 `pending` stub + worker-tree synthetic-virt removal + two RCA docs + worker `tests/integration.rs` de-wiring
- `f034f38f` — production convergence fix in `action_shim/mod.rs` (`FinalizeFailed{Stable}` must not tear down a live Running alloc) + paired regression tests in `alloc_netns_lifecycle.rs`

> **Method note (honesty).** The dispatched `nw-software-crafter-reviewer`
> produced a verdict but (a) did **not** write this artifact and (b) hand-waved
> two of the sharpest adversarial questions (invented-API-surface; convergence-fix
> leak path). The orchestrator independently closed both with concrete git/source
> evidence (Findings A and B below) and authored this file. The verdict stands on
> that combined evidence, not on the subagent's unsubstantiated stamp.

---

## Verdict: **APPROVED** (merge gated on the pinned-6.18 Tier-3 CI run)

- **Blocking issues: 0**
- **Non-blocking findings: 4**
- **Litmus integrity: SOUND** (proven via the client-side mTLS handshake, not map inspection)
- **Hardest CLAUDE.md rules (vertical slice / never-invent-API / deferrals-need-issues / verification discipline): all upheld with concrete evidence**

The single open *condition* (not a defect): the AC is **MERGE-BLOCKING on the
pinned-6.18 appliance-kernel Tier-3 matrix (ADR-0068)**, and the only evidence on
record is dev-Lima 7.0.0-22. That gate runs in CI, is correctly framed as
necessary-but-not-sufficient, and is **not** falsely claimed satisfied — but the
6.18 signal has not yet been observed. APPROVED is conditional on it.

---

## praise:

1. **Vertical slice through real production entry points — the rule this feature exists to defend.** The keystone boots the real `run_server` (`canonical_address_inbound_walking_skeleton.rs:523`), deploys through the production `POST /v1/jobs` handler (`:596-612`), discovers the canonical address from the V2 `AllocStatusRow`, and stops via the production `POST /v1/jobs/{id}/stop` handler (`:625-634`). It runs the **real `EbpfDataplane`** with `dataplane_override: None` (`:516`). This is exactly the corrective the predecessor (#236) precedent demanded — no test hand-installs the production call site.

2. **The synthetic-virt removal is real, not cosmetic.** The test-installed `install_inbound_tproxy(virt, leg_c_port)` and the synthetic `INBOUND_VIRT_IP/INBOUND_VIRT_PORT` loopback virt are gone from the worker tree (`rg` across `crates/overdrive-worker/tests/` finds no surviving reference except in *other* unit-shaped tests that legitimately exercise the production `install_inbound_tproxy` fn directly). The de-wiring in `crates/overdrive-worker/tests/integration.rs` is clean.

3. **Honest STOP-and-surface under pressure.** The execution log shows the crafter hit two production walls and refused to mask either — declining to log `COMMIT PASS` against an incomplete deliverable (`execution-log.json` t=17:41, t=18:40), surfacing the design/boundary contradictions (circular-dep placement; mutually-exclusive `dataplane_override`↔`compose_mtls`) instead of inventing surface to get green.

4. **Paired regression test for the production fix — both gate directions.** `finalize_failed_stable_does_not_tear_down_live_running_alloc` pins the bug; `finalize_failed_genuine_failure_still_tears_down_alloc` pins the *over-gating guard* (a genuine terminal STILL reaps). Both assert in-RAM (slot snapshot) on every host, with the netns assertion root-gated. This is the right shape — a one-directional test would have let a future "gate everything" regression slip.

5. **E04 deferral is textbook.** `pending` by design, real `- Anchor:` lines, deferral cites **#227** (EDD harness) on **#75** (Image Factory MVP) — both verified real OPEN issues whose titles match the deferral scope verbatim. The `runner.sh` self-reports pending and links no `overdrive-*` crate (verification.md black-box rule). It explicitly corrects the *stale* prior deferral premise ("serve XDP-on-lo boot fails") rather than propagating it.

---

## Finding A — `issue` REFUTED: no invented API surface (the hardest rule, confirmed clean)

**Hypothesis tested (adversarial):** did the keystone invent test-only production
API to reach green, violating CLAUDE.md "Implement to the design — never invent
API surface"?

**Refuted with git evidence:**
- Commit `61edf95d` touched **zero** files under any `src/` — it is test + docs + verification only.
- Commit `f034f38f` touched **only** `action_shim/mod.rs` (the convergence fix) — **not** `lib.rs`.
- `mtls_identity_override` was introduced by `20f799b7` (transparent-mtls 06-03) — **pre-exists** 03-02.
- `run_server_with_obs_and_driver` was introduced by `ed5975d8` (#143) — **pre-exists** 03-02.

The keystone composes only pre-existing, design-sanctioned seams
(`mtls_identity_override`, `run_server_with_obs_and_driver`, the existing
`ServerConfig` fields). **No production surface was added for this step.** This is
the load-bearing rule for this feature and it holds.

---

## Finding B — `issue` REFUTED: the convergence fix introduces no netns/slot leak

**Hypothesis tested (adversarial):** by gating `worker.stop_alloc` +
`teardown_and_release_netns` on `!is_stable` in the `FinalizeFailed` arm, did the
fix create a path where a previously-`Stable` alloc's netns/slot leaks?

**Refuted by tracing both terminal paths in `action_shim/mod.rs`:**
- `FinalizeFailed{Failed}` (genuine terminal): `is_stable == false` → teardown fires (`:1077-1089`).
- `StopAllocation` arm (`:1501`): calls `worker.stop_alloc` (`:1563`) and `teardown_and_release_netns` (`:1569`) **unconditionally** — no `is_stable` gate.

So a `Stable` alloc that later genuinely terminates — via the operator stop verb
(`StopAllocation`) **or** a non-`Stable` `FinalizeFailed` — is reaped on either
path. The gate suppresses *only* the wrong teardown (a `Stable` `FinalizeFailed`
is a success claim; the alloc stays Running and serving). The pre-existing
property "a *Running* alloc's netns survives a CP shutdown" is unchanged by this
fix (persistent-workload model) and is not a leak this step introduced. **The fix
is correct in scope.**

---

## Finding C — litmus integrity: SOUND (the keystone's reason to exist)

The litmus — "delete the 03-01 production install → keystone goes RED" — holds,
and holds **without** `LOCAL_BACKEND_MAP` inspection, via the client-side rustls
mTLS handshake:

- The server workload binds a **plain** Python `0.0.0.0:SERVICE_PORT` TCP listener inside its netns (`:708-724`).
- The client performs a **real rustls mTLS handshake** (`drive_client_handshake`, `:763-785`) and only asserts success on a byte-exact `RESPONSE` *and* `!observed_rst`.
- If the 03-01 inbound rule were absent and the dial routed **directly** to the plain Python server, the handshake would fail (Python does not speak TLS) → `fail()` → RED. The round-trip can be green **only** if the production leg-C mTLS worker terminated the connection — which only the production-installed PREROUTING rule diverts it to.
- `REQUEST != RESPONSE` (`:116-121`) is the second guard: it rules out a request-loopback masquerading as a working reply leg (`got == RESPONSE`, never `got == REQUEST`).

The two RCA docs (`docs/analysis/root-cause-analysis-canonical-address-inbound-{roundtrip-hang,reply-leg}.md`) carry **pasted** tcpdump / bpftrace / server-log / client-read evidence at each leg — they are executed evidence, not narrated. Wall 2 (reply-leg) was correctly classified as a *test-composition* gap (echo vs distinct-constant), with the production reply pipe proven working at the wire — not a production defect.

---

## Non-blocking findings

### nitpick (non-blocking): `bidirectional_walking_skeleton.rs` is now a misnomer
`crates/overdrive-worker/tests/integration/bidirectional_walking_skeleton.rs:1-16` —
the file now retains the **outbound leg only** (inbound moved to the control-plane
keystone) but keeps the "bidirectional" name and a `(step 05-01)` doc header. The
module doc *does* mark the inbound removal (per the "behavior change must mark
stale adjacent docs" discipline), so this is purely cosmetic — but the filename
will mislead a future reader grepping for "bidirectional". Consider renaming to
`outbound_walking_skeleton.rs` on next touch. Not blocking; the file belongs to
the transparent-mtls feature, not this one.

### suggestion (non-blocking): shutdown masking can hide a `ServerHandle::shutdown` regression
`canonical_address_inbound_walking_skeleton.rs:537-557` / `:559-589` — both
`Keystone::shutdown` and `Drop` wrap `handle.shutdown(...)` in `let _ =
timeout(...)`, swallowing the result. This is justified test hygiene (the round-trip
assertions all fire **before** shutdown, `:924-982`, so the proof is unaffected),
and the resolution correctly drives the production stop verb → `StopAllocation` →
`worker.stop_alloc` and polls obs to Terminated *before* shutdown — which exercises
*more* production path, not less. The reversal at execution-log t=18:40→t=18:58
("requires production change" → "test-side gap") is **honest**: "`ServerHandle::shutdown`
does not stop running allocs" is consistent with the persistent-workload model, and
the fix uses the operator stop path rather than altering shutdown semantics. Caveat
worth recording: the test **cannot** detect a future `shutdown` hang regression — if
that property ever matters, it needs its own assertion.

### nitpick (non-blocking): `let _ = sysctl ... rp_filter=0` is a debugging.md §8 debt-bomb
`canonical_address_inbound_walking_skeleton.rs:220-222` — the rp_filter relaxation
swallows its `Result`. If it silently fails on some host, the asymmetric reply path
could be dropped and the keystone would go RED with a misleading "leg-C failed"
signature rather than a clear "rp_filter blocked the reply". The comment names *why*
it's defensible (the reply returns on `lo`, not this veth, so the relaxation is
defense-in-depth), so this is acceptable best-effort — but per debugging.md §8,
elevating it to a `SKIP`/warning-on-`PermissionDenied` would remove the latent
misdiagnosis risk.

### question (non-blocking): is `HeldServerIdentity::svid_for(_alloc)` ignoring its arg a litmus weakener?
`canonical_address_inbound_walking_skeleton.rs:443` returns the same server SVID
for **any** alloc id; production's `IdentityMgr` keys by alloc id. **Not** a weakener
— it is the sanctioned `mtls_identity_override` seam, and the transitive litmus does
not depend on alloc-id→SVID keying, only that leg-C completes. The comment (`:429-436`)
states this. Production keying remains "its own DST's job" per the design. Recorded
for completeness, not as a concern.

---

## Acceptance-criteria scorecard

| # | Criterion | Verdict | Evidence |
|---|---|---|---|
| 1 | In-process real `run_server` + real deploy handler on REAL `EbpfDataplane`, NO `dataplane_override`; only seam = `mtls_identity_override` | ✅ | `:516`, `:518`, `:523` |
| 2 | Merge-blocking on pinned-6.18 Tier-3 matrix (ADR-0068), not dev-Lima 7.0 | ✅ (CI-pending) | `:55-59`, `:856`; 6.18 not falsely claimed |
| 3 | Test installs NO rule, supplies NO address, stands in for NO call site; synthetic virt + test-install REMOVED; litmus = delete 03-01 → RED | ✅ | worker-tree removal verified; Finding C |
| 4 | Driven through composition root in-process (not hand-assembled `start_alloc`, not subprocess); transitive round-trip, no map inspection | ✅ | `:596-634`, `:920-935`; Finding A (no invented surface) |
| 5 | E04 stub authored `pending` only; deferral cites real issues | ✅ | E04 `README.md`/`runner.sh`/`INDEX.md`; #227+#75 verified OPEN |

---

## Merge conditions

1. **Pinned-6.18 Tier-3 CI must pass** (ADR-0068 gating kernel). dev-Lima 7.0 GREEN is the inner loop, not the merge signal.
2. **E04 black-box capture stays deferred** to #227 on #75 — correct; not a blocker for this slice.

**Status: APPROVED.**
