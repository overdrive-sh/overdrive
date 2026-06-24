# SPIKE Decisions — dial-by-name-responder

Slice 00 (US-DBN-1, BLOCKING). PROBE executed for real under Lima as root;
evidence in `findings.md` (real pasted `uname -r` / `ss` / `dig` / `getent` /
responder-log output, verified different-fox before this decision).

## Assumption Tested

Can ONE host-side (root-netns) in-agent listener receive and answer DNS queries
sent to N **different** per-netns gateway addresses, on a real kernel? (The
load-bearing, no-Tier-2-backstop routing assumption under D-TME-11's "one
source, three readers" — specifically the name-answering reader.)

## Probe Verdict

**WORKS.** A single root-netns process bound to ONE `0.0.0.0:53` wildcard socket
(`SO_REUSEADDR` + `IP_PKTINFO`) received and correctly answered DNS to BOTH
per-netns gateways (`10.99.0.1`, `10.99.0.5`), validated through BOTH the
explicit `dig @gw` path AND the real `getaddrinfo`/`getent` stub-resolver path,
from BOTH netns. Replies source-pinned to the queried gateway via
`IP_PKTINFO` `ipi_spec_dst`. Kernel: `7.0.0-22-generic` (dev Lima).

## Promotion Decision

**PROMOTE → DESIGN** (user, 2026-06-24). NOT an immediate spike-built walking
skeleton.

**Rationale:** the mechanism is validated decisively, so the feature proceeds —
but the responder's *production API surface* (its type/signatures, how it reads
the shared `ServiceBackendsResolve` index, how it composes into `run_server`, the
bind/teardown lifecycle) is a **DESIGN** decision the feature-delta gate
explicitly blocks on. Hand-building a walking skeleton in the spike would invent
that surface, which `CLAUDE.md` § "Implement to the design — never invent API
surface" forbids. The walking skeleton is **Slice 01, built in DELIVER per the
DESIGN output** — not promoted from the throwaway probe.

The probe is preserved (not deleted) under gitignored `spike-scratch/increment-a/`
as evidence, per `.claude/rules/spike.md` ("preserve prior increments").

## Walking Skeleton

Deferred to **Slice 01 (DELIVER)**, designed by DESIGN. Not built in the spike
(see Promotion Decision rationale).

## Design Implications (DESIGN MUST account for these)

1. **`IP_PKTINFO` source-pinning is MANDATORY, not optional.** A multi-homed
   `0.0.0.0:53` socket must reply with `ipi_spec_dst` = the gateway the query
   targeted (captured from the recv `IP_PKTINFO`). Without it, `getaddrinfo`/
   glibc rejects the reply (source ≠ queried server) and resolution fails
   silently. `dig +short @gw` is lenient and MASKS this — so **Slice 01's
   acceptance test MUST assert the `getent`/`getaddrinfo` path, not just
   `dig @gw`.**
2. **One wildcard `0.0.0.0:53` socket is sufficient on this node config**
   (systemd-resolved binds `127.0.0.53/54:53` as *specific* addresses; an
   `SO_REUSEADDR` wildcard coexists). **Keep a try-wildcard-then-fall-back-to-
   per-gateway-addr-sockets shape** as cheap insurance against an appliance image
   that ever holds a wildcard `:53` — OR confirm the appliance image has no
   wildcard `:53` holder and commit to wildcard-only. (The probe implements the
   fallback; it did not fire.)
3. **The responder runs in the ROOT netns** and answers on each per-netns
   gateway addr (= `plan.responder_addr` = `plan.host_addr`). No per-netns
   listener, no netns-entering. Confirmed reachable by construction (the in-netns
   default route + `ip_forward=1`).
4. **`ip_forward=1` is a prerequisite** for the in-netns→root-netns query path
   (already modeled as a converge-on-boot `EnableIpForward` step).
5. **DNS answer shapes:** `AAAA`-for-an-A-only-name → NODATA (`NOERROR`, 0
   answers) was proven. The `NXDOMAIN` + SOA-negative-TTL shape (a name with no
   running backend) was **NOT** exercised by the probe — it is a Slice 01 build
   concern. (Validates the pinned v1 DNS contract: A → running IPv4; AAAA →
   NODATA; 0 running backends → NXDOMAIN.)

## Constraints Discovered

- Verdict pinned to dev-Lima `7.0.0-22-generic`, NOT the 6.18 appliance pin
  (ADR-0068). The surfaces exercised (`IP_PKTINFO`, multi-homed UDP, per-netns
  `resolv.conf` bind-mount, `SO_REUSEADDR` wildcard coexistence) are long-stable
  (well pre-6.18), so the verdict is expected to hold — **re-confirm on the
  appliance kernel in the DELIVER Tier-3 matrix.**
- The acceptance SIGNAL for the name path is `getaddrinfo`/`getent`, never
  `dig @gw` alone (see Design Implication 1).
