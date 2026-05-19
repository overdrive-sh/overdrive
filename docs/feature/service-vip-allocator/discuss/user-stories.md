<!-- markdownlint-disable MD024 -->

# User Stories — service-vip-allocator

**SSOT**: [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).
Acceptance criteria below are lifted verbatim from the issue body
minus the two operator-pinned-VIP items (scope refined to platform-
issued-only per `wave-decisions.md` § Changed Assumptions), plus one
new AC covering the operator-supplied `vip = Some(...)` rejection.

## System Constraints

These cross-cutting constraints apply to every story in this feature:

- **Phase 1 single-node**. No cross-node consensus on allocations.
  Multi-node coordination is Phase 5+ per `wave-decisions.md`.
- **Platform-issued only**. Operators cannot supply `vip` in Service
  `[[listener]]` blocks. See `wave-decisions.md` § Changed Assumptions
  for the back-propagation against upstream Slice 06 of
  `workload-kind-discriminator`.
- **Allocator lives in `crates/overdrive-dataplane/`**, not control-
  plane. The new primitive is a generalisation of the existing
  `BackendIdAllocator` at `crates/overdrive-dataplane/src/allocator.rs:31`.
- **Solution-neutral**. The AC list below does not name traits,
  modules, persistence shapes, or admission layers. Those are DESIGN-
  wave decisions (see `wave-decisions.md` § Open questions for DESIGN).

---

## US-01: Platform allocates a Service VIP transparently on submit

### Problem

Maya Okonkwo is an Overdrive platform engineer who runs a single-node
control plane on her dev host. She has a Service spec for a stateless
web frontend in `frontend.toml` and wants to submit it without first
having to look up which VIP the platform expects her to use, or worse,
guess at an address and have submit fail with "VIP already taken". She
finds it painful to be on the hook for picking dataplane addresses —
that is the platform's job, not hers.

### Who

- **User type**: Overdrive platform engineer (operator persona).
- **Context**: Submits a Service spec via `overdrive job submit
  frontend.toml` against a local single-node control plane.
- **Motivation**: Trust the platform to materialise dataplane-level
  details (per J-OPS-002: "trust what the CLI tells me"; per J-OPS-003:
  "trust the platform to converge").

### Solution

A platform-issued VIP allocator that assigns a VIP from its pool to
each `[[listener]]` block of every Service workload at submit / first-
observation time, persists the assignment idempotently keyed on the
spec digest, surfaces it in the submit echo and `alloc status` render,
and releases the VIP on terminal-state transition for re-use.

### Domain Examples

#### 1: Happy Path — single Service, single listener, platform allocates

Maya Okonkwo writes `frontend.toml` with one `[[listener]]` block:

```toml
[service]
name = "frontend"

[[listener]]
port = 8080
protocol = "tcp"
```

She runs `overdrive job submit frontend.toml`. The platform allocates
`10.96.42.17` from its pool, persists the assignment keyed on the
spec digest, and the submit echo renders:

```text
Listeners:
  10.96.42.17:8080/tcp
```

`overdrive alloc status --job frontend` reflects the same VIP.

#### 2: Idempotency — same spec resubmitted, same VIP

Maya re-runs `overdrive job submit frontend.toml` against the
unchanged `frontend.toml` file (same spec digest). The platform
returns the same assignment — VIP `10.96.42.17` — without allocating
a fresh one. The pool's available count is unchanged.

#### 3: Error / Boundary — operator supplied a VIP, rejected with named guidance

Diego Hernández edits `frontend.toml` to pin a VIP:

```toml
[service]
name = "frontend"

[[listener]]
port = 8080
protocol = "tcp"
vip = "10.96.42.17"
```

He runs `overdrive job submit frontend.toml`. The platform rejects the
spec with a named error message identifying the field
(`listener.vip`) and explaining that the platform issues VIPs; the
operator should remove the field. No partial state is persisted.

#### 4: Reclamation — terminal-state transition releases the VIP

Maya's `frontend` Service runs for an hour, then she runs `overdrive
job stop frontend`. The workload transitions to a terminal state. The
allocator releases `10.96.42.17` back to its pool. A subsequent
submission of a *different* Service (with a different spec digest)
may receive `10.96.42.17` from the pool — the recycle is observable
via `overdrive alloc status` against the second Service.

#### 5: Pool exhaustion — allocator returns a typed error

Diego runs a stress test that submits 1024 distinct Service specs
against a pool sized for 256 VIPs. The 257th submission fails with a
typed error message naming pool exhaustion as the cause. The first
256 submissions remain admitted; the 257th has no partial state
persisted.

### UAT Scenarios (BDD)

#### Scenario: Operator submits a Service spec without supplying a VIP, platform allocates one

```gherkin
Given Maya Okonkwo has a Service spec `frontend.toml` with one
  `[[listener]]` block declaring `port = 8080`, `protocol = "tcp"`,
  and no `vip` field
And the platform's VIP pool has at least one available address
When Maya runs `overdrive job submit frontend.toml`
Then the platform allocates a VIP from its pool
And the submit echo renders the allocated VIP alongside the listener
And `overdrive alloc status --job frontend` renders the same VIP
```

#### Scenario: Resubmitting the same spec returns the same VIP idempotently

```gherkin
Given Maya Okonkwo has already submitted `frontend.toml` and received
  VIP `10.96.42.17`
And `frontend.toml` is byte-identical to the prior submission (same
  spec digest)
When Maya re-runs `overdrive job submit frontend.toml`
Then the platform returns the same VIP `10.96.42.17`
And the pool's available count is unchanged
```

#### Scenario: Operator-supplied `vip` is rejected with named guidance

```gherkin
Given Diego Hernández has a Service spec with a `[[listener]]` block
  that includes `vip = "10.96.42.17"`
When Diego runs `overdrive job submit <spec>.toml`
Then submission is rejected with a typed error naming the
  `listener.vip` field
And the error message explains that the platform issues VIPs and the
  operator should remove the field
And no allocator state is mutated
And no IntentStore admission occurs
```

#### Scenario: Terminal-state transition releases the VIP for reuse

```gherkin
Given Maya's `frontend` Service is running with allocated VIP
  `10.96.42.17`
When the workload transitions to a terminal state
Then the allocator releases `10.96.42.17` back to its pool
And a subsequent submission of a different Service spec may receive
  `10.96.42.17`
```

#### Scenario: Pool exhaustion produces a typed rejection

```gherkin
Given the VIP pool has zero available addresses
And every allocated VIP is bound to a non-terminal Service
When Diego submits a new Service spec without a `vip` field
Then submission is rejected with a typed error naming pool
  exhaustion as the cause
And no partial allocator state is persisted
And the first N admitted Services remain unaffected
```

### Acceptance Criteria

Derived from the UAT scenarios above and from the SSOT issue #167's
"Acceptance criteria" section (minus the two pinned-VIP items per
`wave-decisions.md` § Changed Assumptions). One AC per checkbox:

- [ ] **AC-01**: Submitting a Service spec without a `vip` field
  results in the platform allocating a VIP from its pool; the VIP is
  observable in the submit echo and `alloc status` render. (#167 AC 1)
- [ ] **AC-02**: Resubmitting the same spec (byte-identical / same
  spec digest) returns the same VIP; the allocation is idempotent on
  spec digest. (#167 AC 2)
- [ ] **AC-03**: On terminal-state transition of the workload, the
  allocated VIP is released back to the pool and made available for
  subsequent allocations. (#167 AC 3)
- [ ] **AC-04**: When the pool is exhausted, submission is rejected
  with a typed error naming pool exhaustion; no partial state is
  persisted, and unrelated admitted Services are unaffected. (#167 AC
  4)
- [ ] **AC-05**: The underlying allocator logic is shared between the
  existing `BackendIdAllocator` and the new Service VIP allocator;
  the shared primitive lives in `crates/overdrive-dataplane/`. (#167
  AC 6, refactored)
- [ ] **AC-06** (new, per `wave-decisions.md` § Changed Assumptions):
  An operator-supplied `vip = Some(...)` in a Service `[[listener]]`
  block is rejected with named guidance identifying the field and
  explaining that the platform issues VIPs. No allocator state is
  mutated and no admission occurs. The exact rejection layer —
  parser vs. admission — is left to DESIGN.

### Outcome KPIs

See `outcome-kpis.md` for the full framework. Story-level summary:

- **Who**: Overdrive platform engineers submitting Service specs
  against a single-node control plane.
- **Does what**: Submit Service specs without supplying a VIP; trust
  the platform to allocate one transparently and idempotently.
- **By how much**: 100% of Service submissions without operator-
  supplied `vip` succeed when the pool is non-empty; 0% silent
  allocation failures.
- **Measured by**: Allocator successful-allocation rate (admission
  success ÷ admission attempts where `vip = None` and pool non-empty);
  allocator-induced admission latency (p50 / p99); VIP reclamation lag
  on terminal-state transition.
- **Baseline**: Pre-feature, operators cannot submit a Service spec
  without a VIP at all (slice-06 lands the spec shape but defers the
  allocator to this feature). Baseline = 0% capability.

### Technical Notes

- The existing `BackendIdAllocator` at
  `crates/overdrive-dataplane/src/allocator.rs:31` is the structural
  precedent. Its API (`allocate(ip, port, proto) -> BackendId`,
  `release(id)`) and its monotonic-counter + memo-table shape inform
  the new primitive but do not constrain DESIGN's choice of factoring.
- Spec digest as the idempotency key (#167 AC 2 wording) suggests
  submit-time allocation, but the choice is DESIGN's per
  `wave-decisions.md` § Open Question 2.
- Reclamation trigger (which reconciler / action shim) is DESIGN's
  call per `wave-decisions.md` § Open Question 1.
- Pool config shape (existing `[dataplane]` block vs. new
  `[vip_allocator]` section) is DESIGN's call per `wave-decisions.md`
  § Open Question 3.
- The `[[listener]]` field shape from upstream slice-06
  (`vip: Option<ServiceVip>`) is preserved on the parsed-spec side.
  AC-06's rejection of operator-supplied `Some(...)` lands at the
  admission boundary (parser or admission layer, per `wave-decisions.md`
  § Open Question 5).
- Cross-references:
  - #167 (SSOT for this feature)
  - #164 (dataplane wiring of `Dataplane::update_service`, downstream)
  - #61 (IPv6 ULA range + DNS naming + auto-wake; usability layer on
    top of this feature, NOT a hard dependency for Phase 1 IPv4
    Service VIPs)
  - #178 (native east-west SPIFFE-ID resolution via local
    ObservationStore; sibling east-west primitive — Overdrive-aware
    workloads bypass VIP/DNS entirely. NOT a dependency of this
    feature; surfaced for context)
  - #163 (referenced in #167, separate concern)
