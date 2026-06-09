# Research synthesis — workload-identity-manager (for the concept/explanation doc)

**Purpose**: evidence base for `website/content/docs/concepts/workload-identity-lifecycle.mdx`.
**Type**: Explanation (DIVIO) → website `concepts/`. Sibling to `identity.mdx`.
**Accuracy constraint (website C-6)**: document only real, implemented behaviour.
Sources are the feature's own SSOT artifacts plus the shipped code on branch
`marcus-sa/svid-lifecycle-trust-bundle`. No external/web sources — this is an
internal subsystem.

## What the feature is (one line)

The `IdentityMgr` subsystem binds a live, chain-verifiable SVID to the *exact
set of currently-running allocations*, holds it in-process where the dataplane
can read it, and drops it the moment a workload stops. GH #35 / roadmap 2.13 /
J-SEC-002. Architecture: ADR-0067 (rev 5).

## The gap it closes (vs. the built-in CA)

- The built-in CA (#28, J-SEC-001, `identity.mdx`) **mints** SVIDs. It can be
  perfect and this job still unmet: the SVID is mintable in principle but never
  held, never readable by the mTLS layer, never dropped on stop.
- Overdrive is sidecarless (whitepaper §7): no in-pod agent to fetch/hold/drop a
  credential. So the credential's lifecycle can *only* be driven from the
  allocation lifecycle the control plane already owns.
- This feature is the **holder / reader / dropper** built on top of the CA.

Source: `feature-delta.md` § Feature Summary; `diverge/job-analysis.md` J-SEC-002.

## The mechanism (verified in code)

1. **`SvidLifecycle` reconciler** (`crates/overdrive-core/src/reconcilers/svid_lifecycle.rs`).
   Pure `reconcile(desired, actual, view, tick) → (Vec<Action>, View)`. No
   `.await`, no CA handle, no ObservationStore handle, wall-clock only via
   `tick.now`. Convergence rule (ADR-0067 D2):
   - `running ∧ ¬held → Action::IssueSvid`
   - `¬running ∧ held → Action::DropSvid`
   - `desired` = the Running-allocation set; `actual` = the `IdentityMgr` held
     set (projected to `HeldSvidFacts { spiffe_id, not_after }` — the leaf key
     is never projected into a reconciler input, K2).
2. **Action-shim executor** (`crates/overdrive-control-plane/src/action_shim/issue_svid.rs`).
   The one place workload-CA I/O happens. Mints via the shipped
   `ca_issuance::issue_and_audit` (binds issuance + the `issued_certificates`
   audit row; audit-write failure refuses issuance) and writes `SvidMaterial`
   into `IdentityMgr`. `DropSvid` calls `IdentityMgr::drop_svid`.
3. **`IdentityMgr`** (`crates/overdrive-control-plane/src/identity_mgr.rs`).
   In-process held set: `BTreeMap<AllocationId, SvidMaterial>` + the current
   `TrustBundle`, behind a `parking_lot::RwLock` (guard never held across
   `.await`). The held set is **ephemeral runtime state** — neither intent nor
   observation; never persisted (the leaf `CaKeyPem` has no `Serialize` and is
   non-reconstructable, ADR-0063 D9). `BTreeMap` so DST iteration is
   deterministic across seeds (K5).
4. **`IdentityRead` port** (`crates/overdrive-core/src/traits/identity_read.rs`).
   Two sync, owned-clone getters: `svid_for(&AllocationId) -> Option<SvidMaterial>`
   and `current_bundle() -> Option<TrustBundle>`. Five contract clauses pinned in
   the trait docstring and enforced by the `identity_read_equivalence` DST test
   across the real `IdentityMgr` and the `SimIdentityRead` double:
   (1) a read never issues, (2) a read never mutates, (3) `None` is explicit
   absence, (4) returns owned clones (no lock held after the read), (5) post-drop
   absence is observable. Bundle is served HYDRATED (set at boot, refreshed by
   the issue executor) — zero CA I/O on the read hot path (O3).

## Restart recovery (Slice 03, ADR-0067 rev 2 D1)

The held set is the reconciler's `actual`, and it is in-process — so on a fresh
boot `actual = ∅`. Every still-Running alloc then reads as `¬held`, which is
exactly the `running ∧ ¬held → IssueSvid` trigger: the control plane re-issues a
fresh, audited SVID for each. The View is **retry memory only**
(`IssueRetry { attempts, last_failure_seen_at }`) — a *failed* re-issue backs off
via `backoff_for_attempt`. The View carries no `serial`/`issued_at`/`expires_at`
(persist inputs, not derived state); issuance success facts live in the
`issued_certificates` observation, never the View.

## Convergence proof + audit

- DST invariant `svid-running-set-holds-valid-svid`
  (`crates/overdrive-sim/src/invariants/svid_running_set.rs`):
  `assert_eventually!("running allocs hold a valid SVID")` walks the held
  `BTreeMap` against the running set at every stable point. Has teeth: a
  deliberately broken hold/drop fails it. North Star = K1.
- The bounded window between Running and held-SVID is **fail-closed, not a race**:
  a workload with no held SVID cannot present identity, so the (future) mTLS
  consumer fails closed. In Phase 2 there is no consumer at all, so the window is
  doubly inert.
- Every issuance writes an `issued_certificates` audit row via
  `issue_and_audit`; an unauditable issuance is refused (no silent issuance, K4).

## Rotation seam — wired but dormant

The near-expiry branch is structurally present (`NEAR_EXPIRY_THRESHOLD_SECS =
28_800`, compared against the held cert's real `not_after` from `actual`), and
targets `Action::StartWorkflow(cert-rotation)`. It is **gated/dormant** until #40
registers the `cert_rotation` workflow — a committed `StartWorkflow` for an
unregistered kind would surface `UnknownWorkflow` every tick. Certificate
rotation is the canonical first workflow (#40).

## Where it stands (the honest boundary — verified in `lib.rs`)

- **Wired into `overdrive serve`**: `svid_lifecycle()` reconciler is registered
  (`lib.rs:1289`); `IdentityMgr` is constructed and threaded into `AppState`
  (`lib.rs:1602-1613`). So in a running control plane today, Running allocations
  *do* get an SVID issued, held, audited, and dropped on stop. This is unlike the
  persistent built-in CA, which is not wired.
- **The CA backing it in `serve` is EPHEMERAL** (`lib.rs:1580-1599`): a fresh
  in-memory P-256 root is minted each boot — NO KEK, NO persistence, NOT the
  `boot_ca` persistent root that `identity.mdx` describes. So SVIDs issued today
  do not survive a restart as *valid* across the old root; the restart-recovery
  path re-issues them under the new boot root. The persistent KEK-backed root +
  operator-render surface are GH #215 (blocked on #35).
- **No consumer yet**: the sockops/kTLS mTLS layer (#26), gateway, and telemetry
  that would *read* the held SVID and present it on the wire are unbuilt. The
  read port + its sim double + a test consumer prove the contract; no production
  consumer is wired.
- **No operator verb**: hold/read/drop is an internal mechanism; there is no
  `overdrive` subcommand. The `alloc status` render of `issued_certificates` and
  the deployed-SVID operator-verify flow are #215's deliverable, blocked on #35.
  (Per CLAUDE.md the workload verb is `overdrive deploy <SPEC>`.)

## Cross-links for the doc

- `/docs/concepts/identity` — the built-in CA that mints (the minter; this is the
  holder).
- `/docs/concepts/reconcilers` — the pure-reconcile primitive this rides on.
- `/docs/concepts/intent-observation` — state-layer hygiene (the held set is
  neither, the audit row is observation).
- `/docs/concepts/workflows` — the rotation seam's destination (#40).
- `/docs/concepts/deterministic-simulation-testing` — the convergence invariant.

## Things to NOT claim (accuracy guardrails)

- Do NOT say the held SVID is presented on a real connection — no consumer exists.
- Do NOT say the root is persistent/envelope-encrypted in `serve` — it is
  ephemeral there (the persistent one is #28/#215, not this feature's serve path).
- Do NOT describe drop-on-stop as memory-zeroization — it is reachability
  (entry removed from the held map). Zeroization is explicitly out of #35.
- Do NOT invent a CLI verb for issuing/holding/inspecting SVIDs.
- Do NOT describe the rotation workflow as active — the seam is dormant until #40.
