# Acceptance specification — microVM-only execution after the greenfield cut

## Authority and reconciliation

This DISTILL handoff derives from GH #293 and the corrected DESIGN approved in
`design/review-design.md` (overall iteration 4). DISCUSS was explicitly skipped;
no persona, story map, or user journey is invented.

Reconciliation passed with zero contradictions. Missing feature DISCUSS,
SPIKE, and DEVOPS artifacts are warnings, not blockers. The project is a Rust
application and inherits `docs/architecture/atdd-infrastructure-policy.md`:
direct Rust tests only, no `.feature` files or Python state-delta port.

The accepted boundary is a pure deletion. Executable evidence describes only
surviving VM, generic-invalid-input, current-V1, HTTP/TCP, lifecycle, and
P-105 behavior. Deleted names, syntax, helpers, variants, parameters, adapters,
accessors, messages, and historical payloads are implementation/reviewer diff
audit—not permanent test vocabulary.

## Test delta authored by DISTILL

- New feature-specific test files: **0**.
- New test functions: **1**, the generic property
  `api_type_shapes::submit_request_rejects_unknown_driver_keys`.
- Modified existing test functions: **4**:
  `s_01_07_missing_supported_driver_rejected`,
  `submit_job_request_round_trips_through_serde_json`,
  `job_description_round_trips_with_typed_spec`, and
  `post_v1_jobs_with_invalid_spec_returns_400_with_error_body_naming_field`.
- Deleted rejected new test files from the superseded DISTILL pass: **8**
  (`remove_*.rs`, including all compile-pass fixtures). Their module
  registrations were also removed before this remediation.
- Pending/ignored feature scaffolds: **0**.

The new property samples the bounded generated representative domain of
lowercase `driver_<suffix>` keys with suffix length 1..12. Every case first
deserializes the unchanged valid VM request as a positive control, then inserts
exactly one unsupported sibling key and asserts ordinary generic serde
rejection. No exact message or removed spelling is pinned.

## Concise single-action specifications

### A1 — VM TOML retains workload kind and payload

@contract-shape:pure-function

Given a valid VM Job, Service, or Schedule declaration
When `WorkloadSpecInput::from_toml_str` parses it
Then its workload kind and VM payload are retained.

Rust evidence: the positive VM cases in
`overdrive-core/tests/acceptance/service_kind_vm_workloads.rs`,
`workload_spec_parser.rs`, and the VM-migrated `coinflip_migration.rs`.

### A2 — VM HTTP request round-trips

@contract-shape:pure-function

Given a valid VM Job request
When the shared HTTP request type serializes and deserializes it
Then the complete typed specification is equal to the original.

Rust evidence:
`api_type_shapes.rs::submit_job_request_round_trips_through_serde_json`.

### A3 — VM workload description round-trips

@contract-shape:pure-function

Given a VM Job description
When the shared describe type serializes and deserializes it
Then the complete typed specification and digest are retained.

Rust evidence: `api_type_shapes.rs::job_description_round_trips_with_typed_spec`.

### B1 — missing supported driver uses ordinary parser failure

@contract-shape:pure-function

Given a Job declaration with no supported driver table
When the existing TOML parser reads it
Then it returns the existing `MissingDriverSection` result whose ordinary
guidance names the supported VM table.

Rust evidence:
`workload_spec_parser.rs::s_01_07_missing_supported_driver_rejected`.

### B2 — generated unsupported driver key uses ordinary serde failure

@contract-shape:pure-function

Given a valid VM Job request that deserializes successfully
When one bounded generated unsupported sibling driver key is inserted
Then the existing serde decoder rejects the changed request.

Rust evidence: the sole new property
`api_type_shapes.rs::submit_request_rejects_unknown_driver_keys`.

### B3 — ordinary validation failure writes no intent

@contract-shape:bounded-change

Given a valid VM-shaped request with an unrelated invalid replica count
When it is submitted through the production HTTP server
Then the existing validation response is returned
And the canonical workload intent key remains absent.

Rust evidence:
`submit_round_trip.rs::post_v1_jobs_with_invalid_spec_returns_400_with_error_body_naming_field`.

### C1 — current Service specification establishes its V1 baseline

@contract-shape:pure-function

Given a current VM-only Service specification
When its owning envelope archives and restores it
Then the current V1 payload equals the original.

Rust evidence transition: rewrite existing
`schema_evolution/service_spec.rs` to one current V1 fixture.

### C2 — current workload intent establishes its V1 baseline

@contract-shape:pure-function

Given each current VM-only workload-intent kind
When its owning envelope archives and restores it
Then the current V1 payload equals the original.

Rust evidence transition: rewrite existing
`schema_evolution/workload_intent.rs` to the current V1 family.

### C3 — current allocation status establishes its V1 baseline

@contract-shape:pure-function

Given a current allocation-status row using surviving lifecycle vocabulary
When its owning envelope archives and restores it
Then the current V1 payload equals the original.

Rust evidence transition: rewrite existing
`schema_evolution/alloc_status_row.rs` to one current V1 fixture.

### C4 — current lifecycle occurrence establishes its V1 baseline

@contract-shape:pure-function

Given a current lifecycle-occurrence row using surviving source and reason
vocabulary
When its owning envelope archives and restores it
Then the current V1 payload equals the original.

Rust evidence transition: rewrite existing
`schema_evolution/alloc_lifecycle_occurrence_row.rs` to one current V1 fixture.

Historical fixture bytes and readers delete; they are not negative cases.
Unrelated envelope fixtures remain byte-identical by reviewer diff audit.

### D1 — one server boot reports its VM capability outcome

@contract-shape:bounded-change

Given an ordinary absent, available, or present-but-failing VMM capability
When the production server performs its one boot-time capability probe
Then the driver registry outcome is respectively empty, `{Vm}`, or startup
refusal.

Rust evidence:
`vm_walking_skeleton.rs::{vm_registry_reports_vm_supported_when_cloud_hypervisor_present_and_healthy,vm_absent_boots_node_with_no_vm_entry_and_classifies_deploy_naming_capability,vm_capability_flag_probe_failure_injected_via_vmm_override_refuses_boot}`.

### D2 — the trusted probe runner reaches the VM driver

@contract-shape:bounded-change

Given one successful probe-runner boot gate and one composed VM driver
When the VM lifecycle registers and terminates an allocation
Then that same runner observes the allocation lifecycle without repeating the
boot probe.

Rust evidence transition: narrow
`service_kind_vm_workloads.rs::one_server_boot_shares_exactly_one_trusted_probe_runner_with_both_drivers`
to its surviving single-VM assertions.

### D3 — VM HTTP/TCP probe result is published

@contract-shape:bounded-change

Given a VM Service with one declared HTTP or TCP probe
When the existing supervisor executes that probe
Then its current target, role/index, threshold, and result row are retained.

Rust evidence: existing `probe_runner_{http,tcp}_outcome.rs`,
`probe_runner_supervised_tick.rs`, and the four VM-only probe tests in
`overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs`.

### D4 — successful interception precedes guest-command release

@contract-shape:bounded-change

Given a VM allocation whose guest-ready start produced an accepted Running row
When transparent-mTLS installation succeeds
Then guest-command release happens afterward.

Rust evidence transition: VM-migrate the existing success-order cases in
`overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs`.

### D5 — failed interception dominates Running and withholds release

@contract-shape:bounded-change

Given a VM allocation with an accepted Running row
When transparent-mTLS installation fails
Then the existing dominating Failed result is written
And guest-command release remains closed
And existing cleanup completes.

Rust evidence transition: VM-migrate the existing failure-order cases in
`mtls_install_fail_closed.rs` without weakening their assertions.

### E1 — WorkloadLifecycle reserves a fresh VM successor

@contract-shape:pure-function

Given an accepted numeric-current Failed or Terminated VM predecessor
When `WorkloadLifecycle::reconcile` evaluates replacement
Then `RestartAllocation` retains the predecessor ID and a distinct durably
reserved fresh successor ID.

Rust evidence transition:
`overdrive-reconcilers/tests/acceptance/vm_recreation_allocation_identity.rs`,
narrowed to VM while retaining every property and boundary assertion.

### E2 — composed replacement retains the P-105 precedence table

@contract-shape:bounded-change

Given an eligible VM predecessor and the existing successor effects
When the production action shim dispatches `RestartAllocation`
Then successor outcome and one exact-old cleanup attempt retain the accepted
P-105 result-precedence contract.

Rust evidence transition:
`overdrive-sim/tests/driver_neutral_allocation_replacement.rs`, narrowed to VM
without weakening its observation, mTLS, network, reopen, or precedence
assertions. Its schedule-racy test is excluded as described below.

### E3 — VM exit remains observable and releases supervision

@contract-shape:bounded-change

Given a composed VM allocation
When its guest-authoritative exit is observed
Then lifecycle evidence retains VM provenance
And the allocation's supervision claim is released.

Rust evidence:
`vm_walking_skeleton.rs::{vm_workload_runs_to_completion_and_exit_code_reaches_operator,vm_non_zero_guest_exit_code_is_reported_not_the_hypervisors}`
and `vm_reclamation_claim_lifecycle.rs::release_supervision_*`.

## Existing-test deletion and migration rule

Dedicated parser/driver/probe/codec/adapter tests whose sole subject disappears
delete with their production symbol. Generic tests using the old driver as a
cheap fixture migrate to VM. No dedicated acceptance, trybuild, compile-pass,
source-token, source-shape, exact-message, or removed-spelling test survives.

GH #295 has no executable scenario in #293. The reviewer verifies that current
VM-used netns/veth/TAP/`host_veth` remains and no shared-switch, per-tap,
shared-DNS, or replacement-mTLS mechanism entered the diff.

## Production-boundary evidence and explicit limitation

Existing CLI integration tests drive `run_server` plus the real
deploy/describe handlers, establishing the production VM composition boundary.
Existing VM completion tests prove a composed VM exit observer consumes an
exit and publishes operator-visible lifecycle evidence.

`ServerHandle.exit_observer_tasks` and the shared shutdown token are private.
No current port exposes exact internal task-vector cardinality or token
identity. DISTILL adds no test-only accessor or large composition suite. Exact
one-task-per-registry-entry, clone-one-token, and join-all ownership remain
implementation-review checks; existing tests retain VM exit publication,
supervision release, and clean shutdown as the behavioral evidence.

## Pre-existing flaky fixture disposition

The existing test
`successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver` is a
schedule-racy oracle, not reproducible seeded production behavior. Mandatory
review ran it 50 times: **39 passed / 11 failed**. Its printed seed does not
control the unbiased Tokio `select!` deciding which already-ready notification
wins. It is out of scope for #293 and is neither changed nor counted as a
blocking preservation signal. The deterministic P-105 properties and remaining
composed precedence cases stay in the evidence map.

## Post-cut black-box verification event

E06 and E08 remain historical SHA-pinned receipts only; neither proves the
post-cut implementation.

The minimal post-cut plan is named `E14-vm-service-post-greenfield-cut`.
DELIVER/DEVOPS captures it against the post-cut SHA by driving the existing
checked-in `examples/service-kind-vm-workloads/run-example.sh run healthy`
through the built default-feature binary on native metal. It records only the
example's stakeholder-visible success and owned cleanup outcomes. It does not
overwrite E08, recreate a spec, invoke tests, import a Rust crate, or duplicate
integration assertions. Generic invalid-input behavior remains in Rust tests.

## Completeness audit

No domain extension applies.

| Check | Result | Evidence or rationale |
|---|---|---|
| C1a | PASS | Existing missing-supported-driver parser test. |
| C1b | PASS | VM control plus one generated unsupported sibling key per property case. |
| C2a | PASS | DESIGN documents unchanged admission, capability, Running/intercept, and P-105 states. |
| C2b | PASS | Generic invalid ingress and existing VMM/intercept failure paths. |
| C3 | PASS (N/A) | No new collection contract; existing probe cardinality suites remain. |
| C4a | PASS | Existing probe re-registration and deterministic P-105 re-drive/idempotency evidence. |
| C4b | PASS (N/A) | The greenfield cut exposes no inverse operation. |
| C5a | PASS (N/A) | No feature or compatibility flag exists. |
| C5b | PASS (N/A) | No flag-orthogonality claim exists. |
| C6a | PASS | Bounded generated unsupported keys plus unrelated VM validation failure. |
| C6b | PASS | Only existing generic errors remain; no new error contract exists. |
| C6c | PASS | No dedicated compatibility error set is admitted. |
| C7a | PASS | Existing ordinary VMM-absence and present-but-failing-probe tests. |
| C7b | PASS (N/A) | Pure deletion adds no interruption behavior; the pre-existing schedule-racy oracle is out of scope. |
| C7c | PASS | Existing observer/re-registration/supervision behavior; exact private ownership is review-audited. |

Mechanical verdict: **15/15 — COMPLETE**. Audit tuple (documentation only; no
telemetry event): `(remove-legacy-exec-workload-driver, C1..C7, 0, none)`.

Mutation testing is not run in DISTILL.
