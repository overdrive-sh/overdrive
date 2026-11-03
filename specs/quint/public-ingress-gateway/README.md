# Quint specification: public ingress gateway (public-ingress-gateway)

Model-checked specification of the concurrent, ordered and crash-sensitive protocols the public-ingress-gateway
DESIGN pins (`.claude/rules/design.md` § "Concurrent protocols carry a model-checked Quint specification"), and the oracle
DISTILL's `quint-connect` drivers replay (`.claude/rules/testing.md` § "Quint conformance"). Design text:
`docs/feature/public-ingress-gateway/feature-delta.md` § "Wave: DESIGN / [REF] Formal protocol model".

Scope: only the interleavings a prose review cannot enumerate. Route+key coherence, bind gating,
last-usable key preservation, identity-loss unbind, cold-boot order and shutdown ordering are single-task sequential
rules and are DISTILL seeded-sim or unit obligations, not modelled.

Every module is a state machine (state variables, `init`, `step`) with named runs for its critical scenarios; every
run is listed in `checks.toml` (`backend = "evaluator"`). Each design decision has one hazard instance in `hazard/`
(the decision's rule switched off through `OFF`), `expect = "violation"`.

Run (metal host, never a local build): `cargo xtask metal run -- cargo xtask quint typecheck`, then
`cargo xtask metal run -- cargo xtask quint check --subsystem specs/quint/public-ingress-gateway [--name <check>] [--ci]`
(`--subsystem` also accepts the bare directory name `public-ingress-gateway`; a value containing `/` is taken as a path).
Evidence of the last full run: `evidence/` (written only by `--record`; `cargo xtask quint verify-evidence --subsystem
public-ingress-gateway` fails when it no longer matches the specs).

## Modules, decisions, scenario -> named run

| Module | Decision | Safety invariants (`Safety`, TLC exhaustive on `*_ok`) | Named runs (main `*_conf`) |
|---|---|---|---|
| `gateway_identity.qnt` | ADR-0225 | `HoldImpliesAudit`, `StaleEpochNeverHolds`, `DisabledSlotIsEmpty` | `issue_hold` (happy path), `reissue_near_expiry`, `disable_races_inflight_issue`, `audit_failure_no_hold`, `restart_reissue` |
| `gateway_application.qnt` (drain and supersession only) | ADR-0222 | `DrainRetainsUntilLeasesZero`, `SupersededResultNeverRestores` | `replace_valid` (supersede, drain, retire), `route_withdraw_drain`, `late_probe_after_supersession` (a superseded staged probe result is discarded) |
| `selection_receipt.qnt` | ADR-0232, ADR-0233, ADR-0241 (context ADR-0228, ADR-0242, ADR-0248; see below) | `IdentityReadableOnlyAfterCommit`, `IdentityImmutablePerBackendId`, `ReceiptMatchesIntent`, `NoResidueAfterCleanup`, `RollbackPreservesPriorGeneration`, `EmptyIsNoBackend` | `commit_then_select`, `conflicting_alloc_rejected`, `restart_destroys_receipts` (conformance-bound, `receipt_conf`); `abandoned_reservation_rolled_back`, `receipt_waits_for_publish_guard` (evaluator-only on `receipt_ok`, not replayed) |
| `relay_exchange.qnt` (conformance target) | ADR-0239 (context ADR-0245, ADR-0246, ADR-0247, ADR-0253, ADR-0254) | `NoBytePastMessageBoundary` | `exchange_cl_keepalive`, `early_response_closes`, `drain_with_inflight_body`, `half_close_after_request_read`, `upstream_close_mid_body_aborts`, `close_delimited_clean_end`, `public_close_mid_request_aborts` (main `relay_conf`) |

ADR-0228 is a single-owner rule (one dataplane owner), so it has no concurrent protocol and no hazard. ADR-0248 (ledger order) is a
single-task sequential connector rule covered by the seeded invariants S-PIG-37 and S-PIG-53; it is not modelled. ADR-0242's outcome
table is context for the receipt module, not a separate decision with a hazard.

## Hazards (one per decision)

| Decision | Hazard (OFF switch -> main) | Invariant that fails |
|---|---|---|
| ADR-0225 | `noEpochCheck` -> `identity_bugNoEpochCheck` | `StaleEpochNeverHolds` |
| ADR-0222 | `retireWhileLeased` -> `app_bugRetireWhileLeased` | `DrainRetainsUntilLeasesZero` |
| ADR-0233 | `publishEarly` -> `receipt_bugPublishEarly` | `IdentityReadableOnlyAfterCommit` |
| ADR-0241 | `rebind` -> `receipt_bugRebind` | `IdentityImmutablePerBackendId` |
| ADR-0232 | `takeAny` -> `receipt_bugTakeAny` | `ReceiptMatchesIntent` |
| ADR-0239 | `relayPastBoundary` -> `relay_bugRelayPastBoundary` | `NoBytePastMessageBoundary` |

## Validation budget

See the header of `checks.toml` (single home).

## Abstraction

- Time is the injected SimClock (`now`, bounded); TTL/RENEW abstract the reused workload SVID half-life policy.
- `gateway_identity`: one slot, epoch-checked Issue Actions, a synchronous clear at disable (`disable_after_drain` =
  `begin_disable`, no waiting disable, no Drop Action), audit row written before the hold, volatile slot, durable audit rows. Environment actions: `audit_fail`, `advance_time`, `restart`.
- `gateway_application`: an admission object is `{gid, rg}` (generation id, Route generation); connector probe result per
  staged token; leases are per-generation connection counts. A result for a superseded or discarded token changes
  nothing.
- `selection_receipt`: two Service keys, two guest addresses, two allocations, <= 2 gateway sockets, backend sets drawn
  from a six-entry catalogue. `receipt_ok` (exhaustive) bounds accepted `update_service` calls at 2 (`MAXUPD`), which also hosts the
  two evaluator-only runs (`abandoned_reservation_rolled_back` needs two accepted updates); `receipt_conf` bounds them at
  3 (`conflicting_alloc_rejected` needs three: the apply, the empty update, the fresh apply); a rejected update changes
  nothing and is not counted. Post-commit orphan-cleanup debt and its retry are not modelled: no invariant depends on it.- Model-only variables (the driver does not project them): see each file's header (`staged`, `inflight`, `leases`;
  identity `now`, ghosts; application `next_gid`, `route_on`, `route_gen`; receipt `outer`, `resv`, `infl`, `guard`, `ckey`, `memo`, `next_bid`, `taken`, `g`; `conf_step`).
  The single classification of every variable (projected accessor, port-return check, or model-only, with the value
  mappings) is in feature-delta "Formal protocol model", modules `gateway_application` and `selection_receipt`.
- `NoAllocationIdentity` and `AtMostOneHeld` are not modelled: both are type-level facts (the slot is keyed by node and is an `Option`, ADR-0225).

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

## Selection receipt conformance (D-PASS7-2)

The conformance owner is the real `EbpfDataplane` on native metal. `receipt_conf` therefore models only what that owner
can be driven through: the fused `update_apply` (update_begin; commit(true); publish), `update_rejected`, `open`,
`kernel_connect_det` (no `pick`; empty and singleton healthy backend sets only), `take` with r == c, `read_identity`,
`cleanup`, `restart` (the `idle` stutter is excluded from `receipt_conf`). The results of `take` and `read_identity` are checked from the port returns, not projected;
the projected state is `identity[id]`, the intent count and the receipts per cookie (`live_receipts()`, R-33). The allocator releases a memo entry at orphan GC after every commit (including an empty-set commit) and never recycles a number; `conflicting_alloc_rejected` pins that a rejected update consumes no id and that the next fresh backend gets the id the counter held (R-36, R-38). The fine-grained phases and rollback live in
`receipt_ok` (exhaustive safety, and the two evaluator-only runs `abandoned_reservation_rolled_back` and
`receipt_waits_for_publish_guard`); they need a pause/fault seam the real owner lacks and are not conformance-replayed (R-36, P-32).

## Relay (`relay_exchange.qnt`, conformance target)

`relay_exchange.qnt` is the CONFORMANCE TARGET: the pure `ExchangeMachine` over one connection (Public leg P keep-alive,
per-exchange Upstream leg U, one request per upstream connection). Phases: Idle -> Relay -> Idle (reuse) or Closed; there
is no Done phase, `conclude` (`finish()`) goes straight to Idle when `reusableOf` holds and otherwise to Closed. Atomic
body units, full-duplex exchange (the response may start before the request body ends), close-delimited responses,
the keep-alive reuse rule, close_notify and drain are modelled. Chunk framing and kTLS record framing/KeyUpdate are NOT
modelled: chunk framing is covered by property tests only (ruling 19), and the gateway never originates a KeyUpdate (ADR-0250).
Every named run is `#[quint_test]`-replayable. Bounds: 2 units, 2 exchanges per connection.

The action -> `ExchangeMachine` call table and the variable -> owner accessor table live ONLY in feature-delta
"Pinned observable projection for Quint conformance"; they are not repeated here. Model-only, not projected: `exch`,
`overread` (ghost for `NoBytePastMessageBoundary`), `ud_units`, `conf_step`. Upstream close before the head and
`force_abort` have no named run; they stay seeded-sim obligations.

The keep-alive reuse rule (a reused Idle always follows a complete, Length-delimited message) is the `conclude` guard
`reusableOf`; the named runs pin it through `expect` on `reusableOf`, so it carries no separate invariant. The decision's
hazard is `relayPastBoundary` (ADR-0239).

## Stutter and owner-autonomous steps (ruling 7)

The `idle` stutter exists only in the exhaustive mains (`identity_ok`, `app_ok`, `receipt_ok`,
`relay_ok`); each conformance main (`*_conf`) guards it with `not(REC)`, so no conformance step maps to "no call". In
`gateway_application.qnt`, `stage` and `retire` remain explicit steps; they are owner-autonomous (the owner performs
them itself and the driver yields until idle and asserts the postcondition via `application_status()`).

Assumptions A-K4, A-K5 and A-K6 and their discharging items: feature-delta "Assumptions and their discharging items".
