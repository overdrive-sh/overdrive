# Adversarial Review — Step 03-02

**Step**: 03-02 — "Bidirectional ping-pong demo + EDD expectation (US-DBN-3)"
**Artifact**: `docs/feature/dial-by-name-responder/deliver/execution-log.json` (step 03-02) + as-landed commits `e035a55e` (scaffold + a/b tomls + E05) and `9579f6ae` (Rust-bin → checked-in `ping_pong.py`)
**Reviewer**: adversarial (`/nw-review`), Opus, with full repo context
**Date**: 2026-06-28
**Verdict**: **NEEDS_REVISION** → **RESOLVED** (see "Re-review" at the end — APPROVED on commits `c7112ce8` + `3901c989`)

---

## Summary

Step 03-02 is a docs/example/test-only slice: it lands two example specs
(`examples/dial-by-name-responder/{a,b}.toml`), a checked-in client program
(`ping_pong.py`), a Tier-3 module (`dns_responder_ping_pong.rs`), and an EDD
expectation (`E05`, honest `pending`). No production source is touched; the
mutation-gate exemption (criterion 9) is therefore valid.

**Most of what landed is honest, well-documented, and correctly disciplined.**
The deferral citations are exemplary (every `#NNN` exists, is OPEN, and
scope-matches), the PLAINTEXT-egress model is applied correctly in the client
(it does *not* repeat the 02-02 RCA model error), E05 faithfully mirrors the
accepted E04 pending-posture, and the second commit's standalone-runnable fix
genuinely improves on the `/tmp`-staged Rust bin.

**The blocking problem is the shape of the proof, not the fixtures.** The
headline behaviour of this step — *two services dial each other by name through
the production path* — is exercised by **nothing**. The Tier-3 module is a
permanent `#[should_panic(expected = "RED scaffold")]` that panics *before*
booting anything, and its GREEN transition is (incorrectly) gated on the
full-system EDD harness `#227/#75`. But the 02-02 walking skeleton already
proves the **single-direction** dial-by-name loop in-process at Tier 3 with a
test-PKI seam, and the bidirectional case is the same machinery against one
`serve` + two `deploy`s. The deferral conflates two distinct surfaces (the
in-process Tier-3 `#[tokio::test]` vs. the black-box E05 capture), and it leaves
the *genuinely new* composition this step exists to prove — a workload that is
**both** a mesh service (inbound leg-C + frontend `F`) **and** an egress dialer
(leg-B), ×2 on one node — entirely unverified. Given 03-01's parallel-collision
finding, that composition carries real risk.

This posture was pre-blessed by DISTILL (`red-classification.md` line 93 +
criteria 3/8), so the crafter executed the accepted design faithfully. The
finding is therefore substantially a **design contradiction surfaced at DELIVER**
(criterion 1 says the test *"proves"*; criteria 3/8 say it lands as a pending
scaffold) that should have been surfaced for a scope decision — the way 03-01
withheld COMMIT for its production gap — rather than silently executed toward the
weaker reading.

---

## Blocking issues

### issue (blocking): the bidirectional proof is exercised by nothing, and the deferral conflates the in-process test with the black-box EDD capture

The primary AC (criterion 1) requires:

> *"a Tier-3 `#[tokio::test]` in `…/dns_responder_ping_pong.rs` … **proves**:
> GIVEN `overdrive serve` booted … WHEN Sam runs `overdrive deploy a.toml` then
> `b.toml`, THEN within ~15s A resolves `b.svc.overdrive.local` … and calls B …
> B resolves `a.svc.overdrive.local` and calls A … each hop is intercepted +
> mTLS'd … and both counters continue advancing on a ~10s ±5s cadence over a 60s
> window."*

What landed (`dns_responder_ping_pong.rs:213-256`) is a `#[tokio::test]` whose
entire body is two `panic!("… RED scaffold …")` calls under
`#[should_panic(expected = "RED scaffold")]`. It panics on both the root and
non-root paths, *before* `run_server_with_obs_and_driver` is ever called. It
proves nothing, asserts nothing, and — critically — **guards nothing**: deleting
the entire bidirectional datapath would leave this test green.

The stated reason it cannot be a real test (file docstring lines 38-66; E05
README lines 5-20; `.config/nextest.toml` comment) is that the bidirectional
proof "is the end-to-end behaviour the full-system EDD harness (#227 on #75)
exercises against the BUILT binary … NOT an in-process `#[test]`," and that the
scaffold "goes GREEN once the full-system EDD harness #227/#75 lands."

That reasoning does not hold for the **in-process** test, for three independent
reasons:

1. **The single-direction loop is already proven in-process — bidirectional is
   the same machinery.** `dns_responder_walking_skeleton.rs` (02-02) has **4
   real `#[tokio::test]` functions, 0 `should_panic`**, with genuine assertions
   (`getaddrinfo`/`getent` resolves to the stable frontend `F ∈ 10.98.0.0/16`,
   never a `10.99.0.0/16` backend addr; a wire-scan proves TLS-1.3 records on the
   inter-agent leg with zero cleartext), driven against
   `run_server_with_obs_and_driver` + `POST /v1/jobs` with a test-PKI seam. Only
   the backend-*churn* halves are `#[ignore]`'d (to #249). The
   ping-pong is **one `serve` + two `deploy`s** (criterion 1's own GIVEN/WHEN),
   not two `serve` instances — so the singleton `:53` / fixed-iface constraint
   does not block it. The production code it needs (responder/serve, re-keyed
   classify, egress leg-B interception) all landed at 01/02; per
   `red-classification.md`'s own timeline note the RED panic was "on the missing
   serve/classify impl," and that impl is no longer missing.

2. **#227/#75 is the wrong gate for an in-process test.** #227 is a *disposable
   full-system Lima VM that runs the BUILT binary black-box*; #75 is the OS
   image. An in-process Tier-3 `#[tokio::test]` with a test-PKI seam needs
   neither. The thing that genuinely needs #227/#75 is the **black-box E05
   capture** (real workload-identity CA → SVID, *no* `mtls_identity_override`
   seam) — and E05 is correctly `pending` on exactly that. Gating the
   `#[tokio::test]` on #227/#75 (test docstring line 47; nextest.toml comment)
   is a category error: it imports the black-box capture's blocker onto a surface
   that does not share it.

3. **The marker contradicts the crafter's own stated blocker.** `.claude/rules/
   testing.md` reserves `#[should_panic(expected = "RED scaffold")]` for *"the
   production code doesn't exist yet,"* and `#[ignore = "reason"]` for *"waiting
   on external resources the implementation cannot synthesize … an integration
   target whose dependency is genuinely missing."* Criterion 8 mirrors this
   ("`#[should_panic …]` (or `#[ignore]` only if blocked on an external
   resource)"). The crafter's framing is "the impl exists; the #227/#75 *harness*
   is missing" — which is precisely the `#[ignore]` case, not the RED-scaffold
   case. The `should_panic("RED scaffold")` marker mislabels "impl pending" when
   the impl is present.

**Why this matters beyond bookkeeping.** 02-02's dialer (`client`) is a plain
workload that only dials; its server (`server.svc`) is a service that only
receives. **No test in the feature exercises a workload that is simultaneously a
mesh service AND an egress dialer** — which is exactly what `a` and `b` are. That
composition (service inbound leg-C + frontend `F` *and* egress leg-B, ×2 on one
node sharing the fixed `ovd-veth-cli`/`ovd-veth-bk` pair) is the new ground
03-02 was meant to cover. It is now unverified — and 03-01 already showed that
even three sibling responder tests collide on the shared `:53` /
`FrontendAddrAllocator` / identical-name state when run together, so "two
services coexisting and dialing each other" is a live composition risk, not a
formality.

This is the `CLAUDE.md` "Build vertical slices through production entry points"
bar: *"the behaviour runs in the binary's real composition … not only in a
`#[test]` that assembles the pieces by hand."* Here it does not even run in a
`#[test]` — it runs in a `panic!`.

**Resolution (a user/architect decision, not a mechanical fix):**

- **(a) Preferred — write the in-process bidirectional Tier-3 `#[tokio::test]`
  now.** Mirror the 02-02 walking skeleton: one `run_server_with_obs_and_driver`
  boot, two `POST /v1/jobs` (a then b) with the test-PKI seam, assert each half
  resolves its peer to a stable `F` via `getent` (NOT `dig` — K2), assert both
  inbound counters advance over a bounded window, and apply the 02-02 `WireScan`
  0x17 oracle to **both** hops. Keep **E05** black-box `pending` on #227/#75
  exactly as written — that surface genuinely needs the real CA. This makes
  criterion 1 true and gives the bidirectional composition a real regression
  guard.
- **(b) If (a) is genuinely infeasible** — e.g. the single fixed
  `ovd-veth-cli`/`ovd-veth-bk` pair cannot carry two simultaneous mesh dialers,
  or service+dialer does not compose on one workload — then **that** is the real
  blocker. Surface it (the 03-01 pattern: withhold and name the gap), file/cite
  the issue, and re-size 03-02 so the criterion 1 wording matches what is
  actually achievable. Do not leave criterion 1 asserting "proves" over a panic.

Either way, if the in-process test stays scaffolded pending an *external
resource*, switch the marker to `#[ignore = "blocked on #227/#75 …"]` per
testing.md / criterion 8, and stop describing it as the in-process "what,
forever" regression witness (E05 README lines 101-106) — a panic is not a
witness.

---

## Non-blocking findings

### suggestion (non-blocking): acknowledge the Rust-bin → Python divergence from criterion 2's literal pin

Criterion 2 pins the client form two ways at once: its headline says *"`command`
pointing at a REAL on-disk staged tiny **Rust** ping-pong bin (K3 — no phantom
paths)"*, while its tail grants DELIVER latitude (*"DELIVER pins the concrete
client-program form (staged Rust bin vs `python3 -c`)"*). The delivery chose a
*third* form — a checked-in `ping_pong.py` run by `/usr/bin/python3` — neither
the pinned Rust bin nor inline `python3 -c`.

The choice is **correct on the merits**: commit `9579f6ae` documents that the
first commit's `/tmp/overdrive-ping-pong` (a Rust bin `rustc`-staged by the test)
*was itself* the phantom-path class the criterion meant to avoid — `overdrive
deploy a.toml` failed unless the test ran first. A checked-in script that runs
by hand with no build step better serves the criterion's **intent** (K3,
no-phantom-path). This is the right call. But it is still a divergence from a
pinned criterion (`CLAUDE.md` "Implement to the design — never invent surface"
applies to fixtures too), and it should have been surfaced as the gap it is
rather than reconciled silently a commit later. Recommend the roadmap/feature-
delta criterion text be updated to the as-landed Python form so the SSOT matches
(this is the "behavior change must mark stale adjacent docs" discipline).

### suggestion (non-blocking): the relative script path leaves the "standalone example" goal half-met

The whole point of commit `9579f6ae` was *"An example must stand on its own."*
The verified mechanics are sound — `ExecDriver::build_command`
(`overdrive-worker/src/driver.rs:351-400`) sets **no** `current_dir` and enters
**only** `CLONE_NEWNET` (no mount namespace), so `args[0] =
"examples/dial-by-name-responder/ping_pong.py"` does resolve against `overdrive
serve`'s cwd. But that couples the example to *serve being launched from the repo
root* — a real residual path-fragility the header honestly documents ("Run
`overdrive serve` from the repo root so this path resolves"). Contrast
`dns-resolver.toml` (`/usr/bin/socat`, absolute) which is fully self-standing.
This is acceptable (the repo has no fixed install path) and well-documented, but
the goal is only half-achieved; consider pinning the exact `serve` cwd in the
E05 `runner.sh` so the future #227 capture cannot drift on it.

### question (non-blocking): criterion 5 ("the Tier-3 AC asserts the pinned-6.18 matrix") is currently aspirational

`record_kernel()` `eprintln!`s `uname -r` and the result is discarded
(`let _kr = …`). The scaffold asserts nothing about the kernel. That is fine for
a scaffold (the 6.18 assertion is a DEVOPS/Tier-3 obligation that lands with the
real test / EDD capture), but criterion 5's verb is "asserts," and nothing
currently does. Fold a real kernel assertion into resolution (a) above, or note
in the roadmap that criterion 5 is a DEVOPS obligation, not an in-test assertion.

---

## Praise

- **praise:** Deferral discipline is exemplary. Every cited issue — #227 (EDD
  harness), #75 (OS image), #249 (backend churn), #243 (feature) — exists, is
  OPEN, and scope-matches, cited at every reference site. No hand-wavy forward
  pointers. This is the `CLAUDE.md` deferral rule done exactly right.
- **praise:** The PLAINTEXT-egress model is correct. `ping_pong.py` dials its
  peer over an ordinary plaintext `socket.create_connection`, presents no
  TLS/SNI, and the docstrings correctly site the mTLS proof on the inter-agent
  leg — it does **not** copy the keystone's TLS-presenting dial shape (the exact
  02-02 RCA model error, which the file calls out by name). The hardest thing to
  get right here was gotten right.
- **praise:** The standalone-runnable fix is a genuine improvement, not churn.
  Moving from a `/tmp`-`rustc`-staged Rust bin to a checked-in stdlib-only Python
  script removes a real phantom-path hazard and makes the example runnable by
  hand (verified two-instance localhost run, per the commit).
- **praise:** nextest single-writer group membership was added proactively
  ("not deferred to the GREEN step") with a clear rationale — correct discipline
  that prevents a future `:53`/`IfaceXdpSlotBusy` collision the moment the
  fixture boots.
- **praise:** E05 mirrors E04 honestly — `pending` status, no fabricated
  capture, explicit "do NOT self-stamp `satisfied`; bounce to a different-fox
  audit," and real anchors (S-DBN-PINGPONG, K-DBN-3, ADR-0072 REV-2). The EDD
  *expectation* is exactly the honest posture the verification rules require.

---

## Verdict rationale

The fixtures, the deferral hygiene, the egress model, and the E05 expectation are
right. The step is blocked on one substantive thing: its headline behaviour has
no executable proof, the in-process proof is achievable now (02-02 is the
existence proof), and the deferral that excuses its absence mis-grounds the
in-process test on the black-box harness's blocker. Because the genuinely-new
service-and-dialer composition is left unverified — with a concrete 03-01
collision precedent making it risky — and because criterion 1 asserts "proves"
over a panic, this is **NEEDS_REVISION**. The corrective is a scope decision
(write the in-process bidirectional test, or formally re-size with the real
blocker named), not a rewrite — and most of the slice's artifacts carry forward
unchanged.

---

## Re-review — RESOLVED (APPROVED)

**Re-reviewer**: different-fox adversarial re-review, Opus, read-only
**Date**: 2026-06-28
**Scope**: the fix — commit `c7112ce8` (real bidirectional `#[tokio::test]` +
`dns_responder_bind` joins the `host-kernel-shared` serial group) and commit
`3901c989` (text-only SSOT criterion reconciliation). User chose **resolution
(a)**.
**Verdict**: **APPROVED** — the single blocking issue is genuinely resolved.

The `#[should_panic("RED scaffold")]` body is gone; the bidirectional behaviour
now runs in a real `#[tokio::test]`
(`two_services_dial_each_other_by_name_counters_advance_each_hop_mtls`) that
boots one `run_server_with_obs_and_driver`, deploys `a` + `b` via `POST
/v1/jobs`, and drives each peer as **simultaneously a mesh service and an egress
dialer** — the genuinely-new composition the blocking issue flagged as
unverified. The proof is not vacuous: it bites at three independent points, each
of which a real regression would turn RED.

- **Frontend resolution bites** — `assert_frontend_subnet` requires the
  `getent`-resolved peer addr ∈ `10.98.0.0/16`, rejects the `10.99/16` backend
  block, and asserts byte-distinct from the peer's real backend addr; resolution
  is via `ip netns exec … getent ahostsv4` (K2), never `dig`. A resolve-to-backend
  regression fails it.
- **Counter-advance bites** — `assert_counter_advances` requires a byte-complete
  `PONG count=<n>` from two distinct dials per direction with a STRICT increase;
  the peer's Python server authors the count (ignores the request body, so no
  echo can satisfy it) and the per-direction REQUEST markers are byte-distinct
  (a→b cannot be confused with b→a). A dead reply pipe / echo / non-advancing
  counter fails it.
- **mTLS oracle bites (load-bearing, both hops)** —
  `assert_inter_agent_hop_is_mtls` requires `has_app_data()` AND
  `records_to_wire_port > 0` AND `records_from_wire_port > 0` of real TLS-1.3
  `0x17` records on the inter-agent `lo:<peer_port>` leg; an empty capture fails
  the directional counts (cannot pass vacuously), and a cleartext-passthrough
  regression produces zero `0x17` records and trips it. The dual-EKU SVID seam is
  structurally required for the handshake — disable leg-B mTLS and the round-trip
  RSTs, failing the counter assertion too.

Boundary & honesty confirmed: `c7112ce8` touches **no `crates/**/src/**`**
(criterion 6 held — it references only pre-existing 01/02 production surfaces via
the sanctioned `mtls_identity_override` port seam, hand-installs nothing
production should install); **E05 stays `pending`** (no self-stamp, no fabricated
capture); the DES log's latest 03-02 COMMIT is a valid PASS naming `c7112ce8`;
the `3901c989` criterion reconciliation is honest (as-landed, no scope change,
`roadmap.json` valid JSON). Waits are poll-budgeted (no CI-racing fixed sleeps);
the module is in the `host-kernel-shared` single-writer group, closing the 03-01
collision risk.

**Non-blocking (carry-forward, do not gate):** the test is root-gated with a clean
non-root SKIP (runs under `cargo xtask lima run` as root) — matches the 02-02 /
03-01 discipline and the pinned-6.18 merge-matrix obligation (ADR-0068);
criterion 5's kernel assertion remains a DEVOPS/Tier-3 matrix gate, now
accurately reconciled in `roadmap.json`. The roadmap
`validation.review.coverage_summary` roll-up still lists "PINGPONG PENDING"
(stale post-fix) — a minor roll-up field, left for a future sweep.

The three test-tier proofs are the `what, forever` regression witnesses; the E05
EDD expectation remains the black-box operator-observable `why`, honestly pending
on #227/#75.
