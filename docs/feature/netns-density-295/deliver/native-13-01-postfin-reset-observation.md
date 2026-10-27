# Native STEP 13-01 post-FIN reset observation falsification

## Metadata and disposition

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Date | 2026-10-27 |
| Role | Bounded DESIGN native-falsification record |
| Substrate | Physical x86_64 Metal; Linux `7.0.0-38-generic` |
| Affected contract | DG-10 reading 2 and D20's leg-F remote-socket post-FIN evidence only |
| Affected roadmap step | `13-01` only |
| Current validation | **PENDING / NON-EXECUTABLE from STEP 13-01** |
| Outcome | Reset remains required and is emitted; the post-EOF `read` oracle is invalid |
| Replacement disposition | **PFRO-1 independently APPROVED under the standing no-outcome-change authority; DISTILL alignment and existing-cell validation remain pending** |

The unchanged STEP 13-01 cell
`dropping_a_claimed_registration_with_the_loopback_leg_half_ended_resets_the_remote_peer`
reaches the real production path, forwards the guest-side FIN to leg B, drops
the owner-held claimed registration, and then times out because the peer's
second and later `read` calls continue to return EOF. Native packet capture
around that same unchanged cell proves that production nevertheless emits an
RST on the same leg-B four-tuple after the FIN. A self-contained native probe
then isolates the endpoint semantics: the peer moves from `TCP_CLOSE_WAIT` to
`TCP_CLOSE` and retains `SO_ERROR=EPIPE`, while a post-FIN `recv` still returns
zero.

The falsified premise is therefore precise: **after an application has
consumed FIN, another `read` on that Linux TCP socket is not an oracle for a
later RST**. This does not falsify DG-10/D20's reset outcome. It falsifies the
DISTILL evidence interpretation that `read` must surface a reset after it has
already returned EOF.

No production source, test body, DES event, execution log, model, lane, host
module, cgroup, management route, or network namespace was changed by this
validation. The isolated probe source and raw captures are retained under
`spike-scratch/netns-density-295-postfin-reset/increment-a/`.

## Reproduced production path

The failure is production-reachable and does not depend on a simulated state:

1. `owner_runtime::tcp_connect` resolves `Mesh`, opens the real loopback peer,
   registers the exact `(forwarder_local, original_destination, backend)` with
   `MtlsInterceptWorker`, connects to leg F, and attaches the pair
   (`owner_runtime.rs:1060-1095`).
2. The owner stores the returned `ForwardedOutbound` by value in
   `OwnedPair::_forwarded` (`owner_runtime.rs:819-858`;
   `flow_owner.rs:285-296`).
3. The worker claims the registration on leg-F accept and starts outbound mTLS
   enforcement (`mtls_intercept_worker.rs:3099-3127,3175-3204`).
4. `HostMtlsEnforcement` owns the accepted leg F and kTLS leg B in one
   `ConnState` (`mtls/mod.rs:82-101`). A clean source end in the encrypt pump
   forwards `shutdown(leg_B, SHUT_WR)`, producing FIN while the reverse
   direction remains live (`splice.rs:682-743`).
5. The real forwarder end record removes the `OwnedPair`
   (`owner_runtime.rs:1293-1316`). Dropping `_forwarded` removes the claimed
   registration and cancels its state. A separate registration-owned task
   takes the handle and awaits the existing enforcement teardown; the
   guest-flow owner's `process_end` does not join that task
   (`mtls_intercept_worker.rs:190-199,1347-1373`).
6. Inside that separate task, teardown removes the `ConnState`, stops and joins
   its pumps, and drops the two owned descriptors (`mtls/mod.rs:294-354`). Leg B has carried
   `SO_LINGER{1,0}` since establishment (`mtls/outbound.rs:104-116`).

The unchanged cell's preserved failure is
`.context/netns-density-295/13-01/green-half-12.log`: the peer recorded
`saw_fin=true` but remained at its recorder's `Termination::Open` until the
deadline. The independent candidate receipt
`.context/netns-density-295/13-01/linger2-triple.log` confirms the real leg B
was `TCP_FIN_WAIT2` (`TCP_INFO=5`) with `TCP_LINGER2=-1`; the same read-based
oracle still timed out. That result disproves `TCP_LINGER2` as an oracle fix,
but it does not establish that no RST was sent.

## Native endpoint observation

The self-contained probe arms a real loopback TCP socket with kTLS TX and RX,
sends FIN, waits until the local endpoint is `TCP_FIN_WAIT2`, applies the
candidate close action, and records the peer's next `recv`, `TCP_INFO` state,
and `SO_ERROR`. Full source and raw output are retained with the probe.

| Candidate | Local pre-close | Peer after action | Peer `SO_ERROR` | Post-FIN `recv` | Disposition |
|---|---:|---:|---:|---:|---|
| Existing `SO_LINGER{1,0}` | `5` (`FIN_WAIT2`) | `7` (`CLOSE`) | `32` (`EPIPE`) | `0` (EOF) | **Reset effect works; read oracle does not** |
| `TCP_LINGER2=-1` plus existing linger | `5` | `7` | `32` | `0` | No additional effect; reject |
| `TCP_REPAIR=1` before close | `5` | `8` (`CLOSE_WAIT`) | `0` | `0` | Suppresses reset; reject |
| Exact-cookie `SOCK_DESTROY` | `5` | `7` | `32` | `0` | Works but adds an unnecessary privileged mechanism; reject |

Evidence:

- command: `cargo xtask metal run -- bash
  spike-scratch/netns-density-295-postfin-reset/increment-a/run.sh`;
- run: `1c399d41d0b18d5213938a51`;
- raw output:
  `spike-scratch/netns-density-295-postfin-reset/increment-a/native-endpoint-observation.log`.

## Unchanged production-path FIN/RST capture

The second native run surrounded only the unchanged failing cell with a
loopback FIN/RST capture. It changed no source or test body. The two target
flows show the exact ordering at the cell's five-second half-open observation
boundary:

| Flow | FIN | Later RST | Meaning |
|---|---|---|---|
| Leg F | `33519 -> 35783` at `20:40:17.433660` | `35783 -> 33519` at `20:40:22.485841` | The accepted leg-F endpoint aborts the forwarder side after the test has held it half-open for `CLOSURE_DEADLINE=5s`. |
| Leg B | `39876 -> 43469` at `20:40:17.433683` | `39876 -> 43469` at `20:40:22.485909` | The same production leg-B endpoint emits RST on the same four-tuple after its forwarded FIN. |

The unchanged cell still exits 100 because its peer recorder calls `read`
again after EOF; the packet capture proves that this failure is an observation
failure, not an absent close effect.

Evidence:

- command: `cargo xtask metal run -- bash
  spike-scratch/netns-density-295-postfin-reset/increment-a/capture-production-rst.sh`;
- run: `944d48d864bb9263b521ef70`;
- raw output:
  `spike-scratch/netns-density-295-postfin-reset/increment-a/native-production-fin-rst.log`.

## Linux 7.0 explanation

The vendored primary source matches both native runs:

- kTLS close restores the saved TCP protocol before it calls the underlying
  close (`vendors/linux/net/tls/tls_main.c:365-395`). A non-`SOL_TLS` socket
  option is likewise delegated to that saved TCP protocol (`:896-905`).
- TCP close with zero linger calls the underlying protocol's `disconnect`
  (`vendors/linux/net/ipv4/tcp.c:3225-3228`). `tcp_need_reset` includes
  `TCP_FIN_WAIT2` (`:3360-3364`), and `tcp_disconnect` sends the active reset
  for that state (`:3400-3419`).
- When the peer receives FIN in `ESTABLISHED`, `tcp_fin` sets
  `RCV_SHUTDOWN`, sets `SOCK_DONE`, and moves it to `TCP_CLOSE_WAIT`
  (`vendors/linux/net/ipv4/tcp_input.c:4947-4961`). A later RST in
  `TCP_CLOSE_WAIT` records `EPIPE` and closes the peer endpoint
  (`:4905-4930`).
- With no copied bytes, `tcp_recvmsg_locked` checks `SOCK_DONE` before
  `sk_err` (`vendors/linux/net/ipv4/tcp.c:2793-2803`). Once FIN has been
  consumed, another `recv` therefore returns EOF without consuming the pending
  `EPIPE`; `SO_ERROR` remains the endpoint error oracle.
- `TCP_REPAIR` is not an abort mechanism: `tcp_disconnect` explicitly takes
  its repair branch and records only local `ECONNABORTED`, skipping
  `tcp_send_active_reset` (`vendors/linux/net/ipv4/tcp.c:3412-3419`).

## Selective invalidation

Only these dependencies return to pending:

- DG-10 reading 2's interpretation of “the remote peer sees a reset” after it
  has already consumed FIN;
- D20's STEP 13-01 post-FIN remote-socket evidence, not D20's reset outcome;
- roadmap validation from STEP 13-01 onward, because the current authored cell
  cannot establish that outcome with its read-after-EOF oracle.

The following remain valid and are not reopened:

- D9 mesh registration, one resolution, and the claimed-registration owner;
- D14 removal of cleartext pass-through;
- DG-10 refusal labels, unclaimed-drop behavior, open-drop reset, clean-close
  FIN, and precedence rules;
- all completed STEP 12-01 commits and reviews (`7105b5807`, `9f7befe04`,
  `83419d308`, `f613be8b3`);
- leg-C/`GuestStream` reset evidence, D14 LEGS-DIAL, and other D20 owner/reset
  paths;
- the eleven other STEP 13-01 mesh/policy cells already passing.

No accepted `zero`, `never`, `must`, `exactly`, fail-closed, security, cleanup,
ownership, or reset outcome is weakened.
