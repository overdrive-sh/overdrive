# ADR-0126 — Converge one fixed node guest-bridge MAC

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records the I4-F01 bridge-MAC decision.

## Context

The TCX endpoint map rewrites intercepted Ethernet destinations to the node
guest bridge MAC. That value is classifier-critical: adopting whatever MAC the
kernel assigned at boot leaves no stable desired state, while changing a live
MAC requires every endpoint value to change in lockstep.

Guest MACs are deterministically derived with bytes `02:00` followed by the
guest IPv4. The bridge needs an equally deterministic but disjoint locally
administered unicast identity.

## Decision

Every node uses fixed bridge MAC `02:01:00:00:00:01`. The first octet is locally
administered and unicast; the second octet differs from the fixed `00` guest-MAC
namespace, proving collision freedom for every possible guest IPv4.

Boot reclamation removes stale TAP ports before the shared-switch owner adopts
only a bridge-kind link, brings it down, converges the fixed MAC and
gateway/prefix, reads back its complete identity, and brings it up. No endpoint
map entry or TAP attachment occurs first. Wrong postconditions refuse startup
through the typed guest-network mismatch error.

Runtime audit reads the bridge MAC and every live endpoint value. Any mismatch
closes EXEC and quiesces managed TAPs before reconverging the fixed MAC and
rewriting mismatched endpoint values. TAPs resume only after full-set read-back;
failure follows ADR-0124's bounded retry/fail-stop policy.

The exact constant home and structured expected/observed fact shapes live only
in the feature delta.

## Alternatives considered

### Adopt the kernel's live bridge MAC

Rejected. It turns ambient state into desired state and makes restart identity
dependent on creation order or host configuration.

### Adopt a changed MAC and rewrite all endpoints fleet-wide

Rejected. #295 is node-local, and a live-adoption protocol adds an unnecessary
multi-object transition and cross-host implication where one fixed local value
suffices.

## Consequences

Positive: bridge identity, classifier rewrite, probes, and runtime audit share
one deterministic value with structural guest-MAC disjointness. Negative: all
nodes intentionally use the same bridge MAC; this is safe only because the
bridge is node-local and cross-host guest L2 is outside #295.
