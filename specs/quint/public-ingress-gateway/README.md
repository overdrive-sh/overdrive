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
`cargo xtask metal run -- cargo xtask quint check --subsystem specs/quint/public-ingress-gateway [--name <check>] [--ci]`
(`--subsystem` also accepts the bare directory name `public-ingress-gateway`; a value containing `/` is taken as a path).
Evidence of the last full run: `evidence/` (written only by `--record`; `cargo xtask quint verify-evidence --subsystem
public-ingress-gateway` fails when it no longer matches the specs).

## Modules, decisions, scenario -> named run

| Module | Decision | Safety invariants (`Safety`, TLC exhaustive on `*_ok`) | Named runs (main `*_conf`) |
|---|---|---|---|
| `gateway_identity.qnt` | ADR-0225 | `HoldImpliesAudit`, `StaleEpochNeverHolds`, `AtMostOneHeld`, `DisableClearsOrTimesOut` | `issue_hold` (happy path), `reissue_near_expiry`, `disable_races_inflight_issue`, `audit_failure_no_hold`, `restart_reissue` |
| `gateway_application.qnt` (drain and supersession only) | ADR-0222 | `DrainRetainsUntilLeasesZero`, `SupersededResultNeverRestores` | `replace_valid` (supersede, drain, retire), `route_withdraw_drain`, `late_probe_after_supersession` (a superseded staged probe result is discarded) |
| `selection_receipt.qnt` | ADR-0228, ADR-0232, ADR-0233, ADR-0241, ADR-0243 (outcome table ADR-0242) | `IdentityReadableOnlyAfterCommit`, `IdentityImmutablePerBackendId`, `ReceiptMatchesIntent`, `NoResidueAfterCleanup`, `RollbackPreservesPriorGeneration`, `EmptyIsNoBackend` | `commit_then_select`, `conflicting_alloc_rejected`, `restart_destroys_receipts` (conformance-bound, `receipt_conf`); `abandoned_reservation_rolled_back`, `receipt_waits_for_publish_guard` (model-level, `receipt_fine`) |
| `relay_exchange.qnt` (conformance target) | ADR-0239 (context ADR-0245, ADR-0246) | `NoBytePastMessageBoundary`, `MidBodyCloseNeverReused` | `exchange_cl_keepalive`, `early_response_closes`, `drain_with_inflight_body`, `half_close_after_request_read`, `upstream_close_mid_body_aborts`, `close_delimited_clean_end`, `public_close_mid_request_aborts` (main `relay_conf`) |
| `relay_ktls.qnt` (model-only) | ADR-0244 | `TxRekeyExcludesWriters` | `keyupdate_waits_for_writer` (main `ktls_ok`) |

If the user adopts P-9 (honour BACKEND_MAP.healthy), add the healthy rule, its invariant and a named run then.

## Hazards (one per decision)

| Decision | Hazard (OFF switch -> main) | Invariant that fails |
|---|---|---|
| ADR-0225 | `noEpochCheck` -> `identity_bugNoEpochCheck` | `StaleEpochNeverHolds` |
| ADR-0222 | `retireWhileLeased` -> `app_bugRetireWhileLeased` | `DrainRetainsUntilLeasesZero` |
| ADR-0233 | `publishEarly` -> `receipt_bugPublishEarly` | `IdentityReadableOnlyAfterCommit` |
| ADR-0241 | `rebind` -> `receipt_bugRebind` | `IdentityImmutablePerBackendId` |
| ADR-0232 | `takeAny` -> `receipt_bugTakeAny` | `ReceiptMatchesIntent` |
| ADR-0239 | `relayPastBoundary` -> `relay_bugRelayPastBoundary` | `NoBytePastMessageBoundary` |
| ADR-0244 | `rekeyDuringWrite` -> `ktls_bugRekeyDuringWrite` (`hazard/relay_ktls_hazards.qnt`) | `TxRekeyExcludesWriters` |

## Validation budget

See the header of `checks.toml` (single home).

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

## Selection receipt conformance (D-PASS7-2)

The conformance owner is the real `EbpfDataplane` on native metal. `receipt_conf` therefore models only what that owner
can be driven through: the fused `update_apply` (update_begin; commit(true); publish), `update_rejected`, `open`,
`kernel_connect_det` (no `pick`; empty and singleton healthy backend sets only), `take` with r == c, `read_identity`,
`cleanup`, `restart`, `idle`. The results of `take` and `read_identity` are checked from the port returns, not projected;
the projected state is `identity[id]` and the intent/receipt counts. The fine-grained phases and rollback live in
`receipt_ok` (exhaustive safety) and `receipt_fine` (MAXUPD = 3, hosts the two non-replayable runs, because the rollback
run needs three accepted updates and `receipt_ok` is bounded at two); they are not conformance-replayable.

## Relay (`relay_exchange.qnt` conformance target, `relay_ktls.qnt` model-only)

The relay is split by what a conformance driver can replay.

`relay_exchange.qnt` is the CONFORMANCE TARGET: the pure `ExchangeMachine` over one connection (Public leg P keep-alive,
per-exchange Upstream leg U). Phases: Idle -> Relay -> Idle (reuse) or Closed; there is no Done phase, `conclude`
(`finish()`) goes straight to Idle when `reusableOf` holds and otherwise to Closed. Atomic body units, keep-alive
reuse rule, close_notify and drain are modelled; chunk framing is not (pure-parser property-test concern). Every named
run is `#[quint_test]`-replayable. Bounds: 2 units, 2 exchanges per connection.

The action -> `ExchangeMachine` call table and the variable -> owner accessor table live ONLY in feature-delta
"Pinned observable projection for Quint conformance"; they are not repeated here. Model-only, not projected: `exch`,
`overread` (ghost for `NoBytePastMessageBoundary`), `ud_units`, `conf_step`. Upstream close before the head and
`force_abort` have no named run; they stay seeded-sim obligations.

`MidBodyCloseNeverReused` (formerly `MidBodyCloseNeverDone`) says a reused Idle always follows a complete message
(`req_left = resp_left = 0`, `resp_state = RDone`, Length-delimited). It is derived from the `conclude` guard
(`reusableOf`), so it has no hazard; this is acceptable under one-hazard-per-decision because the decision's hazard is
`relayPastBoundary` (ADR-0239).

P-13d-dependent elements (close-delimited responses): `len_delim`, `ud_units`, MAXU as the UntilClose bound,
`resp_head` with `ld = false`, and the run `close_delimited_clean_end`. If the user declines P-13d they are removed in
one cut.

`relay_ktls.qnt` is MODEL-ONLY (no `conf_step`, no quint-connect driver) and models the PUBLIC leg only: the kTLS
transmit-lock / rekey exclusion of ADR-0244 (lock holders free/writer/rekeyer as `wr`/`rk`, reply owed `owe`; no key
epochs). The upstream leg terminates at leg-C, which never originates a KeyUpdate (main's transparent-mTLS arm installs
kTLS from extracted secrets and has no rekey path), so nothing there contends with a writer. Its run
`keyupdate_waits_for_writer` is not conformance-replayable. The user may instead rule that the lock is covered by A-K5
plus the Tier-3 check and drop this module.

Assumptions A-K4, A-K5 and A-K6 and their discharging items: feature-delta "Assumptions and their discharging items".
