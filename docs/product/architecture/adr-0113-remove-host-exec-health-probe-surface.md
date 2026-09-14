# ADR-0113 — Remove the host Exec health-probe surface with the Exec workload driver

## Status

**User-approved on 2026-09-14; iteration-1 findings remediated; awaiting
independent DESIGN re-review.** This record is not implementation authority
until that review approves the complete #293 design bundle.

## Context

The current Exec health probe runs a host subprocess in the allocation's host
cgroup through `ExecProber`/`CgroupExecProber`. VM admission rejects that
mechanic before intent because the host subprocess says nothing about guest
health. Once GH #293 removes host-process workloads, no admitted workload has a
successful path to the host Exec probe.

GH #280 separately tracks a possible in-guest command probe. Its own issue says
that feature requires a guest protocol, correlation, cancellation, containment,
overload, and session-lifecycle design; the current host adapter is not that
boundary.

## Decision

Delete the host Exec probe mechanic with the removed workload driver. Retain
the existing HTTP and TCP probe contracts, roles, thresholds, observation rows,
and supervision.

Delete the Exec probe enum/error/parser arms, `ExecProber` port,
`CgroupExecProber` and sim adapter, `ProbeRunner` dependency/dispatch branches,
VM-specific rejection shim, documentation, and tests. An operator-supplied
`type = "exec"` becomes the existing unknown-probe-type result.

GH #280 remains open and independent. If a concrete VM workload later justifies
in-guest command probing, its DESIGN introduces the exact guest-owned API from
a clean boundary; #293 leaves no compatibility placeholder.

Exact signatures belong to the feature delta.

## Supersession scope

This ADR supersedes only the Exec-specific portions of
[ADR-0054](adr-0054-probe-runner-subsystem.md): `ProbeMechanic::Exec`, the
`ExecProber` port, `CgroupExecProber`/`SimExecProber`, the Exec dispatch and
error paths, and the three-port constructor/composition shape. ADR-0054's
HTTP/TCP probe contracts, per-allocation supervisor ownership, probe roles and
thresholds, `ProbeResultRow` publication, cancellation, and Earned-Trust TCP
probe remain authoritative.

This ADR supersedes [ADR-0059](adr-0059-exec-probe-cgroup-placement.md)'s host
Exec-probe placement, timeout, and cleanup decision in full. It does not
supersede shared workload cgroup management or VM confinement/accounting owned
by ADR-0026, ADR-0054's separate cgroup-port record, or the VM driver ADRs.

These are authority links for the approved deletion. Historical ADR prose is
not rewritten and no new probe mechanism is introduced.

## Alternatives considered

### Retain the rejected syntax and host adapter

Rejected in the proposal. It preserves an actionable GH #280 message but keeps
a production port/adapter with no supported successful workload, contrary to
the repository's deletion discipline.

### Reuse the host adapter for VM guests

Rejected. It executes on the host, not in the guest, and would report host
cgroup state as guest health while bypassing the control/session/containment
requirements GH #280 explicitly owns.

### Add a placeholder guest Exec port now

Rejected. No exact API has been approved and no motivating workload has met
GH #280's entry gate. A placeholder would invent public surface and pre-empt a
separate DESIGN.

## Consequences

Positive:

- No unreachable host-process probe subsystem remains after host-process
  workload execution is removed.
- `ProbeRunner` has only the two mechanics current VM Services can use.
- GH #280 starts from the correct guest boundary rather than inheriting host
  cgroup semantics.

Negative:

- The current role/index-localized “VM Exec is unsupported; see GH #280”
  diagnostic becomes the generic unknown-probe-type diagnostic.
- A later GH #280 implementation must add a new probe enum/port/protocol surface
  rather than activating a retained variant.

## Links

- [GH #293](https://github.com/overdrive-sh/overdrive/issues/293)
- [GH #280](https://github.com/overdrive-sh/overdrive/issues/280)
- [Exact feature contract](../../feature/remove-legacy-exec-workload-driver/feature-delta.md)
- [ADR-0054](adr-0054-probe-runner-subsystem.md)
- [ADR-0059](adr-0059-exec-probe-cgroup-placement.md)
