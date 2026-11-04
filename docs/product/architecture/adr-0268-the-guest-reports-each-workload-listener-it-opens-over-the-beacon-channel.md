# ADR-0268: The guest reports each workload listener it opens, over the beacon channel

## Status

User-approved, 2026-11-03; the rulings PR-1 (read seam) and PR-2 (hook scope; V42 held,
[findings](../../feature/service-allocation-readiness-and-ending/spike/findings-workload-cgroup.md))
are recorded in the feature delta's approval record, as are A1 (the board's entry lives from
`open` to `close`; the frozen-at-end-of-wait rule is the host reader's) and A3 (a host-input
conformance projection of the report model), both accepted 2026-11-04; not yet Accepted (remaining gates:
model check, design review). Not implemented. GH #318, GH #334. Contract
and status: [feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md) (D13).
Related: [ADR-0272](adr-0272-the-guest-observes-workload-listeners-with-kernel-hooks-not-a-poll.md)
(what produces the report), [ADR-0269](adr-0269-a-service-with-no-declared-startup-probe-waits-on-a-guest-listener-startup-probe.md)
(who consumes it), [ADR-0256](adr-0256-the-shim-registers-probe-supervision-before-it-releases-the-exit-event-gate.md)
(registration before `EXEC`), ADR-0206 (guest kit), ADR-0217/0218 (guest vsock ports).

## Context

The host learns nothing about the workload's sockets from the guest today. The beacon
channel (guest dials the host over vsock, default port 1234) carries `READY`, `EXEC`,
`EXIT` and `SHUTDOWN`; its grammar is closed (an unknown verb or a wrong field count is a
parse error, `vm/beacon.rs`). `overdrive-init` sends `READY` before the workload exists and
sends nothing else until `EXIT`. The host reader forwards every guest line into a bounded
`mpsc::channel(1)` that is drained only after the VMM exits (`vm_driver.rs:755-835`,
`:580`, `:2171`, `:2208-2222`), so a mid-life line would block the reader and be taken as
the exit report. The other guest-to-host channels are flow-shaped (stream 1240, control
1243); none carries a lifecycle fact.

## Decision

**The guest sends one beacon line per distinct `(protocol, port)` the workload opens
(`LISTEN <tcp|udp> <port>`); the host keeps the allocation's set of reports for one VM
incarnation.**

- The line is guest-to-host, sent after `EXEC` and so after the `Running` row and the
  supervisor's registration (ADR-0256). It is idempotent: a duplicate is a no-op, and the
  host does not store `(protocol, port)` pairs the Service does not declare.
- The host's beacon reader classifies lines: `LISTEN` goes to the set, `EXIT` to the exit
  report as today. A `LISTEN` can never occupy the exit slot.
- The guest has one ordered beacon writer and drains pending `LISTEN` lines before it
  sends `EXIT`.
- There is no close report. The set belongs to one VM incarnation; a restart is a fresh
  allocation id with an empty set (ADR-0105), and a guest reset ends the VMM.
- A failed send is a beacon failure, handled as a failed `EXIT` send is today (never
  silently dropped).

## Alternatives considered

- **The control channel (port 1243, 8-byte `ControlMessage`).** It is the per-VM
  flow-pairing session (`Paired`, `Refused`, `Abort`, `TimedOut`), not a lifecycle channel;
  a listener fact would be a foreign message in a flow protocol.
- **A new vsock port.** A third node-global listener port (ADR-0218), another config
  field in the seven-field cmdline token, and an extra per-VM connection for one message
  type, where the beacon already is the lifecycle channel and the host reader already
  exists.
- **Piggyback on `READY`.** The workload has not run at `READY`; the report is a later
  event.
- **The host dials the guest and asks.** That is the poll being removed.

## Consequences

- The beacon grammar gains its first mid-life guest message; host and guest ship as one
  guest kit (ADR-0206), so no version skew is designed for.
- The host reader's line handling changes (classification): only `EXIT` or EOF ends the
  guest-report wait, and a mid-life line cannot stall the stop or command path.
- Two unsynchronised guest writers corrupt the line stream, and a `LISTEN` written after
  `EXIT` is lost to the host; the single writer and the drain-before-`EXIT` rule are the
  protocol's two ordering rules (modelled in the feature delta's `guest_report` module).
- Delivery latency is a vsock hop: round-trip p99 138.8 us.
