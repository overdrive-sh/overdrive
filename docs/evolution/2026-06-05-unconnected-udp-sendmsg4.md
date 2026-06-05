# Evolution — unconnected-udp-sendmsg4

**Finalized:** 2026-06-05 · **Wave lifecycle:** DIVERGE → DISCUSS → DESIGN
→ DISTILL → DELIVER · **Source brief:** GitHub issue
**[#200](https://github.com/overdrive-sh/overdrive/issues/200)** — *"Same-host
unconnected-UDP delivery (`sendto(VIP)` without `connect()`): the dominant
DNS-resolver idiom is never intercepted by connect4."* Closes **ADR-0053
Open-Question 3** (the Amendment-4 2026-06-03 out-of-scope carve-out).
**SSOT (preserved):** `docs/feature/unconnected-udp-sendmsg4/feature-delta.md`
(lean — the feature workspace is retained, not deleted).

## Summary

Make a same-host UDP service reachable from the client idiom that actually
dominates DNS — the **unconnected** `sendto(VIP)` that never calls
`connect()` — and make the reply appear to the client application to come
from the VIP, with the Sim≡kernel reply-path symmetry pinned so it cannot
silently regress.

Before this feature, the shipped `cgroup_connect4_service` fired at
`connect(2)` time **only**. `dig`, glibc `getaddrinfo`, and musl all
`sendto(VIP)` per query without connecting, so the datagram was never
intercepted, `LOCAL_BACKEND_MAP` was never consulted, the VIP→backend
rewrite never happened — a half-working service (healthy upstream,
unreachable from the client that matters). `overdrive deploy` succeeded,
`alloc status` showed Running, a hand-written *connected*-UDP test even
passed, yet `dig @<vip>` hung.

The fix adds two `cgroup_sock_addr` hooks on the same `cgroup_attach_path`:
**`cgroup/sendmsg4`** (forward: VIP→backend over `LOCAL_BACKEND_MAP`) and
**`cgroup/recvmsg4`** (reply: backend→VIP source rewrite over a NEW
`REVERSE_LOCAL_MAP`), filled by a **reverse-first dual-write** inside the
existing `register_local_backend` (no new trait method). A shared
`#[inline(always)]` `build_local_service_key` helper (key-build + NBO only)
is consumed by all three hooks; the per-hook map lookup and rewrite
direction stay in each program body. Below Tier-3 — where
`cgroup_sock_addr` has no `BPF_PROG_TEST_RUN` backstop — the structural
defense is a Tier-1 `reply_source_rewrite_lockstep` DST equivalence
invariant.

## Business context

DNS resolvers, QUIC clients, and most game/syslog UDP clients use the
unconnected `sendto` idiom; the connected-UDP path delivered in
`udp-service-support` (#163) covered only the minority that `connect()`
first. UDP-bearing services are first-class workloads under vision
principle 4. #200 made the *common* case quietly broken — the worst class
of dataplane defect, because every control-plane signal is green and the
failure surfaces only as a real client timeout.

The job served is the existing **J-OPS-004** operator-trust reachability
contract ("trust the wire signal for a Service-kind workload") + the
**J-PLAT-004** Sim≡kernel equivalence contract, now extended to the
same-host *unconnected*-UDP reachability dimension. No new job was minted
(the `udp-service-support` D5 precedent: fragmenting operator-trust per
client idiom would owe a new job for the next idiom). The single force that
drove the design: **the anxiety of silent asymmetry** — a forward-only or
asymmetric reply-path change of the #163 class can pass every existing test
and ship a half-working service. The whole intent is to convert that
silence into a loud, mechanical PR-time gate failure.

## Key decisions

### DIVERGE — the option study (SSOT: `wave-decisions.md`, `recommendation.md`)

Six options scored on a locked developer-tool taste matrix. Ranking:
**Option 2 (`sendmsg4` + `recvmsg4`, 4.65)** > Option 3 (4.07) > Option 1
(sendmsg4-only, 3.48) > Option 6 (3.05) > Option 4 (2.65) > Option 5 (2.48).
iptables / IPVS were DVF-eliminated at the option-set boundary (vision
principle 2). Option 2 won as the only clean VIP-sourced-reply,
exact-kernel-design-parity, pure-addition option, driven by DVF (5.00) and
the delivery-is-real taste criterion (5) on the reply-path discriminator.

- **D3 — recvmsg4 is load-bearing, not optional.** Kernel commit
  `983695fa6765` demonstrates a source-validating resolver rejecting a
  backend-sourced reply; the fix IS recvmsg4. Resolved #200's
  "consider recvmsg4 (verify; may be out of scope)" hedge → required.
- **D5 — kernel viability is not a blocker.** sendmsg4 (≥4.18), recvmsg4
  (≥4.20), `bpf_sock_addr.protocol`/`user_ip4`/`user_port`
  populated/writable for these contexts — all below the 5.10 LTS floor
  (fresh-verified 2026-06-05). No matrix bump.
- **D7 — reverse store, not conntrack.** UDP is stateless; the reply store
  is a second BPF map written alongside the forward registration, not a
  per-flow conntrack table.
- **Dissent:** Option 1 (sendmsg4-only) wins only if the Phase-1 UDP client
  model is non-source-validating, or as a documented request-path-first
  interim with a tracked follow-up. User confirmed recvmsg4 IS in scope
  (no split).

### DESIGN — locked decisions (SSOT: **ADR-0053 revision 2026-06-05**, as amended by **D3 sub-revision 2026-06-05b**)

- **DDD-1 — second map `REVERSE_LOCAL_MAP`** (`BPF_MAP_TYPE_HASH`), written
  in **ordered (reverse-first)** sequence by `register_local_backend` (two
  BPF map syscalls — an *ordering* guarantee, not atomicity; the trait
  contract is amended so observers never see a forward entry without its
  reverse). A reverse scan is O(N)/datagram on the recvmsg hot path;
  conntrack models flow state UDP doesn't have; a second point-lookup map is
  the only stateless O(1) reply store.
- **DDD-2 — reverse key = `BackendKey (ip, port, proto)`**, reusing the
  existing newtype. `backend_ip` alone is ambiguous when two services share
  a backend IP on different ports; `BackendKey` reuse buys byte-parity with
  the three existing keys + a free Sim mirror. Two-VIPs→one-identical-backend-
  socket is last-writer-wins operator misconfiguration (named, not a silent
  assumption).
- **DDD-3 (CORRECTED 2026-06-05b, UI-1) — rewrite-on-HIT, pure NO-OP on
  MISS.** recvmsg4 on a `REVERSE_LOCAL_MAP` **HIT** rewrites the reply source
  backend→VIP (the map hit is the "this is a service reply" discriminator);
  on a **MISS** it leaves the real source byte-for-byte intact and bumps
  `REVERSE_LOCAL_MISS_COUNTER` for observability only. recvmsg4 **cannot
  deny** — the verifier restricts its return value to exactly `[1,1]`; a
  program returning 0 is rejected at load time. **This corrects the original
  D3 "rewrite-to-sentinel `192.0.2.1` on miss" decision** — see § "The UI-1
  back-propagation arc" below.
- **DDD-3a — layer boundary: application sockaddr, NOT wire.** recvmsg4 fires
  inside `udp_recvmsg()` AFTER the kernel populated the source sockaddr from
  the backend's skb; a `tcpdump -i lo` sees the backend source on every
  round-trip regardless. recvmsg4's domain is the application
  `recvfrom`/`msg_name` sockaddr only; wire no-leak is XDP's concern (the
  out-of-scope connected/remote REVERSE_NAT path). The DISCUSS
  US-01/US-03/K2/K5 ACs were reframed from wire (`tcpdump`) to
  application-sockaddr layer (back-prop CA-2).
- **DDD-4 — Option 3 shared `build_local_service_key` helper** (key-build +
  `user_port` low-16-NBO ONLY) across connect4 + sendmsg4 + recvmsg4. The map
  lookup and rewrite direction stay per-hook (connect4/sendmsg4 →
  `LOCAL_BACKEND_MAP` forward dest-rewrite; recvmsg4 → `REVERSE_LOCAL_MAP`
  reverse source-rewrite). **One helper MUST NOT serve both rewrite
  directions.** User override of the architect's Option-2; **refactors
  shipped connect4** (behavior-preserving, Tier-3-reverified — see back-prop
  CA-1). Reviewer finding F-1 renamed the helper from `local_backend_lookup`
  to `build_local_service_key` to stop the name overselling a shared lookup.
- **DDD-5a — dual-write in `register_local_backend`; NO new trait method**;
  contract rustdoc amended (postcondition + observable invariant + edge
  case).
- **DDD-5b/c — probe attaches both hooks; the `attach()` syscall IS the
  below-floor preflight** (no `/proc`/`uname` parsing — avoids the
  `unwrap_or_default` boundary-read footgun); `#[from]`-routed error
  variants, never `Internal(String)`.
- **DDD-5d — `SimDataplane` reply mirror** `BTreeMap<BackendKey, Ipv4Addr>`
  under the SAME mutex acquisition as `local_backends`; `reply_source_for()`
  test accessor; models the observable contract only, does not shape
  production.
- **DDD-5e — the `user_port` low-16-NBO read idiom** lives verbatim in the
  shared helper; recvmsg4 writable fields confirmed = `user_ip4`/`user_port`
  (`msg_src_ip4` is sendmsg-only).

### The UI-1 back-propagation arc (the load-bearing decision of this feature)

The original DESIGN D3 (user-locked, research-backed) was *"on a reverse-map
miss, rewrite the reply source to a non-backend sentinel `192.0.2.1` (RFC
5737), counted — strictly stronger than Cilium's pass-through-leak."* During
DELIVER **step 01-03** this proved **unworkable**: `cgroup/recvmsg4` attaches
at a cgroup **ancestor** (`overdrive.slice`) and fires on **every**
unconnected-UDP `recvmsg` from **any** descendant — not only service-VIP
replies. A `REVERSE_LOCAL_MAP` miss therefore overwhelmingly means *"this
datagram is not a service reply at all"* (a backend's own inbound-query
`recvfrom`; any unrelated same-host UDP), NOT *"a service reply whose reverse
entry is missing."* Sentinel-ing every miss mangled the sender address every
non-service datagram's app read — it broke the unconnected round-trip AND the
connected-UDP K4 path until caught (Tier-3-observed).

The correction was re-adjudicated by **fresh research** (the research doc's
"Addendum — UI-1 adjudication (2026-06-05)" — verdict: **crafter CORRECT,
the prior research Q5 "strictly stronger than Cilium" was a category error**,
it had assumed a miss = lost service reply). The corrected behavior is
**rewrite-on-HIT, no-op-on-MISS** — which is what Cilium actually does
(`cil_sock4_recvmsg` returns `SYS_PROCEED`; `__sock4_xlate_rev` leaves the
source unchanged on a reverse-SK miss). The K5 no-leak guarantee is preserved
by a **different mechanism**: the reverse-first dual-write guarantees every
registered backend has a visible reverse entry before its forward entry is
usable, so a genuine service reply ALWAYS hits → always VIP-rewritten; there
is no backend-IP-leak path, and a miss is by definition non-service traffic
whose real source must be preserved.

The correction was back-propagated **across all artifacts in one arc**:
ADR-0053 D3 sub-revision 2026-06-05b; the research addendum; `brief.md`;
`feature-delta.md` (DDD-3, CA-3, decisions table, C4, tech-choices, open
questions); `design/upstream-changes.md` Change A § A2; and the
acceptance-designer's re-scope of S-03-01 + K5/US-03 (test renamed
`reverse_miss_rewrites_source_to_sentinel_not_backend_ip` →
`non_service_unconnected_udp_reads_real_source_recvmsg4_noop_on_miss`). The
dead `SENTINEL_SOURCE_HOST` was deleted (commit `6c231ebf`); stale
sentinel-on-miss comments were corrected to no-op (commit `fe237d36`). Full
record: `deliver/upstream-issues.md` § UI-1.

**The lesson:** a user-locked, research-backed decision proved wrong at the
implementation bar; the correct response was not to force the decision but to
re-adjudicate it with fresh research, correct it across every artifact, and
record the prior research's error honestly.

## Four-tier coverage — and the deliberate Tier-2 gap

`cgroup_sock_addr` programs return **ENOTSUPP** under `BPF_PROG_TEST_RUN` on
kernel ≤ 6.8 (`cg_sock_addr_verifier_ops.test_run` is null in mainline). So
the two new programs have **NO Tier-2 triptych** — deliberately not
scaffolded. This is the same constraint that governs the shipped connect4
hook (`.claude/rules/development.md` § "`bpf_sock_addr.user_port`").

| Tier | Surface | Coverage |
|---|---|---|
| **Tier 1 (DST)** | `reply_source_rewrite_lockstep` invariant on the per-PR critical path | The structural defense below Tier-3: asserts the `SimDataplane` reply source = the VIP for the declared frontend; a forward-only Sim mutation turns it RED. The J-PLAT-004 piece. **PRESENT + GREEN.** |
| **Tier 2 (BPF unit)** | — | **NONE** (`cgroup_sock_addr` ENOTSUPP under `BPF_PROG_TEST_RUN`). Not a gap to fix — a kernel constraint. |
| **Tier 3 (real kernel)** | 9 acceptance scenarios (Lima, `cargo xtask lima run --`), real `dig`/`sendto`/`recvfrom` round-trips, real cgroup attach, veth, per-test bpffs pin-dir, fixtures binding off systemd-resolved's UDP 5353 | **THE correctness gate.** Forward + reverse map presence after one registration; VIP-sourced reply at the app sockaddr; no-op-on-miss; below-floor refusal. |
| **Tier 4 (verifier/perf)** | the recvmsg4 `[1,1]` cannot-deny return-range constraint is a kernel-verifier fact, not a perf gate | The verified half the design rests on. |

The honest signal is **Tier-1 invariant (present + green) ∪ Tier-3 acceptance
(the gate)** — neither alone; there is no Tier-2 middle for this hook class.

## Steps completed

All 7 DELIVER steps reached COMMIT/EXECUTED/PASS (execution-log.json,
2026-06-05). DES integrity over the deliver tree: **"All 7 steps have
complete DES traces", exit 0.** `RED_UNIT` was `SKIPPED — NOT_APPLICABLE` on
the cgroup-hook steps because `cgroup_sock_addr` has no Tier-2 backstop and
the userspace key/value + dual-write surface is already covered by the
shipped `ReverseLocalMapHandle`/`LocalBackendMapHandle` host-order proptests
(step 01-02) and the Tier-1 reply-mirror equivalence (step 02-01).

| Step | Story | Outcome | Commit |
|---|---|---|---|
| **01-01** | US-01 | D4 shared `build_local_service_key` helper; connect4 refactored to consume it (behavior-preserving, Tier-3-reverified) | `44aa6fbc` |
| **01-02** | US-01 | `REVERSE_LOCAL_MAP` kernel map + `ReverseLocalMapHandle` (host-order proptests) | `4dbece77` |
| **01-03** | US-01 | sendmsg4 + recvmsg4 hooks + reverse-first dual-write; **UI-1 no-op-on-miss correction landed here** | `e71ad780` (+ `692a7a64` UI-1 docs) |
| **02-01** | US-02 | `SimDataplane` reply-mirror write + `reply_source_rewrite_lockstep` Tier-1 invariant | `7a591a4f` |
| **02-02** | US-02 | S-02-03 Tier-3 reply-source equivalence pin meets the Tier-1 reply mirror at backend identity | `eb6baacb` |
| **03-01** | US-03 | recvmsg4 no-op-on-miss hardening + delete dead sentinel | `6c231ebf` |
| **03-02** | US-03 | typed below-floor attach refusal + `REVERSE_LOCAL_MAP` probe round-trip | `a8585f9b` |

### Quality gates

- **Phase 3.5 post-merge integration gate:** PASS — 8 Tier-3 + 184 sim green
  under Lima.
- **Phase 3 refactor:** done — stale sentinel-on-miss comments corrected to
  no-op (`fe237d36`).
- **Phase 4 adversarial review:** APPROVED — zero testing theater. (The
  per-step `RED_UNIT` skips were validated as honest: a default-lane `Display`
  unit test on a `derive(Error)` would be redundant Testing-Theater on a hook
  with no Tier-2 surface; the typed `#[from]`/variant routing is Tier-3-asserted
  via the real `EbpfDataplane::probe` path.)
- **Phase 5 mutation:** **100% kill rate** after closing the one missed mutant
  — `ReverseLocalMapHandle::remove` no-op (commit `7b78c2bc`).
- **Per-wave peer reviews:** DISCUSS APPROVED (Eclipse, 9/9 dimensions);
  DESIGN APPROVED (Atlas, 0 blocking/critical/high; 4 low findings F-1…F-4 all
  actioned in-revision or carried to DELIVER).

## Outcome measurement (K1–K5 first-GA baselines)

> **Why these baselines live here, not in `docs/product/kpi-contracts.yaml`:**
> that file is the **docs-platform** feature's single-feature DEVOPS
> instrumentation contract (KPI-1..6 for the website). Injecting this
> feature's K1–K5 would corrupt an unrelated SSOT — a decision explicitly
> taken at DISTILL (feature-delta § DISTILL "KPI contracts"). The K1–K5
> definitions/targets are owned by DISCUSS (feature-delta § Outcome KPIs); the
> first-GA measured baselines are recorded here, the feature's permanent home,
> so future deltas have a reference point. A scoping note was added to the top
> of `kpi-contracts.yaml` pointing here.

| # | KPI (North Star = K2) | Target | First-GA measurement | Method |
|---|---|---|---|---|
| **K1** | unconnected `dig @<vip>` round-trip reachability | 100% return a correct answer | **ACHIEVED** — Tier-3 walking-skeleton S-01-01 green (real unconnected `sendto`/`recvfrom` round-trip) | Tier-3 acceptance, Lima |
| **K2** | reply source the client app reads = VIP (**North Star — the discriminator**) | 100% VIP-sourced; 0 backend-IP-sourced to the app | **ACHIEVED** — Tier-3-verified at the application sockaddr layer (S-01-01/S-01-02) | Tier-3 app-sockaddr assertion (NOT `tcpdump` — DDD-3a) |
| **K3** | Sim≡kernel reply-path equivalence | invariant present + green every PR; forward-only mutation killed | **ACHIEVED** — `reply_source_rewrite_lockstep` present + green; the forward-only Sim mutation is killed (Phase 5, 100% kill rate) | Tier-1 DST invariant + Tier-3 meet-at-backend-identity (S-02-03) |
| **K4** | add the unconnected path without re-migrating connect4 call sites | 0 net-new connect4 behavior | **ACHIEVED (restated)** — connect4's key-build refactored to the shared helper (non-zero diff, behavior-preserving, Tier-3-reverified); forward-map shape / action variants / hydrator classifier UNCHANGED. (The DISCUSS "0 diff / pure addition" claim was corrected to "0 net-new behavior" — back-prop CA-1.) | PR diff review + connected-round-trip Tier-3 re-run |
| **K5** | reply path fails safe on reverse miss / below floor | 0 backend-IP leak to the app; below-floor hosts refuse observably | **ACHIEVED (mechanism corrected)** — no-leak holds via the reverse-first dual-write (always-hit → always-VIP-rewritten), NOT a sentinel; below-floor attach fails → `health.startup.refused` (S-03-01/S-03-02). | Tier-3 no-op-on-miss + below-floor acceptance |

### Residual operational note (DESIGN NOTE — not a tracking issue)

**`REVERSE_LOCAL_MISS_COUNTER` operational semantics (for DEVOPS).** Because
recvmsg4 fires on ALL subtree unconnected UDP, the counter increments on
every non-service recv (DNS clients, backend inbound-query recvs, unrelated
same-host UDP) — its absolute value is dominated by non-service traffic and
is NOT a "service reply failed to translate" alarm; it cannot isolate the
should-never-happen evicted-reply case from routine non-service misses.
Whether to keep, demote, or replace it (e.g. a control-plane reconciler
comparing forward-vs-reverse map cardinality, or a `bpftool map dump`
differential) is a **metric-semantics decision for DEVOPS** — recorded as a
DESIGN NOTE (feature-delta § Open questions #1), **NOT** a GitHub issue (per
`feedback_no_unilateral_gh_issues`). The no-op-on-miss behavior is correct
regardless of how the counter is treated.

## Verification catalogue (EDD)

**This feature has NO separate `verification/` catalogue** — neither a
feature-dir catalogue nor repo-root expectations were minted for it. Its
operator-surface behavior is asserted directly by the **9 Tier-3 acceptance
tests** (real `dig`/`sendto`/`recvfrom` round-trips under Lima), which ARE the
executable evidence the catalogue would otherwise pin. Per
`.claude/rules/verification.md`, an in-process/operator surface already
covered by an executed test tier does not also get a duplicated catalogue
expectation (duplication dilutes the signal). There is therefore nothing to
archive into `docs/evolution/unconnected-udp-sendmsg4/verification/` — this is
recorded explicitly rather than dropped silently as a missed obligation.

## Issues encountered

- **UI-1 — D3 "sentinel on miss" was unworkable** (the central back-prop arc
  above). Caught at DELIVER step 01-03; corrected to no-op-on-miss across all
  artifacts. The clean takeaway: recvmsg4's cgroup-ancestor attach scope means
  a reverse-map miss is non-service traffic, not a lost service reply.
- **Research Gap 1 (non-blocking).** The exact verifier `check_return_code`
  file:line for the recvmsg4 `[1,1]` range and the v5.10 `udp_recvmsg`
  `RECVMSG_LOCK` call site were not pinned (raw source fetch blocked). The
  *facts* are established by the selftest error string + the commit hunk; only
  the line citation is missing. Optional for a future local 5.10 checkout.

## Lessons learned

- **A user-locked, research-backed decision can still be wrong at the
  implementation bar — re-adjudicate, don't force it.** UI-1: the correct move
  was fresh research that overturned the prior Q5, then a clean back-prop
  across every artifact, recording the prior research's error honestly.
- **Attach scope determines miss semantics.** A cgroup-ancestor hook fires on
  ALL descendant traffic, so a map miss ≠ "my thing failed"; it means "this
  isn't my thing." The map HIT is the discriminator. This is why no-op-on-miss
  is correct and sentinel-on-miss corrupts unrelated traffic.
- **The layer a hook can honor is not always the layer the AC was written in.**
  recvmsg4 owns the application sockaddr, not the wire; the wire ACs were a
  layer error (XDP's domain), corrected to app-sockaddr without losing intent.
- **A shared helper across N hooks necessarily refactors the shipped Nth
  hook.** Option 3's single key-build site is genuinely better, but "0 connect4
  changes / pure addition" was Option-2-specific framing — under the locked
  Option 3, connect4 is EXTEND (behavior 0, diff non-zero, Tier-3-reverified).
  Name the refactor's regression surface honestly when there's no Tier-2
  backstop.
- **No Tier-2 backstop is a DESIGN fact to plan around, not an in-slice
  surprise.** The Tier-1 equivalence invariant was scaffolded as the
  structural defense below Tier-3 from DISTILL onward, so no step carried an
  "how do we test this without PROG_TEST_RUN?" open question into delivery.

## Permanent artifacts

| Artifact | Location |
|---|---|
| Architecture SSOT | `docs/product/architecture/adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md` (revision 2026-06-05 + D3 sub-revision 2026-06-05b) |
| Brief — shipped component inventory | `docs/product/architecture/brief.md` § "Unconnected-UDP sendmsg4 extension" → "Shipped — Component Inventory (FINALIZE 2026-06-05)" |
| C4 delta | `docs/product/architecture/c4-diagrams.md` § "Unconnected-UDP sendmsg4 + recvmsg4" |
| Research (reply-source rewrite + miss semantics, incl. UI-1 addendum) | `docs/research/dataplane/recvmsg4-reply-source-rewrite-and-miss-semantics-research.md` |
| Feature SSOT (preserved) + slice briefs | `docs/feature/unconnected-udp-sendmsg4/feature-delta.md`, `…/slices/slice-0{1,2,3}-*.md` |
| Acceptance scenario spec (GIVEN/WHEN/THEN, never executed) | `docs/feature/unconnected-udp-sendmsg4/distill/test-scenarios.md` |
| Persona / journey | `docs/product/personas/ana-platform-engineer.yaml`, `docs/product/journeys/reach-an-unconnected-udp-service.yaml` |
| Implementation | `crates/overdrive-bpf/src/programs/cgroup_{sendmsg4,recvmsg4}_service.rs`, `…/src/shared/build_local_service_key.rs`, `…/src/maps/reverse_local_map.rs`; `crates/overdrive-dataplane/src/maps/reverse_local_map_handle.rs`, `…/src/lib.rs`; `crates/overdrive-sim/src/invariants/reply_source_rewrite_lockstep.rs`, `…/src/adapters/dataplane.rs` (see git history `44aa6fbc` → `7b78c2bc`) |
| Tier-3 acceptance | `crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs`, `…/unconnected_udp_reply_hardening.rs` |

## What this unblocks

- **Real same-host DNS** — `dig @<vip>` / `getaddrinfo` against
  `<vip>` in `resolv.conf` now resolve through a same-host UDP service; the
  unconnected idiom is first-class.
- **The reply-path equivalence invariant as a template** — any future
  cross-adapter same-host dataplane invariant follows the Tier-1-DST-defense ∪
  Tier-3-gate shape proven here for a hook class with no Tier-2 backstop.
- **Further `cgroup_sock_addr` reply paths** — the shared key-build + NBO
  helper and the reverse-first dual-write pattern generalize to any future
  same-host source-rewrite hook.

## Links

- ADR-0053 (rev 2026-06-05, D3 sub-rev 2026-06-05b) —
  `docs/product/architecture/adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md`
- Research — `docs/research/dataplane/recvmsg4-reply-source-rewrite-and-miss-semantics-research.md`
- UI-1 back-prop record — `docs/feature/unconnected-udp-sendmsg4/deliver/upstream-issues.md`
- DESIGN back-prop — `docs/feature/unconnected-udp-sendmsg4/design/upstream-changes.md`
- Predecessor (connected-UDP / #163) —
  `docs/evolution/2026-06-03-udp-service-support.md`
- GitHub — [#200](https://github.com/overdrive-sh/overdrive/issues/200)

Feature commits: `44aa6fbc`, `4dbece77`, `e71ad780`, `692a7a64`, `7a591a4f`,
`eb6baacb`, `6c231ebf`, `a8585f9b`, `fe237d36`, `7b78c2bc`.
