# DISTILL typed-contract remediation audit

Status: **accepted by the independent full DISTILL review at iteration 4** (`review.md`). The full D1–D17 DESIGN review was approved at iteration 4 in `../design/review.md`; the roadmap review remains pending. The 2026-11-09 user approvals pin D11-E1, D14-E2 and D14-A1. This audit covers the new or amended acceptance evidence for D11-E1, D14-E2 and the per-test Contract Shape remediation. No production behaviour was implemented.

## Exact typed failures and scenario trace

| Designed failure | Approved cause and ingress mapping | Acceptance body | DELIVER step |
|---|---|---|---|
| Open or `Lagged` relist read returns `Unreachable { peer }`, `Io(error)` or `Envelope { source }` | Exactly `ServiceStreamError::ObservationRead { source: original ObservationStoreError }`; one error, then end | `service_stream_recorded_verdict.rs::{a_failed_read_at_open_ends_the_stream_with_an_error_and_never_waits, a_failed_relist_after_lagged_ends_the_stream_with_an_error}` | 04-02 |
| Existing `emit_service_line` fails | `ServiceStreamError::LineEmission { source: original io::Error }` | The approved separate variant is compile-pinned. Ordinary in-memory emission has no reachable injected failure; no emitter seam was added. | 04-02 |
| Raw Service `GuestListeners` has a wrong containing vector or submitted descriptor role | `AggregateError::GuestListenersInvalidRole { vector_role, descriptor_role, index }`; `ControlPlaneError::Aggregate`; HTTP 400 `validation` and exact vector/index field | `guest_listeners_input_rejection.rs::raw_service_guest_listeners_wrong_role_is_typed`; `guest_listeners_http_rejection.rs::invalid_guest_listeners_raw_service_is_rejected_at_http_ingress` | 05-04 |
| Raw Service correctly tagged `GuestListeners` shares startup with another descriptor | `AggregateError::GuestListenersNotSoleStartup { count }` before `ProbeCountExceeded`; same HTTP carrier, field `startup_probes` | `guest_listeners_input_rejection.rs::raw_service_guest_listeners_not_sole_startup_precedes_count_check`; same HTTP body | 05-04 |
| Service operator TOML contains canonical `guest_listeners`, case-insensitive, in any role | `ParseError::GuestListenersNotOperatorConfigurable { role, probe_idx }`; both deploy modes retain `CliError::ParseError`, exit 1 before HTTP | `guest_listeners_input_rejection.rs::operator_toml_guest_listeners_is_typed_for_every_role_and_case`; `probes_kind_rejection_cli.rs::canonical_guest_listeners_token_is_typed_in_each_role_on_both_cli_modes` | 05-04 |

The valid raw Service sole-startup control passes. Existing Job and Schedule TOML kind-gate tests remain the relevant non-Service evidence; raw Job and Schedule JSON payloads structurally reject unknown probe fields before aggregate admission. The tests use exact variant and payload matches. They never infer semantic classification from `Display` strings.

## Test shape and coverage self-audit

| Category | Evidence or applicability |
|---|---|
| C1 equivalence and boundary | S-42 runs open and relist reads with each of three causes; S-57 covers valid sole startup, invalid roles, and non-sole startup counts 2, 7, 8 and 9 around the general probe cap. |
| C2 state and transition | S-42 tests the stream's open/read and lagged/relist paths and verifies that an error closes it permanently. D14-E2 pure input rejection has no state transition. |
| C3 count cardinality | Zero GuestListeners retains existing Service input behaviour; one correctly tagged raw descriptor is accepted; two, seven, eight and nine startup descriptors are rejected with the exact count. |
| C4 idempotency and inverse | These are read and validation paths with no mutating operation to repeat or invert. S-42 compares store rows before/after; the HTTP test compares intent-store scans before/after rejection. |
| C5 mode decision table | Raw vector role × descriptor role × index is a proptest; TOML role × canonical token case × CLI deploy mode is fully enumerated; the noncanonical `GuestListeners` spelling remains unknown. Both modes preserve one typed cause and exit code. |
| C6 negative and closed error set | Each approved read cause and each invalid GuestListeners semantic cause has a named test; matches reject all other variants, including `LineEmission` for read faults, generic validation/string errors for raw input, and `UnknownProbeType` for the canonical token. |
| C7 degraded resource and interruption | `Unreachable` and `Io` simulate read faults; `Lagged` induces relist; no concurrent-safety claim is introduced by these typed-error amendments. Existing feature conformance drivers cover the broader ordering contracts. |

Canonical 15-item check on the bounded amendment: **15/15 pass or inapplicable**.

| Item | Disposition |
|---|---|
| C1a empty/zero | Pass: zero recorded rows and zero GuestListeners descriptors. |
| C1b partition boundaries | Pass: 7, 8 and 9 descriptors around `GUEST_PROBE_SLOTS` 8, plus the sole-startup boundary 1/2. |
| C2a state model | Pass: S-42's module documents Accepted, subscription/read, Lagged/relist and closed outcomes. |
| C2b illegal event from each state | Pass: an error at either read site ends the stream; every later poll remains ended. Invalid GuestListeners input is refused at admission. |
| C3 zero/one/many | Pass: zero and one accepted controls; 2/7/8/9 rejected counts. |
| C4a repeat mutating operation | Inapplicable: these reads and validations do not mutate. Store snapshots are unchanged. |
| C4b inverse without prerequisite | Inapplicable: there is no inverse operation in D11-E1 or D14-E2. |
| C5a mode combinations | Pass: role × case × CLI mode; vector × descriptor role × index. |
| C5b mode orthogonality | Pass: both CLI modes return the same typed cause and exit code. |
| C6a malformed input | Pass: invalid placement/cardinality and forbidden canonical TOML; existing parser tests separately cover unknown types. |
| C6b every declared error | Pass: `ObservationRead`'s three nested causes and each D14-E2 variant. `LineEmission` has no reachable fault injection in ordinary in-memory emission. |
| C6c closed error set | Pass: exact variant matches reject all alternatives. |
| C7a degraded resource | Pass: `Unreachable` and `Io` read failures. |
| C7b interruption | Pass: `Lagged` relist read failure. |
| C7c concurrent actor | Inapplicable: no new concurrent-safety claim; the unchanged D11 ordering suite covers bus races. |

All 15 canonical checklist items are satisfied for this bounded amendment or inapplicable to its non-mutating validation boundary. No `SPECIFICATION_AMBIGUITY` remains. The D11 line-emission limit and structural Job/Schedule exclusion are approved design boundaries, not gaps. Six new test identities were added to the existing Metal `host-pure` placement inventory; no new lane was created. S-55, S-34 and S-46 now have their required per-test `/// CONTRACT_SHAPE: pure-function.` line. Every new test has a per-test declaration; the source-local pure property uses the exact same line. No `.feature` files or production binary-spawning Rust tests were added.

The existing roadmap activates S-46 in **02-04**, S-34 in **04-01**, S-42 in **04-02**, S-55 in **05-01**, and the new S-57 bodies in **05-04**. All pending markers remain in place for DELIVER. The roadmap validation status remains `pending` for independent re-review.

## Iteration-3 review F-01: per-test Contract Shape handoff

The independent DISTILL reviewer found 122 of 156 pre-authored `#[ignore = "pending DELIVER: ..."]` identities without a declaration in the test's own doc/attribute block. All 122 now have a per-test `/// CONTRACT_SHAPE:` line immediately before their pending marker; the reviewer's published mechanical scan reports **0/156 missing**. The 46 pending Quint identities use `bounded-change` for their production-owner state transitions. The remaining 76 use their actual universe: pure function derivation, projection, wire parse and render tests use `pure-function`; reconciler state-delta, composed, ingress, adapter and Tier-3 tests use `bounded-change`. Existing exact declarations before multiline Quint attributes were moved to the final attribute boundary so the scan can see them; inline explanatory comments were retained. No test body, assertion, pending marker or scenario activation changed. Changed-file `rustfmt --check` and `git diff --check` passed; Metal Clippy passed for `overdrive-core`, `overdrive-control-plane`, `overdrive-cli`, `overdrive-worker` and `overdrive-reconcilers` with all targets, integration tests and warnings denied. The reviewer withdrew F-02 and F-03, so no mapped-nonloopback case, S-53 schedule or test seam was added.

The independent reviewer rechecked F-01 after this metadata change and recorded **APPROVED** for the complete DISTILL suite at iteration 4 in `review.md`. F-02 and F-03 remain withdrawn; the approval does not change the pending roadmap validation status.
