# Acceptance Test Scenarios — phase-1-control-plane-core

**Feature**: phase-1-control-plane-core
**Author**: Quinn (acceptance-designer)
**Date**: 2026-04-23
**Status**: Draft — awaits peer review + crafter translation

> **Amendment 2026-04-26.** `overdrive cluster init` was removed from
> Phase 1 in commit `d294fb8`. The walking-skeleton scenario §1.1 now
> starts at `serve` (the sole Phase 1 CA-minting site); §2b.1 / §2b.2
> are revised to assert against `serve`'s trust-triple write rather
> than `cluster init`'s; §2b.2's "re-init re-mints" property survives
> as "re-starting `serve` re-mints" (ADR-0010 §R1 as amended
> 2026-04-26). The `cluster init` verb returns in Phase 5 with the
> persistent CA + operator-cert ceremony per ADR-0010 §Amendment
> 2026-04-26 and GH #81. RCA:
> `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`.

Per `.claude/rules/testing.md` — **no `.feature` files**. Every scenario
below is a fenced `gherkin` markdown block. The crafter translates each
to a Rust `#[test]` / `#[tokio::test]` function in
`crates/{crate}/tests/acceptance/*.rs` or `tests/integration/*.rs`
(per ADR-0005 and the integration-vs-unit gating rule).

## Tag taxonomy

| Tag | Meaning |
|---|---|
| `@us-XX` | Originating user story (traceability; DWD-08) |
| `@walking_skeleton` | Walking-skeleton scenario (per DWD-01) |
| `@driving_adapter` | Enters through the `overdrive` CLI subprocess (the user-facing port) |
| `@library_port` | Enters through a Rust public-API surface (in-process library call) |
| `@real-io` | Exercises a real local-resource adapter (real redb, real axum, real reqwest, real rcgen, real libsql) |
| `@adapter-integration` | Proves adapter wiring, not just layer composition |
| `@property` | Universal invariant — crafter translates as proptest (DWD-05) |
| `@error-path` | Error / boundary / invariant-red scenario |
| `@kpi KN` | Enforces outcome KPI N (K1–K7) |
| `@journey:submit-a-job` | Derived from the submit-a-job journey |

---

## 1. Walking-skeleton scenarios (end-to-end)

### 1.1 Ana submits a job and reads the same spec digest back

```gherkin
@walking_skeleton @real-io @adapter-integration @driving_adapter
@us-01 @us-02 @us-03 @us-05 @journey:submit-a-job @kpi K1
Scenario: Ana submits a job and sees the spec digest round-trip byte-identical
  Given Ana has a freshly cloned overdrive workspace
    And a scratch data directory on a temporary filesystem path
    And a TOML file payments.toml that describes a single-replica payments service
    And the control plane has been started via overdrive serve against that directory
    And serve has minted the trust triple in-process and written it to that directory's .overdrive/config
    And the server is listening on the default local endpoint
  When Ana runs overdrive job submit payments.toml as a subprocess
    And Ana runs overdrive alloc status --job payments as a subprocess
  Then both subprocesses exit with status zero
    And the submit output names the job ID payments
    And the submit output names the canonical intent key jobs/payments
    And the submit output reports a commit index equal to or greater than one
    And the alloc status output shows a spec digest byte-identical to what Ana can compute locally from the same file
    And the alloc status output explicitly states that zero allocations are placed
    And the alloc status output names phase-1-first-workload as the next feature
```

### 1.2 Ana confirms the reconciler primitive is alive via cluster status

```gherkin
@walking_skeleton @real-io @adapter-integration @driving_adapter
@us-04 @us-05 @journey:submit-a-job @kpi K4 @kpi K5
Scenario: Reconciler primitive is registered and observable after clean boot
  Given a freshly initialised and started control plane in single mode
  When Ana runs overdrive cluster status as a subprocess
  Then the subprocess exits with status zero
    And the output names mode single
    And the reconcilers section lists noop-heartbeat
    And the broker counters section names queued and cancelled and dispatched
    And every broker counter renders as a non-negative integer
    And the commit index reported matches what the intent store reports
```

### 1.3 Byte-identical resubmit is idempotent; different spec at same key is a conflict

```gherkin
@walking_skeleton @real-io @adapter-integration @driving_adapter @error-path
@us-03 @us-05 @journey:submit-a-job @kpi K1 @kpi K6
Scenario: Ana resubmits the same spec and then submits a different one at the same key
  Given a running control plane with a previously-committed job payments at commit index 17
    And Ana has the original payments.toml on disk
    And Ana has a modified payments-altered.toml whose content differs by one replica count
  When Ana runs overdrive job submit payments.toml a second time
    And Ana runs overdrive job submit --job-id payments payments-altered.toml
    And Ana runs overdrive job submit payments.toml a third time
  Then the second submit exits with status zero and reports commit index 17
    And the third invocation of the original spec exits with status zero and reports commit index 17
    And the submit of payments-altered.toml exits with status one
    And its error output names the conflict by explaining that a different spec exists at the same intent key
    And its error output does not contain a raw Rust panic or a raw reqwest error format
```

---

## 2. US-01 — Job / Node / Allocation aggregates + canonical intent keys

### 2.1 Happy path — library-port construction and round-trip

```gherkin
@us-01 @library_port @property
Scenario: Any valid Job aggregate round-trips through rkyv with byte-identical archives
  Given any valid Job value produced by the aggregate generator
  When Ana archives the Job via rkyv and then accesses and deserialises it
  Then the resulting Job equals the original
    And two independent archivals of the same logical Job produce byte-identical bytes
```

```gherkin
@us-01 @library_port @property
Scenario: Any valid Job aggregate round-trips through serde-JSON
  Given any valid Job value produced by the aggregate generator
  When Ana serialises the Job to JSON and deserialises the result
  Then the resulting Job equals the original
    And the two round-trips are non-substitutable: the rkyv bytes and the JSON bytes are different formats with the same semantic content
```

```gherkin
@us-01 @library_port @property
Scenario: Canonical intent key derivation is stable for any valid JobId
  Given any valid JobId value
  When Ana calls the intent-key function with that JobId twice
  Then both calls produce identical bytes
    And the canonical string form is jobs/ followed by the JobId's display
```

```gherkin
@us-01 @library_port
Scenario: Constructing a Node reuses the Resources type already exposed by the driver trait
  Given a Resources value constructed through the driver-trait's public constructor
  When Ana constructs a Node using that Resources value
  Then the Node's capacity field holds the same Resources value
    And no duplicate Resources type is introduced anywhere in overdrive-core
```

```gherkin
@us-01 @library_port
Scenario: Allocation links a Job and a Node through typed newtypes only
  Given a JobId and a NodeId
  When Ana constructs an Allocation pairing them with a fresh AllocationId
  Then the Allocation's public fields expose the three newtypes
    And no raw String or u64 identifiers appear in the Allocation's public field signatures
```

### 2.2 Error boundaries

```gherkin
@us-01 @library_port @error-path
Scenario: Node construction rejects zero-byte memory capacity
  Given a Node spec whose capacity names zero bytes of memory
  When Ana calls the Node validating constructor
  Then Ana receives an error naming the zero-memory violation
    And no Node value is constructed
```

```gherkin
@us-01 @library_port @error-path
Scenario: Job construction rejects a zero-replica count
  Given a Job spec whose replicas field is zero
  When Ana calls the Job validating constructor
  Then Ana receives an error naming the replicas field and the invalid value
    And no Job value is constructed
```

```gherkin
@us-01 @library_port @error-path
Scenario: Job construction rejects a malformed JobId before any archive attempt
  Given a Job spec whose id contains a forbidden character
  When Ana calls the Job validating constructor
  Then Ana receives an error naming the parse failure in the id field
    And no rkyv archive attempt is made on the invalid input
```

```gherkin
@us-01 @library_port
Scenario: Intent-side Job and observation-side AllocStatusRow are distinct Rust types
  Given the intent-side Job aggregate exported from overdrive-core's aggregate module
    And the observation-side AllocStatusRow exported from the observation-store trait module
  When Ana inspects the two exported type paths
  Then the two paths are different Rust types in different modules
    And no surviving JobSpec-named struct appears in the observation-store trait module
```

---

## 3. US-02 — Control-plane HTTP/REST service surface

### 3.1 Happy path — submit round-trip through the REST port

```gherkin
@us-02 @real-io @adapter-integration @kpi K1
Scenario: HTTP POST to the submit endpoint commits through the real intent store
  Given a running control plane on https://127.0.0.1:7001 backed by real redb
  When a reqwest client posts a valid Job spec as JSON to /v1/jobs
  Then the response carries status 200
    And the response body is a JSON object with a job_id field and a commit_index field
    And the commit_index is greater than or equal to one
    And a subsequent GET to /v1/jobs/{id} returns the same spec as the request body
```

### 3.2 Happy path — server start and clean shutdown

```gherkin
@us-02 @driving_adapter @real-io
Scenario: The server binds over TLS and shuts down cleanly on SIGINT
  Given Ana starts overdrive serve as a subprocess
  When the server has bound to the default endpoint and accepted a readiness ping
    And Ana delivers a SIGINT to the server process
  Then the server stops accepting new connections
    And any in-flight request completes before the process exits
    And the process exit code is zero
```

### 3.3 OpenAPI schema derivation and drift detection

```gherkin
@us-02 @library_port
Scenario: The OpenAPI schema derived from the Rust types matches the checked-in document
  Given the overdrive-control-plane crate exports typed request and response structs with utoipa annotations
  When Ana runs cargo xtask openapi-check
  Then the subprocess exits with status zero
    And no diff is printed
```

```gherkin
@us-02 @library_port @error-path
Scenario: Handler drift from the schema fails the openapi-check gate
  Given a handler whose request type has been modified without regenerating the schema
  When Ana runs cargo xtask openapi-check
  Then the subprocess exits with non-zero status
    And the output names the schema field that drifted
    And the output suggests running cargo xtask openapi-gen to regenerate
```

### 3.4 Error-path — connection refused is actionable

```gherkin
@us-02 @us-05 @driving_adapter @error-path @kpi K6
Scenario: The CLI renders connection-refused as an actionable message naming the endpoint
  Given no control plane is running on the default endpoint
  When Ana runs overdrive job submit payments.toml
  Then the CLI exits with status one
    And the output names the endpoint Ana tried to reach
    And the output suggests starting the control plane as a concrete next step
    And the output contains no raw ECONNREFUSED token or reqwest debug format
```

### 3.5 Error-path — invalid TLS trust material is actionable

```gherkin
@us-02 @us-05 @driving_adapter @error-path @kpi K6
Scenario: The CLI rejects a malformed trust triple without pretending to trust the server
  Given Ana's ~/.overdrive/config carries a corrupted CA cert
  When Ana runs overdrive cluster status
  Then the CLI exits with status one
    And the output names the config file and the field that could not be parsed
    And the output does not suggest --insecure as a workaround because the flag does not exist
```

### 3.6 Endpoints match the ADR-0008 table

```gherkin
@us-02 @real-io
Scenario: Every walking-skeleton endpoint declared in the architecture responds at its documented path
  Given a running control plane
  When a reqwest client issues requests to /v1/jobs, /v1/jobs/{id}, /v1/cluster/info, /v1/allocs, and /v1/nodes
  Then each response carries a status that is either 200 or 404 or 400, never 501 not-implemented
    And each response body is valid JSON or an empty body as documented per endpoint
```

---

## 4. US-03 — API handlers commit to IntentStore + ObservationStore reads

### 4.1 Submit then Describe round-trips the spec byte-identical

```gherkin
@us-03 @real-io @adapter-integration @kpi K1
Scenario: Submit then Describe returns the same spec Ana submitted
  Given a running control plane in single mode
    And Ana has submitted a valid Job spec via the submit endpoint
  When Ana GETs /v1/jobs/{id} using the returned JobId
  Then the response spec equals the submitted spec after rkyv access
    And the response spec_digest equals ContentHash::of the archived submitted bytes
```

### 4.2 Validation fires before any IntentStore write

```gherkin
@us-03 @real-io @error-path @kpi K2
Scenario: Malformed spec is rejected before the intent store is touched
  Given a running control plane with an empty IntentStore
    And a JSON body whose replicas field is zero
  When a reqwest client posts that body to /v1/jobs
  Then the response carries status 400
    And the JSON error body names the replicas field
    And the IntentStore contains zero entries for the malformed input
```

### 4.3 Typed validation error maps to HTTP 400

```gherkin
@us-03 @real-io @error-path @kpi K2
Scenario: Every aggregate validation failure surfaces as 400 with a structured body
  Given any validation-failing aggregate input drawn from the aggregate-error generator
  When a reqwest client posts the input to /v1/jobs
  Then the response carries status 400
    And the JSON error body carries an error field naming the error class
    And the JSON error body carries a field naming the offending field when one exists
    And the body is not a raw stack trace
```

### 4.4 Describe on an unknown job returns 404

```gherkin
@us-03 @real-io @error-path
Scenario: Describe on an unknown JobId returns 404 with a structured body
  Given a running control plane that has committed no job called unknown-id
  When a reqwest client GETs /v1/jobs/unknown-id
  Then the response carries status 404
    And the JSON error body carries an error field equal to not_found
    And the IntentStore is unchanged
```

### 4.5 Commit index is strictly monotonic across successive submits

```gherkin
@us-03 @real-io @property @kpi K3
Scenario: Any sequence of valid submits produces a strictly increasing commit index
  Given a running control plane and any sequence of at least three distinct valid Job specs
  When Ana submits each spec in order
  Then each response's commit_index is strictly greater than the previous response's commit_index
```

### 4.6 Commit index accessor returns a raw sequence, not an internal handle

```gherkin
@us-03 @library_port
Scenario: LocalStore exposes commit_index as a plain u64 accessor
  Given a LocalStore backed by a redb file on a temporary path
  When Ana calls the commit_index accessor on that store
  Then the return type is u64
    And no redb transaction or internal handle type leaks into the return signature
```

### 4.7 AllocStatus returns an empty row set in Phase 1

```gherkin
@us-03 @real-io @kpi K7
Scenario: AllocStatus returns zero rows when no scheduler or driver has run
  Given a running control plane in single mode with no scheduler and no node agent
  When a reqwest client GETs /v1/allocs
  Then the response carries status 200
    And the rows field in the JSON body is an empty array
    And no fabricated placeholder row appears in the body
```

### 4.8 NodeList returns an empty row set in Phase 1

```gherkin
@us-03 @real-io @kpi K7
Scenario: NodeList returns zero rows when no node agent has registered
  Given a running control plane in single mode with no node agent
  When a reqwest client GETs /v1/nodes
  Then the response carries status 200
    And the rows field in the JSON body is an empty array
    And no fabricated local node row appears in the body
```

### 4.9 Byte-identical re-submit is an idempotent 200

```gherkin
@us-03 @real-io @adapter-integration
Scenario: Re-submitting the exact same spec at the same key returns the original commit index
  Given a running control plane where Ana has already submitted a Job spec at commit index 17
  When Ana submits the byte-identical spec a second time at the same intent key
  Then the response carries status 200
    And the response commit_index is 17
    And the IntentStore contains only one entry at the intent key
```

### 4.10 Different spec at same key is 409 Conflict

```gherkin
@us-03 @real-io @error-path
Scenario: A different spec at an occupied intent key is a 409 Conflict
  Given a running control plane where Ana has already submitted a Job spec at intent key jobs/payments
  When Ana submits a semantically different spec at the same intent key
  Then the response carries status 409
    And the JSON error body carries an error field equal to conflict
    And the IntentStore still carries the original spec under that intent key
```

### 4.11 Infrastructure failure returns 500 with structured body

```gherkin
@us-03 @real-io @error-path
Scenario: A simulated IntentStore IO failure surfaces as 500 with a structured body
  Given a running control plane whose intent store is configured to fail on the next write
  When a reqwest client posts a valid spec to /v1/jobs
  Then the response carries status 500
    And the JSON error body carries an error field equal to internal
    And the body does not contain a raw Rust panic or a stack trace
```

---

## 5. US-04 — Reconciler primitive, runtime, and evaluation broker

### 5.1 Runtime registers the noop-heartbeat reconciler at boot

```gherkin
@us-04 @library_port @kpi K4
Scenario: Control-plane boot registers at least one reconciler
  Given a fresh control plane configured in single mode
  When the reconciler runtime completes its boot sequence
  Then the registry reports a non-empty set of reconcilers
    And the set contains the noop-heartbeat reconciler by its canonical name
```

### 5.2 Broker collapses duplicate evaluations

```gherkin
@us-04 @library_port @kpi K4
Scenario: Three concurrent evaluations at the same key collapse to one dispatch
  Given a reconciler named noop-heartbeat registered with the runtime
    And the broker is empty
  When three evaluations arrive at the key noop-heartbeat target job/payments within one broker tick
    And the broker drains its pending queue
  Then exactly one evaluation is dispatched
    And the cancelled counter increases by exactly two
    And the queued counter returns to zero after the drain
```

### 5.3 Cancelable-eval-set reaper bounds the set

```gherkin
@us-04 @library_port
Scenario: The cancelable set is reclaimed in bulk and does not grow unboundedly
  Given N cancelled evaluations accumulate across K broker ticks
  When the in-runtime reaper runs
  Then the cancelable set is emptied in bulk
    And the cancelled counter never grows faster than the number of submissions
```

### 5.4 Per-primitive libSQL databases are filesystem-isolated

```gherkin
@us-04 @real-io
Scenario: Two reconcilers get distinct libSQL database paths
  Given two reconcilers named alpha and beta registered with the runtime
  When the runtime provisions their private memory databases
  Then the file path for alpha starts with <data-dir>/reconcilers/alpha/
    And the file path for beta starts with <data-dir>/reconcilers/beta/
    And neither path escapes the configured data directory
```

### 5.5 Reconciler cannot read another reconciler's DB handle

```gherkin
@us-04 @library_port @error-path
Scenario: Alpha's injected Db handle cannot read beta's data
  Given two reconcilers alpha and beta are registered
    And beta's DB contains a row named secret
  When alpha's reconcile function is invoked with its injected Db handle
  Then alpha cannot access beta's secret row through its Db handle
    And any attempted path-traversal in a reconciler's name was rejected at registration
```

### 5.6 cluster status surfaces the registry and counters

```gherkin
@us-04 @us-05 @real-io @driving_adapter @kpi K5
Scenario: Ana runs cluster status and sees the reconciler registry rendered
  Given a running control plane with noop-heartbeat registered
  When Ana runs overdrive cluster status
  Then the output contains a reconcilers section naming noop-heartbeat
    And the output contains a broker section naming queued, cancelled, and dispatched with their integer values
    And the output contains a mode line naming single
```

### 5.7 DST invariant — at_least_one_reconciler_registered

```gherkin
@us-04 @library_port @property @kpi K4
Scenario: The DST harness asserts the registry is non-empty on every boot
  Given the DST harness boots the control-plane subsystem against any valid seed
  When the at_least_one_reconciler_registered invariant evaluates at a stable post-boot tick
  Then the invariant passes on every seed in the default catalogue
```

### 5.8 DST invariant — duplicate_evaluations_collapse

```gherkin
@us-04 @library_port @property @kpi K4
Scenario: The DST harness proves N concurrent evaluations at one key collapse to one dispatch
  Given the DST harness submits three or more evaluations at the same reconciler-target key within one broker tick
  When the duplicate_evaluations_collapse invariant evaluates after the broker drain
  Then exactly one dispatched invocation is observed
    And exactly N minus one cancelled invocations are observed
    And the invariant reports pass
```

### 5.9 DST invariant — reconciler_is_pure

```gherkin
@us-04 @library_port @property @kpi K4
Scenario: Twin invocation with identical inputs produces identical action sequences
  Given any registered reconciler and any arbitrary desired-and-actual state pair
  When the DST harness invokes the reconciler's reconcile twice with the same inputs
  Then the two returned Vec<Action> sequences are equal element-for-element
    And the reconciler_is_pure invariant reports pass
```

### 5.10 Reconciler that attempts forbidden I/O is refused at dst-lint time

```gherkin
@us-04 @library_port @error-path
Scenario: Smuggled Instant::now in a reconciler body is blocked by the lint gate
  Given a reconciler body that calls std::time::Instant::now inside reconcile
  When Ana runs cargo xtask dst-lint
  Then the subprocess exits with non-zero status
    And the output names the file line and the banned symbol
    And the output names the Clock trait as the remediation
```

---

## 6. US-05 — CLI handlers for job / alloc / cluster / node

### 6.1 job submit round-trips and prints actionable next steps

```gherkin
@us-05 @driving_adapter @real-io @adapter-integration @kpi K1
Scenario: overdrive job submit prints the job ID, intent key, commit index, and a Next hint
  Given a running control plane on the default endpoint
    And a file payments.toml containing a valid Job spec
  When Ana runs overdrive job submit payments.toml
  Then the CLI exits with status zero
    And the output contains the Job ID payments
    And the output contains the canonical intent key jobs/payments
    And the output contains a commit index greater than or equal to one
    And the output ends with a line suggesting overdrive alloc status --job payments as the next step
```

### 6.2 alloc status renders a spec digest equal to the local compute

```gherkin
@us-05 @driving_adapter @real-io @adapter-integration @kpi K1 @kpi K7
Scenario: alloc status shows the same spec digest Ana can compute locally
  Given Ana has previously submitted payments.toml and received a commit index
  When Ana runs overdrive alloc status --job payments
  Then the output names a spec digest
    And that digest equals what Ana computes locally by archiving the same payments.toml via rkyv and hashing
    And the output states explicitly that zero allocations are placed
    And the output names phase-1-first-workload as the next feature
```

### 6.3 node list renders an honest empty state

```gherkin
@us-05 @driving_adapter @real-io @kpi K7
Scenario: overdrive node list prints an explicit empty state when no nodes exist
  Given a control plane with zero registered nodes
  When Ana runs overdrive node list
  Then the CLI exits with status zero
    And the output is not a blank table or a silent exit
    And the output names node agent as the next feature that will populate the list
```

### 6.4 cluster status renders the reconciler registry

```gherkin
@us-05 @driving_adapter @real-io @kpi K5
Scenario: overdrive cluster status prints the reconciler registry and broker counters
  Given a running control plane with noop-heartbeat registered
  When Ana runs overdrive cluster status
  Then the output lists noop-heartbeat in the reconcilers section
    And the output reports the broker's queued, cancelled, and dispatched counters
    And each counter is a non-negative integer
```

### 6.5 Unreachable endpoint renders an actionable error

```gherkin
@us-05 @driving_adapter @error-path @kpi K6
Scenario: job submit against a down endpoint renders an actionable multi-line error
  Given the control plane is not running on the default endpoint
  When Ana runs overdrive job submit payments.toml
  Then the CLI exits with status one
    And the output explains that the endpoint could not be reached
    And the output gives at least three concrete next steps
    And the output does not contain a raw Rust panic, a raw ECONNREFUSED token, or a raw reqwest error debug format
```

### 6.6 Malformed spec error names the field

```gherkin
@us-05 @driving_adapter @error-path @kpi K2 @kpi K6
Scenario: job submit with an invalid spec names the offending field
  Given a file broken.toml whose replicas field is zero
  When Ana runs overdrive job submit broken.toml
  Then the CLI exits with status one
    And the output names the field replicas
    And the output names the invalid value zero
    And the CLI does not fall back to submitting anything to the server
```

### 6.7 Empty alloc status is honest, not blank

```gherkin
@us-05 @driving_adapter @real-io @kpi K7
Scenario: alloc status on an unknown job names that the job is not committed
  Given a running control plane where no job called mystery has been committed
  When Ana runs overdrive alloc status --job mystery
  Then the CLI exits with status one
    And the output names the job id mystery
    And the output explains that no such job is committed
    And the output suggests overdrive job submit as the next step
```

### 6.8 Endpoint precedence — flag over env over default

```gherkin
@us-05 @driving_adapter
Scenario: --endpoint flag overrides OVERDRIVE_ENDPOINT env which overrides the default
  Given the environment variable OVERDRIVE_ENDPOINT is set to https://env.example:9001
  When Ana runs overdrive --endpoint https://flag.example:9002 cluster status
  Then the CLI attempts to connect to https://flag.example:9002
    And the effective endpoint printed in the CLI output is https://flag.example:9002
```

### 6.9 First output within a modest localhost budget

```gherkin
@us-05 @driving_adapter @real-io
Scenario: job submit prints its first output line within a modest localhost budget
  Given a running control plane on the default endpoint
  When Ana runs overdrive job submit payments.toml on an M-class laptop
  Then the first line of output appears within 100 milliseconds of process start
    And no artificial spinner appears in the output
```

---

## 2b. US-01 — TLS bootstrap and trust triple (driving-adapter gate)

These scenarios live under US-02's numbering in §3 but are TLS-bootstrap-
specific; they prove the ADR-0010 adapters. Renumbered §2b here for
readability.

> Per ADR-0010 §R1 as amended 2026-04-26, `serve` is the sole Phase 1
> cert-minting site (`cluster init` removed in commit `d294fb8`). The
> bootstrap-asserting scenarios below are written against `serve`'s
> trust-triple write; the "re-init re-mints" property is preserved as
> "re-starting `serve` re-mints."

### 2b.1 First boot writes a valid trust triple

```gherkin
@us-02 @driving_adapter @real-io @adapter-integration
Scenario: overdrive serve writes a fully-formed trust triple on first boot
  Given a scratch home directory with no previous overdrive state
  When Ana runs overdrive serve against that directory
  Then serve binds the TLS listener
    And the file <home>/.overdrive/config exists and parses as ADR-0019 TOML
    And the TOML carries a current-context pointer and a [[contexts]] array-of-tables
    And each context carries an endpoint, a ca field with base64-encoded PEM, a crt field, and a key field
    And the CA's subject alternative names include 127.0.0.1 and ::1 and localhost
```

### 2b.2 Re-starting serve re-mints the ephemeral CA

```gherkin
@us-02 @driving_adapter @real-io
Scenario: A second overdrive serve start re-mints the ephemeral CA
  Given a previous overdrive serve start produced a CA certificate C1 and was stopped cleanly
  When Ana runs overdrive serve a second time against the same directory
  Then a new CA certificate C2 is present in the config
    And C2 is not byte-identical to C1
    And serve does not prompt for a password or for confirmation
```

### 2b.3 No --insecure flag exists

```gherkin
@us-02 @driving_adapter @error-path
Scenario: Passing --insecure is rejected by the argument parser
  Given the overdrive binary built from this repository
  When Ana runs overdrive --insecure cluster status
  Then the CLI exits with a usage-error status
    And the output names --insecure as an unknown argument
    And no subcommand executes
```

---

## Adapter coverage summary

Cross-reference to DWD-09. Every adapter new to this feature is
covered by at least one scenario above:

| Adapter | Primary scenarios |
|---|---|
| `rcgen` ephemeral CA | §2b.1, §2b.2 |
| `axum` + `rustls` server | §1.1, §3.1, §3.2 |
| `utoipa` schema derivation | §3.3, §3.4 |
| `reqwest` CLI client | §1.1, §6.1, §6.5 |
| `libsql` per-primitive memory | §5.4, §5.5 |
| Evaluation broker | §5.2, §5.3, §5.6, §5.8 |
| `LocalStore::commit_index` | §4.5, §4.6 |
| `SimObservationStore` wired as Phase 1 server impl | §4.7, §4.8, §6.3 |
| `Reconciler` trait + runtime | §5.1, §5.7, §5.9, §5.10 |
| HTTP error mapping `ControlPlaneError::to_response` | §4.2, §4.3, §4.4, §4.10, §4.11 |
| Idempotent re-submit | §1.3, §4.9 |

---

## Scenario counts

| Section | Total | Error-path or property |
|---|---|---|
| §1 Walking skeletons | 3 | 1 (§1.3) |
| §2 US-01 aggregates | 9 | 5 (§2.1 property, §2.2 property, §2.3 property, §2.4 error, §2.5 error, §2.6 error) — 4 error + 3 property |
| §2b US-01 TLS bootstrap | 3 | 1 error (§2b.3) |
| §3 US-02 REST surface | 6 | 3 (§3.2 error, §3.3 error, §3.4 error, §3.5 error) |
| §4 US-03 handlers | 11 | 7 (§4.2, §4.3 property, §4.4, §4.5 property, §4.10, §4.11) |
| §5 US-04 reconciler | 10 | 4 error + 3 property (§5.5, §5.7, §5.8, §5.9, §5.10) |
| §6 US-05 CLI | 9 | 3 error (§6.5, §6.6, §6.7) |
| **Total** | **51** | **22 @error-path + 8 @property** = **30 boundary-exercising** |

Raw error-path ratio: 22/51 ≈ 43%. With property-shaped boundary
coverage: 30/51 ≈ 59%. Target ≥ 40% per DWD-10 — met on raw count
alone.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial DISTILL acceptance scenarios for phase-1-control-plane-core. 51 scenarios across 3 walking skeletons + 5 US sections + TLS bootstrap sub-section. |
| 2026-04-26 | Amendment — `cluster init` removed from Phase 1 (commit `d294fb8`). §1.1 Given clause revised to drop the `cluster init` step (`serve` is now the sole minter). §2b.1 / §2b.2 rewritten to assert against `serve`'s trust-triple write (ADR-0010 §R1 as amended 2026-04-26). §2b.3 unchanged. RCA: `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`. Phase 5 reintroduction: GH #81. |
