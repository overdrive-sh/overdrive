# Evolution — canonical-workload-address-inbound-tproxy (GH #241 · ADR-0071 Path A)

**Finalized:** 2026-06-24 · **Wave arc:** SPIKE → DESIGN → DISTILL → DELIVER
(no DISCUSS/DEVOPS triad — feature started at SPIKE; mechanism settled
empirically by 3 committed Tier-3 spikes) · **Branch:**
`marcus-sa/path-a-inbound-tproxy` · **Architect:** Apex (nw-platform-architect)

> **STATUS — honest, load-bearing.** Implementation complete; all 6 steps GREEN
> on **dev-Lima 7.0.0-22**; **MERGE-GATED on the pinned-6.18 appliance-kernel
> Tier-3 CI run (ADR-0068) — not yet observed.** The S-WS keystone is APPROVED
> *conditional* on that 6.18 Tier-3 matrix; the only evidence on record is
> dev-Lima 7.0. This feature is **not** merged, **not** fully accepted, and the
> 6.18 signal has **not** passed. This document records the work as it stands at
> the close of the artifact-authoring half of finalize.

---

## Feature summary

The **keystone slice** of the transparent-mtls-enrollment arc (GH #236 /
ADR-0071 Path A). It productionises the inbound nft-TPROXY install that
ADR-0071's `start_alloc` deferred (recorded `tproxy_guard = None`) and flips the
`BackendDiscoveryBridge` advertise address from `host_ipv4:port` to the canonical
per-workload `workload_addr:port` — so a workload becomes reachable at its
canonical `workload_addr:service_port` over mTLS, driven end-to-end through
`overdrive serve` + `overdrive deploy`. This closes the **inbound** half of the
Path-A bidirectional mTLS loop that ADR-0071 named as #241's job.

The mechanism is **production wiring + one canonical-address contract**, not a
new subsystem — **zero CREATE-NEW components**. The canonical `workload_addr`
flows as a pure in-memory `AllocationSpec` channel (set at the C3
`provision_and_inject_netns` seam off `plan.workload_addr`, the same channel as
`netns`/`host_veth`), is persisted as an *observed input* on a new
`AllocStatusRow` V2 envelope, and the declared service ports are single-sourced
through a new `WorkloadLifecycle::project_service_listen_ports` (mirroring the
existing `project_probe_descriptors`). `start_alloc` installs one inbound
capture rule per declared service port (N ports → N rules; Job-kind / 0
listeners → 0 rules), keyed on the **declared service port** (not the ephemeral
leg-C port). `ServiceMapHydrator` gains a three-way subnet-membership gate so
Path-A/mesh backends program neither the cgroup `LOCAL_BACKEND_MAP` nor the XDP
path — nft-TPROXY owns delivery, and the dead XDP writes B2's reclassification
would otherwise introduce are prevented.

The end-to-end shape (from the DESIGN architecture summary):

```text
client workload (netns B, /30)                server workload (netns C, /30)
  connect(workload_addr_C:service_port)
    └─ veth egress → host PREROUTING
        └─ nft-TPROXY (ip daddr workload_addr_C tcp dport service_port)   ← install
            └─ leg-C IP_TRANSPARENT listener (getsockname → orig_dst)      ← reused
                └─ mTLS handshake → splice → server workload

bridge advertises Backend.addr = workload_addr_C:service_port             ← D-B2
egress MtlsResolve.by_addr[workload_addr_C] = Mesh                        ← B2's reader
ServiceMapHydrator: workload_addr_C ∈ WORKLOAD_SUBNET_BASE → skip LB      ← D-GATE
  → cgroup_connect4 LOCAL_BACKEND_MAP miss → nft-TPROXY owns delivery
```

## Business context

GH #241 ("Path-A canonical workload address + production inbound mTLS TPROXY
install") is the inbound keystone split out of #178 (CLOSED — decomposed into
#241 / #242 / #243 / #244). Its predecessor, transparent-mtls-enrollment (#236),
built the inbound interception leg (leg-C `IP_TRANSPARENT` listener + accept
loop + server-side mTLS + kTLS + splice) and wired it into `start_alloc` — but
**neither inbound leg could be driven through a real `serve` + `deploy`**,
because the production inbound nft-TPROXY rule that feeds leg-C was deferred
(`tproxy_guard = None`) and the bidirectional test had to hand-install the rule
itself. That was the exact "mechanism built, wiring deferred → dead on the
production path" trap CLAUDE.md § "Build vertical slices through production
entry points" exists to prevent.

This feature is the honest re-size: a thin **canonical-address** slice that
advertises the per-workload `workload_addr` as the routable inbound identity,
installs the inbound rule for it from the production `start_alloc` call site, and
routes between the per-workload `/30`s — the loop that makes inbound *usable*
through `serve` + `deploy`. Intended-peer pinning (#242) and the DNS responder
daemon (#243) are explicitly later, independently-drivable slices, not part of
this cut.

## Key decisions

| D# | Decision | Rationale |
|---|---|---|
| D-A1 | `AllocationSpec.{workload_addr: Option<Ipv4Addr>, service_ports: Vec<NonZeroU16>}` + per-port `install_inbound_tproxy` in `start_alloc` (replacing `tproxy_guard = None`) | `AllocationSpec` is the existing slot-derived per-alloc in-memory channel (no serde/rkyv); `workload_addr` set at the C3 site off `plan.workload_addr`; closes the named #241 deferral. |
| D-BLOCKER1 | Inbound rule keys on `ip daddr <workload_addr> tcp dport <service_port>` = the **declared** Service listener port | D-TME-10 one-source/two-readers: the same value `service_backends` advertises and the egress `MtlsResolve` keys on. NOT the ephemeral `leg_c_addr.port()` (the inert self-referential shape `start_alloc` rejected). |
| D-B2 | Bridge advertises `Backend.addr = workload_addr:port`; `ServiceBackendRow.vip` UNCHANGED | What makes the egress resolve index classify a canonical-addr dial as Mesh (else inbound fail-closed). Proven SAFE by spike increment-c (no live VIP-LB consumer). |
| D-BLOCKER2 | Persist `workload_addr` **materialized** on `AllocStatusRow` (V2 envelope), NOT persist-`NetSlot`-and-recompute | The addr is a slot×base-at-provision join the inbound rule already committed to. Recompute-at-bridge requires a core relocation AND re-derives against a future-tunable base (#239) → install/advertise divergence. Persisting the materialized addr keeps install/observe/advertise byte-identical. The #239 base-change is a redeploy (re-provision + re-observe), not a live re-tune — a documented single-cut-greenfield boundary. |
| D-GATE / D-GATE-PRED | Gate `ServiceMapHydrator` off mesh backends; predicate = `addr.ip() ∈ WORKLOAD_SUBNET_BASE (10.99.0.0/16)` | Content-derived, deterministic; extends the existing partition-on-addr classifier. Cannot misfire (no non-mesh consumer — increment-c). IN-SCOPE for #241 because B2 reclassifies these backends LOCAL→REMOTE; shipping B2 without the gate ships dead XDP writes in this slice. TEACH deferred to #61 (dialable-VIP). |
| D-C1 / D-D1 | Reuse `ensure_shared_routing_infra` (Bar-1 converge-on-boot; Bar-2 → #234); `ip_forward`/`/30` routes/`rp_filter` already converged by `veth_provisioner` | No new boot call site, no new routing primitive (increment-a: the recipe IS the existing production triple). |

### Spike verdicts that settled the design (kernel-pinned evidence)

All three probes ran on **dev-Lima kernel 7.0.0-22-generic** (nftables v1.1.6);
the verdicts are pinned to that kernel. Per the spike constraints, the
authoritative re-confirmation is the pinned-6.18 appliance-kernel Tier-3 matrix
(ADR-0068) when the slice lands — all primitives used predate 6.18.

- **increment-a** (`spike/findings.md`, DISCARD-gated, no production code
  touched): **WORKS.** /30 routing + inbound TPROXY capture proven; orig-dst
  recovered exactly (`10.99.0.6:18241` = `workload_addr_C:port`). The routing
  recipe is **exactly** the existing production triple
  (`ensure_shared_routing_infra` + `install_inbound_tproxy` +
  `make_transparent_listener`) — **no new routing primitive needed.** Two
  sharpening findings: capture is **forwarding-independent** (the TPROXY fwmark +
  `local` route divert before the forwarding decision), and `rp_filter`
  relaxation is **NOT load-bearing** for the host-local-termination topology
  (both sub-probes pass with strict `rp_filter=1`).
- **increment-b** (`spike/findings-cgroup-firing-scope.md`): the
  `cgroup_connect4_service` LB hook (ADR-0053) **FIRES** for Path-A
  netns+cgroup connects → "just retire it" is FALSIFIED → the reconciliation
  must **GATE or TEACH**.
- **increment-c** (`spike/findings-vip-lb-inert.md`): under a real `serve` +
  `deploy`, the VIP/XDP-LB path has **no live v1 consumer** → B2 is SAFE → **GATE
  is correct and sufficient; TEACH is unnecessary** until a dialable-VIP path
  ships (#61).

## Steps completed

All 6 steps reached COMMIT/PASS (legacy 5-phase TDD contract; SKIP reasons
logged as NOT_APPLICABLE where a phase had no surface). From
`execution-log.json` and `roadmap.json`:

| Step | Name | Outcome |
|---|---|---|
| 01-01 | `AllocStatusRow` V1→V2 rkyv envelope (additive `workload_addr`) | COMMIT/PASS (S-V2; FIXTURE_V1 untouched, V2 + offset re-pin same commit) |
| 01-02 | `AllocationSpec` fields + C3 injection + listen-port projection + exit-observer copy | COMMIT/PASS; **mutation 100%** on `project_service_listen_ports` |
| 02-01 | Bridge advertises `workload_addr:port` + `RunningAllocSet` Set→Map + `hydrate_actual` | COMMIT/PASS (S-BRIDGE both arms, S-PORTSET `@property`) |
| 02-02 | `ServiceMapHydrator` GATE — three-way subnet-membership split | COMMIT/PASS; **mutation 100%** on the partition (mesh + two non-mesh arms) |
| 03-01 | `start_alloc` per-port inbound install (replace `tproxy_guard = None`) | COMMIT/PASS (S-NRULES / S-DPORT / S-JOB0 Tier-3 real nft) |
| 03-02 | **Keystone** Tier-3 bidirectional mesh e2e on the production inbound rule (S-WS) | COMMIT/PASS (dev-Lima 7.0; **merge-gated on 6.18 Tier-3 CI**) |

### The 03-02 keystone — two production walls, honestly surfaced

The keystone (S-WS) is the vertical-slice gate: it drives the production
composition root **in-process** (real `run_server` boot + the real in-process
deploy/stop handlers for two mesh workloads) on the **REAL** `EbpfDataplane`
(`dataplane_override: None`; only injected seam = `mtls_identity_override` test
PKI), with the litmus being the **transitive** successful mTLS round-trip — no
`LOCAL_BACKEND_MAP` inspection. It took several dispatches and surfaced two
genuine production walls before going GREEN; the crafter refused to mask either
(declining to log `COMMIT PASS` against an incomplete deliverable — execution-log
t=17:41, t=18:40), surfacing the design/boundary contradictions instead of
inventing surface:

1. **Placement / boundary contradiction (R1).** The keystone was first authored
   in the worker test tree, but in-process `run_server` / `ServerConfig` /
   `mtls_identity_override` all live in `overdrive-control-plane`, and
   `overdrive-control-plane` depends-on `overdrive-worker` — so a worker test
   physically cannot reach `run_server` (a reverse edge is a Cargo-rejected
   cycle). Resolved by relocating the keystone to the control-plane test tree
   (the only crate from which `run_server` is reachable), with the synthetic-virt
   removal staying in the worker tree.

2. **Wall 1 — `FinalizeFailed{Stable}` tore down a live Running alloc
   (`f034f38f`).** The first real run hung at connect: the convergence path
   reaped a live alloc's netns/inbound-rule when a `Stable` `FinalizeFailed`
   fired (a success claim, not a terminal). The production fix gates
   `worker.stop_alloc` + `teardown_and_release_netns` on `!is_stable` in the
   `FinalizeFailed` arm; a genuine terminal (`FinalizeFailed{Failed}`) and the
   operator `StopAllocation` path both still reap unconditionally. Paired
   regression tests pin **both** gate directions (the bug AND the over-gating
   guard). Review Finding B traced both terminal paths and confirmed no
   netns/slot leak.

3. **Wall 2 — test-side `StopAllocation`-teardown gap (test fix, not
   production).** After the convergence fix, connect succeeded (Leg-1 routing
   fixed) but teardown blocked to the 120 s slow-timeout: the tokio multi-thread
   runtime drop blocked on the per-alloc mTLS inbound accept-loop
   `spawn_blocking` thread that `ServerHandle::shutdown` does not stop. The
   crafter's first hypothesis ("requires a production change") was reversed
   **honestly** (execution-log t=18:40→t=18:58) to the correct diagnosis: a
   test-side gap — the resolution drives the production stop verb
   (`StopAllocation` → `worker.stop_alloc`) and polls obs to Terminated *before*
   shutdown, which exercises *more* production path, not less. Consistent with
   the persistent-workload model (a Running alloc's netns surviving a CP shutdown
   is by design, not a leak).

The reply-leg behaviour (Wall 2's earlier "byte-exact reply did not return")
was correctly classified as a **test-composition** gap (echo vs distinct-constant
`REQUEST != RESPONSE`), with the production reply pipe proven working at the wire
in the two RCA docs (`docs/analysis/root-cause-analysis-canonical-address-inbound-{roundtrip-hang,reply-leg}.md`,
pasted tcpdump/bpftrace/server-log evidence) — not a production defect.

## Lessons learned

- **A mechanism that composes in a test is not a vertical slice.** The #236
  precedent (the test hand-installing the omitted production call site) is the
  reason this feature exists; the keystone's litmus — *delete the 03-01
  production install → keystone goes RED* — is the structural defense, and it
  holds via the client-side rustls handshake (not map inspection). The proof is
  the round-trip on the production path, not green unit assertions.
- **Persist the materialized join when the downstream contract has already
  committed to it.** D-BLOCKER2 is a deliberate exception to "persist inputs, not
  derived state": the inbound nft rule is installed against `workload_addr =
  base_t0 + slot*4 + 2`, so recompute-at-bridge against a future-tunable base
  (#239) would advertise an addr no rule captures. The materialized addr IS the
  input the contract depends on. The single-cut-greenfield boundary ("base change
  = redeploy, not live re-tune") is what makes this honest rather than a stale
  cache.
- **Surfacing a boundary contradiction costs one message; inventing past it costs
  a rework cycle.** The R1 placement contradiction and the two production walls
  were each surfaced as blockers rather than worked around — no test-only
  production CLI surface was invented to force green (review Finding A confirmed
  with git evidence: `61edf95d` touched zero `src/` files; `f034f38f` touched
  only `action_shim/mod.rs`).
- **A first hypothesis reversed honestly is the discipline working, not
  failing.** Wall 2's "production change required" → "test-side gap" reversal,
  recorded in the execution log with its reasoning, is the debugging.md §4
  discipline (predict, falsify, re-model) under delegation.

## Issues / deferrals

All deferrals cite **verified-OPEN** GitHub issues (re-checked at finalize);
none were created by this finalize, none invented.

| Item | Disposition | Issue |
|---|---|---|
| E04 black-box mesh-mTLS E-surface capture (real `serve` + real `deploy` ×2, no test PKI) | DEFERRED — needs a converged full-system deployment + the production CA→SVID→leg-C path proven black-box (no `mtls_identity_override` seam) | **#227** (EDD harness) on **#75** (Image Factory MVP) — both OPEN |
| Intended-peer SVID pinning (`expected_peer` SAN-match) | Out of scope (v1 authn-only) — a later, independently-drivable slice | **#242** (OPEN) |
| In-agent name responder (dial-by-name) | Out of scope (workload dials concrete `workload_addr` directly — the thin live loop) — a later, independently-drivable slice | **#243** (OPEN) |
| Shared-routing-infra Bar-2 reconciler | Deferred (Bar-1 converge-on-boot suffices for single-node v1) | **#234** (OPEN) |
| Tunable `WORKLOAD_SUBNET_BASE` | Deferred; the BLOCKER-2 single-cut-greenfield risk documented and accepted for single-node v1 | **#239** (OPEN, phase/2+) |
| VIP-dial path / multi-node VIP-LB (TEACH trigger) | Out of scope (no live VIP-dial consumer; the VIP *allocator* #167 already shipped) | **#61** (OPEN) |

**F-1 (informational follow-up candidate, NOT a deferral needing an issue).**
The L1–L6 refactor pass surfaced a duplicated 4-line "record a dispatch in the
View" mutation in `service_map_hydrator::reconcile` (the V6-VIP arm and the
V4-path tail). It is **pre-existing in `origin/main`** — both copies predate this
feature; this feature only wrapped the V4-path copy in a guard. Collapsing it
(a private `record_dispatch_attempt` helper, no public surface) is a valid,
behavior-preserving future refactor of the hydrator's dispatch-bookkeeping, out
of scope for this feature's hunks. Recorded informationally; no GitHub issue
required because this feature did not introduce it.

## Status conditions for merge

Mirror of `deliver/03-02-review.md` § "Merge conditions":

1. **Pinned-6.18 Tier-3 CI must pass** (ADR-0068 gating kernel). dev-Lima 7.0
   GREEN is the inner loop, **not** the merge signal — and the 6.18 signal has
   not yet been observed.
2. **E04 black-box capture stays deferred** to #227 on #75 — correct; not a
   blocker for this slice.

## Preserved feature-dir artifacts

`docs/feature/canonical-workload-address-inbound-tproxy/` is **preserved** (the
wave matrix derives status from it). The lasting artifacts referenced by this
evolution record:

- `feature-delta.md` — the lean SSOT (DESIGN/DISTILL/DELIVER `[REF]` sections).
- `design/wave-decisions.md` — the locked DESIGN decisions + the two design
  reviews (APPROVE-WITH-FIXES → APPROVED post-citation-fix).
- `spike/findings{,-cgroup-firing-scope,-vip-lb-inert}.md` + `spike/wave-decisions.md`
  — the three kernel-pinned spike verdicts (the empirical ground for the design).
- `deliver/{roadmap.json,execution-log.json,refactoring-log.md,03-02-review.md,02-02-review.md,03-01-review.md}`
  — the DELIVER audit trail and per-step reviews.
- `docs/analysis/root-cause-analysis-canonical-address-inbound-{roundtrip-hang,reply-leg}.md`
  — the two keystone-wall RCA docs (already in a permanent location).
- `verification/expectations/E04-workload-reachable-at-canonical-address-mtls/`
  — the E04 expectation stub (`pending`; capture deferred to #227 on #75).
