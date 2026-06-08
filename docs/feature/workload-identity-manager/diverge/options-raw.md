# Options (raw, unfiltered) — workload-identity-manager (GH #35)

**Wave**: DIVERGE (Phase 3 of 4) · **Agent**: Flux (nw-diverger) · **Date**: 2026-06-08

> **Separation principle (Osborn 1953).** This file is GENERATION ONLY. No scoring, no
> ranking, no "this is best / this won't work / this is risky" language. Every option is
> described neutrally. Evaluation happens exclusively in `taste-evaluation.md` (Phase 4).
>
> **Hard constraint that bounds the space (not an evaluation — a correctness rule).**
> Per `.claude/rules/development.md` § "Reconciler I/O", reconcilers are **pure-sync**:
> `reconcile(desired, actual, view, tick) -> (Vec<Action>, View)` — no `.await`, no I/O,
> no `Ca` handle, no wall-clock. So "issue an SVID synchronously via `Ca::issue_svid`"
> means the reconciler **emits a typed `Action`**; the **action-shim executor** calls the
> CA and writes the result. Any option placing `Ca` I/O inside `reconcile()` is a *rule
> violation* and is noted as such where it arises — it is excluded structurally, not on
> taste grounds. The shipped mirror is `ServiceMapHydrator` → `Action::DataplaneUpdateService`
> → `action_shim/dataplane_update_service.rs` executor.

---

## Phase 1: HMW framing

The feature request names a mechanism (`IdentityMgr`, an `Arc`, a reconciler). The HMW
must strip the mechanism and open the space, per the validated job J-SEC-002.

- **Feature-shaped (rejected):** "How might we build an `IdentityMgr` that holds SVIDs?"
- **Solution-embedded (rejected):** "How might we add an `Arc<RwLock<BTreeMap>>` for SVIDs?"
- **HMW (chosen):**

> **How might we make sure every running workload's live, chain-verifiable identity is
> readable by the dataplane consumers that must present it — and held for exactly as long
> as the workload runs — without an in-workload agent?**

The chosen HMW opens the real space: the answer need not be a struct called `IdentityMgr`,
need not be an `Arc<RwLock<map>>`, need not even be "a held store" (a consumer could read
the View directly, or an observation row, or a kernel map). It keeps the four job axes
(O1 held-when-running, O2 dropped-when-stopped, O3 readable-by-consumers, O4
idempotent-across-restart) in view without prescribing the mechanism.

---

## Phase 2: SCAMPER — one option per lens

### S — Substitute: replace the in-memory store with a kernel BPF map (`IDENTITY_MAP`)

**Core idea:** Instead of an in-process Rust struct holding SVIDs, the held material is
written into a kernel-side BPF map (`IDENTITY_MAP`, the roadmap's named map for step 2.13)
keyed by allocation/connection identity; the sockops/kTLS consumer reads identity straight
from the kernel map it already operates in, the way `ServiceMapHydrator` writes
`SERVICE_MAP`.
**Key mechanism:** A `SvidLifecycle` reconciler emits `Action::IssueSvid`; the executor
mints via `Ca::issue_svid`, writes the leaf/key handle (or a reference) into the BPF map
via a dataplane port; the kernel-side mTLS layer reads the map.
**Key assumption:** Identity material (or a usable reference to it) can live in / be keyed
by a BPF map that the kernel-side handshake path can consume, and the userspace gateway/
telemetry consumers can read the same map.
**SCAMPER origin:** S (Substitute the storage substrate: Rust heap → kernel map).
**Closest competitor:** Cilium's identity maps; this project's own `SERVICE_MAP` hydrator.

### C — Combine: fold SVID lifecycle into the existing `WorkloadLifecycle` reconciler

**Core idea:** Don't add a new reconciler. `WorkloadLifecycle` already observes every
allocation's Running↔Stopped transition and already emits `StartAllocation`/`StopAllocation`.
Extend it to also emit `Action::IssueSvid` when an alloc reaches Running and
`Action::DropSvid` when it stops; the executor writes a shared `IdentityMgr`.
**Key mechanism:** The same reconcile body that decides start/stop also decides
issue/drop; one reconciler, one View, two new action variants. The held store
(`Arc<IdentityMgr>`) is hydrated by the executor.
**Key assumption:** The identity lifecycle is *the same lifecycle* as the workload
lifecycle and benefits from sharing one reconciler's state/decision surface rather than
running as a separate convergence target.
**SCAMPER origin:** C (Combine the identity job with the adjacent workload-lifecycle job).
**Closest competitor:** Kubernetes' kubelet doing both pod lifecycle and (via projected
service-account tokens) identity provisioning in one component.

### A — Adapt: borrow istio's SDS push model — a `watch`-channel read surface

**Core idea:** Adapt the istio-agent/Envoy SDS "push down a subscription" pattern to an
in-process Rust shape: `IdentityMgr` exposes a `tokio::sync::watch` (or
`tokio::sync::broadcast`) channel per identity (or one channel of "identity set changed");
consumers *subscribe* and are *notified* when their SVID changes, rather than polling a
getter.
**Key mechanism:** A `SvidLifecycle` reconciler emits `Action::IssueSvid`/`DropSvid`; the
executor updates the held store AND sends on the watch channel; sockops/gateway/telemetry
hold a `watch::Receiver` and react to changes.
**Key assumption:** Consumers benefit from change-notification (push) over on-demand
read (pull) — and the future #40 rotation will push a rotated SVID down the same channel,
mirroring SDS's no-restart swap.
**SCAMPER origin:** A (Adapt istio SDS push-on-change into an in-process watch channel).
**Closest competitor:** istio-agent `SecretManager` → SDS `StreamSecrets` push;
Envoy hot-swap on SDS update.

### M — Modify/Magnify: amplify the read surface into a typed `IdentityRead` port trait

**Core idea:** Make the *read surface* the centerpiece. Define an `IdentityRead` port trait
in `overdrive-core` (`fn svid_for(&AllocationId) -> Option<SvidMaterial>`, `fn
current_bundle() -> TrustBundle`) that `IdentityMgr` implements; every consumer
(sockops/gateway/telemetry) depends on `Arc<dyn IdentityRead>`, never on the concrete
struct — matching the project's `Clock`/`Transport`/`Ca` port-trait discipline.
**Key mechanism:** A `SvidLifecycle` reconciler + `Action::IssueSvid`/`DropSvid` executor
populate the concrete `IdentityMgr`; consumers and their tests are written against the
trait, with a `SimIdentityRead` for DST.
**Key assumption:** The consumer-facing contract is the load-bearing surface and is worth
elevating to a testable, mockable port (so #26 sockops, the gateway, and telemetry can be
DST-tested against a fixture identity store without the real CA/runtime).
**SCAMPER origin:** M (Magnify the read-surface dimension into a first-class port trait).
**Closest competitor:** The SPIFFE Workload API as an *interface contract* (consumers
code to the API, not to SPIRE's internals).

### P — Put to other use: a unified credential store seam ready for ACME (gateway certs)

**Core idea:** Build the held store as a *generic credential store* from day one — keyed
not by "SVID for alloc X" but by a credential-identity that admits both internal SVIDs
*and* the future public-trust ACME certs the gateway will need (whitepaper §11/4.7:
"both lanes share `IdentityMgr`"). #35 populates the SVID lane; the ACME lane (4.7) plugs
into the same store + read surface later.
**Key mechanism:** A `SvidLifecycle` reconciler + `Action::IssueSvid`/`DropSvid` executor
populate the *SVID lane* of a store whose shape already admits a second (ACME) lane and a
second writer; consumers read by credential-identity.
**Key assumption:** Designing the store's key/shape to serve the *second* known consumer
(gateway ACME, 4.7) now avoids a reshape later, and the two credential kinds share enough
of "held material + read surface + lifecycle" to live in one store.
**SCAMPER origin:** P (Put the store to the other known use — ACME gateway certs).
**Closest competitor:** istio's `SecretManager` holding both workload certs and the root
bundle as differently-keyed resources in one cache.

### E — Eliminate: no separate held store — consumers read the reconciler's View directly

**Core idea:** Remove the `IdentityMgr` struct entirely. The `SvidLifecycle` reconciler's
**persisted `View`** *is* the store-of-record (it already must persist issuance inputs for
O4 idempotence-across-restart). Consumers read the held SVID material from the runtime's
View store via a thin read accessor; the runtime's `bulk_load` on boot reconstructs the
held set, and `write_through` after each tick is the single persistence path.
**Key mechanism:** The reconciler's `View` carries the issuance facts; the executor's
write of the *material* is reconciled against the View; consumers read through a
View-backed accessor. One persistence mechanism (the existing `ViewStore`), no parallel
`Arc<RwLock<map>>`.
**Key assumption:** The reconciler View (CBOR in the `ViewStore`, `bulk_load`/`write_through`)
can serve as the consumer read surface without a separate in-memory cache — and the leaf
*private key* (which must NOT be persisted, O2/ADR-0063 D6) can be handled out-of-band
while the View holds only the persistable issuance inputs.
**SCAMPER origin:** E (Eliminate the separate store; reuse the View as store-of-record).
**Closest competitor:** SPIRE's cache being *derived from* the authoritative entry set
(the entries are the SSOT; the cache is a projection).

### R — Reverse: rebuild the held set on boot from the `issued_certificates` observation rows

**Core idea:** Invert the source of truth. Instead of the reconciler View driving the held
store, the **`issued_certificates` observation rows** (already written per issuance for
O5/audit) are the SSOT; on boot, `IdentityMgr` **re-issues** (or re-derives) the held set
by reading the observation rows for currently-Running allocs and re-minting via the CA.
Steady-state, the reconciler still emits issue/drop, but recovery is observation-driven.
**Key mechanism:** The audit row (serial, spiffe_id, issuer, validity, node) is the
durable record; a boot path reads rows for Running allocs and rehydrates the held store by
re-issuing fresh leaves; the reconciler converges steady-state.
**Key assumption:** The observation store (gossiped, eventually-consistent) is an
acceptable SSOT for rebuilding held identity, and re-issuing on boot from audit facts is
preferable to persisting/replaying issuance inputs in the intent-side View.
**SCAMPER origin:** R (Reverse the SSOT: observation rows drive the store, not the View).
**Closest competitor:** SPIRE agent re-syncing its whole cache from the server's
authoritative entry set on restart (server state is SSOT; agent cache is rebuilt).

---

## Phase 3: Crazy 8s supplements (structurally distinct from the SCAMPER set)

### X1 — Pure action-shim executor, no reconciler at all

**Core idea:** Skip a dedicated `SvidLifecycle` reconciler. Hang identity issuance/drop
**off the existing alloc-lifecycle action executors**: when the action-shim dispatches
`StartAllocation` (and the alloc confirms Running) it *also* issues + holds the SVID; when
it dispatches `StopAllocation` it *also* drops it. The shim is the only new code; no new
convergence target, no new View.
**Key mechanism:** Extend `action_shim/start_allocation` and `.../stop_allocation` (or a
post-Running hook) to call `ca_issuance::issue_and_audit` + write `IdentityMgr`; no new
`Action` variant, no new reconciler.
**Key assumption:** Identity issuance can ride the existing allocation-start/stop executor
path as a side effect, and a separate observe→converge loop for identity is unnecessary
because the alloc executor already fires exactly on the transitions that matter.
**Crazy 8s supplement.** (Distinct from C: C keeps a reconciler decision + new actions;
this removes the reconciler/action entirely and bolts onto the executor.)
**Closest competitor:** kubelet issuing the projected SA token inline in the pod-start
path (no separate identity reconciler).

### X2 — Per-allocation owned task (actor-per-identity)

**Core idea:** Spawn one supervised async task per running allocation that owns that
allocation's identity end-to-end: it issues the SVID on start, holds it, answers reads via
a message channel, and tears down (dropping the key) when its allocation stops. Identity
state is sharded per-task, not a single shared map.
**Key mechanism:** On Running, the runtime spawns an `identity_task(alloc_id)`; consumers
message the task (or a registry of task handles) to read; on Stop, the task is cancelled
and its held material dropped with it.
**Key assumption:** Per-identity ownership (an actor owning each credential's full
lifecycle) is a cleaner concurrency model than a shared lock over a map, and the
task-cancellation-drops-the-key shape gives O2 for free (mirrors Linkerd's
process-death-drops-the-key, scaled to a task).
**Crazy 8s supplement.** (Distinct from B/E: no shared map and no View-as-store — state is
per-task.)
**Closest competitor:** Linkerd's per-proxy in-memory identity (one holder per workload),
re-expressed as one task per workload inside a single process.

---

## Phase 4: Curation to 6 + diversity test

All nine generated (S, C, A, M, P, E, R, X1, X2). Curating to the evidence-backed sweet
spot of 6 by merging genuine variations (exact-mechanism overlaps) and keeping the
strongest representative of each structural family.

### Merge / fold decisions (not evaluations — structural de-duplication)

- **M (typed `IdentityRead` port) is folded as a cross-cutting *attribute*, not a
  standalone option.** The port-trait shape is orthogonal to *where material lives* — it
  can wrap the `Arc` store (A), the View (E), or a unified store (P). It is a read-surface
  *refinement* applicable to several options, not a distinct store mechanism. It is
  carried forward as a **dimension** (does the option expose a port trait?) rather than a
  competitor in its own row. *(This keeps the 6 genuinely about distinct mechanisms.)*
- **X1 (pure executor, no reconciler) is folded into C's family** as the "thinnest wiring"
  end of the lifecycle-wiring spectrum. C and X1 both answer the *lifecycle-wiring* fork
  (A in the dispatch's fork taxonomy); to keep the 6 mechanism-distinct, the curated set
  carries **one** lifecycle-wiring contrast — the standalone `SvidLifecycle` reconciler
  (the issue-named shape) vs. the no-new-reconciler shape (C/X1) — represented by Option 2.
  X1's "no Action variant at all" nuance is preserved inside Option 2's description as the
  variation it is.
- **X2 (actor-per-identity)** is kept — it is a genuinely distinct concurrency *and*
  storage model (sharded per-task vs shared map vs View vs observation-row), not a
  variation of any other.

### The curated 6

| # | Option (curated name) | From | Distinct mechanism | Distinct assumption | Distinct cost profile |
|---|---|---|---|---|---|
| **1** | **Shared `Arc<IdentityMgr>` store + `SvidLifecycle` reconciler + `IssueSvid`/`DropSvid` actions** *(the issue-named shape)* | A-base + S-rejected-store + S/M read variants | A standalone reconciler emits typed actions; executor mints + holds in `parking_lot::RwLock<BTreeMap<AllocId, SvidMaterial>>` in a shared `Arc`; consumers read via sync getters (optionally behind an `IdentityRead` port — the folded-M dimension). | Identity has its own convergence target; a shared in-memory map is the store-of-record; consumers pull. | New reconciler + 2 new Actions + 2 new executors + a new shared struct. Moderate, all on shipped runtime shapes. |
| **2** | **No new reconciler — fold lifecycle into `WorkloadLifecycle` (or bolt onto the alloc executor)** | C + X1 | `WorkloadLifecycle` (which already sees Running↔Stopped) emits the issue/drop actions — *or* the alloc-start/stop executor issues/drops inline with no new reconciler/action (the X1 thin end). Held store still a shared `Arc<IdentityMgr>`. | The identity lifecycle *is* the workload lifecycle; a separate convergence target is redundant. | Smallest new surface — no new reconciler (and possibly no new Action). Couples identity into the workload-lifecycle code. |
| **3** | **istio-SDS-style push: `watch`-channel read surface on the shared store** | A | Shared store as in #1, but the read surface is a `tokio::sync::watch`/`broadcast` channel: consumers *subscribe* and are notified on change, not poll. | Consumers benefit from push-on-change, and #40 rotation will push the rotated SVID down the same channel (no-restart swap). | Store + reconciler + actions as #1, *plus* channel plumbing and consumer subscription handling. Higher wiring cost; sets up the rotation seam. |
| **4** | **Kernel `IDENTITY_MAP`: held identity in a BPF map consumers read directly** | S | Executor writes identity (or a reference) into a kernel-side `IDENTITY_MAP` via a dataplane port; the sockops/kTLS consumer reads the map in-kernel; the `SvidLifecycle` reconciler drives it like `ServiceMapHydrator` drives `SERVICE_MAP`. | The kernel-side consumer is the *primary* reader and should read identity where it already operates (a BPF map), not from a userspace struct. | New BPF map + dataplane port + reconciler + actions. Highest mechanism cost; couples the read surface to the kernel/eBPF stack. |
| **5** | **No separate store — reconciler `View` is the store-of-record; consumers read the View** | E | Eliminate `IdentityMgr`; the `SvidLifecycle` View (persisted via the existing `ViewStore`, `bulk_load`/`write_through`) holds the issuance facts and is the consumer read surface; the non-persistable leaf key is handled out-of-band. | The reconciler View can double as the consumer read surface with one persistence mechanism; no parallel in-memory cache is needed. | Reuses the runtime's `ViewStore` entirely; fewest *new* persistence mechanisms. Forces a split between persistable facts and the non-persistable leaf key. |
| **6** | **Observation-row-driven: rebuild the held set on boot from `issued_certificates`** | R | The `issued_certificates` observation rows are the SSOT; on boot, the held set is rebuilt by reading rows for Running allocs and re-minting via the CA; steady-state the reconciler still emits issue/drop. | The (gossiped) observation store is an acceptable SSOT for rehydrating held identity; re-issue-on-boot beats persisting/replaying intent-side issuance inputs. | Reuses the audit-row plumbing for recovery; no intent-side issuance-input View needed. Adds a boot-time re-issue path; leans on eventually-consistent rows. |

> **Note:** Option X2 (actor-per-identity) was generated and is structurally distinct, but
> the curated 6 already span the four real forks the dispatch named (lifecycle wiring,
> where material lives, read surface, rotation seam) across maximally different mechanisms.
> X2's per-task concurrency model is **logged here as the 7th generated option** and is
> intentionally NOT promoted into the scored 6 — the 6 above already include the shared-map
> (1/2/3), kernel-map (4), View (5), and observation-row (6) storage models; adding a
> per-task model would crowd the matrix without adding a *new fork axis*. It remains
> available if Phase 4 / review finds the 6 insufficiently diverse on the concurrency axis.

### 3-point diversity test on the curated 6

| Pair check | Different mechanism? | Different assumption? | Different cost? | Verdict |
|---|---|---|---|---|
| 1 vs 2 | Yes — standalone reconciler+actions vs no-new-reconciler/executor-bolt | Yes — identity-is-own-target vs identity-is-workload-lifecycle | Yes — moderate vs smallest | Distinct |
| 1 vs 3 | Yes — sync getters vs watch/push channel | Yes — pull vs push-on-change | Yes — store-only vs store+channel | Distinct |
| 1 vs 4 | Yes — userspace `Arc` map vs kernel BPF map | Yes — userspace reader vs kernel-primary reader | Yes — heap struct vs eBPF map+port | Distinct |
| 1 vs 5 | Yes — separate store vs View-as-store | Yes — cache needed vs View suffices | Yes — new struct vs reuse ViewStore | Distinct |
| 1 vs 6 | Yes — intent-side held map vs observation-row-rebuilt | Yes — held SSOT vs audit-row SSOT | Yes — steady-state map vs boot re-issue | Distinct |
| 2 vs 5 | Yes — shared `Arc` store (folded reconciler) vs View-as-store (no struct) | Yes — share workload-lifecycle vs reuse View persistence | Yes — couple-into-WL vs reuse-ViewStore | Distinct |
| 3 vs 4 | Yes — userspace watch channel vs kernel map | Yes — push to userspace subs vs kernel reads map | Yes — channel plumbing vs eBPF stack | Distinct |
| 4 vs 6 | Yes — kernel map store vs observation-row store | Yes — kernel-primary vs audit-SSOT | Yes — eBPF port vs boot re-issue | Distinct |
| 5 vs 6 | Yes — View (intent-side, persisted) vs observation rows (gossiped) | Yes — persist inputs vs re-issue from audit | Yes — ViewStore reuse vs boot re-issue path | Distinct |

All 6 pass the 3-point test against each other (every pair differs in mechanism,
assumption, *and* cost profile). The set spans: **storage substrate** (heap struct / View
/ observation rows / kernel map), **lifecycle wiring** (standalone vs folded reconciler vs
executor), and **read surface** (sync getters / watch push / kernel map / View accessor).

---

## Gate G3 evaluation

- [x] **6 curated options** — Options 1–6 above (plus X2 logged as the un-promoted 7th).
      **PASS.**
- [x] **Each passes the 3-point diversity test** — all 9 representative pairs differ in
      mechanism, assumption, and cost. **PASS.**
- [x] **No evaluation language in this file** — options described neutrally; the one
      "constraint" called out (reconciler purity) is a *correctness rule* per project
      docs, explicitly flagged as structural-exclusion not taste-judgment; merge decisions
      are structural de-duplication, not scoring. **PASS.**
- [x] **SCAMPER coverage documented** — all 7 lenses produced an option (S/C/A/M/P/E/R) +
      2 Crazy 8s (X1/X2); curation trail shows which folded where. **PASS.**

**Phase 3 gate: PASS.** Ready for Phase 4 (taste evaluation).
