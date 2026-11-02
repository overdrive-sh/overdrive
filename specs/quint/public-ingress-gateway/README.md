# Quint specification: public ingress gateway (public-ingress-gateway)

Model-checked specification of the concurrent, ordered and crash-sensitive protocols the public-ingress-gateway
DESIGN pins (`.claude/rules/design.md` § "Concurrent protocols carry a model-checked Quint specification"), and the oracle
DISTILL's `quint-connect` drivers replay (`.claude/rules/testing.md` § "Quint conformance"). Design text:
`docs/feature/public-ingress-gateway/feature-delta.md` § "Wave: DESIGN / [REF] Formal protocol model".

Scope (remediation r1): only the interleavings a prose review cannot enumerate. Route+key coherence, bind gating,
last-usable key preservation, identity-loss unbind, cold-boot order and shutdown ordering are single-task sequential
rules and are DISTILL seeded-sim or unit obligations, not modelled.

Every module is a state machine (state variables, `init`, `step`) with named runs for its critical scenarios; every
run is listed in `checks.toml` (`backend = "evaluator"`). Each design decision has one hazard instance in `hazard/`
(the decision's rule switched off through `OFF`), `expect = "violation"`.

Run (metal host, never a local build): `cargo xtask metal run -- cargo xtask quint typecheck`, then
`cargo xtask metal run -- cargo xtask quint check --subsystem public-ingress-gateway [--name <check>] [--ci]`.
Evidence of the last full run: `evidence/` (written only by `--record`; `cargo xtask quint verify-evidence --subsystem
public-ingress-gateway` fails when it no longer matches the specs).

## Modules, decisions, scenario -> named run

| Module | Decision | Safety invariants (`Safety`, TLC exhaustive on `*_ok`) | Named runs (main `*_conf`) |
|---|---|---|---|
| `gateway_identity.qnt` | ADR-0225 | `HoldImpliesAudit`, `StaleEpochNeverHolds`, `AtMostOneHeld`, `DisableClearsOrTimesOut` | `issue_hold` (happy path), `reissue_near_expiry`, `disable_races_inflight_issue`, `audit_failure_no_hold`, `restart_reissue` |
| `gateway_application.qnt` (drain and supersession only) | ADR-0222 | `DrainRetainsUntilLeasesZero`, `SupersededResultNeverRestores` | `replace_valid` (supersede, drain, retire), `route_withdraw_drain`, `late_probe_after_supersession` (a superseded staged probe result is discarded) |
| `selection_receipt.qnt` | ADR-0228, ADR-0232, ADR-0233, ADR-0241, ADR-0243 (outcome table ADR-0242) | `IdentityReadableOnlyAfterCommit`, `IdentityImmutablePerBackendId`, `ReceiptMatchesIntent`, `NoResidueAfterCleanup`, `RollbackPreservesPriorGeneration`, `EmptyIsNoBackend` | `commit_then_select`, `abandoned_reservation_rolled_back`, `conflicting_alloc_rejected`, `receipt_waits_for_publish_guard`, `restart_destroys_receipts` |

If the user adopts P-9 (honour BACKEND_MAP.healthy), add the healthy rule, its invariant and a named run then.

## Hazards (one per decision)

| Decision | Hazard (OFF switch -> main) | Invariant that fails |
|---|---|---|
| ADR-0225 | `noEpochCheck` -> `identity_bugNoEpochCheck` | `StaleEpochNeverHolds` |
| ADR-0222 | `retireWhileLeased` -> `app_bugRetireWhileLeased` | `DrainRetainsUntilLeasesZero` |
| ADR-0233 | `publishEarly` -> `receipt_bugPublishEarly` | `IdentityReadableOnlyAfterCommit` |
| ADR-0241 | `rebind` -> `receipt_bugRebind` | `IdentityImmutablePerBackendId` |
| ADR-0232 | `takeAny` -> `receipt_bugTakeAny` | `ReceiptMatchesIntent` |

## Validation budget

21 checks: 3 exhaustive TLC safety (identity, application, receipt), 5 hazards, 13 named runs; down
from 30. Only `receipt-safety-tlc` is slow (about 900 s, `ci = false`, timeout 2400 s, bounded at 2 accepted `update_service` calls); every other check is under two minutes (a few seconds each) and `ci = true`. Growth needs user
approval.

## Abstraction

- Time is the injected SimClock (`now`, bounded); TTL/RENEW abstract the reused workload SVID half-life policy.
- `gateway_identity`: one slot, epoch-checked Issue/Drop Actions, audit row written before the hold, volatile slot,
  durable audit rows. Environment actions: `audit_fail`, `advance_time`, `restart`.
- `gateway_application`: an admission object is `{gid, rg}` (generation id, Route generation); connector probe result per
  staged token; leases are per-generation connection counts. A result for a superseded or discarded token changes
  nothing.
- `selection_receipt`: two Service keys, two guest addresses, two allocations, <= 2 gateway sockets, backend sets drawn
  from a six-entry catalogue. The exhaustive main bounds accepted `update_service` calls at 2 (`MAXUPD`), the
  conformance mains at 3 (`abandoned_reservation_rolled_back` needs three); a rejected update changes nothing and is
  not counted. The bound was reduced, not the property.
- Model-only variables (the driver does not project them): see each file's header (`staged`, `inflight`, `leases`;
  identity `now`, ghosts; receipt `outer`, `resv`, `infl`, `guard`, `debt`, `ckey`, `taken`, `g`; `conf_step`).
- `NoAllocationIdentity` is not modelled: it is a type-level fact (the slot is keyed by node, ADR-0225).

## Assumptions and discharging items

| Id | Assumption | Discharged by |
|---|---|---|
| A-K1 | The outer `SERVICE_MAP` slot replacement is atomic for the hook's lookup (`commit`). | V-GW-1, existing atomic-swap Tier-3 (`.claude/rules/testing.md`) |
| A-K2 | The pre-connect `SO_COOKIE` equals the cookie the hook reads (the intent key in `open`/`kernel_connect`). | V-GW-5 HELD (10/10 plain, 10/10 prepared, 7.0.0-38; `net/core/sock.c:2109-2113`, `net/core/filter.c:5151-5154 @ v7.0`) |
| A-K3 | A socket created by a `serve` task lies in the attach cgroup subtree (the hook fires for gateway sockets). | V-GW-2 HELD plus source: `cgroup_sk_alloc`, `kernel/cgroup/cgroup.c:7420-7431`, called from `net/core/sock.c:2330`; `connect4` runs against the socket cgroup (`kernel/bpf/cgroup.c:1694 @ v7.0`); end to end by AT-PIG-E2E-1 |
| A-3 | Every Service backend address lies in the guest prefix. | ADR-0190 consequence; GH #317 |
| GI-1 | `issue_and_audit` is the existing CA issue+audit path; an audit failure leaves no held material. | `audit_failure_no_hold` run replayed against the production path with a failing Sim ObservationStore write (S-PIG-51 / `ca_issuance` tests) |
| GI-2 | The injected clock is monotonic. | `SimClock` contract (unit) |
| GA-1 | The connector probe result is delivered at most once per staged token. | The probe owner's unit-level test: completing a staged probe twice leaves the second ignored |
| SR-1 | The hook writes exactly one receipt per registered cookie and none for an unregistered one. | S-PIG-50 and the Tier-3 V-GW-1 run (control (d): `receipt=None` for an unregistered cookie) |
| SR-2 | `restart` empties the Service maps, memo, receipts and applied associations with the dataplane instance. | Source: `EbpfDataplane::new_with_pin_dir` (`crates/overdrive-dataplane/src/lib.rs:430-454`), allocator memo (`allocators/backend_id.rs:9-18`); DISTILL `restart_destroys_receipts` replay plus Tier-3 503-until-rehydrated check |

GA-2 (a lease is released exactly once) is dropped as an obligation: the lease is an RAII guard.
