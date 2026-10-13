# ADR-0177 — A guest-opened host vsock connection is attributed by its peer CID and the launch token its first record carries

## Status

**Accepted — decisions ATTR-TOKEN and ATTR-LIVE-CLAIM approved by user
directive 2026-10-09; its dependence on RESET-EXIT (ADR-0182) approved by
user 2026-10-11.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN*. Depends on ADR-0157 (beacon), ADR-0166
(control session), ADR-0158 (flow requests), ADR-0156 (the lease's CID) and
ADR-0182 (a guest reset ends the VMM).

Amends on acceptance: ADR-0082 §D4's READY line (a `launch` field is added;
the message set and READY's meaning are unchanged).

## Context

Every guest-opened connection to the host — the beacon (port 1234), flow
connections (1240, 1241) and the control session (1243) — arrives on a
node-shared `AF_VSOCK` listener (ADR-0157, ADR-0166). Attributing each one
by the kernel-reported peer CID alone is safe only if the kernel resets a
dead VM's connections, accept-queue entries included, before the CID is
reused.

V-22 measured that condition on native metal (feature delta P-37,
`spike/findings-v22.md`, kernel 7.0.0-29). Established connections are reset
before reuse, but a connection still in a host accept queue when its VMM dies
is accepted later — after the CID's next VM holds it included — with the new
VM's CID as peer, and its first read returns the dead VM's buffered bytes
(4,000 of 4,000). A guest that wrote its control open, a flow request or
READY and then died leaves a record the host would act on for the next
allocation: a stale session holding the CID's session claim, a stale flow
resolved and connected under the new allocation's identity with its
`Paired` naming a flow id of the new launch, a stale READY releasing the new
VM's command.

The reset is visible at accept as a pending socket error, but the same flags
appear when a live guest aborts with a reset, and whether a poll at accept is
free of races was not measured.

## Decision

- **A launch token per lease.** Admission (`GuestAddressPool::assign`) draws
  a 16-byte, never-zero token from the injected entropy port when it assigns
  the lease. It identifies that launch: the lease, its CID and the one VMM
  that runs on it, which runs one guest boot (ADR-0182). It is not persisted
  and not a secret.
- **The guest learns it at boot** from its kernel command line, beside the
  workload address and the gateway.
- **Every guest-opened connection's first record carries it:** the control
  session's fixed-size opening record, every flow request, and READY's line.
- **The host attributes by peer CID and the CID's live launch token.** A
  CID has a *live launch token* only while its launch is live: for the
  guest-flow owner, while the CID is Provisioned or Active; for the VM
  driver, from the launch's start until its stop. Teardown (the move to
  Retiring) and the driver's stop end that liveness in the same per-CID step
  in which they close the launch's session and release its claim. Every
  token comparison is against the live launch token, and a CID with none
  matches no token.
- **The claim is taken in the comparison's step (ATTR-LIVE-CLAIM).** A
  session or beacon claim is taken by one per-CID serialized step after the
  first record has been read (the deadline-bounded read stays outside it),
  which requires, together, that the record's token equals the CID's live
  launch token and that the claim is free. A state check made earlier
  (at accept) only closes connections early; it never admits a claim. So a
  connection accepted while the launch was live and read after its teardown
  or stop is closed at its read, whatever its token, and cannot take a claim
  that outlives the launch.
- **Flows.** A flow is registered by the existing admission check
  (ADR-0158) and its registration is completed only after its request's
  token matches the live launch token, before policy, resolution, matching,
  install or a control message. A mismatch closes the connection, removes
  any registration, sends no message, has no effect, and is counted.
- **Claims are free before the CID is leased again.** The session and beacon
  claims are released when the allocation's driver stops and its network is
  torn down, both before the lease is released, and no later read can
  re-take them (above), so a new launch never meets the previous launch's
  claim.
- **One rule for every receiver.** Host-opened requests carry the token too,
  and the guest applies the same comparison.

The exact records, the newtype and the error and telemetry surface are pinned
in the feature delta (§ *Core vocabulary*, § *Wire contracts*, § *Control
session*, § *Beacon intake*).

## Alternatives considered

- **Treat a pending error or reset at accept as "stale".** It cannot tell a
  dead VM from a live guest that aborted with a reset, and its freedom from
  races was not measured; it infers a fact the token states exactly.
  Rejected.
- **A host-side generation counter per CID.** Unique only within one `serve`
  process, and it needs per-CID state that outlives each lease; a token drawn
  at assignment needs neither. Rejected.
- **The allocation id on the wire.** Variable length, and an identity the
  guest has no other use for. Rejected.
- **Keep attribution by peer CID and rely on the kernel.** Falsified by V-22.
  Rejected.
- **Check the CID's state at accept and compare the token at the later read
  against the lease's token.** The formal model finds the
  counterexample: a dead launch's queued control connection, accepted while
  the CID is still Provisioned and read after teardown (which found no
  session to close), matches the Retiring lease's token and takes the
  session claim; the claim survives the lease's release, the next lease
  activates on a dead session, and its own session is refused as held.
  Rejected; the claim is decided in the comparison's step.
- **Teardown also closes every accepted connection whose first record is
  unread.** Closes the same window, but only if teardown's set of in-flight
  reads is complete — a bookkeeping convention across two listeners and the
  VM driver, where the comparison against a token that teardown has already
  ended makes the stale claim unrepresentable. Rejected as the gate.

## Consequences

- No stale connection of a dead launch is attributed to the CID's next
  launch: it is closed at its first read (no buffered bytes) or at the token
  comparison (buffered bytes).
- The flow request is 32 bytes (its fields and the token), the control
  session opens with a 24-byte record, and READY's line carries a `launch`
  field. Only control records grow; no payload crosses userspace.
- The session and beacon claims are taken in the token comparison's step
  against a live launch token, so a stale queued connection can never hold a
  claim against the next launch, and no claim outlives the launch it names.
- A new VM's connect from a 4-tuple a stale entry still holds is refused until
  the host has accepted and closed that entry; the host's listeners accept
  continuously and the guest's session and beacon connects retry.
- The token is readable inside the guest; knowing its own launch's token
  gives a guest application nothing it could not already do from that VM.
- **The token is sound only with RESET-EXIT.** It reaches the guest on the
  kernel command line, which does not change when a guest reboots inside the
  same VMM; the fork reboots a reset guest in place unless it carries the
  change of ADR-0182 (V-30: same VMM pid, CID and token, new `boot_id`).
  ADR-0182 makes every guest reset end the VMM, so one token names one guest
  boot and the next launch draws a new one.
- The session and flow admission rules are covered by the feature delta's
  § *Formal protocol model*, which models the measured kernel behaviour of
  V-22.
