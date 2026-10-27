# STEP 13-01 DISTILL alignment

## Scope and authority

This alignment contains two bounded DISTILL-owned corrections exposed during
STEP 13-01: the observation contract of the existing
`dropping_a_claimed_registration_with_the_loopback_leg_half_ended_resets_the_remote_peer`
cell and its private peer recorder, plus the setup of three existing
shared-owner lifecycle bodies that bypassed D9 registration. Both corrections
retain their existing scenarios, bodies, outcomes, owners, lanes, and Contract
Shape declarations.

The authoritative contract is PFRO-1 in
`design-13-01-postfin-reset-observation.md`, independently approved through
iteration 3 of `review-design-13-01-postfin-reset-observation.md` and folded
into `feature-delta.md` DG-10 reading 2. Native evidence in
`native-13-01-postfin-reset-observation.md` proves that the unchanged
production leg-B four-tuple emits FIN and then RST. On Linux 7.0 the peer
socket that consumed FIN reaches `TCP_CLOSE` with pending `SO_ERROR=EPIPE`,
while subsequent reads remain EOF because `SOCK_DONE` is checked before
`sk_err`.

PFRO-1 changes no outcome. Its user-authorized boundary is the existing cell
and test-local observation support. The additional fixture correction follows
already-approved D9/D14 through the existing registration port. Across both,
no scenario, test body, lane, model, production implementation, public or
cross-crate API, production accessor, typed project error, owner, task,
lifecycle promise, wire format, persisted format, configuration, or CLI
surface is added.

## Existing-cell correction

The peer recorder still reads the production mTLS connection through the same
accepted socket and records the same request, response, raw TLS wire, FIN,
ownership, timing, and resolver-call evidence. Its terminal observation after
FIN now has an exact private representation:

```text
Fin { tcp_state, pending_error }
```

After the first `read` returns EOF and records `saw_fin`, the recorder performs
no further read on that peer socket. It polls `TCP_INFO.tcpi_state` on the
same descriptor. When that descriptor reaches `TCP_CLOSE`, it consumes
`SO_ERROR` exactly once and retains the returned value in the session record
for the cell assertion.

The half-ended cell now requires exactly:

```text
Fin { tcp_state: TCP_CLOSE, pending_error: EPIPE }
```

It does not accept EOF alone, `Open`, a default observation, any other
nonzero socket error, or a reset observed on another flow. The accepted peer socket is
the same session that completed the mTLS handshake, received the request,
returned the response, and recorded the FIN, so the terminal state/error
belongs to the exact existing leg-B flow.

## Preserved contrasting controls

| Existing case | Preserved exact evidence |
|---|---|
| Claimed registration dropped while both loopback directions remain open | `Termination::Reset` before any FIN, plus reset on leg F's forwarder side. |
| Claimed registration dropped after guest-to-remote FIN while the reverse direction remains open | Same peer session records FIN, then terminal `TCP_CLOSE` with pending `EPIPE`; leg F's forwarder side resets. |
| Both directions end cleanly while the registration is held | Terminal `TCP_CLOSE` with pending socket error `0`; dropping the already-finished registration leaves that record unchanged. |

The two clean-close assertions no longer infer “no reset” from repeated reads
after EOF. They use the same terminal state/error observation and require zero
pending socket error. The open-drop cell remains the positive pre-FIN reset
control. All three cases retain their current deadlines, exact registration
owner, registered tuple, mTLS exchange, and zero re-resolution assertions.

## Contract Shape and test boundary

The existing half-ended cell retains its exact
`/// CONTRACT_SHAPE: bounded-change.` declaration. Its mutation universe is
unchanged: the claimed registration and the two sockets already owned by the
production connection. The observation universe is the existing forwarder
socket plus the existing peer session's FIN, terminal TCP state, and pending
socket error. The unchanged complement remains every other registration,
flow, peer session, resolver call, route, nft object, cgroup, module, task,
configuration value, and durable row.

The private `TCP_INFO` / `SO_ERROR` helpers are DISTILL-owned Tier-3 support.
They inspect only the test peer's existing descriptor and do not expose a new
production seam. `SO_ERROR` is Linux endpoint evidence, not a new Overdrive
error or mapping.

## Handoff and verification boundary

This source/documentation alignment is ready for a fresh independent DISTILL
review. No DES event or commit is part of this work. Native execution remains
with the original STEP 13-01 crafter after source approval, using the existing
durable Metal route and unchanged test selection:

1. the open-drop, half-ended-drop, and clean-close triple; and
2. the complete existing 12-cell `vsock_mesh_egress_mtls` suite.

The validation must preserve the exact half-ended observation
`TCP_CLOSE`/`EPIPE`, the open-drop `Reset`, and both clean-close
`TCP_CLOSE`/zero-error controls. A green read-after-EOF result, EOF-only
result, new scenario, alternate lane, or production probe does not satisfy
PFRO-1.

## Same-step D9 fixture migration

The original crafter's full 1,332-test package gate exposed three additional
existing bodies whose setup bypassed the accepted D9 forwarded-outbound port.
Each body opened a source-bound socket directly to the published leg-F
listener after `start_alloc`, without first calling
`register_forwarded_outbound`. Production correctly rejected those
unregistered tuples in `MtlsInterceptWorker::handle_shared_outbound`; relaxing
that refusal would violate D9 and reintroduce D14's removed pass-through path.

The failures were fixture failures before the bodies reached their retained
lifecycle outcomes:

| Existing body | Preserved failure evidence | Retained purpose |
|---|---|---|
| `netns_density_shared_owner::owner_shutdown_waits_the_active_claim_then_drains_every_shared_capability_and_listener` | Metal run `e623caae0d03d4aa5ae30cad`, exit 100 | Owner shutdown closes admission, waits for the in-flight claim, drains all handles/capabilities, closes both listeners, and preserves the exact owner surface. |
| `outbound_enforce_substrate_splice::real_owner_shutdown_closes_admission_waits_one_claim_and_drains_every_shared_handle` | Metal run `785ec2db92073e43085fc6c8`, exit 100; early EOF at the first exchange | Real-kernel owner shutdown waits for the enforcement-held third claim and drains the exact shared handles/listeners. |
| `outbound_enforce_substrate_splice::two_real_shared_capabilities_keep_the_unrelated_tls_handle_live_after_one_stops` | Same Metal run; early EOF at the first exchange | Stopping one real shared capability preserves the unrelated allocation's TLS handle, traffic, listeners, node program, and dynamic-member complement. |

All three bodies are retained. Their sole purpose is shared-owner/capability
lifecycle coverage, not the deleted per-allocation egress TPROXY or cleartext
pass-through behavior. The obsolete D14 substrate scenario and its strace/wire
helpers were already deleted from
`outbound_enforce_substrate_splice.rs`; these bodies exercise the replacement
forwarded-outbound composition and therefore must migrate rather than be
removed.

### Exact fixture correction

Each affected client setup now follows the existing production order:

1. bind its existing source-specific `TcpSocket` to obtain the exact
   `forwarder_local` four-tuple;
2. call the existing `MtlsInterceptWorker::register_forwarded_outbound` for
   that allocation, direct leg-F destination, and the body’s already-selected
   mesh backend;
3. retain the returned move-only `ForwardedOutbound` for the flow lifetime;
4. connect the same bound socket to the existing leg-F listener; and
5. run the unchanged exchange, stop, shutdown, drain, listener, node-program,
   dynamic-member, and unrelated-handle assertions.

The real-kernel bodies pass the same backend address to `AllMeshResolve` and
the forwarded registration; leg F therefore consumes the owner's one selected
backend and does not re-resolve or invent another route. The recording-owner
body likewise centralizes its pre-existing loopback backend value so the
fixture's resolver and registration cannot diverge.

No production source or interface changed. The two private helper functions
are test-local fixture composition over the already-approved public port. No
scenario, test body, body count, Contract Shape declaration, lane, model,
timeout, assertion, production seam, or compatibility path was added, removed,
or weakened.

### Expanded review and replay boundary

The fresh independent DISTILL review must include the two fixture-source files
and verify that every changed connection is registered before connect while
all retained assertions remain byte-for-byte unchanged. After approval, the
original crafter's durable Metal replay adds these exact three existing bodies
to the PFRO-1 triple/full-suite and package gates. An exclusion, ignored body,
deleted lifecycle cell, unregistered-tuple allowance, or production fallback
does not satisfy D9/D14.

Source-package preparation ran scoped `rustfmt --check` and
`git diff --check`; both passed. A zero-context diff scan found no changed test
attribute, Contract Shape declaration, or assertion in either fixture file.
No native test, Metal replay, DES event, or commit was performed before the
independent DISTILL review.

## Shared-peer abort observation correction

The reviewed registration migration's first focused replay reached all three
retained lifecycle paths. The recording-enforcement shared-owner body passed.
Both real-kernel `outbound_enforce_substrate_splice` bodies reached their
registered production flows and failed only when `SharedPeerOwner::finish`
joined peer sessions whose `shared_peer_connection` returned `false` for the
expected abortive teardown:

- durable Metal run `ad70ecaabd63f14b53bc6012`;
- nextest run `895a9f95-aa4a-4ca1-b9bd-6f844ecc5868`;
- log `.context/netns-density-295/13-01/focused3-postdistill.log`;
- result: one of the exact three bodies passed; the two real-kernel bodies
  failed only at `outbound_enforce_substrate_splice.rs`'s peer-owner join.

This is a second test-local observation mismatch, not production failure.
`HostMtlsEnforcement::teardown` marks the connection for reset and the existing
reclaim path installs `SO_LINGER{1,0}` before closing its owned legs. D9/DG-10
therefore require the remote mesh peer to observe an abortive reset when an
allocation stop or owner shutdown drains these still-open flows.
The production path is `crates/overdrive-dataplane/src/mtls/mod.rs:320-353`
and `:428-446`; outbound leg B is armed at
`crates/overdrive-dataplane/src/mtls/outbound.rs:107-111`.

Because that first receipt collapsed every non-EOF/non-timeout read error to
`false`, DISTILL first added a bounded failing diagnostic without changing its
acceptance. The exact two-body replay retained at
`.context/netns-density-295/13-01/focused2-peer-error-diagnostic.log` recorded:

- durable Metal run `d612c668bf4e92d7fa10006e`;
- nextest run `f74b2eeb-c710-49f9-82ca-d30e47448a88`;
- three `ConnectionReset` / raw errno 104 observations in the owner-shutdown
  body; and
- one `ConnectionReset` / raw errno 104 observation for the first allocation
  stopped by the independent-capability body.

The independent-capability body's second client is explicitly dropped before
the body stops its second allocation. That peer's final cleanup is not an
abort assertion and produced no diagnostic error; it had already completed
the existing post-first-stop byte-exact exchange. The private expectation is
therefore explicit per connection instead of globally weakening every peer:

- the mTLS server handshake must complete first; a TLS/authentication or early
  handshake failure still returns `false`;
- every application read still must be echoed and flushed successfully;
- `AbortRequired` accepts only `ConnectionReset`, `ConnectionAborted`, or
  `BrokenPipe` after that handshake;
- all three owner-shutdown peers and the first stopped allocation use
  `AbortRequired`;
- only the unrelated second allocation's final post-client-drop cleanup uses
  `EofOrAbort`, which accepts EOF, rustls `UnexpectedEof`, or the
  same exact abort taxonomy; and
- timeout, echo failure, and every other error remain failures for every peer.

The two data-bearing connections in each body retain their existing
`assert_shared_exchange` calls before stop/shutdown, proving the handshake and
byte-exact request/response completed. The owner-shutdown body's third
connection remains the deliberate in-flight claim: its peer barrier is
released only after shutdown is proven to wait, and the helper accepts its
reset only after `drive_server_handshake` completes. Thus the correction does
not convert failed TLS, authentication, data, or early connection setup into a
successful lifecycle observation.

No body assertion, timeout, connection count, scenario, lane, production
surface, or teardown behavior changed. This private peer taxonomy correction
requires independent DISTILL review before the original crafter repeats the
same exact three-body durable Metal replay.
