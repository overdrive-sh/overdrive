# STEP 13-01 post-FIN reset observation replacement design

## Status

**APPROVED by independent DESIGN review iteration 2 under the standing
no-outcome-change authority. DISTILL alignment and the existing-cell native
validation remain required before STEP 13-01 resumes.**

This is a bounded evidence-contract correction for DG-10 reading 2 and D20's
post-FIN leg-F remote-socket proof. It changes no production mechanism,
component boundary, public or cross-crate API, typed project error, ownership,
lifecycle, wire/persisted format, configuration, CLI, model, lane, or scenario.
The accepted reset outcome is unchanged.

## Decision PFRO-1

When a claimed `ForwardedOutbound` registration is dropped after the loopback
leg's guest-to-remote direction has ended with FIN while the reverse direction
remains open, leg F must abort both sockets. For the remote leg B, the exact
observable is:

1. FIN is emitted on leg B after the last accepted guest-to-remote byte.
2. Dropping the claimed registration synchronously removes it and cancels its
   state. A separate registration-owned task takes any retained handle and
   awaits the existing `MtlsEnforcement::teardown`; the guest-flow owner does
   not join that task and gains no new completion guarantee.
3. That existing asynchronous teardown makes the same leg-B four-tuple emit
   RST after the FIN, within the cell's existing observation deadline.
4. On Linux 7.0 the peer endpoint that consumed FIN transitions from
   `TCP_CLOSE_WAIT` to `TCP_CLOSE` and retains `SO_ERROR=EPIPE` until consumed.
5. A subsequent `read` after EOF is explicitly not evidence for or against the
   reset, because Linux checks `SOCK_DONE` before `sk_err` and may return EOF
   forever on that descriptor.

The existing `SO_LINGER{1,0}` mechanism is retained. Clean close still means
both loopback directions ended with FIN while the registration was held; only
that case clears leg B's linger and emits FIN without RST. An open or
half-ended loopback leg keeps linger set and must emit RST. No fallback to
FIN-only behavior is permitted.

## Exact validation contract

The existing STEP 13-01 cell remains the one scenario and lane. DISTILL must
realign its observation without changing its Given/When/outcome:

- Given a claimed flow whose guest-to-remote direction has produced FIN and
  whose reverse direction remains open;
- When the owner drops the claimed registration;
- Then the leg-F-side forwarder endpoint is reset, and the remote leg-B
  endpoint observes the same-flow RST after its FIN through either the external
  packet boundary or the peer's terminal TCP state plus pending `SO_ERROR`;
- And the test must not use a second `read` after EOF as the reset oracle.

The proof must distinguish all three existing cases in the same Tier-3 suite:

| Loopback state at registration drop | Required leg-B observation |
|---|---|
| Both directions open | RST before any FIN; peer socket error, never EOF-only |
| Guest-to-remote FIN consumed, reverse direction open | Same-flow FIN then RST; peer `TCP_CLOSE` with pending `EPIPE` on Linux 7.0, or external same-flow packet evidence |
| Both directions ended cleanly | FIN and no later RST; no pending socket error |

Test-support representation remains DISTILL-owned. DESIGN does not add a
production accessor, packet-capture port, runtime hook, public enum variant, or
test-only capability. The existing external Tier-3 boundary may inspect the
peer socket and loopback wire directly.

## Interface and typed-error audit

PFRO-1 introduces no new production operation or failure condition. The exact
public/cross-crate API remains:

- `ForwardedOutbound` remains the owner-held move-only registration token;
- `MtlsEnforcement::teardown(handle)` remains the awaited effect-completion
  boundary;
- `MtlsEnforcementError` gains no variant;
- `register_forwarded_outbound`, `OwnedPair`, and the owner end-record dispatch
  retain their current signatures and meanings.

There is therefore no new semantic error mapping to invent. `EPIPE` is the
Linux peer endpoint's typed socket error for RST received in `TCP_CLOSE_WAIT`;
it is test evidence, not a new Overdrive error envelope or operator-facing
failure.

## Ownership, lifecycle, and contract shape

The architecture remains ports and adapters. `HostMtlsEnforcement` owns leg F,
leg B, both pump tasks, and their teardown. The guest-flow owner owns the
`ForwardedOutbound` token through `OwnedPair`; removing the pair drops that
token and requests the enforcement teardown. No new owner, background task,
recovery protocol, persistence boundary, or consistency mechanism is added.

**Contract shape: bounded-change.** The production mutation universe remains
the two already-owned connection endpoints and existing registration entry.
The verification universe is the peer TCP endpoint plus the two same-flow
packets that distinguish FIN and RST. The unchanged complements are: no new
registration, no re-resolution, no cleartext pass-through, no extra flow,
socket, task, config, route, nft element, cgroup, module, or durable row.

No C4 diagram changes: containers, components, ports, adapters, arrows and
trust boundaries are unchanged.

## Reopened alternatives

| Alternative | Native result and disposition |
|---|---|
| `TCP_LINGER2=-1` in addition to existing linger | Peer reaches `TCP_CLOSE`/`EPIPE`, but post-FIN `recv` still returns EOF. It adds a socket option without changing the outcome or fixing the invalid oracle. **Rejected.** |
| `AF_UNSPEC` disconnect before close | kTLS `tls_disconnect` unconditionally returns `EOPNOTSUPP`; it does not reach TCP disconnect. **Rejected.** |
| `TCP_REPAIR=1` before close | Peer remains `TCP_CLOSE_WAIT` with no error because TCP repair explicitly suppresses active reset. **Rejected.** |
| Exact-cookie `NETLINK_SOCK_DIAG/SOCK_DESTROY` | Produces the same peer `TCP_CLOSE`/`EPIPE` while post-FIN `recv` remains EOF. It adds a privileged control path and failure surface for an effect the existing close already provides. **Rejected.** |
| Withhold the first FIN until both loopback directions end | Changes accepted half-close propagation and the already-observed order. **Rejected; would require explicit outcome-weakening approval.** |
| Accept EOF-only after drop | Removes the required reset. **Forbidden without explicit user approval; not proposed.** |
| Observe the existing RST through wire/endpoint state | Matches the unchanged production effect and Linux endpoint semantics with no production change. **Selected.** |

## Quality attributes and bounded fallout

- **Correctness:** proves the required same-flow RST rather than inferring it
  from an API call that cannot expose it after EOF.
- **Security/fail-closed:** unchanged; a half-ended claimed flow is still
  aborted, never converted to a clean close or cleartext path.
- **Reliability:** no new privileged runtime dependency, task, retry, or
  best-effort cleanup.
- **Performance:** no production instruction, syscall, allocation, or packet
  is added.
- **Maintainability:** the validation names the Linux 7.0 endpoint semantics
  that future test authors must observe.

The bounded caller fallout is documentary and DISTILL-only:

1. fold PFRO-1 into DG-10 reading 2 and the affected roadmap validation;
2. realign the existing half-ended cell's oracle within the existing file and
   lane, without adding a scenario;
3. rerun the existing STEP 13-01 triple and full 12-cell mesh suite on Metal;
4. resume the original crafter's GREEN phase only after DISTILL alignment
   receives its own independent review.

No production implementation change is authorized by PFRO-1 beyond the
already-designed D20 `SO_LINGER{1,0}` behavior that the native wire capture
proves works.

## Changed assumption

Original assumption in `docs/feature/netns-density-295/feature-delta.md`, DG-10
reading 2:

> `SO_LINGER{1,0}`: the remote peer sees a reset

The outcome remains. The replaced evidence assumption is: “after that peer has
already consumed FIN, a later `read` returns a reset error.” The corrected
assumption is: the same-flow RST and the peer's `TCP_CLOSE` plus pending
`SO_ERROR=EPIPE` establish the reset; post-EOF `read` is not an oracle on Linux
7.0.

## Standing authority and handoff boundary

The user's standing ruling is: “Stop only for a gap that changes the outcome;
for anything else, take the design's evident intent and report it after the
step.” The corresponding working rule escalates only decisions that change an
outcome; otherwise the independently reviewer-agreed recommendation governs.

PFRO-1 changes no outcome: native evidence proves the unchanged production
reset, and the correction replaces only an observation that Linux cannot
produce after EOF. Independent review iteration 2 found no weakening and no
new mechanism, API, error, owner, body, scenario, lane, or model. PFRO-1
therefore requires no separate user approval.

The standing authority covers only the canonical evidence-contract fold and
existing-cell DISTILL alignment. It does not authorize `TCP_LINGER2`,
`TCP_REPAIR`, `SOCK_DESTROY`, a new runtime probe, a public/test-support API, a
new scenario/lane/model, or any weakening of DG-10/D20. Roadmap validation
remains pending until the independently reviewed DISTILL correction and exact
existing-cell native validation pass; this DESIGN approval does not
self-approve that execution gate.
