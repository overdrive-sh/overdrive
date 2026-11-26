# ADR-0358 — Upstream resolvers are read from the host resolver file on each forwarded query; only the gateway is excluded; an unusable source is SERVFAIL, never a boot refusal

## Status

**Accepted** (GH #382, DESIGN decision GPD-3, option A; approved by the user 2026-11-25, including ruling B: no boot check of the resolver file).
Interface contract: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts* (upstream ports). Evidence: `docs/research/dataplane/dns-forwarder-upstream-config-failure-surfacing-research.md`.

## Context

D-1 says "the host's upstream resolvers". The appliance image's resolver configuration is not specified by any ADR (ADR-0068 is silent); the metal test host gets its resolver from dhcpcd. DHCP can finish after `serve` starts, so the set at boot may differ from the set a minute later. The responder is wildcard-bound on `:53` and the gateway is host-local (ADR-0152), so a nameserver entry equal to the gateway would forward to the responder itself and loop; every other host-local entry terminates, because the gateway-address gate (ADR-0356) drops the datagram and the attempt times out inside the budget.

Forwarders that read their source per use or poll it do not refuse to start on a bad file; they answer the client with an error and warn once per failure episode. Forwarders that read once validate at start. Whether `serve`'s startup checks are needed at all on the appliance OS is tracked by [#376](https://github.com/overdrive-sh/overdrive/issues/376).

## Decision

- The source is the host's resolver file. It is read on every forwarded query, with no memo, through a port with a host adapter and a scripted sim adapter.
- Selection is pure: the gateway address, in its IPv4 and IPv4-mapped IPv6 forms, is the only excluded entry. Order is the file's order; port 53.
- A missing file, an unreadable file, an empty list or no usable entry gives SERVFAIL for that query, with a distinct cause for each. Nothing is checked at boot and nothing refuses boot, because the network may legitimately arrive later and the source is read per use.
- A failure is warned once when an episode starts and once when it clears, not once per failed query; per-query outcomes are counts. A node health condition readable through the CLI is tracked by [#386](https://github.com/overdrive-sh/overdrive/issues/386) and is not designed here.
- No `serve` configuration key is added. The host resolver file is the source until upstreams can be set in the worker node configuration, tracked by [#384](https://github.com/overdrive-sh/overdrive/issues/384) (depends on #327).
- Image requirement on #259: the appliance image writes a usable resolver file (at least one non-loopback nameserver) before guest traffic starts.

## Alternatives considered

- **Read once at boot, and refuse boot on an unreadable file.** Stale after a late DHCP lease (the node forwards nowhere until restart) and refuses a node whose only fault is a file that may be fixed a moment later; the read-per-use family does not refuse. Rejected.
- **Watch the file and cache the set.** Adds a watcher and shared state to save a microsecond-scale read per query. Rejected.
- **A `serve` configuration list of upstreams, alone or overriding the file.** Adds a config surface with no grounded failure while the image provides a resolver file (the #259 requirement). Not chosen; the planned direction is #384.
- **Detect and reject every host-local address, or drop loopback, unspecified and multicast entries.** Needs interface enumeration or a list for failures that already terminate by the gate and the budget. Rejected as more than correct.

## Consequences

- Positive: follows the host's own network configuration with no operator step; always fresh; no boot-time failure mode tied to the file.
- Negative: dependent on the image providing the file (the #259 requirement); a node without it answers every public lookup with SERVFAIL (definite, within budget) and shows it only through the episode warn until #386. No node-wide override exists (limitation L-3, tracked by #384).
