# ADR-0152 — `workload_addr` is made host-local by one shared `local` route, which boot writes only behind attached steering and which is never removed

## Status

**Proposed — approved by user (D8 2026-10-05; route convergence, D8a-ROUTE
and U-4 2026-10-06, current wording 2026-10-08; the node-address check
D8-PREFIX-CFG 2026-10-08); pending independent DESIGN review.** GH #295.
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN — PROPOSED 2026-10-05*.

Related decisions, each its own ADR: which connections reach the prefix
(ADR-0172), the steering mechanism (ADR-0171) and how boot replaces its
program (ADR-0175), intake listeners and host-initiated flows (ADR-0173), and
the client address the guest sees (ADR-0174).

## Context

`workload_addr` is the persisted, observed backend and probe address. Its
consumers are the backend discovery bridge, cgroup service delivery
(ADR-0053), probes (ADR-0094), leg-S plaintext delivery (ADR-0120) and the
intercept rules of ADR-0125. Under ADR-0145 the guest has no NIC; the guest
still carries `workload_addr` on a dummy device (ADR-0150).

To terminate host-local connections to `workload_addr:port` on the host, the
guest prefix must be local to the host. A `local` route makes every address
of the prefix local, and the kernel then delivers a connection to whatever
socket matches it unless a lookup decision intervenes (ADR-0171, ADR-0172).

A `local` route is a kernel object that outlives the process that installed
it: after a SIGKILL of `overdrive serve` it is still present at the next boot.

When an address is assigned to an interface, the kernel adds a `local` route
for it (protocol `kernel`). A node address inside the guest prefix — set by
the node's network configuration, which the operator owns — would therefore
put an untagged route inside the prefix and make that address both a node
address and a possible `workload_addr`. On the appliance nothing else writes
routes (ADR-0068).

## Decision

1. **One shared route.** The guest prefix is host-local through one shared
   `local` route on `lo` whose preferred source is the node's guest gateway
   address (ADR-0174), tagged with Overdrive's route protocol identifier.
   There is no per-allocation route or netdevice.
2. **The prefix holds no node address.** Before any boot effect, startup
   refuses with a typed configuration error when an address configured on a
   node interface lies inside the guest prefix, naming the address and the
   interface.
3. **Written at boot, only behind attached steering.** Boot keeps an
   identical tagged route, replaces a tagged route for the prefix whose
   attributes differ, or adds a missing one — and does so only after this
   boot's steering program is loaded and attached (ADR-0175). If the steering
   cannot be loaded or attached, boot refuses startup and does not write the
   route: a `local` route left by an earlier boot then stands only over that
   boot's steering, and before the first successful boot there is no route.
   The route is `local` only while a pinned steering link exists. Nothing
   writes the route at runtime; a crash at any boot step leaves the earlier
   state or the new one, and the next boot converges it.
4. **Never removed.** `serve` never removes the route at shutdown, graceful or
   not. It persists across restarts; the pinned steering keeps the prefix
   fail-closed while `serve` is down (ADR-0171).

## Alternatives considered

- **No `local` prefix route; intake listeners bind with `IP_FREEBIND` and
  inbound reaches them by TPROXY only.** Leg-S and probes are exempt from
  TPROXY by design, so they could not reach an intake without changing the
  meaning of ADR-0125's rules 1 and 6. Rejected.
- **Per-allocation `/32` local routes added at activation.** One route write
  per allocation, each needing its own convergence, with the same need for a
  lookup decision. Rejected.
- **Converge the route at boot before the steering is attached.** On a first
  boot whose steering convergence fails — a program the verifier rejects, a
  refused attach, or a crash part-way — the prefix would be locally
  delivered with no lookup decision. Rejected.
- **Remove the route when the steering cannot be converged.** The prefix is
  then not local: host-local connects and arriving packets follow the default
  route and leave the host instead of being refused. Rejected.
- **Observe the routes covering the prefix and refuse an untagged one.** The
  only producer of an untagged route inside the prefix on the appliance is a
  node address configured inside it; checking the configuration names the
  cause and the fix directly, before any effect. Rejected in favour of
  point 2.
- **Audit and repair the route at runtime.** On the appliance nothing but
  boot writes it; a runtime audit could detect only a change made by software
  that does not exist on the appliance. Boot convergence covers Overdrive's
  own crashes. Rejected.
- **Remove the route at graceful shutdown.** A crash skips that step anyway,
  so boot convergence must already handle a left-behind route; removing it
  makes graceful and crash restarts differ for no gain. Rejected.
- **Address backends by CID instead of `workload_addr`.** Ripples through
  persisted rows, discovery, probes and dataplane maps. Rejected.

## Consequences

- Observation rows, backend identity and probe targets are unchanged.
- A node whose network configuration places an address inside the guest
  prefix does not start; the refusal names the address and interface.
- The prefix is never locally deliverable without a steering program
  deciding every lookup, at any boot step, after any crash, and while `serve`
  is down.
- Only Overdrive writes the route on the appliance; the image (ADR-0068)
  carries no network manager or daemon that removes routes it did not create.
  That is an assumption of the design, discharged by the image, not by a
  runtime check.
- The boot order of route and steering is checked by the formal model of the
  guest-flow owner (ADR-0168, `specs/quint/guest-flow-owner/`, module
  `prefix_boot`).
