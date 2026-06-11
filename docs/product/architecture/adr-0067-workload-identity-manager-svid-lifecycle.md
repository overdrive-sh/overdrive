# ADR-0067: Workload Identity Manager — `SvidLifecycle` Reconciler, `IdentityMgr` Holder, `IdentityRead` Port, and the Two Identity Actions

## Status

Accepted (2026-06-08). **Revised rev 2 (2026-06-08)** — see § Revision (rev 2)
at the end for the DESIGN-review findings this revision addresses (restart model,
held-set-as-`actual`, retry-memory View, the enqueue/handoff trigger, the
`SpiffeId` consolidation, and the O4/K3 reframe). **Revised rev 3 (2026-06-08)** —
see § Revision (rev 3): the D4 `HeldSvidFacts.not_after` / `held_snapshot` text is
made TRUE against the code (`SvidMaterial` gains a real `not_after` field via the
ADR-0063 rev 2 amendment) and the D8 near-expiry seam is confirmed sound and
DST-deterministic; a PINNED SURFACE SPEC for the crafter is appended.
**Revised rev 4 (2026-06-08)** — see § D3 rev-4 correction: the D3 production-CA
composition claim ("composes `Arc<dyn Ca>` from the existing `ca_boot` path …
the same adapter the boot path already builds") was **false** — `lib.rs:50` is a
bare `pub mod ca_boot;` and `boot_ca`/`RcgenCa` are never called in `lib.rs`.
Corrected to the ratified plan: Phase 2 composes an **ephemeral workload
`RcgenCa` directly in `run_server`** (no KEK, no persistence); the persistent
KEK-backed root (ADR-0063 D2/D8) + operator surface are **#215**.
**Revised rev 5 (2026-06-09)** — see § Revision (rev 5): resolves a restart-recovery
defect surfaced by adversarial review of the DELIVER implementation
(characterization test
`restart_after_successful_issue_before_clear_stalls_reissue_until_backoff_elapses`).
Record-on-emit conflated two distinct durable signals — a *success* fact
("this alloc was issued before") with a *failure* fact ("the last attempt did
not hold"). A successful issue persisted a retry entry that, after a crash
before the clearing tick, suppressed the immediate restart re-issue D1
promises. Rev 5 adds the `issued_certificates` audit row to `actual` as the
restart-recovery success signal (checked BEFORE the backoff gate), keeps the
retry View strictly as failure-backoff memory, and reconciles the addition with
**A3** (reading an already-durable observation row as `actual` is what every
reconciler does — it is NOT persisting a new success fact). Touches **D1, D4,
D8, A3** and adds **D10**. **Revised rev 6 (2026-06-09)** — see § Revision
(rev 6): the `built-in-ca-operator-composition` feature (folds #40 + #215)
reframes the #40 near-expiry rotation seam from a *gated workflow* to a *live
reconciler action* (`Action::IssueSvid` rotate-correlation). A5's rejection is
reframed (it correctly rejected a *synchronous in-`reconcile` mint*, NOT an
action-emit); D8's near-expiry branch and the #40-boundary section drop the
"wait-for-DNS-propagation workflow" framing (that was external ACME, never
internal SVID reissue) and emit `IssueSvid`; D1/D8 restart-re-mint is
re-validated. Touches **A5, D8, D1, #40-boundary**. Builds on
**ADR-0063** (built-in CA — the `Ca` port, `SvidMaterial`, `TrustBundle`,
`ca_issuance::issue_and_audit`)
and the reconciler / action-shim machinery of **ADR-0013 / ADR-0023 / ADR-0035 /
ADR-0036**. Governs the workload **identity holder/reader/dropper** — the loop
that binds a live, chain-verifiable SVID to the exact set of currently-running
allocations, holds it where the dataplane can read it, and drops it on stop
(GH #35, roadmap step 2.13). Supersedes nothing — ADR-0063 mints; this ADR
holds, reads, and drops what ADR-0063 mints.

## Context

ADR-0063 shipped the `Ca` port and `ca_issuance::issue_and_audit` — the
platform **can mint** a SPIFFE SVID and bind an `issued_certificates` audit row
to each issuance. But **nothing holds it.** A workload reaches Running and there
is no live credential bound to it that the mTLS layer can present; when the
workload stops, any minted credential (and its node-held leaf private key) would
linger with no entity to drop it. The whitepaper's structural-security promise
(§4, design principle 3 — *"every packet carries cryptographic workload
identity"*) is true only *in principle* (mintable) and not *operationally true
for the running set* (held, readable, dropped).

Overdrive is **sidecarless** (whitepaper §7): there is no in-pod agent to
fetch / hold / drop a credential, so the credential's lifecycle can *only* be
driven from the allocation lifecycle the control plane already owns. This is
**J-SEC-002** (validated in DIVERGE; distinct from J-SEC-001 / #28's "mintable
in principle"). The architecture is **LOCKED to DIVERGE Option 1** (a standalone
`SvidLifecycle` reconciler + typed `Action::IssueSvid` / `DropSvid` + an
action-shim executor → a shared `Arc<IdentityMgr>`; consumers read via sync
getters behind an `IdentityRead` port; the held set is the reconciler's `actual`
and the View carries retry memory — rev 2) — designed over, not re-litigated.

**Quality drivers** (priority order, from the feature-delta KPIs K1–K5 and ODI
outcomes O1–O6):

1. **Identity availability for the running set (K1 / O1 — North Star)** — every
   Running allocation holds a valid, chain-verifiable SVID at every stable
   convergence point. Proven by `assert_eventually!("running allocs hold a valid
   SVID")`, not asserted.
2. **Leak resistance on stop (K2 / O2)** — no `SvidMaterial` (incl. the node-held
   leaf private key) held for an allocation no longer Running. Drop-on-stop
   removes the entry so the leaf key is no longer reachable in the held set.
3. **Read latency (O3)** — a dataplane consumer reads the current SVID + trust
   bundle in-process (sync getter, `Arc`, no gRPC / IPC), never re-issuing per
   read. Whitepaper §7 "no gRPC, no IPC".
4. **Restart recovery — bounded, audited, no stale/silent credential (K3 / O4,
   reframed rev 2)** — a control-plane restart leaves no Running allocation
   without a held SVID. The held `SvidMaterial` (incl. the node-held leaf private
   key) is *non-persistable* (`CaKeyPem` has no `Serialize` —
   `crates/overdrive-core/src/traits/ca.rs:100`, by ADR-0063 D9) and
   *non-reconstructable* (each `issue_and_audit` mints a FRESH leaf with a
   distinct serial — `crates/overdrive-control-plane/src/ca_issuance.rs:34-40`),
   so "recompute held state without re-issue" is **impossible**. The honest model
   is **re-issue on boot** for every still-Running allocation (one per running
   alloc, bounded, each audited via `issue_and_audit`). This is **RECOVERY** — a
   distinct reconciler branch from the near-expiry reissue branch (D8 — rev 6: a
   live `Action::IssueSvid`, NOT a workflow), though both emit `Action::IssueSvid`. The
   View carries **retry memory** so a *failed* re-issue backs off instead of
   hammering every tick; no secret reaches disk.
5. **No silent issuance (K4 / O5)** + **mechanism economy (O6)** — every issuance
   leaves an `issued_certificates` row (reuse `issue_and_audit`); no new
   concurrency / storage mechanism beyond the shipped reconciler runtime + `Ca`
   port + ObservationStore.
6. **DST determinism (K5)** — the held-identity subsystem reproduces
   bit-identically from a seed (`BTreeMap` iteration + serials via `Entropy` +
   fixture keys).

**Constraints** (locked, from DISCUSS System Constraints + project rules):

- **Reconciler purity is a CORRECTNESS constraint** (DIVERGE D-WIM-3): CA I/O
  lives in the action-shim executor, NEVER in `reconcile()`. The `SvidLifecycle`
  reconciler is a pure `reconcile(desired, actual, view, tick) → (Vec<Action>,
  View)` — no `.await`, no `Ca` handle, no `ObservationStore` handle, wall-clock
  only via `tick.now`. dst-lint enforces it at the crate boundary.
- **Persist INPUTS, not derived state** (`.claude/rules/development.md`): the
  View persists **retry-request inputs** (`attempts` + `last_failure_seen_at` —
  the `RetryMemory` shape from `development.md` § "Reconciler I/O"), **never** a
  derived `expires_at` / `next_renewal_at` AND **never** an issuance *success
  fact* (no `serial`, no `issued_at`-as-proof). Two reasons the View cannot hold
  success facts (rev 2): (a) `serial` is a *post-dispatch executor output* — the
  pure reconciler cannot know it, and the runtime persists `next_view` BEFORE
  dispatch (`crates/overdrive-control-plane/src/reconciler_runtime.rs:1222-1226`
  `persist_view`, vs dispatch at `:1324`), so a View entry claiming "issued" could
  be durably written when the CA / audit write then fails; (b) the success fact
  *already lives* in the `issued_certificates` observation row. Near-expiry (for
  the near-expiry reissue branch — rev 6: a live `Action::IssueSvid`, NOT a
  workflow) is recomputed every tick from the **held cert's real `not_after`**
  (read off `actual` — D4), not a View field. A future-event field or a success
  fact in the View is a review-rejection smell.
- **State-layer hygiene** (whitepaper §4, ADR-0063 D2/D6): the held
  `SvidMaterial` (incl. the node-held leaf private key) lives **in-process** in
  `IdentityMgr` — neither intent nor observation; ephemeral runtime state bounded
  to the running set, intentionally never persisted (the leaf key is not an audit
  fact and must not reach disk). **The held set IS the reconciler's `actual`**
  (rev 2 — D1/D4): the runtime projects a held-snapshot into `SvidLifecycle`'s
  `actual` exactly as it projects the workflow engine's live-task set into
  `WorkflowLifecycle`'s `actual` (`reconciler_runtime.rs:2206-2209` →
  `hydrate_workflow_actual_instances:2152-2186` → `live_instances():2166`). The
  `issued_certificates` audit row is **observation** (ADR-0063 D6). The
  `SvidLifecycle` View is **reconciler memory** (the runtime-owned ViewStore),
  persisting only retry inputs. These layers never merge.
- **`BTreeMap`, not `HashMap`** (`.claude/rules/development.md` §
  "Ordered-collection choice"): the held-SVID map IS iterated — the
  `assert_eventually!` invariant walks it — so its iteration order must be
  deterministic across seeds. The View's per-allocation map is `BTreeMap` for the
  same reason (bulk-loaded + observed).
- **Single-node (Phase 2)** — one co-located node; the held set is one node's
  running allocations. Multi-node (per-node held sets, gossiped audit rows, node
  attestation) is owned by **#36 [2.14]**.
- **OOP / ports-and-adapters** — the established project paradigm; `IdentityRead`
  is a port trait mirroring `Clock` / `Transport` / `Ca`.

## Decision

### D1 — A standalone `SvidLifecycle` reconciler converges `desired = running` vs `actual = held set`

A new pure-sync `Reconciler` (`overdrive-core/src/reconcilers/svid_lifecycle.rs`,
class `core`) converges two sets and emits the diff:

- **`desired`** = the set of currently-**Running** allocations for this workload,
  projected from the `alloc_status` observation rows (the same
  `obs.alloc_status_rows()` filter the `WorkloadLifecycle` /
  `BackendDiscoveryBridge` arms already use —
  `reconciler_runtime.rs:2210-2220, 2298-2325`).
- **`actual`** = the set of allocations the `IdentityMgr` currently **holds** an
  SVID for — the **held-set-as-actual** (rev 2, the key addition). The runtime
  projects an `IdentityMgr` held-snapshot into `actual` exactly as it projects the
  workflow engine's live-task set into `WorkflowLifecycle`'s `actual` (D4 wires it;
  feasibility grounded against `reconciler_runtime.rs:2206-2209 →
  hydrate_workflow_actual_instances:2152-2186 → live_instances():2166`).

The pure convergence rules:

| desired (running) | actual (held) | emit |
|---|---|---|
| running | ¬held | `Action::IssueSvid` |
| ¬running | held | `Action::DropSvid` |
| running | held (valid — `not_after` not near-expiry) | no-op (`Noop`) |
| running | held (near-expiry) | `Action::IssueSvid` **unconditionally** (rev 6 — `"rotate-svid"` correlation; threshold ½ × `WORKLOAD_SVID_TTL` = 1800s; NO `ROTATION_ENABLED` gate, NO `StartWorkflow`) |

**Restart recovery falls out for free.** On a control-plane restart the in-memory
`IdentityMgr` is empty (the held set was never persisted — the leaf key cannot
reach disk), so `actual = ∅`; every still-Running allocation matches `running ∧
¬held` and is **re-issued** (one `IssueSvid` per running alloc, bounded, each
audited via `issue_and_audit`). This is **RECOVERY** — the `running ∧ ¬held →
issue` branch — and is a *distinct branch* from the near-expiry reissue branch
(`running ∧ held(near-expiry) → Action::IssueSvid`, rev 6 — also a live action, NOT
a workflow). Both emit `Action::IssueSvid` through the same executor; neither is the
forbidden synchronous-rotation path. Re-issue-on-restart is the first-issue path
running again because the
holder was reset. There is no "recompute held state without re-issue" — that is
impossible (the leaf key is non-persistable per ADR-0063 D9 and non-reconstructable
per `ca_issuance.rs:34-40`), and rev 1's claim that it could is the Critical
finding this rev corrects.

> **rev 5 correction — restart recovery must be IMMEDIATE, and the retry-memory
> View alone cannot make it so.** The model above is correct in shape but
> rev-2..4 left "every still-Running alloc … is re-issued, immediately" served
> *only* by the volatile `actual = ∅` condition. With record-on-emit (D8) a
> **successful** issue still persisted a retry entry, cleared only on a later
> converged tick. A crash *after* the issue succeeded (SVID minted,
> `issued_certificates` row written, held in-memory) but *before* the clearing
> tick leaves restart with an empty held set (volatile) PLUS a surviving
> persisted retry entry — and the backoff gate suppresses the immediate reissue
> this branch promises until the backoff window elapses. The held set is volatile
> but the retry View is durable, and they diverge exactly on restart.
>
> **The fix (D10):** the reconciler reads a SECOND durable success signal —
> presence of an `issued_certificates` audit row for the alloc's derived
> `spiffe_id` — projected into `actual`. `audit-row ∧ ¬held` is an unambiguous
> *restart marker* (audit-before-hold, D6, means a row exists only on a *prior
> successful* mint), and the reconciler re-issues **immediately**, bypassing the
> backoff gate. The retry View is narrowed to a strict *failure*-backoff signal
> governing only the genuinely-never-succeeded path (`¬held ∧ no audit row`). See
> **D10** for the precise `actual` shape and reconcile-body predicate, and **A3**
> for why reading the audit row does not contradict A3's rejection of
> persisting success facts.

It is a *separate* convergence target from `WorkloadLifecycle` (DIVERGE Option 1)
— identity availability warrants its own desired-vs-actual loop and its own
`assert_eventually!` North-Star invariant (O1). It mirrors the shipped
`ServiceMapHydrator` → `Action::DataplaneUpdateService` → executor pattern (and the
`WorkflowLifecycle` held-state-in-`actual` pattern) exactly.

`reconcile()` is pure: no `.await`, no CA / observation handle, wall-clock only
via `tick.now`; it passes dst-lint. The reconciler **builds the `SpiffeId`** for
each allocation (pure derivation — see D5) and passes it in the `IssueSvid`
action; CA I/O stays entirely in the executor (D3). Because `actual` is the held
set, the reconciler does **not** GC a View "issued" map — there is no such map
(D8); it converges the held set against the running set directly.

### D2 — Two typed Actions: `IssueSvid` and `DropSvid` (plain enum, additive)

`Action` (`overdrive-core/src/reconcilers/mod.rs`) gains two variants:

```rust
Action::IssueSvid {
    alloc_id:  AllocationId,
    spiffe_id: SpiffeId,        // built PURE by the reconciler (D5)
    node_id:   NodeId,
    correlation: CorrelationKey,
}
Action::DropSvid {
    alloc_id:  AllocationId,
    correlation: CorrelationKey,
}
```

`Action` stays a **plain enum** — these are 2 additive variants, the same shape
`Action::DataplaneUpdateService` / `Action::HttpCall` / `Action::StartWorkflow`
were added. The change is additive to:

- the `Action` enum (+2 variants),
- `action_shim::dispatch_single` (+2 match arms — D3),
- the dispatch enums `AnyState` / `AnyReconciler` / `AnyReconcilerView` (+3
  variants apiece, the standard reconciler-registration triple). `AnyState::
  SvidLifecycle` wraps `SvidLifecycleState { desired: <running allocs>, actual:
  <held snapshot> }` (D1); `hydrate_actual` gains one new `AnyReconciler::
  SvidLifecycle(_)` arm reading `state.identity.held_snapshot()` (D4 — the held-
  set-as-`actual` projection, grounded against the `WorkflowLifecycle` arm at
  `reconciler_runtime.rs:2206-2209`).

**`node_id` is KEPT on `IssueSvid`.** Rationale: (a) the action is
**self-describing** — the issuance request names the node the SVID is issued on,
which is the `issued_certificates` row's `node_id` column (ADR-0063 D6) and the
`issue_and_audit(…, node, …)` argument; (b) **#36-forward-compat** — when
multi-node lands, the issuing node is no longer "the only node," and a
self-describing action needs no reshaping; (c) in Phase 2 the executor MAY read
`AppState.node_id` instead, but carrying it on the action keeps the action the
SSOT for what was requested rather than coupling the executor's behaviour to
ambient state. The redundancy is deliberate and cheap (one `NodeId`).

**Correlation** is derived, not a request ID:
`correlation = CorrelationKey::derive(target = "svid-lifecycle/<alloc>",
spec_hash, "issue-svid")` (the ADR-0035 § "Reconciler I/O" correlation
discipline — links cause to the audit/observation surface deterministically
across ticks, unlike a per-attempt request id).

### D3 — CA I/O lives in the action-shim executor; `AppState` is extended to wire it

A new executor `action_shim/issue_svid.rs` (mirroring
`action_shim/dataplane_update_service.rs`) handles the two arms in
`dispatch_single`:

- **`IssueSvid`**: calls the shipped `ca_issuance::issue_and_audit(ca,
  observation, clock, node, request)` (which mints the leaf via `Ca::issue_svid`,
  writes the `issued_certificates` row, and **refuses issuance on audit-write
  failure** — ADR-0063 D6, O5 served wholesale, NOT re-implemented), then
  `identity.hold(alloc_id, svid)`, then opportunistically refreshes the held
  bundle: `identity.set_bundle(ca.trust_bundle()?)` (D6).
- **`DropSvid`**: calls `identity.drop_svid(alloc_id)` — removes the entry so the
  node-held leaf private key is no longer reachable in the held set (O2).

This is the one place CA I/O happens. To wire it, **`AppState` is extended** (this
is the "found wiring" — an ADR consequence, recorded explicitly):

- `AppState` gains `ca: Arc<dyn Ca>` and `identity: Arc<IdentityMgr>`.
- `ca` / `clock` / `identity` are threaded into `dispatch` / `dispatch_single`
  for the two new arms.
- Production composes `Arc<dyn Ca>` by constructing an **ephemeral workload
  `RcgenCa` directly in `run_server`** (see rev 4 correction below). `AppState.ca`
  stays a **required `Arc<dyn Ca>`**.

> **rev 4 correction (2026-06-08).** Rev 1–3 of this section asserted that
> production "composes `Arc<dyn Ca>` from the existing `ca_boot` path
> (`overdrive-control-plane/src/lib.rs:50`) — the same `Ca` adapter the boot
> path already builds for ADR-0063." **That was false.** `lib.rs:50` is
> `pub mod ca_boot;` — a bare module declaration. `boot_ca` / `RcgenCa` are
> **never called in `lib.rs`**; ADR-0063 shipped the workload-CA boot
> *functions* but never wired them into `run_server`. They exist only in tests
> (`rcgen_ca_chain_verify.rs`, `ca_equivalence.rs`, `ca_boot_and_audit.rs`).
> The only CA constructed in `lib.rs` today is `tls_bootstrap::mint_ephemeral_ca()`
> (`lib.rs:1208`) — the **operator/control-plane HTTPS** ephemeral CA (ADR-0010),
> which is NOT a `Ca` and CANNOT issue workload SVIDs. Do not conflate the two.
>
> **What Phase 2 actually composes (the ratified plan).** `run_server`
> constructs an **EPHEMERAL workload `RcgenCa` in process** — fresh in-memory
> P-256 root each boot, **NO KEK, NO persistence** (this is NOT `boot_ca`). The
> composition mirrors the shipped test precedent
> (`crates/overdrive-host/tests/integration/rcgen_ca_chain_verify.rs:74-79,132-142`):
>
> ```rust
> let subject = SpiffeId::new("spiffe://overdrive.local/overdrive/ca")?;
> let ca: Arc<dyn Ca> = Arc::new(RcgenCa::new(Arc::new(OsEntropy), subject));
> ca.root()?;                        // ephemeral P-256 root (cached via RaceOnceCell)
> ca.issue_intermediate(&node_id)?;  // node intermediate signed by root (cached)
> let bundle = ca.trust_bundle()?;   // root anchor + intermediate
> let identity = Arc::new(IdentityMgr::new(Some(bundle)));
> ```
>
> The wiring inputs already exist in `run_server`: `node_id` =
> `NodeId::new("local")` (`lib.rs:1415`), `OsEntropy` (`lib.rs:359, 1539`), and
> `AppState` is constructed at `lib.rs:1544`
> (`AppState::new_with_workflow_engine`). `RcgenCa::new(entropy: Arc<dyn Entropy>,
> subject: SpiffeId)` (`crates/overdrive-host/src/ca/rcgen_ca.rs:119`) caches its
> root/intermediate internally via `RaceOnceCell`. All `Ca` methods are sync,
> `Result`-returning (`crates/overdrive-core/src/traits/ca.rs`): `root()` (:649),
> `issue_intermediate(&NodeId)` (:672), `issue_svid(&SvidRequest)` (:729),
> `trust_bundle()` (:757).
>
> **What is deferred — #215** ("Compose built-in CA into operator surface +
> satisfy EDD expectations"). *(rev 6, 2026-06-09: #215 is now CLOSED by
> `built-in-ca-operator-composition` — no longer blocked on #35; the
> "blocked on #35 / not yet wired" description below is rev-2-era historical
> state. See § "Downstream boundary" → "#215 (operator surface — CLOSED …)".)*
> The **persistent KEK-backed root**
> (`boot_ca` + `SystemdCredsKeyring`, ADR-0063 D2/D8) and the operator surface
> (`alloc status` SVID render, deployed-SVID operator-verify) are #215's scope,
> not yet wired. ADR-0063's persistent design is the **upgrade target** for #215;
> it is not contradicted by the ephemeral Phase-2 composition — the ephemeral
> `RcgenCa` and the persistent KEK-backed root implement the **same `Ca` trait**,
> so swapping the composition root in `run_server` is the only change #215 makes
> to this seam. `AppState.ca` remains a required `Arc<dyn Ca>` across both.

The executor is the async boundary (ADR-0023's sanctioned shim boundary) — the
pure reconciler drives it through typed Actions and observes its effect through
the `issued_certificates` ObservationStore row, exactly as `ServiceMapHydrator`
drives `EbpfDataplane`.

### D4 — `IdentityMgr` holds the SVID set + the trust bundle in process

A new `IdentityMgr` (`overdrive-control-plane/src/identity_mgr.rs`, class
`adapter-host`):

```rust
pub struct IdentityMgr {
    inner: parking_lot::RwLock<IdentityState>,
}

struct IdentityState {
    held:   BTreeMap<AllocationId, SvidMaterial>,
    bundle: Option<TrustBundle>,
}
```

- **`new(bundle: Option<TrustBundle>)`** — constructed at the composition root
  with the boot trust bundle (D6).
- **Mutators** (write-lock → mutate → drop guard, never across `.await`):
  `hold(&self, alloc, svid)`, `drop_svid(&self, alloc)`, `set_bundle(&self,
  bundle)`.
- **`impl IdentityRead for IdentityMgr`** — reads via read-lock → `.cloned()` out
  (the guard is dropped *within* the read expression; a guard is NEVER held
  across `.await`, per `.claude/rules/development.md` § "Concurrency & async").
- **`held_snapshot(&self) -> BTreeMap<AllocationId, HeldSvidFacts>`** (rev 2 —
  the `actual`-projection reader). A sync read-lock → `.iter().map(…).collect()`
  → drop-guard read that materialises, per held alloc, the *facts the reconciler's
  pure convergence needs*: the `AllocationId` (presence = "held"), the
  `SpiffeId` (via `svid.spiffe_id()`), and the cert's real **`not_after`** read
  **directly off the held material via `svid.not_after()`** (`SvidMaterial`'s
  validity end — used by the near-expiry branch, D8). It returns a *projection*,
  NOT the `SvidMaterial` itself (the leaf key never leaves `IdentityMgr` except
  through the `IdentityRead` getter the dataplane consumer holds — keeping the
  `core`-class reconciler's `actual` key-free). `HeldSvidFacts { spiffe_id:
  SpiffeId, not_after: UnixInstant }` is a small `core` type the snapshot yields.

  **`SvidMaterial::not_after()` is a real accessor** as of the ADR-0063 rev 2
  amendment (2026-06-08): `SvidMaterial` gains a `not_after: UnixInstant` field
  populated at mint from the validity window `ca_issuance::issue_and_audit`
  threads through `SvidRequest` from its single injected-`Clock` read — the SAME
  window written to `issued_certificates.not_after`, so `svid.not_after() ==
  issued_certificates.not_after` for the same issuance, by construction, and the
  value is DST-deterministic under `SimClock`. (Rev 1/rev 2 of THIS ADR asserted
  `not_after` was "the cert's real validity end" before that field existed — the
  contradiction the ADR-0063 amendment resolves; see that ADR's Changelog entry
  "rev 2 amendment: `SvidMaterial` gains `not_after`".) Because the held
  `not_after` and the reconciler's `tick.now_unix` now derive from one clock, the
  near-expiry comparison in D8 is sound and replayable, not a comparison against
  a wall-clock/fixture value.

**How `actual` is hydrated (the runtime wiring — grounded).** `IdentityMgr` is an
`Arc<IdentityMgr>` field on `AppState` (D3), exactly as `workflow_engine:
Arc<WorkflowEngine>` is (`lib.rs:281`). The runtime's `hydrate_actual`
(`reconciler_runtime.rs:2190`) is a `match` over `AnyReconciler`; the new
`AnyReconciler::SvidLifecycle(_)` arm reads `state.identity.held_snapshot()`
**synchronously, in-process** and builds `SvidLifecycleState { desired: <running
allocs from obs.alloc_status_rows()>, actual: <held snapshot> }`. This is the
**identical shape** to the `WorkflowLifecycle` arm (`:2206-2209`), which calls
`hydrate_workflow_actual_instances(state)` (`:2152`) → `state.workflow_engine.
live_instances()` (`:2166`) to project the engine's non-persisted live-task set
into `actual.has_live_task`. The held set is a non-persisted in-process runtime
set with a sync reader, just like the live-task set — so projecting it into
`actual` is feasible against the runtime **as written**, with no runtime-mechanism
change (one new `match` arm). **Feasibility verdict: FEASIBLE — no blocker.**

**rev 5 — `actual` ALSO carries the per-alloc `ever_issued` audit-row fact (D10).**
The same `AnyReconciler::SvidLifecycle(_)` `hydrate_actual` arm additionally reads
`state.obs.issued_certificate_rows()` (an `async` ObservationStore read the
runtime already performs for other arms; `hydrate_actual` is already `async`) and
projects, per running alloc, whether an `issued_certificates` row exists for its
derived `spiffe_id` — the durable restart-recovery success signal. This is keyed
on `spiffe_id` because the audit row carries `spiffe_id`, not `alloc_id`
(`issued_certificate_row.rs:82-97`); `SpiffeId::for_allocation` (D5) derives the
expected identity per alloc deterministically. The projected fact is a boolean
presence, NOT the row contents — `serial` / `not_after` / `issued_at` stay out of
`actual`; the near-expiry `not_after` continues to come from the *held* cert
(`HeldSvidFacts`, above). `SvidLifecycleState.actual` therefore yields, per
allocation, both "held?" (volatile `HeldSvidFacts`) and "ever issued?" (durable
boolean). See **D10** for the reconcile-body model and **A3** for the A3
reconciliation.

**`BTreeMap` is MANDATORY** — the held map is iterated by the `assert_eventually!`
North-Star invariant (O1, K1) AND by `held_snapshot` (whose output the runtime
folds into `actual`, itself iterated by the convergence), so its iteration order
must be deterministic across DST seeds (K5). **`parking_lot::RwLock`, NOT
`tokio::sync`** — the critical section is a synchronous map mutation / clone-out
that does not cross an `.await`; `parking_lot` is the project default for sync
critical sections (faster uncontended path, no poisoning) and the
read/mutate/clone-and-drop shape keeps the guard off every await point.
`held_snapshot` is sync precisely so the `hydrate_actual` arm reads it without an
`.await` (mirroring `live_instances()`, which is sync).

### D5 — `SpiffeId::for_allocation` — the canonical EXTRACTION of an existing helper shape (rev 2: consolidation, not net-new)

The allocation → SPIFFE-URI derivation **already exists twice** as private
reconciler helpers, both building the identical
`spiffe://overdrive.local/job/<workload>/alloc/<alloc>` string:

- `mint_alloc_identity(&WorkloadId, &AllocationId) -> SpiffeId`
  (`crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs:424`), and
- `mint_identity(&WorkloadId, &AllocationId) -> SpiffeId`
  (`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:808`).

Both `format!("spiffe://overdrive.local/job/{}/alloc/{}", …)` and validate via
`SpiffeId::new(&raw).expect(…)`. What is genuinely **unbuilt** is the *public*
constructor on `SpiffeId` — `SpiffeId::new` (`id.rs:251`) validates a raw string
but there is no allocation-shaped constructor on the type. So this is a
**consolidation / extraction**, not net-new surface (the Medium finding rev 1
mislabelled it). This ADR adds the canonical public extraction:

```rust
impl SpiffeId {
    /// Derive the SVID identity for a workload allocation:
    /// `spiffe://overdrive.local/job/<workload>/alloc/<alloc>`.
    #[must_use]
    pub fn for_allocation(workload: &WorkloadId, alloc: &AllocationId) -> Self { … }
}
```

(`overdrive-core/src/id.rs`, `impl SpiffeId`.) It is **infallible** (`-> Self`,
`#[must_use]`), uses the trust-domain const `overdrive.local`, builds the
`spiffe://overdrive.local/job/<workload>/alloc/<alloc>` string, and validates it
through the existing `SpiffeId::new` with `unwrap_or_else(|| unreachable!(…))` —
the documented logically-unreachable idiom (`.claude/rules/development.md` §
"Logically unreachable `None` / `Err` — use `unreachable!()`"). The inputs are
already-validated newtypes whose grammars cannot produce an invalid SPIFFE path,
so `new` cannot fail — and `unreachable!` (not `?`, not `.expect()`) is the
honest way to say so.

**The pure reconciler builds the `SpiffeId`** and passes it in `Action::IssueSvid`
(D2) — identity *derivation* is pure and belongs in `reconcile()`; identity
*issuance* (CA I/O) is the executor's (D3). The derivation never reaches for the
CA.

**DELIVER migration obligation (prevents a THIRD implementation).** The two
existing private helpers MUST be migrated to call `SpiffeId::for_allocation` in
the same feature — `mint_alloc_identity` (`backend_discovery_bridge.rs:424`) and
`mint_identity` (`workload_lifecycle.rs:808`) become thin call-throughs (or are
deleted and their call sites point at `for_allocation` directly), per the
single-cut discipline (`.claude/rules` — no parallel duplicate paths). Shipping
`for_allocation` *alongside* the two private helpers would leave three
implementations of one identity string — the exact drift the reviewer flagged.
This is a DELIVER handoff item recorded in the Reuse Analysis and the Consequences.

### D5b — The enqueue/handoff trigger: `SvidLifecycle` is level-triggered via `Action::EnqueueEvaluation` (rev 2, High-2)

A reconciler does not run unless the broker is told to evaluate it. Rev 1 said
`SvidLifecycle` "observes allocation Running ↔ Stopped" but never specified **how
the broker is poked when allocation state changes** — so as written, the
reconciler would build correctly and *never tick at the moments the feature
depends on*. This decision pins the missing trigger, mirroring the shipped
production handoffs (`WorkloadLifecycle` already emits `Action::EnqueueEvaluation`
for `backend-discovery-bridge` and `service-lifecycle`).

**(a) Target key — `job/<workload_id>`.** `SvidLifecycle` is keyed by
`TargetResource::new("job/<workload_id>")`, the same scope
`backend-discovery-bridge` / `service-lifecycle` use
(`workload_lifecycle.rs:186-190, 236-240`; the exit observer's
`exit_observer.rs:231`). *Justification*: every existing alloc-lifecycle handoff
addresses the broker at workload grain (the broker is LWW at `(ReconcilerName,
TargetResource)` per ADR-0013 §8 / whitepaper §18), the running-alloc set the
reconciler converges is exactly the set of running allocs *for one workload*, and
keying at `job/<workload_id>` means duplicate enqueues across producer sites
collapse to one dispatch per drain cycle. An `alloc/<alloc_id>`-grain key would
fragment the running-set view the convergence needs and break dedup with the
existing handoffs. Reviewer's suggested `job/<workload_id>` is adopted.

**(b) Producer sites — the two existing alloc-lifecycle handoff emitters, plus a
new emission to `svid-lifecycle`.** Both are GROUNDED:

1. **`WorkloadLifecycle::reconcile`** (`workload_lifecycle.rs:181`) already emits
   `Action::EnqueueEvaluation` to `backend-discovery-bridge` (`:191-195`) and (for
   Service kind) `service-lifecycle` (`:241-245`) whenever
   `actions.iter().any(is_alloc_mutating_action)` —
   `is_alloc_mutating_action` (`:279-285`) = `StartAllocation | RestartAllocation
   | StopAllocation | FinalizeFailed`. **Add a third emission, ungated by kind**
   (identity is needed by *every* running allocation, not only Service): inside the
   same `if actions.iter().any(is_alloc_mutating_action)` block, push
   `Action::EnqueueEvaluation { reconciler: SVID_LIFECYCLE_NAME, target:
   job/<workload_id> }`. Use the same compile-time `NAME`-alias anti-drift const
   the file already uses (`const SVID_LIFECYCLE_NAME: &str = <SvidLifecycle as
   Reconciler>::NAME;`, mirroring `BACKEND_DISCOVERY_BRIDGE_NAME` at `:258`).
   `is_alloc_mutating_action` is the correct predicate — every one of the four
   variants ADDs or REMOVEs a Running alloc, which is exactly when the held set
   must re-converge (Start/Restart → `running ∧ ¬held → IssueSvid`; Stop/Finalize →
   `¬running ∧ held → DropSvid`).

2. **The exit observer** (`worker/exit_observer.rs:230-256`) submits
   `Evaluation`s directly to `runtime.broker().submit(...)` for `workload_lifecycle`
   (`:233-236`) and `backend_discovery_bridge` (`:253-256`) on an observed
   alloc-exit transition (Running → Failed/Stopped) — the path that flips the
   *actual* outside the main workload-lifecycle action vector (the reviewer's
   "exit-observer path"). **Add a sibling `runtime.broker().submit(Evaluation {
   reconciler: svid_lifecycle_name(), target })`** there, next to the bridge
   submit, so a workload that exits ticks `SvidLifecycle` and the `¬running ∧ held
   → DropSvid` branch fires (O2 — the leaf key is dropped on stop even when the
   stop is an *exit*, not an operator-driven `StopAllocation`). This is unconditional
   (not kind-gated) for the same reason the GAP-9 service-lifecycle enqueue there is
   (`:265-289`): the exit observer holds no `IntentStore`, and a spurious enqueue
   for an already-dropped/never-held alloc runs exactly one empty reconcile (the
   held snapshot has no entry → `desired ⊇ actual` already → `Noop`) and drains —
   it cannot busy-loop.

**(c) Emissions + dedup.** One `EnqueueEvaluation` per tick per producer (NOT per
action) — the broker is LWW at `(ReconcilerName, TargetResource)`, so duplicate
enqueues for the same `job/<workload_id>` collapse to a single dispatch per drain
cycle (the exact discipline `workload_lifecycle.rs:173-180` documents). The two
producer sites addressing the same broker key is intentional and safe (the
bridge/service-lifecycle handoffs already do this).

**(d) Regression-test obligation (DELIVER).** A test proving **both** a Running
transition (`StartAllocation`/`RestartAllocation`) AND a Stopped transition
(`StopAllocation`/`FinalizeFailed`/observed exit) cause `SvidLifecycle` to be
submitted to the broker for `job/<workload_id>` **with no manual broker poke** —
mirroring the UI-06 / GAP-9 enqueue regression coverage. Without this, Slice 01
could build a correct reconciler that never runs. This is a DELIVER acceptance
obligation recorded here (Slice 01 AC + § Earned Trust).

This decision is **additive** to: `WorkloadLifecycle::reconcile` (one more
`EnqueueEvaluation` push in the existing alloc-mutating block), the exit observer
(one more `broker().submit`), and the `NAME`-alias const set. No new mechanism —
the broker, `Action::EnqueueEvaluation` (`reconcilers/mod.rs:485-490`), and the
submit path all already exist.

### D6 — Trust-bundle currency is HYDRATED into `IdentityMgr` (DIVERGE fork C → option 5-A)

DIVERGE left the trust-bundle currency mechanism open (Open-Question #5):
pull `Ca::trust_bundle()` on demand vs. hydrate the bundle into `IdentityMgr`.
**Decision: HYDRATED.** The `TrustBundle` is held *in* `IdentityMgr`:

- **Set at boot** (composition root): `Ca::trust_bundle()` → `IdentityMgr::new(Some(bundle))`.
- **Refreshed opportunistically** by the issue executor (which already holds
  `&dyn Ca`): after `issue_and_audit`, `identity.set_bundle(ca.trust_bundle()?)`.
- **`current_bundle()` reads in-process** — ZERO CA I/O on the read hot path (O3).

`set_bundle` is the seam a future **external-ACME / public-trust root rotation**
later uses to push a rotated bundle through the same surface, with no consumer
change. (Internal SVID near-expiry reissue re-mints a leaf under the unchanged
intermediate, so it does not touch this bundle seam.) This satisfies O3 (the read
hot path touches no CA) and keeps the bundle current without a per-read CA pull.

### D7 — `IdentityRead` port: sync, owned-clone read surface

A new port trait `overdrive-core/src/traits/identity_read.rs` (class `core`):

```rust
pub trait IdentityRead: Send + Sync {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial>;
    fn current_bundle(&self) -> Option<TrustBundle>;
}
```

Sync getters returning **owned clones**. Per `.claude/rules/development.md` §
"Trait definitions specify behavior", the rustdoc pins five behaviour clauses
every adapter MUST honor:

1. **A read never issues.** `svid_for` does not call `Ca::issue_svid`; the SVID
   is served from the held map (the O3 guarantee).
2. **A read never mutates.** No held-map / bundle mutation as a side effect of a
   read.
3. **`None` is explicit absence** — not an error, not an empty-but-present
   credential. A consumer reading an absent allocation refuses the handshake
   rather than presenting a stale credential.
4. **Returns owned clones** — the caller holds no lock after the read (the
   read-lock is dropped within the read expression, D4).
5. **Post-`DropSvid(alloc)`: `svid_for(alloc) == None`** — drop-on-stop is
   observable through the read surface (O2).

`SimIdentityRead` (`overdrive-sim`, class `adapter-sim`) implements the port over
a preloaded `BTreeMap<AllocationId, SvidMaterial>` + `Option<TrustBundle>`. A
`tests/integration/identity_read_equivalence.rs` DST equivalence test drives the
real `IdentityMgr` read surface and `SimIdentityRead` through the same call
sequence and asserts identical observable reads (mirrors ADR-0063's
`ca_equivalence`). Consumers — and the Slice-02 **test consumer/fixture** that
proves the contract — take `Arc<dyn IdentityRead>` as a **required constructor
parameter** (never defaulted). Production consumers (sockops #26 / gateway /
telemetry) are deferred to those features; this ADR ships the port + sim double
+ the contract-proving test consumer.

### D8 — The View is RETRY MEMORY (request inputs); the near-expiry branch emits a live `Action::IssueSvid` (rev 6)

The `SvidLifecycle` View (`overdrive-core/src/reconcilers/svid_lifecycle.rs`)
holds **retry memory only** — the `development.md` § "Reconciler I/O"
`RetryMemory` shape — so that a *failed* `IssueSvid` (a CA error or an
audit-write failure inside `issue_and_audit`) backs off instead of re-firing every
tick. It holds **no issuance success facts** (rev 2):

```rust
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
pub struct SvidLifecycleView {
    /// Per-allocation issue-retry memory. Absent entry ⇒ no failed
    /// issue attempt recorded; the next `running ∧ ¬held` tick issues.
    #[serde(default)]
    retry: BTreeMap<AllocationId, IssueRetry>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct IssueRetry {
    /// Failed-issue attempt count (input to the backoff schedule).
    attempts: u32,
    /// When the last failed issue was observed (input; the backoff
    /// DEADLINE is recomputed each tick from this + the policy).
    #[serde(default = "epoch_zero")]   // UnixInstant: !Default
    last_failure_seen_at: UnixInstant,
}
```

- **NO `serial`, NO `issued_at`-as-success-fact, NO `spiffe_id`.** Rev 1's
  `IssuedInputs{issued_at, spiffe_id, serial}` was wrong on two counts (the
  Critical + High-1 findings): (a) `serial` is a **post-dispatch executor
  output** — the pure reconciler cannot know it, and the runtime persists
  `next_view` BEFORE dispatch (`reconciler_runtime.rs:1222-1226` vs `:1324`), so a
  View claiming "issued serial X" could be durably written when the CA/audit write
  then *fails*, leaving the View lying about a hold that does not exist; (b)
  persisting success facts is the broken idea — the success fact lives in the
  `issued_certificates` **observation row** (written inside `issue_and_audit`,
  ADR-0063 D6) and "is this alloc held?" is answered by `actual` (the held set,
  D1/D4), not by the View. The View's only job is *retry policy memory* for a
  failed request.
- **6 derive bounds on the View** (`Serialize, Deserialize, Default, Clone,
  PartialEq, Eq` + the auto `Send + Sync`), NOT the usual 4: the runtime's
  NextView **diff** needs `Eq` (`reconcile` returns `next_view` and the runtime
  compares it against the prior to decide whether to write through).
- **`UnixInstant` has no `Default`**, so `IssueRetry` needs `#[serde(default =
  "epoch_zero")]` on `last_failure_seen_at` plus a **manual `impl Default for
  IssueRetry`** — the `ServiceMapHydrator::RetryMemory` precedent (a View input
  field whose type is `!Default`).
- **The backoff deadline is RECOMPUTED each tick** from `last_failure_seen_at` +
  `attempts` against the live backoff schedule (`now_unix >= last_failure_seen_at
  + backoff_for_attempt(attempts)` — the exact `development.md` § "Reconciler I/O"
  worked-example shape), never persisted. The backoff gate governs **only the
  `running ∧ ¬held ∧ ¬ever_issued` path** (rev 5 — D10): it emits `IssueSvid`
  only when no `IssueRetry` entry exists OR the backoff window has elapsed. A
  `next_attempt_at` field would be a persist-derived-state smell.
- **rev 5 — the backoff gate is SECOND, after the restart-recovery short-circuit
  (D10).** The `running ∧ ¬held ∧ ever_issued` case (a prior successful mint whose
  hold was lost to a restart, proven by the `issued_certificates` audit row in
  `actual`, D4/D10) re-issues **IMMEDIATELY and bypasses the backoff gate**, and
  clears the alloc's retry entry. This is the fix for the
  `restart_after_successful_issue_before_clear_stalls_reissue_until_backoff_elapses`
  characterized defect: a stale retry entry from a successful issue can no longer
  suppress restart recovery, because the audit-row branch is evaluated before the
  gate and evicts the entry. **Record-on-emit is RETAINED but made restart-aware**
  by this short-circuit (D10 spells out the retained-vs-record-on-observed-failure
  latitude and the three non-negotiable invariants). The retry entry is cleared
  whenever the alloc becomes `held` OR `ever_issued` — so it never persists as a
  live failure across a restart.
- **GC** `IssueRetry` entries for allocations no longer Running (mirror
  `ServiceMapHydrator`'s `retain`).

> **rev 6 reframe (2026-06-09) — the near-expiry branch emits a LIVE
> `Action::IssueSvid`, not a gated `StartWorkflow`.** Below, this section
> describes the `running ∧ held(near-expiry)` branch as targeting
> `Action::StartWorkflow(cert_rotation)` behind the `ROTATION_ENABLED` gate. The
> `built-in-ca-operator-composition` feature retires that: internal SVID
> near-expiry reissue is a reconciler **action** — the branch emits
> `Action::IssueSvid { alloc_id, spiffe_id: held.spiffe_id, node_id:
> running.node_id, correlation: identity_correlation(alloc, &held.spiffe_id,
> "rotate-svid") }` **unconditionally** (the existing variant, unchanged — no new
> field). The `ROTATION_ENABLED` gate const, `CERT_ROTATION_WORKFLOW` name, and
> `StartWorkflow`/`WorkflowName` imports are deleted. The threshold is **½ ×
> `WORKLOAD_SVID_TTL` = 1800s** (verified TTL = 3600s), derived-from-TTL. The
> `near_expiry` `<=` boundary becomes a LIVE mutation target (the gate is gone),
> so its `#[mutants::skip]` + `.cargo/mutants.toml` `exclude_re` entry are removed
> and a boundary kill-test lands in the same slice. Everything below about
> reading the held cert's *real* `not_after` from `actual` stays exactly right —
> only the emitted action and the gate change.

**Near-expiry keys off the HELD cert's real `not_after` (from `actual`), NOT a
View field.** The near-expiry reissue branch (`running ∧ held(near-expiry)` —
rev 6: a live `Action::IssueSvid`, NOT a workflow) reads the
`not_after` the `held_snapshot` projected into `actual` (D4 — `HeldSvidFacts.
not_after`, sourced from `SvidMaterial::not_after()`, the cert's true validity
end) and compares it against `tick.now_unix` + the live near-expiry threshold.
There is no `expires_at` anywhere — `not_after` is an observed fact of the held
material, not a derived View value. **As of the ADR-0063 rev 2 amendment
(2026-06-08) this `not_after` is a real `SvidMaterial` field** (it was a design
placeholder before), and — load-bearing for this branch — it derives from the
SAME injected `Clock` as `tick.now_unix`, so the comparison is between two values
off one clock and is replayable bit-for-bit under a DST seed. Before the
amendment, `held.not_after` would have been a host wall-clock read (sub-second
skew from `tick`) or a `SimCa` frozen-fixture value (unrelated to `SimClock`,
non-deterministic) — the near-expiry branch would have tested against garbage. The
amendment is what makes this branch sound.

**The near-expiry branch emits `Action::IssueSvid` unconditionally (rev 6).** The
`running ∧ held(near-expiry)` branch in `reconcile()` emits the existing
`Action::IssueSvid` variant — unchanged, `"rotate-svid"` correlation — through the
same action-shim executor as first-issue and restart-reissue. There is **no**
`ROTATION_ENABLED` gate, **no** `CERT_ROTATION_WORKFLOW`, **no**
`StartWorkflow(cert_rotation)`, and **no** `#39`/workflow dependency. Nothing is
committed to the workflow engine, so there is **no** `UnknownWorkflow` risk — the
empty-registry-engine concern is gone entirely with the action reframe. This branch
is **distinct from restart-recovery** (D1): restart re-issue is `running ∧ ¬held →
IssueSvid` (RECOVERY); near-expiry reissue is `running ∧ held(near-expiry) →
IssueSvid`. Both emit `Action::IssueSvid` through the same executor and neither is
the forbidden synchronous-rotation path; keeping the two branches distinct is
load-bearing only so restart recovery is evaluated immediately (D10), not because
one routes through a workflow. Because near-expiry reads `actual.not_after` (D4),
the branch needs **no** View field to carry an issuance timestamp — the held cert's
own validity is the input. **NO throwaway synchronous sync-rotate path.**

> **Provenance (rev 1–5, SUPERSEDED by rev 6 above — do not implement):** earlier
> revisions modelled this branch as an EMIT-GATED `StartWorkflow(cert_rotation)`
> behind `const ROTATION_ENABLED: bool = false`, kept a clean no-op against the
> production empty-registry `WorkflowRegistry::new()` so a committed
> `StartWorkflow` for an unregistered kind would not loop on `UnknownWorkflow`,
> and deferred flipping the gate to #40. rev 6 deletes the gate const, the
> `CERT_ROTATION_WORKFLOW` name, and the `StartWorkflow`/`WorkflowName` imports
> (single-cut). A DELIVER agent must NOT preserve any of them; only external-ACME /
> public-trust root rotation remains workflow-shaped.

### D9 — Architecture-rule enforcement

- **dst-lint** (existing) keeps `reconcile()` pure: no `.await` / no real-infra
  call on the `overdrive-core` compile path → the `SvidLifecycle` reconciler and
  `SpiffeId::for_allocation` stay pure; the `Ca` handle cannot leak into core.
- **`tests/integration/identity_read_equivalence.rs`** — the DST equivalence test
  driving `IdentityMgr` and `SimIdentityRead` through the same calls (the
  enforcement for the `IdentityRead` trait contract, per `development.md` § "The
  DST equivalence test is the structural guard").
- **`assert_eventually!("running allocs hold a valid SVID")`** — the North-Star
  (O1 / K1) DST invariant over the held `BTreeMap` vs the running set; a
  deliberately-broken executor (drops the hold, or fails to drop) fails it.
- **Earned Trust (probe contract)** — see § Earned Trust below.

### D10 — The `issued_certificates` audit row is the durable restart-recovery signal in `actual`; the retry View is strictly failure-backoff memory (rev 5)

**The defect (characterized, not hypothetical).** The
`restart_after_successful_issue_before_clear_stalls_reissue_until_backoff_elapses`
test (a passing characterization in
`crates/overdrive-control-plane/tests/integration/workload_identity_manager/lifecycle.rs`)
pins it: with record-on-emit (D8 rev 2–4) a **successful** `IssueSvid` bumps
`IssueRetry { attempts, last_failure_seen_at }` on EMIT. The runtime persists
`next_view` BEFORE dispatch (ADR-0035 §5 step 7→8). A crash *after* the executor
succeeds but *before* the next converged tick clears the entry leaves restart
with `actual = ∅` (held set volatile — ADR-0063 D9) PLUS a durable, stale retry
entry. The reconcile body's backoff gate
(`tick.now_unix < last_failure_seen_at + backoff_for_attempt(attempts)`) then
SUPPRESSES the immediate restart re-issue D1 promises until the backoff window
elapses. The reconciler's only durable cross-tick memory was its `View`, and its
`actual` mixed a *volatile* success signal (the in-memory held set) with nothing
durable to disambiguate "minted-then-lost-hold-on-restart" from "still failing."

**Two distinct durable signals were conflated.** They must be carried by two
distinct durable inputs:

1. **Restart-recovery signal (a SUCCESS fact)** — "this alloc was successfully
   issued before." The durable carrier is the `issued_certificates` **audit row**
   (written inside `issue_and_audit` on mint success, BEFORE hold — D6's
   audit-before-hold ordering). So `audit-row-exists ∧ ¬held ⟹
   minted-but-lost-hold ⟹ restart ⟹ re-issue IMMEDIATELY, never back off`.
2. **Failure-backoff signal (a FAILURE fact)** — "the last issue attempt did not
   produce a held SVID." A *failed* mint leaves **no** audit row (the audit write
   is inside `issue_and_audit`, downstream of a successful mint; an `issue_and_audit`
   failure either failed to mint or refused on audit-write failure — D6 — so no
   row lands), so the audit row alone cannot carry this. The durable carrier stays
   the `IssueRetry` View entry — but it now governs ONLY the
   `¬held ∧ no-audit-row` path.

**The `actual` projection gains the audit-row fact.** `SvidLifecycleState.actual`
is extended so the reconciler can read, per allocation, BOTH "is it held?" (the
volatile held snapshot, unchanged) AND "does an `issued_certificates` audit row
exist for its identity?" (the durable success fact). The runtime's
`hydrate_actual` arm for `SvidLifecycle` already reads
`state.identity.held_snapshot()`; it additionally reads the audit rows it already
has a handle to — `state.obs.issued_certificate_rows()` — and projects, per
running alloc's derived `spiffe_id`, whether a row exists. **Keying:** the
`issued_certificates` row carries `spiffe_id` (NOT `alloc_id` —
`crates/overdrive-core/src/ca/issued_certificate_row.rs:82-97`). `SpiffeId::for_allocation`
is a deterministic derivation (D5), so the reconciler derives the expected
`spiffe_id` for each running alloc and tests membership against the set of
`spiffe_id`s observed in the audit rows. The fact projected is a **boolean
presence** — `ever_issued: bool` per running alloc — NOT the row contents
(`serial`, `not_after`, `issued_at` stay out of the reconciler's `actual`; the
near-expiry `not_after` continues to come from the *held* cert via
`HeldSvidFacts`, D4/D8, because only a held cert has a live validity window the
near-expiry reissue branch cares about).

**The reconcile-body model the crafter implements (pin — this is the model, not a
signature).** For each running alloc, in priority order:

| held | ever_issued (audit row) | branch | retry-View effect |
|---|---|---|---|
| held | (any) | no-op; evaluate near-expiry → emit `IssueSvid` if near-expiry (D8 — rev 6, unconditional, not gated) | **clear** the alloc's retry entry (clear-on-success) |
| ¬held | true | **restart recovery** → emit `IssueSvid` IMMEDIATELY, bypassing the backoff gate | **clear** the alloc's retry entry (a prior success is durably proven; no failure is pending) |
| ¬held | false | first-issue / failing path → emit `IssueSvid` **backoff-gated** by the retry entry | record/keep the attempt (failure-backoff memory) |

The load-bearing ordering: **the `ever_issued` check is evaluated BEFORE the
backoff gate.** When it fires (the restart case), the backoff gate is never
reached, so a stale retry entry cannot suppress the recovery re-issue — and the
entry is cleared in the same tick because a durable success is proven. The
backoff gate governs ONLY the `¬held ∧ ¬ever_issued` case — a genuinely
never-succeeded alloc that may be repeatedly failing.

**Record-on-emit vs record-on-observed-failure — what rev 5 pins.** Record-on-emit
is **retained, but made restart-aware** by the `ever_issued` short-circuit above —
it is no longer a "persist-a-failure-not-yet-observed" violation in *effect*,
because the durable success fact (the audit row) overrides it the instant it
appears: a success makes the alloc `held` (→ clear) or, post-restart, `¬held ∧
ever_issued` (→ clear + immediate re-issue), in both cases evicting the entry
before it can gate anything. The retry entry is therefore only ever *consulted as
a backoff timer* for an alloc with no audit row — the one case where "no success
has been observed" is durably true. Equivalent and also acceptable: shift to
strict **record-on-observed-failure** (bump only when `¬held ∧ ¬ever_issued ∧ an
entry already exists OR an emit was made this tick), which records the attempt but
treats the audit row as the sole authority on "did it actually succeed." The
crafter MAY pick either; both produce the identical observable behaviour because
the `ever_issued` branch is what makes restart immediate. **The non-negotiable
invariants** the crafter must satisfy: (1) `¬held ∧ ever_issued` re-issues with
NO backoff gate; (2) a successful issue's retry entry is cleared by the *next*
tick's `held` OR `ever_issued` branch (it never persists as a live failure across
a restart); (3) the `¬held ∧ ¬ever_issued` path remains backoff-gated so a
genuinely failing mint does not hammer every tick.

**Why this is NOT a synchronous-rotation path and NOT a new mechanism.** The
recovery branch is the ordinary `running ∧ ¬held → IssueSvid` first-issue branch
(D1); the audit-row read adds no `Action`, no concurrency primitive, no store —
it reads an ObservationStore row the runtime already has a handle to (the same
`state.obs` the WS test reads via `issued_certificate_rows()`), exactly as
`alloc_status` rows are read into `desired`. The pure reconciler stays pure: it
holds no `ObservationStore` handle (the runtime does the read in `hydrate_actual`
and hands the projected `actual` in), reads no wall-clock except `tick.now`, and
passes dst-lint.

**Persist-inputs-not-derived-state compliance.** Nothing new is persisted. The
audit row is an existing observation INPUT (D6 records it as an audit input — not
a derived value); the reconciler READS it, it does not write it. The retry View
continues to persist only `attempts` + `last_failure_seen_at` (inputs), with the
backoff deadline recomputed each tick from those inputs + the live policy. The
`ever_issued` boolean is a *recomputed-each-tick projection* of the durable audit
rows — derived at read time, never persisted. This is strictly more
persist-inputs-compliant than rev 2–4: record-on-emit's latent "persist a failure
that hasn't happened" is now overridden by the durable success fact rather than
trusted across a restart.

**Interaction with `view_has_backoff_pending` (the runtime self-re-enqueue
predicate) — CONFIRMED UNCHANGED.** The `SvidLifecycle` arm of
`view_has_backoff_pending` (`reconciler_runtime.rs:1574`) is `!view.retry.is_empty()`
— "re-enqueue while any retry entry survives, so a mid-backoff alloc is re-ticked
rather than draining the broker empty." Rev 5 does NOT change this predicate, and
it remains correct under D10:

- A retry entry now exists ONLY for an alloc in the `¬held ∧ ¬ever_issued`
  backoff path — a genuinely-never-succeeded, possibly-failing alloc. That is
  exactly the case the self-re-enqueue must keep alive (the reconcile emits no
  `IssueSvid` mid-window but `actual ≠ desired`, so without the predicate the
  broker drains and the loop sleeps). `!retry.is_empty()` correctly flags it.
- A restart-recovery alloc (`¬held ∧ ever_issued`) re-issues immediately and its
  retry entry is CLEARED that tick (D10), so it does not depend on
  `view_has_backoff_pending` at all — the emitted `IssueSvid` is itself the
  `has_work` signal, and the entry is gone before the next tick. No stale entry is
  left to spuriously hold the predicate true.
- A successful issue clears its entry on the `held` OR `ever_issued` branch, so
  the predicate flips to false once convergence is reached — no busy-loop. This is
  the same "flips false at terminal" discipline the `WorkloadLifecycle` /
  `ServiceLifecycle` arms maintain.

So the already-shipped `!view.retry.is_empty()` predicate is exactly right under
the rev-5 model: it is true IFF a genuinely-failing alloc is mid-backoff, false
once every alloc is held / ever-issued / GC'd. **No adjustment needed.**

## Alternatives Considered

These are the DIVERGE options and the per-surface alternatives weighed in
PASS-1; each is rejected with its reason.

### A1 (DIVERGE Option 2) — Fold identity into `WorkloadLifecycle`

Have the existing `WorkloadLifecycle` reconciler emit the issue/drop actions
inline rather than a standalone `SvidLifecycle`. **Rejected:** identity
availability is its own convergence target with its own North-Star invariant
(O1) — coupling it to workload lifecycle entangles two desired-vs-actual
relations and makes the `assert_eventually!("running allocs hold a valid SVID")`
invariant harder to express and reason about. A separate reconciler is the
locked Option 1; it mirrors the established `ServiceMapHydrator` precedent (a
dedicated reconciler per convergence concern).

### A2 (DIVERGE Option 3) — `watch`/`broadcast` push read surface

Expose held identity as a `watch` channel consumers subscribe to (notified on
change) rather than sync getters. **Rejected (deferred):** speculative until a
real consumer demands change-notification — no consumer exists this phase (#26 is
unbuilt). The sync-getter `IdentityRead` port is the sound first step and is a
**non-breaking** surface: a future external-ACME / public-trust root rotation can
push a rotated bundle down the *same* port later (`set_bundle` is already the push
seam, D6). Adding a channel now is mechanism the
running set does not yet need (O6).

### A3 (DIVERGE Option 4) — Persist issuance success facts / a derived renewal deadline in the View

Two rejected sub-options collapse here (rev 2 merged them once the View became
retry-memory):

- **Persist a derived `expires_at` / `next_renewal_at`** so the near-expiry check
  is a direct comparison. **Rejected (CORRECTNESS):** persist-derived-state
  anti-pattern. Near-expiry is recomputed each tick from the held cert's real
  `not_after` (read off `actual`, D4/D8) + the live threshold; persisting a
  deadline ships a stale cache of today's TTL.
- **Persist issuance success facts (`serial` / `issued_at` / `spiffe_id`) as
  `IssuedInputs`** (rev 1's design). **Rejected (CORRECTNESS — the Critical +
  High-1 findings):** `serial` is a post-dispatch executor output the pure
  reconciler cannot know, and the runtime persists `next_view` BEFORE dispatch
  (`reconciler_runtime.rs:1222-1226` vs `:1324`) — a View entry claiming "issued"
  could be durably written when the CA/audit write then fails. "Is this alloc
  held?" is answered by `actual` (the held set, D1/D4); the success fact lives in
  the `issued_certificates` observation row. The View carries only retry inputs
  (`attempts`, `last_failure_seen_at` — D8).

**A3 reconciliation with the rev-5 restart-recovery signal (D10) — explicit.**
D10 reads the `issued_certificates` audit row as part of `actual` to drive
immediate restart recovery. This is a *durable success fact*, so the reconciliation
with A3's rejection above must be stated, not implied:

- **A3 rejects WRITING a new success fact into the reconciler's durable memory**
  (the View) or into intent. The rev-1 `IssuedInputs` design tried to make the
  *reconciler itself* persist `{serial, issued_at, spiffe_id}` as proof-of-issue
  into the View — a fact the pure reconciler cannot even know (post-dispatch
  output) and which the BEFORE-dispatch persist ordering makes a potential lie.
  That is still rejected, unchanged.
- **D10 does NOT write any success fact. It READS one that already exists.** The
  `issued_certificates` row is written by `issue_and_audit` (D6) into the
  ObservationStore — it is *observation*, the same state layer `alloc_status` (the
  `desired` source) and `service_backends` live in. Projecting an observation row
  into a reconciler's `actual` is what EVERY reconciler does — it is the definition
  of `actual` (whitepaper §18; `development.md` § "Reconciler I/O" rule 5:
  "`HttpCall` responses are observation … reconcilers read it locally via
  `actual`"). The reconciler holds no store handle, derives no new fact, persists
  nothing; the runtime reads the row in `hydrate_actual` and folds a boolean
  presence into `actual`, recomputed every tick.
- **The distinction is WHO persists and WHEN.** A3 forbids the *reconciler* from
  durably claiming a success it cannot verify before the executor confirms it. D10
  relies on the *executor* (via `issue_and_audit`) having ALREADY durably recorded
  the success — audit-before-hold (D6) — and the reconciler merely observes that
  durable record. There is no window where the reconciler asserts a success the
  executor has not committed: the audit row exists IFF `issue_and_audit` succeeded
  far enough to write it, and D6 binds issuance to refuse on audit-write failure,
  so `audit-row-exists ⟹ the mint succeeded and was audited`. Reading it is sound;
  persisting a parallel copy in the View would not be (and is not done).

So the rule preserved across rev 1 → rev 5 is: **the reconciler never WRITES a
success fact; it READS the one the executor already durably wrote.** D10 honours
A3 exactly because it adds a *read of observation into `actual`*, not a *write of
derived success into memory*.

### A4 (DIVERGE Option 5) — Pull `Ca::trust_bundle()` on demand on every read

`current_bundle()` calls through to the CA each read. **Rejected (chosen 5-A
hydrated instead):** a CA pull on the read hot path violates O3 ("minimize time a
dataplane consumer takes to read the current SVID + trust bundle"). The bundle
changes only on **root / external rotation** (future external-ACME / public-trust);
hydrating it into `IdentityMgr` (set at boot, refreshed by the issue executor,
pushed by a future root-rotation path via `set_bundle`) makes the read touch zero
CA I/O while staying current. A cheap pull was *permitted* by the DISCUSS AC, but
hydration is strictly better for O3 and gives the future root-rotation path its
push seam. (Internal SVID near-expiry reissue does not change the bundle — it
re-mints a leaf under the same intermediate — so it is unrelated to this seam.)

### A5 (DIVERGE Option 6) — A throwaway synchronous in-`reconcile` mint

> **rev 6 reframe (2026-06-09).** This section's *substance* was right —
> rotation is mint+swap, single-cut — but its *framing* was wrong: it called
> rotation "a multi-step durable sequence — a workflow (#40)" and described the
> rejected option as a "throwaway synchronous *sync-rotate* path." The
> `built-in-ca-operator-composition` feature corrects this: **internal SVID
> near-expiry reissue is the PERMANENT design as a reconciler *action*
> (`Action::IssueSvid` rotate-correlation), not a workflow.** The reconciler only
> EMITS the action; the action-shim *executor* does the CA I/O (the sanctioned
> ADR-0023 action-shim boundary), exactly as first-issue and restart-reissue do.
> A5's rejection still stands, but it applies ONLY to the genuinely-rejected
> shape: a *synchronous mint INSIDE `reconcile`* (CA I/O on the pure reconciler
> hot path, which would break dst-lint + the pure-`reconcile` contract). The
> action-emit is NOT that — it is the ordinary `IssueSvid` branch firing on a
> near-expiry condition. The `ROTATION_ENABLED` gate and the `cert_rotation`
> workflow name are deleted in that feature; see § Revision (rev 6).

Ship a minimal in-`reconcile` rotation (a SYNCHRONOUS mint-fresh + swap *inside*
`reconcile`). **Rejected:** CA I/O inside the pure-sync `reconcile` body breaks
dst-lint and the pure-`reconcile` contract (ADR-0035) — minting is the executor's
job, not the reconciler's. The correct shape is the reconciler EMITTING
`Action::IssueSvid` (a near-expiry rotate-correlation) and the action-shim
executor performing the mint+swap — which is exactly what
`built-in-ca-operator-composition` ships (rev 6). That is NOT a "throwaway"
path and NOT a workflow: it is the permanent rotation design as a reconciler
action.

### A6 — Drop `node_id` from `Action::IssueSvid`

Read the node from `AppState.node_id` in the executor and keep the action
slimmer. **Rejected (KEEP node_id):** the action should be self-describing — the
issuance request names the node it is issued on (the `issued_certificates` row's
`node_id`, the `issue_and_audit(…, node, …)` argument), and #36 multi-node makes
"the only node" assumption false. The executor MAY still read `AppState.node_id`
in Phase 2, but carrying `node_id` on the action keeps the action the SSOT for
what was requested and needs no reshaping when multi-node lands. One `NodeId` is
cheap.

### A7 — `IdentityMgr` over `tokio::sync::RwLock`

Use an async lock so the read surface could be `async`. **Rejected:** the
critical section is a synchronous map mutation / clone-out that does not (and must
not) cross an `.await`; `parking_lot::RwLock` is the project default for sync
critical sections and the read/clone/drop shape keeps the guard off every await
point (`.claude/rules/development.md` § "Concurrency & async"). An async lock
would invite holding the guard across `.await` — the exact bug the sync lock
prevents.

### A8 — `IdentityMgr` held map as `HashMap`

`HashMap<AllocationId, SvidMaterial>` for the held set. **Rejected:** the held
map is iterated by the `assert_eventually!` North-Star invariant, so its
iteration order must be deterministic across DST seeds (`.claude/rules/development.md`
§ "Ordered-collection choice"; K5). `BTreeMap` is mandatory.

## Consequences

### Positive

- **Identity is operationally true for the running set** (O1 / K1): the held set
  is bound to the running-alloc set and proven converged by the North-Star DST
  invariant, not asserted.
- **No leak on stop** (O2 / K2): drop-on-stop removes the entry so the node-held
  leaf private key is no longer reachable; observable through the read surface
  (`svid_for == None`).
- **Zero CA I/O on the read hot path** (O3): the hydrated bundle + held-map
  serve reads in-process behind a sync getter; whitepaper §7 "no gRPC, no IPC".
- **Restart recovery — bounded, audited, no stale/silent credential** (O4 / K3,
  reframed rev 2): on boot the held set is empty (the leaf key is non-persistable),
  so every still-Running alloc is re-issued (`running ∧ ¬held → IssueSvid`), one
  per running alloc, each audited via `issue_and_audit`. No secret at rest; no
  silent re-use of an unrecoverable key. A *failed* re-issue backs off via the
  retry-memory View. (There is no "recompute without re-issue" — that is impossible;
  rev 1's claim was the Critical finding.)
- **No silent issuance** (O5 / K4): `issue_and_audit` is reused wholesale — every
  issuance writes an `issued_certificates` row and an audit-write failure refuses
  the issuance; no unaudited `SvidMaterial` ever reaches the held map.
- **Mechanism economy** (O6): no new concurrency / storage mechanism — the
  shipped reconciler runtime, `Ca` port, ObservationStore, and one in-process
  `RwLock<BTreeMap>` hold the whole subsystem. Internal SVID near-expiry reissue
  reuses the same `Action::IssueSvid` executor (rev 6 — no new mechanism); a future
  root-rotation path reuses the same `set_bundle` push seam.
- **Reuse-heavy**: `Ca` / `ca_issuance::issue_and_audit` / `SvidMaterial` /
  `TrustBundle` / `IssuedCertificateRow` / reconciler runtime / action-shim /
  `CorrelationKey` / `AllocationId` / `NodeId` / `WorkloadId` / `CertSerial` /
  `UnixInstant` / `Entropy` are all reused as-is (see brief § Reuse Analysis).

### Negative / costs

- **`AppState` grows two fields** (`ca: Arc<dyn Ca>`, `identity:
  Arc<IdentityMgr>`) and the shim signature threads `ca` / `clock` / `identity`
  into the two new arms. This is the "found wiring" — recorded here as an ADR
  consequence, not a silent change. It is additive (the existing `AppState`
  consumers are untouched) and the production `Arc<dyn Ca>` is the *same* adapter
  `ca_boot` already builds (lib.rs:50).
- **The near-expiry reissue branch emits `Action::IssueSvid` unconditionally**
  (rev 6 — NO gating caveat, NO `ROTATION_ENABLED`, NO `StartWorkflow`): a single
  internal mint+swap through the existing executor, `"rotate-svid"` correlation.
  Nothing is committed to the workflow engine, so the empty-registry
  `UnknownWorkflow` concern is gone. The near-expiry branch reads the held cert's
  real `not_after` off `actual` (D4), so it needs no View field to drive it.
  Restart re-issue (`¬held → IssueSvid`) and near-expiry reissue (`held ∧
  near-expiry → IssueSvid`) are distinct branches that both emit `Action::IssueSvid`
  through the same executor; neither is the forbidden synchronous-rotation path.
  *(Provenance: rev 1–5 modelled this as an EMIT-GATED `StartWorkflow(cert_rotation)`
  behind `const ROTATION_ENABLED: bool = false`; rev 6 deletes that gate, name, and
  the `StartWorkflow`/`WorkflowName` imports. A DELIVER agent must NOT preserve
  them.)*
- **`SvidLifecycle` needs its enqueue/handoff wiring landed with it** (rev 2,
  D5b): a third `Action::EnqueueEvaluation` emission in
  `WorkloadLifecycle::reconcile` (`workload_lifecycle.rs:181` block) and a sibling
  `broker().submit` in the exit observer (`exit_observer.rs:230-256`). Additive to
  both producers; without it the reconciler builds correctly but never ticks. A
  DELIVER regression test (Running AND Stopped transitions tick `SvidLifecycle`,
  no manual broker poke) is the gate.
- **The two existing private SPIFFE helpers MUST migrate to `for_allocation` in
  the same feature** (rev 2, D5): `mint_alloc_identity`
  (`backend_discovery_bridge.rs:424`) + `mint_identity`
  (`workload_lifecycle.rs:808`). Shipping `for_allocation` alongside them leaves
  three implementations of one identity string.
- **The retry-memory View needs `Eq` + a manual `impl Default`** — 6 derive
  bounds (not 4) because the runtime NextView diff compares views; `UnixInstant:
  !Default` forces a `#[serde(default = "epoch_zero")]` + manual `Default` on
  `IssueRetry` (the `RetryMemory` precedent). Minor, but a crafter who derives only
  the usual 4 bounds will not compile against the runtime's diff.
- **`hydrate_actual` for `SvidLifecycle` now reads the `issued_certificates`
  rows** (rev 5, D10) — one additional `state.obs.issued_certificate_rows()` read
  per tick to project the `ever_issued` boolean per running alloc. `hydrate_actual`
  is already `async` and the runtime already holds `state.obs`; the read is
  O(rows) and cheap at single-node scale. The reconciler stays pure — it holds no
  store handle; the runtime does the read and folds the boolean into `actual`. No
  new `Action`, no new store, no new persisted View field. The append-only growth
  of `issued_certificates` (one row per first-issue / restart re-issue / near-expiry
  reissue — all `Action::IssueSvid`, rev 6; already noted for #215) means the
  per-tick scan grows over an
  allocation's lifetime; #36 (multi-node, gossiped audit rows) and any future
  read-index are the place to bound it if it becomes a hot path — out of scope for
  Phase 2 single-node.

### Earned Trust (probe contract)

Every dependency the identity path leans on that *could lie* is probed before the
system relies on it — *wire then probe then use*:

- **`Ca` is reachable and the boot bundle is composable** — the composition root
  pulls `Ca::trust_bundle()` to seed `IdentityMgr::new(Some(bundle))` at boot;
  this is the ADR-0063 CA-probe path (KEK present, persisted root decrypts and is
  adopted) — if the CA refuses to start, the identity subsystem never wires. The
  identity layer inherits ADR-0063's `health.startup.refused` posture: no held
  bundle ⇒ the CA boot already refused.
- **The held map honors drop** — the `assert_eventually!("running allocs hold a
  valid SVID")` invariant + a deliberately-broken executor (drops the hold / fails
  to drop) is the behavioural probe that the holder actually holds and actually
  drops (K1 / K2). The `identity_read_equivalence` DST test exercises the
  `IdentityRead` contract (incl. clause 5: post-drop `svid_for == None`) against
  both adapters.
- **No silent issuance** — `issue_and_audit`'s existing binding (refuse issuance
  on audit-write failure, ADR-0063 D6) is the probe that the audit row is real
  before the SVID is held; the host-adapter fault-injection (audit-write failure)
  is flagged for DISTILL.
- **The reconciler actually ticks at the lifecycle moments** (rev 2, D5b) — the
  enqueue/handoff regression test is the probe that `SvidLifecycle` is *reachable*:
  a Running transition AND a Stopped transition each cause an `EnqueueEvaluation`
  / broker submit for `job/<workload_id>` with no manual broker poke. A reconciler
  that builds but is never enqueued is the silent failure this probe catches —
  the `assert_eventually!` invariant only holds if the convergence actually runs.
- **Restart recovery actually re-issues** (rev 2, D1) — a DST restart-mid-run
  scenario (empty the `IdentityMgr` held set, retick) is the probe that
  `running ∧ ¬held` re-issues every still-Running alloc, each leaving a fresh
  `issued_certificates` row, with no stale/silent credential. A surviving leaf
  verifies (`openssl verify`) at the TEST tier.
- **Restart recovery is IMMEDIATE even after a crash mid-issue** (rev 5, D10) —
  the characterization test
  `restart_after_successful_issue_before_clear_stalls_reissue_until_backoff_elapses`
  is FLIPPED to assert the desired behaviour: a crash AFTER a successful issue but
  BEFORE the clearing tick, followed by restart, re-issues the still-Running alloc
  IMMEDIATELY (logical-0 tick, no backoff wait) because `¬held ∧ ever_issued`
  short-circuits the backoff gate. This is the probe that the durable audit-row
  success signal — not the volatile held set alone — drives recovery, and that a
  stale retry entry can no longer suppress it. A genuinely-failing alloc
  (`¬held ∧ ¬ever_issued`) still backs off — a sibling assertion proves the gate
  is NOT bypassed for the never-succeeded case.

These probes are the composition-root invariant; the `identity_read_equivalence`
DST test + the North-Star invariant + the enqueue-handoff regression test + the
restart-recovery DST scenario + the inherited ADR-0063 CA boot probe exercise the
substrate. Fault-injection scenarios (audit-write failure, broken hold/drop) are
flagged for DISTILL.

## References

- GH #35 [2.13] — Workload identity manager (this feature).
- Feature delta: `docs/feature/workload-identity-manager/feature-delta.md`
  (DISCUSS + DESIGN).
- DIVERGE: `docs/feature/workload-identity-manager/diverge/recommendation.md`
  (Option 1 locked; the 5 design-sensitive surfaces) + `job-analysis.md`
  (J-SEC-002, ODI outcomes O1–O6).
- ADR-0063 — built-in CA (`Ca` port, `SvidMaterial`, `TrustBundle`,
  `ca_issuance::issue_and_audit`, `issued_certificates` audit row). This ADR
  holds/reads/drops what ADR-0063 mints.
- ADR-0013 / ADR-0035 / ADR-0036 — reconciler primitive runtime, typed-View
  ViewStore, `AnyState`/`AnyReconciler` registration triple.
- ADR-0023 — action-shim placement (the executor's async boundary).
- ADR-0064 / ADR-0066 — `Workflow` trait + journal (the destination for
  **external-ACME / public-trust** rotation only; the engine stays
  empty-registry in production — internal SVID near-expiry reissue is a
  reconciler *action*, NOT a workflow, so it does NOT register here).
- Whitepaper §4 (state layers — held material is in-process, audit is observation,
  View is reconciler memory), §7 (sidecarless — no in-pod agent; node-held leaf
  key), §18 (durable workflows — applies to **external-ACME** rotation only;
  **internal** SVID near-expiry reissue is an `Action::IssueSvid`, see the rev 6
  reframe in § "Downstream boundary"), §21 (DST).
- Deferrals (rev 6, 2026-06-09 — all cite EXISTING issues / roadmap lines):
  **#215 — CLOSED by `built-in-ca-operator-composition`** (operator `alloc
  status` render of `issued_certificates` (O05) + the exported-PEM `openssl
  verify` chain proof (E03); no longer blocked on #35); **#40 internal SVID
  near-expiry rotation — CLOSED** (a live `Action::IssueSvid`, NOT a workflow,
  does NOT need **#39**); **#26** — sockops/kTLS mTLS consumer; **#36** [2.14]
  multi-node held sets / node attestation; **roadmap step 4.7** — ACME /
  public-trust certs unified into `IdentityMgr` (the only remaining
  **workflow-shaped** rotation, future-owned); **Phase 5** — SVID revocation
  (CRL/OCSP). **#217** (input_digest) is unaffected by this ADR. *(Historical,
  superseded: "#215 blocked on #35; #40 [3.3] rotation needs #39 [3.2] workflow
  primitive" — that framing was external-ACME, never internal SVID reissue.)*

## Downstream boundary with #40 (rotation) and #215 (operator surface)

Rev 2 sharpens the boundary the re-issue-on-restart model creates with the two
downstream features. **rev 6 (2026-06-09):** #40 + #215 are both folded into the
`built-in-ca-operator-composition` feature; the #40 boundary is rewritten below
(internal SVID near-expiry reissue is a reconciler *action*, not a workflow), and
this ADR's emit/gate text is corrected accordingly.

### Near-expiry reissue — owned HERE as `Action::IssueSvid` (rev 6; #40 owns nothing for internal SVID reissue)

Re-issue-on-restart makes **#35** own *issue + hold + swap-into-`IdentityMgr` +
converge*: when a fresh `SvidMaterial` is minted (first issue, restart re-issue,
OR near-expiry reissue), `IdentityMgr::hold(alloc, svid)` **replaces** the prior
entry under the same `AllocationId`. Internal near-expiry reissue is owned by THIS
feature as a live `Action::IssueSvid` (rev 6); #40 no longer owns any part of it —
only external-ACME / public-trust root rotation remains future-/#40-adjacent.

> **rev 6 — internal SVID near-expiry reissue is OWNED HERE as a reconciler
> ACTION, not deferred to a workflow.** The prior framing ("#40 owns the durable
> rotation *workflow* — the near-expiry → request → wait-for-DNS-propagation →
> validate → publish sequence, the textbook Bar-2 workflow") was a **fiction for
> internal SVID reissue**: that 4-step external-wait shape is **external ACME
> public-cert** rotation, not internal SVID reissue. An internal near-expiry SVID
> reissue is a SINGLE internal mint+swap — the SAME `Action::IssueSvid` the
> first-issue and restart-reissue paths already emit, with a `"rotate-svid"`
> correlation purpose. It coordinates no external steps and has no external-wait
> terminal, so it fails the `.claude/rules/workflows.md` workflow-candidacy test.
> The `SvidLifecycle` reconciler EMITS `IssueSvid`; the action-shim *executor*
> performs the mint+swap (the sanctioned ADR-0023 boundary). The
> `built-in-ca-operator-composition` feature ships this: it deletes the
> `ROTATION_ENABLED` gate and the `cert_rotation` workflow name, and the
> near-expiry branch emits `IssueSvid` unconditionally.
>
> **External-ACME public-cert rotation** (if it ever ships) is a SEPARATE
> concern — that one genuinely coordinates ≥2 external steps and would be a
> workflow (the TBD candidate first production workflow, Phase 5,
> revocation-coupled; see `.claude/rules/workflows.md` § precedent). It does not
> touch the internal SVID path.

**The leaf-key-on-the-wire boundary (rev 2, unchanged):** the leaf private key
MUST NEVER enter the gossiped ObservationStore (`CaKeyPem` has no `Serialize`,
ADR-0063 D9; the leaf key is not an audit fact). Reissue *status / audit* lands as
observation (the `issued_certificates` row `issue_and_audit` writes); the
**`SvidMaterial` itself is swapped IN-PROCESS** into `IdentityMgr` (via `hold` +
`set_bundle`), exactly as #35's first-issue and restart-re-issue paths do. No
material on the wire.

**Three branches, all the SAME action (rev 6):** first-issue, restart re-issue
(`running ∧ ¬held → IssueSvid`, RECOVERY), and near-expiry reissue (`running ∧
held(near-expiry) → IssueSvid` with a `"rotate-svid"` correlation, keyed off the
held cert's real `not_after` from `actual`) all emit `Action::IssueSvid` and
dispatch through the same executor. None is a synchronous in-`reconcile` mint
(A5, rejected); none is a workflow.

### #215 (operator surface — CLOSED by `built-in-ca-operator-composition`, rev 6) — append-only audit shape

> **rev 6 (2026-06-09) — #215 is CLOSED, no longer blocked on #35.** The
> `built-in-ca-operator-composition` feature builds the operator surface this
> section describes: its Slice ② wires the persistent CA into `serve` (boot-side)
> and its Slice ③ lands the consumer-side `AllocStatusResponse.issued_certificates`
> summary + CLI render. **Surface split (load-bearing for DELIVER):** the
> summary render is the **O05** surface (operator-legible metadata — `serial /
> spiffe_id / issuer_serial / not_after`, NO cert bytes/key); **E03** (full chain
> verifies) is proven SEPARATELY by an exported-PEM `openssl verify` capture
> (test-only env-gated export from `rcgen_ca_chain_verify.rs`), NOT by the
> summary render. The current owner/status is unambiguous: **this feature, not a
> future #35-unblock.**

Re-issue-on-restart **plus** near-expiry reissue means **many `issued_certificates`
rows per allocation over time** (the table is append-only audit — one row per
issuance: first issue, each restart re-issue, each near-expiry reissue — all
`Action::IssueSvid`, rev 6). The operator surface therefore renders the **current**
cert — the **max-`issuance_ordinal`** row matching `SpiffeId::for_allocation(...)`
(the strictly-ordered selection key; `issued_at` is retained as an audit fact,
NOT the selection key) (cross-referenced via `IdentityRead` / the held set),
*(Amended 2026-06-11, feature-delta § D1-AMEND: selection key moved from
latest-by-`issued_at` to max-`issuance_ordinal` — a fixed `SimClock` can tie two
issuances on `issued_at`, so the timestamp is not strictly ordered; the global
monotonic ordinal breaks the tie deterministically.)* NOT one row per alloc and NOT
the whole history as if each were live. A **serial change after a restart** should
read as *legible* ("re-issued on restart"), NOT as an anomaly — the operator
surface must treat a post-restart serial change as expected recovery, not a
security event. O05 ("no silent issuance") is *reinforced* by this: every restart
re-issue and every near-expiry reissue leaves an audit row, so the operator can
always see *why* the current serial is what it is. *(Historical, superseded: "#215 is blocked on #35 and owns the
render — this ADR does not build it" was true for ADR-0067 rev 2; rev 6 folds
#215 into `built-in-ca-operator-composition`.)*

## Revision (rev 2, 2026-06-08) — DESIGN-review findings

This revision addresses the 5 findings of the REJECTED-pending-revisions DESIGN
review (`docs/feature/workload-identity-manager/design/review-design.md`). Each
finding, the resolution, and where it landed:

| Finding (severity) | Rev-1 defect | Rev-2 resolution | Where |
|---|---|---|---|
| **Critical** — restart-idempotence impossible | "recompute held state on boot with no re-issue" — impossible: leaf key non-persistable (`CaKeyPem` no `Serialize`, ADR-0063 D9) + non-reconstructable (`ca_issuance.rs:34-40` mints fresh each call) | **Re-issue on restart** for every still-Running alloc (`running ∧ ¬held → IssueSvid`, bounded, audited) — RECOVERY, distinct from the near-expiry reissue branch (rev 6: also a live `Action::IssueSvid`, NOT a workflow). O4/K3 reframed "no redundant re-issue" → "bounded, audited restart re-issue; no stale/silent credential." | D1; Context driver 4; § upstream-changes.md |
| **High-1** — View holds executor outputs | `IssuedInputs{issued_at, spiffe_id, serial}`: `serial` is post-dispatch (`reconciler_runtime.rs:1222-1226` persists `next_view` BEFORE dispatch at `:1324`); persisting success facts is broken | **The held set becomes `actual`** (the key addition — grounded against the `WorkflowLifecycle`/`live_instances()` precedent, `:2206-2209`/`:2166`). The View drops to **retry memory** (`IssueRetry{attempts, last_failure_seen_at}`). Success facts live in `issued_certificates`. | D1; D4; D8; A3 |
| **High-2** — no enqueue/handoff trigger | `SvidLifecycle` would never tick | **`Action::EnqueueEvaluation`** from `WorkloadLifecycle::reconcile` (`:181` block, third emission, ungated) + the exit observer (`:230-256`, sibling submit); target `job/<workload_id>`; broker LWW dedup; DELIVER regression test obligation. | **D5b** (new) |
| **High-3** — stale slice briefs | slices 01/02/03 still say "DESIGN call" | Slices updated: top-of-file "implement ADR-0067 rev 2" note + inlined decisions; slice-03 restart model corrected (re-issue, not recompute). | slice-01/02/03 |
| **Medium** — `SpiffeId::for_allocation` mislabelled net-new | two private helpers already derive the same string | Reframed as **canonical extraction** of `mint_alloc_identity` (`backend_discovery_bridge.rs:424`) + `mint_identity` (`workload_lifecycle.rs:808`); DELIVER migrates both call sites. | D5; Reuse Analysis |

**Held-set-as-`actual` feasibility verdict (the #1 grounding question): FEASIBLE,
no blocker.** The runtime's `hydrate_actual` (`reconciler_runtime.rs:2190`) is a
`match` over `AnyReconciler`; the `WorkflowLifecycle` arm (`:2206-2209`) already
projects a *non-persisted in-process runtime set* — the workflow engine's
live-task set (`hydrate_workflow_actual_instances:2152` →
`state.workflow_engine.live_instances():2166`) — into `actual`. `IdentityMgr` is an
`Arc<...>` field on `AppState` exactly as `workflow_engine` is (`lib.rs:281`); a
new `AnyReconciler::SvidLifecycle(_)` arm reading `state.identity.held_snapshot()`
(sync, in-process — D4) is the identical shape, with no runtime-mechanism change
(one new `match` arm). The held-set-as-`actual` is implementable against the
runtime as written.

## Revision (rev 3, 2026-06-08) — `SvidMaterial.not_after` made real

This revision fixes a **design/code contradiction** caught while reviewing this
ADR against the shipped CA code, and pins the exact surface the #35 crafter
implements. It touches D4 and D8 only; D1/D2/D5/D5b/D6/D7/D9 are unchanged.

| Finding | Defect | Resolution | Where |
|---|---|---|---|
| **D4 `not_after` asserted a non-existent field** | D4/D8 (rev 1/2) described `HeldSvidFacts.not_after` as "the cert's real `not_after` (`SvidMaterial`'s validity end)", but `SvidMaterial` (`crates/overdrive-core/src/traits/ca.rs:298-357`) has NO `not_after` field — its fields are `cert_pem, cert_der, serial, spiffe_id, leaf_key`. The validity window was an adapter-internal value (`RcgenCa` read `SystemTime::now()`; `SimCa` returned a frozen-fixture window) that never crossed the trait boundary. | **ADR-0063 rev 2 amendment** (2026-06-08): `SvidMaterial` gains `not_after: UnixInstant` (+ accessor), populated at mint from the validity window `ca_issuance::issue_and_audit` computes ONCE from its injected `Clock` and threads through `SvidRequest`. `held_snapshot` reads `svid.not_after()` directly; D4 is now literally true. | D4; ADR-0063 Changelog "rev 2 amendment" |
| **D8 near-expiry seam was comparing against garbage** | The near-expiry branch (rev-3-era: described as a "gated #40 branch"; **SUPERSEDED — rev 6 makes it a live unconditional `Action::IssueSvid`, no gate, no workflow**) compares `actual.not_after` vs `tick.now_unix`. With `held.not_after` a host wall-clock read (sub-second skew) or a `SimCa` fixture value (unrelated to `SimClock`, non-deterministic), the comparison was unsound and DST-non-deterministic. | The threaded window derives from the SAME injected `Clock` as `tick.now_unix`. `svid.not_after() == issued_certificates.not_after` by construction (one clock read, one value, used for both cert and audit row). The branch is now sound and replayable under a seed. | D8 |

**Consistency mechanism (why `svid.not_after == row.not_after` by construction).**
`issue_and_audit` (`overdrive-control-plane/src/ca_issuance.rs:142-198`) already
computes the audit window from its injected `clock` (`issued_at =
UnixInstant::from_clock(clock); not_before = issued_at − SKEW_TOLERANCE; not_after
= not_before + WORKLOAD_SVID_TTL`, `:171-184`). Under this amendment it computes
that window **once, before minting**, builds the windowed `SvidRequest`, passes it
to `ca.issue_svid(...)`, and **reuses the exact same `not_before`/`not_after`
`UnixInstant` values** for the `IssuedCertificateRow`. There is no second clock
read on either path — the cert window and the row window are *the same two
variables*. DST-determinism follows because the one read is
`UnixInstant::from_clock(clock)` over the injected `SimClock`.

**Not a persist-derived-state violation.** `not_after` on `SvidMaterial` /
`HeldSvidFacts` / the held set is an **observed fact of the minted credential**
(the leaf is non-reconstructable and its window is fixed at mint and embedded in
the signed bytes), the same shape as `issued_certificates.not_after` (D6 records
it as an audit *input*). It is NOT a `next_attempt_at`-style recompute-from-policy
deadline — a reviewer must not flag it as one. The recompute-from-inputs deadline
in this feature is D8's `IssueRetry` backoff, which is unchanged.

### PINNED SURFACE SPEC (the exact shapes the #35 crafter implements)

Production `.rs` is the crafter's; this pins the contract so no surface is
invented. Every shape below is grounded against HEAD (`file:line` cited).

**1. `SvidMaterial` — gains `not_after` (`overdrive-core/src/traits/ca.rs`,
amending the struct at `:298-357`).**

```rust
pub struct SvidMaterial {
    cert_pem:  CaCertPem,
    cert_der:  CaCertDer,
    serial:    CertSerial,
    spiffe_id: SpiffeId,
    leaf_key:  CaKeyPem,
    not_after: UnixInstant,   // NEW (rev 2 ADR-0063) — the leaf's validity end
}

impl SvidMaterial {
    // trailing param appended; existing 5 params unchanged
    pub const fn new(
        cert_pem:  CaCertPem,
        cert_der:  CaCertDer,
        serial:    CertSerial,
        spiffe_id: SpiffeId,
        leaf_key:  CaKeyPem,
        not_after: UnixInstant,   // NEW
    ) -> Self { /* ... */ }

    pub const fn not_after(&self) -> UnixInstant { self.not_after }   // NEW accessor
}
```
Add `use overdrive_core::wall_clock::UnixInstant;` (or the in-crate path) to
`ca.rs` imports (`:28-29` currently import only `KekId` + `{CertSerial,
CertSpecError, NodeId, SpiffeId}`). `not_after` participates in derived
`PartialEq`/`Eq`/`Clone`/`Debug` (`UnixInstant` derives all four). The leaf key
stays redacted in `Debug`; `not_after` is non-secret and prints plainly.

**2. `SvidRequest` — carries the validity window (`ca.rs:263-279`).**

```rust
pub struct SvidRequest {
    spiffe_id:  SpiffeId,
    not_before: UnixInstant,   // NEW
    not_after:  UnixInstant,   // NEW
}

impl SvidRequest {
    pub const fn new(
        spiffe_id:  SpiffeId,
        not_before: UnixInstant,   // NEW
        not_after:  UnixInstant,   // NEW
    ) -> Self { /* ... */ }

    pub const fn spiffe_id(&self)  -> &SpiffeId   { &self.spiffe_id }
    pub const fn not_before(&self) -> UnixInstant { self.not_before }   // NEW
    pub const fn not_after(&self)  -> UnixInstant { self.not_after }    // NEW
}
```
`Ca::issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial>` — **signature
unchanged** (`ca.rs:673`); the window rides on the request.

**3. `RcgenCa::issue_svid` (`overdrive-host/src/ca/rcgen_ca.rs:388-503`).**
- DELETE the `SystemTime::now()` read: remove the `now = date_time_ymd(1970,1,1)
  + Duration::from_secs(Self::seconds_since_epoch())` line (`:478`) and the
  `seconds_since_epoch` helper (`:300-316`).
- STAMP the threaded window: convert `req.not_before()` / `req.not_after()`
  (`UnixInstant`) to rcgen `OffsetDateTime` via the same idiom —
  `params.not_before = date_time_ymd(1970,1,1) +
  req.not_before().as_unix_duration(); params.not_after = date_time_ymd(1970,1,1)
  + req.not_after().as_unix_duration();` (replacing `:479-480`).
- CARRY it on the result: append `req.not_after()` to the `SvidMaterial::new(...)`
  call (`:496-502`).

**4. `SimCa::issue_svid` (`overdrive-sim/src/adapters/ca.rs:329-372`).**
- CARRY the threaded window: append `req.not_after()` to the
  `SvidMaterial::new(...)` call (`:365-371`). **Fixture cert bytes
  (`FIXTURE_SVID_CERT_PEM/DER`) unchanged** — consistent with the documented
  fixed-identity limitation (`:348-364`: structured fields track the request,
  opaque bytes are fixed). `SimCa` needs NO clock (its `new` takes only
  `Entropy`, `:18-22`).

**5. `ca_issuance::issue_and_audit`
(`overdrive-control-plane/src/ca_issuance.rs:142-198`).**
- COMPUTE the window FIRST (move the `issued_at`/`not_before`/`not_after` block
  from `:171-175` to BEFORE the mint), then build the windowed request and mint:
  ```rust
  let issued_at  = UnixInstant::from_clock(clock);
  let not_before = UnixInstant::from_unix_duration(
      issued_at.as_unix_duration().saturating_sub(SKEW_TOLERANCE));
  let not_after  = not_before + WORKLOAD_SVID_TTL;
  let windowed   = SvidRequest::new(request.spiffe_id().clone(), not_before, not_after);
  let svid = ca.issue_svid(&windowed).map_err(CaIssuanceError::ca)?;
  ```
  Build `IssuedCertificateRow { not_before, not_after, .. }` from the SAME two
  values (`:177-185` unchanged). Result: `svid.not_after() == row.not_after`.
- **Parameter choice:** `issue_and_audit` keeps taking `request: &SvidRequest`
  and reads only `request.spiffe_id()` (it already passes nothing else from the
  request — `:161`); it IGNORES any window on the passed request and builds its
  own windowed copy, because the clock is the single window SSOT. (Equivalent and
  also acceptable: change the param to `spiffe_id: &SpiffeId`. The crafter picks
  one; do NOT compute the window in the executor.) **STOP-and-surface if the
  reviewer wants the param narrowed to `&SpiffeId`** — that is a signature change
  beyond the minimum; default is to keep `&SvidRequest`.

**6. Call sites the crafter updates** (all verified against HEAD):
`SvidMaterial::new` — `rcgen_ca.rs:496`, `sim/adapters/ca.rs:365`,
`core/traits/ca.rs:875` (Debug-redaction test). `SvidRequest::new` /
`ca.issue_svid` — `rcgen_ca_chain_verify.rs:277/343/416`,
`sim_ca_fixture_cert_key_match.rs:46`, `sim_ca_deterministic.rs:197/291/361`,
`ca_equivalence.rs:358/475`, `ca_boot_and_audit.rs:729` (`workload_request()`
helper) + `:618` (sad-path mock `issue_svid`, ignores `_req` — unchanged).
`issue_and_audit` — `ca_boot_and_audit.rs:755/808/867/905/908` + the #35 executor
(roadmap 01-06, not yet built). Tests pass a fixed
`UnixInstant::from_unix_duration(..)` window; this is the same mechanical sweep
D9's `leaf_key` addition required.

**`HeldSvidFacts` / `held_snapshot` (D4) — now backed by the real field.**
`HeldSvidFacts { spiffe_id: SpiffeId, not_after: UnixInstant }` is built per held
alloc from `svid.spiffe_id().clone()` + `svid.not_after()`. No surface beyond D4's
already-named shape is added.

## Revision (rev 5, 2026-06-09) — restart-recovery success signal vs failure-backoff signal

This revision resolves a restart-recovery defect surfaced by adversarial review
of the DELIVER implementation and characterized by the passing test
`restart_after_successful_issue_before_clear_stalls_reissue_until_backoff_elapses`
(`crates/overdrive-control-plane/tests/integration/workload_identity_manager/lifecycle.rs`).
It touches **D1, D4, D8, A3** and adds **D10**; D2/D3/D5/D5b/D6/D7/D9 are
unchanged.

| Finding (severity) | Defect | Resolution | Where |
|---|---|---|---|
| **High — restart recovery is not immediate when a crash lands between issue-success and the clearing tick** | Record-on-emit (D8 rev 2–4) persists an `IssueRetry` entry on a SUCCESSFUL `IssueSvid`. The runtime persists `next_view` BEFORE dispatch (ADR-0035 §5 7→8). A crash after the executor succeeds (SVID minted, `issued_certificates` row written, held in-memory) but before the next converged tick clears the entry leaves restart with `actual = ∅` (held set volatile, ADR-0063 D9) PLUS a stale durable retry entry. The backoff gate then suppresses the immediate restart re-issue D1 promises until the backoff window elapses. The reconciler conflated a volatile success signal (held set) with a durable one (audit row) that diverge exactly on restart. | Add the `issued_certificates` audit row to `actual` as an `ever_issued: bool` per running alloc (keyed on the derived `spiffe_id` — audit rows carry `spiffe_id`, not `alloc_id`). `¬held ∧ ever_issued` is an unambiguous restart marker (audit-before-hold, D6) → re-issue IMMEDIATELY, bypassing the backoff gate, evaluated BEFORE the gate, clearing the retry entry. The retry View narrows to a strict failure-backoff signal governing only the `¬held ∧ ¬ever_issued` path. Record-on-emit is retained but made restart-aware by the short-circuit (or, equivalently, shifted to record-on-observed-failure — crafter's latitude, three invariants pinned). | **D10** (new); D1; D4; D8 |
| **A3 reconciliation (required, was implicit)** | A3 rejects "persist issuance success facts." D10 reads a durable success fact (the audit row). The relationship must be explicit, not implied. | Made explicit in A3: A3 rejects the *reconciler WRITING* a success fact into its durable memory; D10 *READS* an observation row the executor already durably wrote (`issue_and_audit`, D6). Reading observation into `actual` is what every reconciler does (whitepaper §18; Reconciler I/O rule 5). No new persisted fact; the reconciler holds no store handle. The preserved rule: the reconciler never writes a success fact, it reads the one the executor wrote. | A3 |
| **`view_has_backoff_pending` interaction** | The runtime self-re-enqueue predicate (`!view.retry.is_empty()`, `reconciler_runtime.rs:1574`) was already fixed for the immediate stall. Confirm it stays correct under the new model. | CONFIRMED unchanged. A retry entry now exists ONLY for a genuinely-failing `¬held ∧ ¬ever_issued` alloc (exactly the case the predicate must keep alive); restart-recovery and successful-issue allocs clear their entries the same tick, so no stale entry spuriously holds the predicate true. No adjustment. | D10 |

**Persist-inputs verdict (rev 5): COMPLIANT and strictly improved.** Nothing new
is persisted — the audit row is an existing observation input the reconciler
reads; the retry View persists only `attempts` + `last_failure_seen_at` (inputs)
with the deadline recomputed each tick; `ever_issued` is a recomputed-each-tick
projection of durable observation, never persisted. Record-on-emit's latent
"persist a failure not yet observed" is now overridden by the durable success
fact rather than trusted across a restart.

**The pinned crafter model (rev 5).** `SvidLifecycleState.actual` yields per
allocation: `held?` (volatile `HeldSvidFacts`, unchanged) AND `ever_issued?`
(durable boolean from `issued_certificates` rows keyed on the derived
`spiffe_id`). The reconcile body, per running alloc, in priority order:
`held → no-op + clear retry, then if near-expiry emit IssueSvid (rev 6 —
unconditional, NOT gated, NOT a workflow)`; `¬held ∧ ever_issued → emit
IssueSvid IMMEDIATELY (no backoff gate) + clear retry`; `¬held ∧ ¬ever_issued →
emit IssueSvid backoff-gated + record/keep the attempt`. The three non-negotiable
invariants: (1) `¬held ∧ ever_issued` re-issues with no backoff gate; (2) a
successful issue's retry entry is cleared by the next tick's `held`/`ever_issued`
branch; (3) `¬held ∧ ¬ever_issued` stays backoff-gated. The exact Rust signature
of the `actual` projection (a `bool` field on the per-alloc actual value vs a
separate set) is an implementation detail — the crafter picks the cleanest shape
that satisfies the three invariants and keeps the leaf key off `actual`; do NOT
invent a new `Action` variant, a new store, or a new persisted View field. The
characterization test that pins the defect must be FLIPPED from
characterization-of-defect to assert the DESIRED behaviour (immediate
post-restart re-issue) once D10 lands — it is the regression anchor.

## Revision (rev 6, 2026-06-09) — #40 rotation is a reconciler ACTION, not a workflow

This revision is the DESIGN output of the `built-in-ca-operator-composition`
feature (folds #40 + #215; see `docs/feature/built-in-ca-operator-composition/
feature-delta.md`). It corrects a framing error that ran through rev 1–5: the #40
near-expiry rotation seam was modeled as a *gated `cert_rotation` workflow* that
would emit `Action::StartWorkflow`. That is wrong for **internal SVID reissue**.
It touches **A5, D8, D1, and the #40-boundary section**; D2/D3/D4/D5/D6/D7/D9/D10
are unchanged.

| Finding | Defect | Resolution | Where |
|---|---|---|---|
| **#40 internal near-expiry reissue was framed as a workflow** | D8 + the #40-boundary section described "near-expiry → request → wait-for-DNS-propagation → validate → publish" as the textbook Bar-2 workflow #40 ships, with the reconciler emitting `StartWorkflow(cert_rotation)` behind the `ROTATION_ENABLED` gate. That 4-step external-wait shape is **external ACME public-cert rotation**, NOT internal SVID reissue. An internal near-expiry SVID reissue is a SINGLE internal mint+swap — it coordinates no external steps and has no external-wait terminal, so it is NOT workflow-shaped. | The near-expiry branch emits `Action::IssueSvid` (the EXISTING variant, with a `"rotate-svid"` correlation purpose) UNCONDITIONALLY. The `ROTATION_ENABLED` gate const, the `CERT_ROTATION_WORKFLOW` name, and the `StartWorkflow`/`WorkflowName` imports are DELETED (single-cut). The action dispatches through the SAME action-shim executor as first-issue/restart-reissue. | D8, #40-boundary |
| **A5's framing called the rejected option a "synchronous sync-rotate path" and rotation "a workflow"** | A5 was substantively right (rotation is mint+swap, single-cut) but mis-framed: it rejected a "throwaway sync-rotate path" and asserted rotation "is a workflow (#40)." | Reframed: A5 rejects ONLY a *synchronous mint INSIDE `reconcile`* (CA I/O on the pure-sync hot path — breaks dst-lint + the pure-`reconcile` contract). The reconciler EMITTING `IssueSvid` and the executor doing the mint+swap is the PERMANENT design — not throwaway, not a workflow. | A5 |
| **Near-expiry threshold const was wrong on two counts** | `NEAR_EXPIRY_THRESHOLD_SECS = 28_800` (8h) assumed a 24h TTL; the real `WORKLOAD_SVID_TTL` is **3600s (1h)**. It was also a bare literal not tied to the TTL. | Threshold = **½ × `WORKLOAD_SVID_TTL` = 1800s**, derived-from-TTL (persist-inputs spirit; SPIRE half-life norm). | D8 |
| **The `near_expiry` `<=` boundary was a permanently-excluded mutation target** | Because the emit was gated (`ROTATION_ENABLED == false`), the `<=` boundary was a genuine equivalent mutant and was excluded via `#[mutants::skip]` + a `.cargo/mutants.toml` `exclude_re` entry. | With the gate retired the boundary is OBSERVABLE (a real `IssueSvid` emit). The `#[mutants::skip]` and the `exclude_re` entry are REMOVED, and a boundary kill-test (`held.not_after <= now + 1800s` → exactly one rotate `IssueSvid`; none otherwise) lands in the same DELIVER slice. A mandatory mutation target. | D8 |

**External-ACME rotation is the separate concern.** If public-trust gateway-cert
rotation ever ships (roadmap step 4.7 / Phase 5, revocation-coupled), THAT
genuinely coordinates ≥2 external steps and would be a workflow — the TBD
candidate first first-party production workflow per `.claude/rules/workflows.md` §
"Codebase precedent" (corrected in the same feature). It does not touch the
internal SVID path this ADR governs.

**D1/D8 restart-re-mint re-validation (rev 6).** Restart = re-mint (leaf keys are
non-persistable, ADR-0063 D9; on boot the held set is empty) is **re-validated
against this feature** and is correct as-is. The rev 5 D10 `running ∧ ¬held ∧
ever_issued → IssueSvid` branch (re-issue IMMEDIATELY, bypassing backoff, on the
audit-row restart marker) is correct as-is — `built-in-ca-operator-composition`
confirms it and does NOT reshape it. The near-expiry rotate path (rev 6) and the
restart-recovery path both emit `Action::IssueSvid`; they remain distinct branches
(the rotate path fires on `running ∧ held(near-expiry)`, recovery on `running ∧
¬held ∧ ever_issued`).
