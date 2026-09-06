# ADR-0091: Reuse the Service driver union and reject VM Exec probes before intent commit

## Status

**Accepted** (2026-09-06), user-selected during guided DESIGN and approved by
independent architecture review iteration 2. GH #257. Optional in-guest Exec
probes remain separately scoped by GH #280.

**Proposed greenfield persistence amendment** (2026-09-06; user-authorized;
independent DESIGN review is required before DELIVER step 01-01 resumes). No
deployment uses the prior persisted ServiceSpec representations. This replaces
the temporary boxed-V3 archival-compatibility amendment: the greenfield schema
does not support, decode, migrate, or re-archive V1/V2 bytes. It changes no
operator, parser, wire request, intent, allocation, describe, probe, or
lifecycle behavior.

Partially supersedes ADR-0083 D4's blanket `[service] + [vm]` rejection. It
retains ADR-0083's existing `DriverInput::{Exec, Vm}` and
`WorkloadDriverV2::{Exec, Vm}` unions.

## Context

The current parser-side `ServiceSpecV2` still carries an Exec-only `exec`
field, and `SectionPresence::validated` rejects `[service] + [vm]`. Downstream
of that stale edge, the platform already has the required union:

- wire `ServiceSpecInput.driver: DriverInput`;
- persisted `ServiceV2.driver: WorkloadDriverV2`;
- workload-to-allocation projection through `DriverPayload::{Exec, Vm}`;
- driver-registry dispatch; and
- describe output `ServiceSpecOutput.driver: DriverInput`.

GH #222 delivered the guest network/intercept prerequisite that justified the
old blanket rejection. GH #257 now admits host-originated HTTP/TCP probes for
VM Services. Arbitrary in-guest Exec probes are not required for that outcome
and are tracked by GH #280.

## Decision

Widen only the parser payload and its existing projections. Re-alias
`ServiceSpec` and `ServiceSpecLatest` to V3, and replace V2's
`exec: ExecInput` with `driver: workload_spec::DriverInput` in this exact V3
payload:

```rust
pub struct ServiceSpecV3 {
    pub id: String,
    pub replicas: u32,
    pub driver: workload_spec::DriverInput,
    pub resources: workload_spec::ResourcesInput,
    pub listeners: Vec<Listener>,
    pub startup_probes: Vec<ProbeDescriptor>,
    pub readiness_probes: Vec<ProbeDescriptor>,
    pub liveness_probes: Vec<ProbeDescriptor>,
}
```

### Greenfield ServiceSpec persistence contract

`ServiceSpec` and `ServiceSpecLatest` both alias the direct, unboxed
`ServiceSpecV3` payload above. `ServiceSpecEnvelope` remains the sole
per-type codec boundary and has exactly one declaration:

```rust
pub enum ServiceSpecEnvelope {
    V3(ServiceSpecV3),
}
```

The sole arm's rkyv discriminant is `0`; consequently
`ServiceSpecEnvelope::known_discriminants()` is exactly `[0]`. The variant
name remains `V3` because it is the existing current parser payload type; its
numeric archive tag is not a retained V3 tag. `latest(payload)` returns
`ServiceSpecEnvelope::V3(payload)` directly, and `into_latest()` returns that
payload directly. There is no `Box<ServiceSpecV3>` at the envelope or any other
parser/persistence boundary.

`ServiceSpecV1`, `ServiceSpecV2`, their envelope arms and re-exports, their
conversion implementations, all V1/V2 fixture literals, their decode tests,
and every re-archive or migration assertion are deleted. The only
schema-evolution evidence is one current direct `FIXTURE_V3`, written through
`latest`, which decodes to the exact `ServiceSpecLatest` and round-trips to the
same current bytes. Bytes produced by either removed representation are not a
supported persisted format and must not be accepted via a fallback decoder,
compatibility prefix, or migration path.

This is a persistence-contract reset authorized for the greenfield project,
not a runtime-indirection decision. Parser construction, TOML parsing, CLI
projection, API admission, intent persistence, allocation projection, and
describe carry the same direct `ServiceSpecV3` / existing driver union. No
ServiceSpec `exec_command` compatibility accessor exists or is added.

`SectionPresence::validated` applies the existing exactly-one-of
`[exec]`/`[vm]` rule to Service as it already does for Job/Schedule. The stale
`VmNotAllowedOnServiceKind` path is removed. Both existing Service CLI deploy
lanes project the parser variant field-for-field into the existing wire union;
no new command, request variant, Service kind, or persisted intent version is
added.

`ServiceV2::from_submit` authoritatively projects and validates either driver
arm using the same driver validation already used by `JobV2::from_submit`.
`ServiceV2::to_describe` projects both persisted arms into the existing
describe driver union rather than treating VM as unreachable.

### VM Exec-probe exclusion

After the Service driver and all three role lists are known, both ingress
boundaries select the first `ProbeMechanic::Exec` by a fixed total order:
**Startup -> Readiness -> Liveness**, then the lowest zero-based vector
position inside the selected role. `ProbeDescriptor.idx` is not used to choose
the winner because it is role-local and a wire caller's value is untrusted.

The selected location is projected through the existing error fields:

| Role | `ParseError::Field.section` | `AggregateError::Validation.field` |
|---|---|---|
| Startup | `"[[health_check.startup]]"` | `"startup_probes"` |
| Readiness | `"[[health_check.readiness]]"` | `"readiness_probes"` |
| Liveness | `"[[health_check.liveness]]"` | `"liveness_probes"` |

The parser `message` starts `entry [{position}]:`; the aggregate `message`
starts `[{position}]:`. Both append the exact common diagnostic `exec probes
are not supported for VM Service workloads; use HTTP or TCP; optional VM Exec
probes are tracked by GH #280`. No public error type, variant, field, or Exec
implementation is added. A syntactically invalid TOML probe may still fail
while its descriptor is being constructed; the total-order rule applies once
the three parsed role vectors exist. `ServiceV2::from_submit` performs the
same cross-field selection for direct API clients.

Both checks happen before a `ServiceV2` value exists. The submit handler can
therefore neither archive nor commit the invalid intent. Exec-backed Service
Exec probes remain unchanged.

### Lifecycle Gate Ownership

**Not applicable:** admission determines whether an intent may be constructed;
it does not change the meaning or ordering of allocation Running, Service
Stable, readiness health, `ServiceLifecycle` liveness termination, or
`WorkloadLifecycle`'s sole restart authority.

## Alternatives considered

1. **New `VmServiceSpec` and VM deploy path — rejected.** Driver kind is
   already an axis inside the existing Service contract; duplicating the kind
   forks parsing, validation, CLI behavior, and lifecycle.
2. **Parser accepts VM; server alone rejects VM Exec — rejected.** Persistence
   remains protected, but the CLI sends a request it can already prove invalid
   and violates the requested parse-time feedback.
3. **New persisted Service/WorkloadIntent version — rejected.** The live
   persisted Service payload already stores the VM-capable driver union. Only
   parser `ServiceSpec` changes archived shape and therefore needs V3.
4. **Ship VM Exec machinery inside GH #257 — rejected.** HTTP/TCP meet the
   active Service-health outcome; Exec introduces a separate guest-control and
   supervision capability already tracked by GH #280.
5. **Keep the temporary boxed-V3 archival shape — rejected.** Its only
   purpose was preserving retired V1/V2 bytes. It adds an unnecessary pointer
   indirection to a schema that is greenfield by user-authorized assumption.
6. **Add a legacy decoder, migration, re-archive path, or second format —
   rejected.** No prior ServiceSpec bytes are supported, and adding any of
   those paths would recreate the retired compatibility contract.

## Consequences

- Positive: `overdrive deploy <spec>` and workload describe remain the only
  operator surfaces, now honest for either Service driver.
- Positive: one existing union flows parser → wire → intent → allocation →
  describe; no parallel VM-Service model.
- Positive: invalid VM Exec probes are rejected locally and authoritatively
  before persistence.
- Negative: existing persisted V1/V2 ServiceSpec bytes are intentionally
  unreadable after this greenfield cut; operators must not carry them into the
  new project.
- Negative: two validation boundaries intentionally repeat the same rule for
  local feedback plus server-side authority; their behavior must remain
  equivalent.

## Evidence obligations

- One current direct `FIXTURE_V3` written through `latest`; it decodes to the
  exact payload and re-archives byte-identically. Pin the sole tag to `[0]`.
  No V1/V2 fixture, legacy decode, re-archive, or migration test remains.
- Parser properties for exactly-one driver, VM HTTP/TCP acceptance, and every
  multi-role permutation selecting Startup before Readiness before Liveness,
  then the lowest vector position; assert the exact role `section`, message
  index, and GH #280 diagnostic.
- Direct-API admission properties proving the same total order and exact
  `field`/message-index localization before intent construction, including
  caller-supplied `ProbeDescriptor.idx` values that disagree with vector
  position.
- Both driver arms round-trip through submit, persistence, describe, and the
  existing allocation projection without being collapsed to Exec.
- Built-default-feature operator evidence for accepted VM HTTP/TCP and rejected
  VM Exec; host-process Exec probe behavior remains unchanged.

## Changed-assumption provenance

The user explicitly states that this is a greenfield project and nobody uses
the old persisted ServiceSpec versions. That supersedes the temporary
boxed-V3 amendment, whose only purpose was preserving archived V1/V2 bytes
after rkyv root-layout growth. The change is deliberately narrow: it retires
only the obsolete ServiceSpec compatibility contract and does not modernize
other persistence formats or change the live `ServiceV2` intent model.

## Delivery handoff

Step 01-01 remains one atomic ingress cut. Its original crafter must audit the
existing exploratory diff and implement the exact one-arm direct envelope above:
remove the V1/V2 types, arms, exports, conversions, fixtures, decode,
re-archive, and migration tests; retain one direct current V3 fixture; and add
no decoder, accessor, alternate format, or roadmap edit. Then run the
already-listed Lima schema, acceptance, and workspace compilation checks. No
other VM Service behavior is reopened.

## References

- `docs/feature/service-kind-vm-workloads/feature-delta.md`
- `docs/feature/service-kind-vm-workloads/design/wave-decisions.md`
- ADR-0048, ADR-0051, ADR-0057, ADR-0058, ADR-0064, ADR-0083
- GH #280 — optional in-guest Exec health probes
