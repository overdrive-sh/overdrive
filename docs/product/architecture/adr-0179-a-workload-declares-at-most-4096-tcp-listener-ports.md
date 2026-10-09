# ADR-0179 — A workload declares at most 4,096 TCP listener ports

## Status

**Accepted — decision INTAKE-PORTS approved under the user's 2026-10-09
remediation directive**, a consequence of ADR-0178; the value is surfaced to
the user for confirmation. GH #295. Recorded in the #295 feature delta,
§ *[REF] vsock Attachment Replacement DESIGN* (§ *Core vocabulary*; § *Spike
remediation decisions — 2026-10-09*).

## Context

Under ADR-0178 the intake listener of each declared TCP port binds its own
port of one fixed host block on the workload's address. A guest may listen on
every declared port at once, so a workload cannot declare more TCP listener
ports than the block holds; a spec may declare up to 64,510 ports today.

## Decision

- **Deploy bound.** A workload spec declares at most 4,096 distinct TCP
  listener ports; more is refused at deploy at both ingresses, before any
  intent write.
- **Value.** 4,096 is the block's size (20,480–24,575): the block sits below
  the kernel's default ephemeral range (32,768–60,999) and holds 256 times
  the average of 16 declared ports per attachment that ADR-0176 sizes the
  node for.

## Alternatives considered

- **No bound.** A workload's ports beyond the block's size could never be
  served. Rejected.
- **A larger block.** It would reach into the ports host services commonly
  use or into the ephemeral range. Rejected for now; the value is the user's
  to confirm.

## Consequences

- Every declared port of an admitted workload can hold a steered intake
  listener.
- A residual difference from today's behaviour (D15): workloads with more
  than 4,096 TCP listener ports are refused.
- The node-wide capacity (ADR-0176, 262,144) is unchanged.
