# DISTILL test scenarios — Service-kind VM workloads

**Feature:** `service-kind-vm-workloads` (GH #257)
**Design base:** `8c89c71eee2`
**Scope:** HTTP/TCP VM Service health only; optional VM Exec probes are GH #280.

This file is non-executable stakeholder prose under the repository's Rust
acceptance convention. Its 32 scenario IDs map to 24 Rust tests and eight
built-product expectation claims. E11 owns three independently identified
before/unavailable/after claims (S-SVM-27A/B/C); every other expectation owns
one. E08/S-SVM-01 is the sole walking skeleton.

## Reconciliation and environment

Reconciliation passed with zero contradictions. `Some(workload_addr)` remains
a production-only VM invariant and no scenario manufactures its absence.

The generic missing-DEVOPS fallback variants `clean | with-pre-commit |
with-stale-config` do not apply to this runtime acceptance boundary:

- a pre-commit hook changes developer commit workflow, not the behavior of the
  built default-feature binary;
- stale local config is excluded because every expectation creates an isolated
  config/data directory and invokes only the newly built binary; and
- `clean` is subsumed by the fail-closed native-metal preparation and cleanup
  contract.

The executable environment is therefore the source-grounded qualified matrix:
native non-virtualized x86_64 Linux, accessible KVM, canonical kernel/rootfs,
the repository's leased `cargo xtask metal run --` boundary, isolated config,
fresh data directory, and zero owned-resource cleanup delta. These are
preconditions beneath the single walking skeleton, not additional skeletons.

## Built-product journeys

### S-SVM-01 — A healthy VM Service becomes Stable and serves a peer VM client Job

```gherkin
@walking_skeleton @driving_port @real-io @native-metal @US-SVM-1 @US-SVM-2
@kpi:K1 @kpi:K2 @contract-shape:bounded-change
Scenario: A healthy VM Service becomes Stable and serves a peer VM client Job
  Given Ana has started Overdrive on qualified metal
    And the checked-in VM Service has reported Accepted followed by Stable
    And workload describe shows passing guest health
    And the checked-in plaintext VM client Job is deployed
  When the client calls the Service name
  Then the VM client Job succeeds only after receiving "SVM-E08-GUEST-OK" from the VM guest
    And all example-owned resources are removed
```

### S-SVM-25 — Bound and unbound guest ports are truthful in 100 paired trials

```gherkin
@driving_port @real-io @native-metal @US-SVM-1 @kpi:K1
@contract-shape:bounded-change
Scenario: Bound and unbound guest ports are truthful in 100 paired trials
  Given Ana has one VM Service whose guest binds the declared port and one whose guest does not
  When each pair is deployed through a fresh built-product instance 100 times
  Then all 100 healthy deployments become Stable and serve the exact guest reply to a peer VM Job
    And all 100 unbound-port deployments fail with StartupProbeFailed
    And no failed deployment serves its peer VM Job
    And no trial is retried or discarded
```

### S-SVM-26 — Application-health outcomes agree across supported workload forms

```gherkin
@driving_port @real-io @native-metal @US-SVM-2 @kpi:K2
@contract-shape:bounded-change
Scenario: Application-health outcomes agree across supported workload forms
  Given the same checked-in application is deployed in each supported workload form
  When Ana runs the canonical application-health examples
  Then a healthy response makes each Service Stable
    And a redirect is rejected rather than followed
    And a missing application path is rejected
    And an unavailable Service response is rejected
    And both workload forms report the same outcome for every example
```

### S-SVM-27A — A ready VM Service serves its peer VM client Job

```gherkin
@driving_port @real-io @native-metal @US-SVM-3 @kpi:K3
@contract-shape:bounded-change
Scenario: A ready VM Service serves its peer VM client Job
  Given a Stable VM Service is ready to receive traffic
    And the checked-in plaintext VM client Job is deployed
  When the client calls the Service name
  Then it receives the exact VM guest reply
```

### S-SVM-27B — An unready VM Service is withdrawn from peer traffic

```gherkin
@driving_port @real-io @native-metal @US-SVM-3 @kpi:K3
@contract-shape:bounded-change
Scenario: An unready VM Service is withdrawn from peer traffic
  Given a Stable VM Service reports that it is unavailable
    And the checked-in negative-control VM client Job is deployed
  When the client calls the Service name
  Then it cannot receive the VM guest reply within the promised health-check bound
    And workload describe keeps the allocation Running and Stable without a restart
```

### S-SVM-27C — A recovered VM Service serves peer traffic again

```gherkin
@driving_port @real-io @native-metal @US-SVM-3 @kpi:K3
@contract-shape:bounded-change
Scenario: A recovered VM Service serves peer traffic again
  Given the unavailable VM Service reports healthy again
    And the checked-in recovery VM client Job is deployed
  When the client calls the Service name
  Then it receives the exact VM guest reply within the promised health-check bound
```

### S-SVM-28 — Liveness failure invokes the existing restart policy visibly

```gherkin
@driving_port @real-io @native-metal @US-SVM-3 @error
@contract-shape:bounded-change
Scenario: Liveness failure invokes the existing restart policy visibly
  Given a Stable VM Service later reports an unavailable liveness result
  When its declared failure threshold is reached
  Then workload describe shows a liveness-caused terminal allocation
    And shows the existing restart policy creating the replacement allocation
    And the dead allocation never becomes eligible again
    And no VM-specific lifecycle state appears
```

### S-SVM-29 — Zero declared health checks retain inferred startup behavior

```gherkin
@driving_port @real-io @native-metal @US-SVM-1 @compatibility
@contract-shape:bounded-change
Scenario: Zero declared health checks retain inferred startup behavior
  Given two VM Services declare a listener and no health checks
    And one guest binds its listener while the other does not
    And Ana has deployed both Services and their matching VM client Jobs through the built product
  When Ana describes both Services
  Then the bound listener produces an inferred passing startup check and Stable
    And its VM client Job succeeds only after receiving the exact guest reply
    And the unbound listener produces an inferred failure and StartupProbeFailed
    And its VM client Job never receives the guest reply
    And describe identifies each Service's guest as the destination for its inferred check
```

## Admission and compatibility scenarios

### S-SVM-02 — A VM Service with network health is accepted unchanged

```gherkin
@in-memory @property @US-SVM-1 @contract-shape:pure-function
Scenario: A VM Service with network health is accepted unchanged
  Given Ana writes a VM Service specification with supported network health checks
  When she submits the specification for validation
  Then it is accepted as a VM Service
    And its declared health intent and limits are preserved
```

### S-SVM-03 — The earliest unsupported in-guest health command is rejected

```gherkin
@in-memory @property @US-SVM-1 @error @contract-shape:pure-function
Scenario: The earliest unsupported in-guest health command is rejected
  Given a VM Service contains unsupported in-guest health commands in several roles and positions
  When the specification is validated
  Then the first Startup offender is selected before Readiness and Liveness
    And the lowest declared position in that role is selected
```

### S-SVM-04 — Rejection identifies the exact offending declaration

```gherkin
@in-memory @property @US-SVM-1 @error @contract-shape:pure-function
Scenario: Rejection identifies the exact offending declaration
  Given a VM Service contains an unsupported in-guest health command
  When Ana submits the deployment specification
  Then the error names its health-check section and entry number
    And advises the supported network-health choices and points to GH #280
    And no Service is created
```

### S-SVM-05 — Direct submission cannot bypass unsupported-health rejection

```gherkin
@in-memory @property @US-SVM-1 @error @contract-shape:pure-function
Scenario: Direct submission cannot bypass unsupported-health rejection
  Given a client has submitted the equivalent VM Service directly
  When the deployment is evaluated
  Then the same earliest unsupported probe is rejected before any Service is created
    And the error names its role and position
```

### S-SVM-06 — Every submission path gives the same rejection guidance

```gherkin
@in-memory @property @US-SVM-1 @error @contract-shape:pure-function
Scenario: Every submission path gives the same rejection guidance
  Given the same unsupported in-guest health command was rejected through each submission path
  When Ana compares the guidance
  Then both rejections name the supported network-health choices
    And both point to GH #280 for optional in-guest health commands
```

### S-SVM-07 — Existing saved Service specifications remain readable

```gherkin
@in-memory @property @US-SVM-1 @compatibility @contract-shape:pure-function
Scenario: Existing saved Service specifications remain readable
  Given Services saved in either earlier supported format
  When the current version reads them
  Then both retain their original workload form and health behavior
    And a newly saved VM Service does not change either earlier record
```

### S-SVM-08 — Describe preserves either supported workload form

```gherkin
@in-memory @property @US-SVM-1 @compatibility @contract-shape:pure-function
Scenario: Describe preserves either supported workload form
  Given Ana has deployed one process Service and one VM Service
  When she describes each Service
  Then each still identifies its originally selected workload form
    And the VM's boot settings remain unchanged
```

### S-SVM-09 — Process Services retain in-process health commands

```gherkin
@in-memory @property @US-SVM-1 @compatibility @contract-shape:pure-function
Scenario: Process Services retain in-process health commands
  Given a process Service declares an in-process health command
  When it is validated after VM Service support is enabled
  Then it remains accepted unchanged
```

## Health target and mechanic scenarios

### S-SVM-10 — VM default health destinations reach the provisioned guest

```gherkin
@in-memory @property @US-SVM-1 @contract-shape:bounded-change
Scenario: VM default health destinations reach the provisioned guest
  Given a VM Service does not name a specific network-health destination
  When its allocation begins health supervision
  Then every default check reaches that allocation's provisioned guest address
    And describe still shows the operator's original declaration
```

### S-SVM-11 — Explicit health destinations are never rewritten

```gherkin
@in-memory @property @US-SVM-1 @compatibility @contract-shape:unbounded-preservation
Scenario: Explicit health destinations are never rewritten
  Given a process Service or VM Service declares a specific network-health destination
  When its health check runs
  Then the connection uses exactly the declared destination
```

### S-SVM-12 — Process Service default health still reaches its own process

```gherkin
@in-memory @property @US-SVM-1 @compatibility @contract-shape:unbounded-preservation
Scenario: Process Service default health still reaches its own process
  Given a process Service does not name a specific network-health destination
  When its health check runs after VM support is enabled
  Then the health check still reaches that Service's own process
```

### S-SVM-13 — Guest listener refusal is health failure, not loss of Running

```gherkin
@in-memory @US-SVM-1 @error @contract-shape:bounded-change
Scenario: Guest listener refusal is health failure, not loss of Running
  Given a VM has started but its declared guest listener is closed
  When startup checks exhaust their existing budget
  Then describe keeps the allocation's truthful Running history
    And reports StartupProbeFailed with the connection reason
```

### S-SVM-14 — VM application health uses the existing bounded response policy

```gherkin
@in-memory @property @US-SVM-2 @contract-shape:bounded-change
Scenario: VM application health uses the existing bounded response policy
  Given a VM guest answers its declared health path
  When the canonical application-health responses are evaluated
  Then the healthy response passes
    And redirect and unavailable responses fail with bounded diagnostics
    And no unbounded response content appears in the result
```

## Lifecycle scenarios

### S-SVM-15 — VM Services participate in the existing health lifecycle

```gherkin
@in-memory @US-SVM-3 @contract-shape:bounded-change
Scenario: VM Services participate in the existing health lifecycle
  Given the server can supervise health for a process Service
  When Ana deploys an equivalent VM Service
  Then startup, readiness, liveness, and terminal observations follow the same public lifecycle
    And public lifecycle behavior remains unchanged
```

### S-SVM-16 — Restart supervision remains singular

```gherkin
@in-memory @US-SVM-3 @contract-shape:bounded-change
Scenario: Restart supervision remains singular
  Given a supervised VM Service allocation is restarted
  When health supervision begins again for the replacement
  Then describe shows one current result stream for that allocation
    And each declared check runs once per interval rather than being duplicated
```

### S-SVM-17 — Terminal state wins over concurrent health work

```gherkin
@in-memory @seed:25717 @US-SVM-3 @concurrency
@contract-shape:unbounded-preservation
Scenario: Terminal state wins over concurrent health work
  Given a seeded schedule in which a VM Service terminates while health work is in flight
  When the system converges
  Then the allocation remains terminal
    And its dead backend never returns to eligibility
```

### S-SVM-18 — Startup failure does not rewrite Running

```gherkin
@in-memory @US-SVM-3 @error @contract-shape:unbounded-preservation
Scenario: Startup failure does not rewrite Running
  Given a VM has started and its startup port remains closed
  When startup checks reach their deadline
  Then the allocation retains its Running history
    And the Service fails specifically with StartupProbeFailed
```

### S-SVM-19 — Startup success changes only Stable

```gherkin
@in-memory @US-SVM-3 @contract-shape:bounded-change
Scenario: Startup success changes only Stable
  Given a Running VM Service receives its required startup success
  When Service health is reconciled
  Then describe gains the existing Stable witness and timing
    And neither backend eligibility nor restart policy changes from that event alone
```

### S-SVM-20 — Readiness changes only backend eligibility

```gherkin
@in-memory @US-SVM-3 @kpi:K3 @contract-shape:bounded-change
Scenario: Readiness changes only backend eligibility
  Given a Running Stable VM Service has reported a healthy, unavailable, then healthy readiness sequence
  When Service health is reconciled
  Then backend eligibility changes from healthy to unhealthy and back to healthy
    And Running and Stable remain unchanged
    And readiness does not request a restart
```

### S-SVM-21A — Liveness threshold requests the existing liveness stop

```gherkin
@in-memory @US-SVM-3 @error @contract-shape:bounded-change
Scenario: Liveness threshold requests the existing liveness stop
  Given below-threshold failures have not stopped the VM Service
    And a successful check has reset its failure streak
    And a new failure streak is one check below its threshold
  When the next failed check reaches the threshold
  Then it produces only the existing liveness stop
```

### S-SVM-21B — Workload policy alone decides what follows liveness stop

```gherkin
@in-memory @US-SVM-3 @error @contract-shape:bounded-change
Scenario: Workload policy alone decides what follows liveness stop
  Given a VM allocation has been stopped for liveness failure
  When the ordinary workload policy observes that terminal result
  Then it alone chooses restart or final failure under the existing budget
```

### S-SVM-22 — One server boot supports health for both workload forms

```gherkin
@in-memory @driving_port @contract-shape:bounded-change
Scenario: One server boot supports health for both workload forms
  Given the production server passes its startup trust checks
  When Ana deploys one process Service and one VM Service
  Then both receive health results through the same server lifetime
    And existing VM capability refusal remains actionable
```

### S-SVM-23 — Detached deployment preserves the selected VM Service

```gherkin
@in-memory @driving_port @contract-shape:bounded-change
Scenario: Detached deployment preserves the selected VM Service
  Given Ana selects detached deployment for a VM Service
  When she submits it
  Then the deployment is accepted exactly once as a VM Service
    And the existing acknowledgement behavior is unchanged
```

### S-SVM-24 — Streaming deployment preserves the selected VM Service

```gherkin
@in-memory @driving_port @contract-shape:bounded-change
Scenario: Streaming deployment preserves the selected VM Service
  Given Ana selects streaming deployment for the same VM Service
  When she submits it
  Then the deployment is accepted exactly once as that VM Service
    And Accepted, Stable, failure rendering, and exit behavior remain unchanged
```

## Technical contract mapping

Technical names belong here and in Rust documentation, not in stakeholder
Given/When/Then prose.

| Scenario | Executable artifact | Technical boundary | Contract Shape |
|---|---|---|---|
| S-SVM-01 | E08 | built binary + VM client Job + Service frontend | bounded-change |
| S-SVM-02 | core `service_vm_http_tcp_spec_parses_to_vm_driver_without_rewriting_probe_intent` | ServiceSpecV3/DriverInput | pure-function |
| S-SVM-03 | core `parser_rejects_first_vm_exec_probe_in_role_then_position_order` | parser validation order | pure-function |
| S-SVM-04 | core `parser_vm_exec_rejection_is_role_and_entry_localized_before_aggregate_creation` | ParseError localization | pure-function |
| S-SVM-05 | core `authoritative_admission_rejects_first_vm_exec_probe_before_intent_exists` | ServiceV2::from_submit | pure-function |
| S-SVM-06 | core `both_vm_exec_rejection_layers_share_the_exact_gh_280_diagnostic` | diagnostic parity | pure-function |
| S-SVM-07 | core `service_spec_v1_v2_compatibility_and_v3_golden_bytes_are_preserved` | ServiceSpecEnvelope V1/V2/V3 | pure-function |
| S-SVM-08 | core `service_driver_roundtrip_preserves_both_existing_union_arms` | intent/allocation/describe unions | pure-function |
| S-SVM-09 | core `exec_service_exec_probe_compatibility_is_unchanged` | cross-field validation | pure-function |
| S-SVM-10 | worker `vm_default_and_wildcard_network_probe_targets_resolve_to_workload_addr_once` | ProbeRunner + AllocationSpec | bounded-change |
| S-SVM-11 | worker `explicit_network_probe_hosts_are_preserved_for_both_drivers` | target projection | unbounded-preservation |
| S-SVM-12 | worker `exec_default_network_probe_targets_remain_loopback` | Exec target compatibility | unbounded-preservation |
| S-SVM-13 | worker `vm_tcp_probe_records_guest_connect_outcome_without_owning_running` | TCP adapter + observation | bounded-change |
| S-SVM-14 | worker `vm_http_probe_preserves_status_policy_and_bounded_body_handling` | HTTP 204/302/503 adapter policy + bounded body observation | bounded-change |
| S-SVM-15 | worker `vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner` | VmDriver constructor/hooks | bounded-change |
| S-SVM-16 | worker `vm_restart_reregistration_is_idempotent_and_does_not_duplicate_probe_tasks` | registration/task ownership | bounded-change |
| S-SVM-17 | sim `terminal_state_wins_and_dead_vm_backend_never_returns_to_eligibility` | seeded lifecycle/backend invariant | unbounded-preservation |
| S-SVM-18 | reconcilers `vm_startup_failure_leaves_running_owned_by_beacon_and_fails_only_startup` | Running/startup ownership | unbounded-preservation |
| S-SVM-19 | reconcilers `vm_startup_pass_changes_only_service_stable` | Stable action | bounded-change |
| S-SVM-20 | reconcilers `vm_readiness_flaps_only_backend_eligibility_and_recovers` | Backend.healthy action | bounded-change |
| S-SVM-21A | reconcilers `vm_liveness_threshold_emits_only_the_existing_liveness_stop` | ServiceLifecycle | bounded-change |
| S-SVM-21B | reconcilers `workload_lifecycle_alone_decides_restart_after_liveness_stop` | WorkloadLifecycle | bounded-change |
| S-SVM-22 | control-plane `one_server_boot_shares_exactly_one_trusted_probe_runner_with_both_drivers` | production composition/Arc ownership | bounded-change |
| S-SVM-23 | CLI `detached_service_deploy_forwards_vm_driver_without_parallel_request_shape` | deploy_service | bounded-change |
| S-SVM-24 | CLI `streaming_service_deploy_has_driver_projection_parity_with_detached_lane` | deploy_streaming_service | bounded-change |
| S-SVM-25 | E09 | 100 paired TCP product trials + VM client Jobs | bounded-change |
| S-SVM-26 | E10 | Exec/VM x 204/302/404/503 product matrix; 503 carries the nonempty `SVM-E10-FAILURE-BODY-MUST-NOT-LEAK` sentinel and the operator ledger requires zero sentinel exposure | bounded-change |
| S-SVM-27A | E11 `client-readiness-before.toml` | ready baseline + VM client Job Service traffic | bounded-change |
| S-SVM-27B | E11 `client-readiness-during.toml` | unavailable window + VM client Job Service traffic | bounded-change |
| S-SVM-27C | E11 `client-readiness-after.toml` | recovered window + VM client Job Service traffic | bounded-change |
| S-SVM-28 | E12 | liveness terminal/restart describe sequence | bounded-change |
| S-SVM-29 | E13 | inferred TCP pair + complementary VM client Jobs | bounded-change |

## AT-completeness audit

| Category | Result | Evidence |
|---|---|---|
| C1/C3 boundary/cardinality | PASS | 100 paired K1 trials, eight K2 cells, 2/2 K3 transitions, exact one server health owner |
| C2 state/order | PASS | Accepted before Stable, readiness withdrawal/recovery, liveness stop before workload restart |
| C4 idempotency/inverse | PASS | singular re-registration, readiness recovery, complementary inferred success/failure |
| C5 modes | PASS | VM/Exec, detached/streaming, TCP/HTTP, explicit/default/inferred, native-metal/default-feature |
| C6 errors | PASS | TCP refusal, HTTP 302/404/503, parser/direct rejection, capability refusal |
| C7 degradation/concurrency | PASS | seeded terminal authority, restart re-registration, liveness budget handoff |

No scenario authorizes a new public method, type, enum variant, parameter,
lifecycle state, persistence row, command, route, health mechanism, or guest
Exec protocol.
