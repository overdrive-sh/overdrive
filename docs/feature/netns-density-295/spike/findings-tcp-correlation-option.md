# TCP capture correlation via the handshake-completing ACK

> **Fast Open conclusion superseded 2026-10-23.** A production-composition
> native run proved that Linux skips SockOps header parsing while a Fast Open
> child is `TCP_SYN_RECV`; the final ACK therefore cannot perform the claimed
> parse-callback `Pending -> Ready` transition. This probe closed its producer
> immediately after `MSG_FASTOPEN`, so a FIN/later established packet made its
> apparent Fast Open success a false positive. Ordinary/syncookie final-ACK
> consumption, active socket storage, option production and staging-pin facts
> remain valid. The native replacement evidence is
> [`findings-tcp-correlation-candidate.md`](findings-tcp-correlation-candidate.md).

## Verdict

**WORKS** on Linux `7.0.0-38-generic` aarch64 in the repository Lima VM with
the production-pinned Aya generations (`aya-ebpf 0.1.1`, bindings 0.1.2,
`aya 0.13.1`) and nightly `2026-08-05`. The userspace probe pins `libc
0.2.190`.

The bounded probe is
`spike-scratch/netns-density-295-tcp-correlation-option/`. It is standalone
Aya Rust and changes no production crate. One SockOps program writes the exact
application cookie and original destination in an 18-byte experimental option
on the non-SYN ACK that completes the local TCP handshake. The passive side
loads that current ACK. It covers:

- an ordinary handshake followed by immediate application release before
  userspace `accept()`;
- a forced Linux syncookie handshake (`tcp_syncookies=2`);
- an actual TCP Fast Open child created by a data-bearing SYN, whose state is
  `Pending` until the final ACK changes it to `Ready`;
- a direct connection with no captured origin, which becomes `Reject`; and
- active socket storage deleted by post-handshake peer input, plus an armed
  connect to an unused port whose storage is still present at `TCP_CLOSE` and
  then removed automatically with that socket.

The active transient is `BPF_MAP_TYPE_SK_STORAGE` with `max_entries = 0`, so
it has no fixed capacity and follows the exact socket. The probe creates its
BTF-backed map, lets Aya reuse a private staging pin, then removes that pin
and directory before loading or attaching the program. The child handoff is a
supported ordinary Aya hash map.

## Hypothesis, prediction and falsification

**Hypothesis.** Moving the complete metadata from the SYN to the
handshake-completing ACK avoids SYN-option competition with Fast Open and
also covers syncookies, because the passive callback receives the current
final ACK. A Fast Open child can remain keyed by its own cookie as `Pending`
until its final ACK arrives. Active metadata can live on the exact active
socket without a capacity: the first later peer packet deletes it, or socket
destruction removes it automatically.

**Prediction.** With the SockOps header callbacks armed at
`BPF_SOCK_OPS_TCP_CONNECT_CB` after `connect4` recorded the origin:

- `BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB` precedes the final ACK;
- the final ACK reserves and stores the exact 18-byte option;
- ordinary and syncookie passive establishment loads the current ACK with
  `bpf_load_hdr_opt(..., flags = 0)`;
- Fast Open passive establishment on the SYN creates `Pending`, and its
  parse callback loads the final ACK and creates `Ready`;
- post-handshake peer input deletes active socket storage;
- a refused socket still has that storage at `TCP_CLOSE`, immediately before
  automatic socket-lifetime removal; and
- an uncorrelated direct connection cannot produce `Ready`.

**Falsification.** Verifier/load rejection; no final-ACK header callback;
option reserve/store/decode failure; a cookie or destination mismatch;
missing metadata after immediate close; syncookie or Fast Open refusal;
active storage missing before emit, surviving post-handshake input or not
following a refused socket through `TCP_CLOSE`; or
metadata on a direct connection.

## Executed evidence

Command:

```text
cargo xtask lima run -- bash spike-scratch/netns-density-295-tcp-correlation-option/run.sh
```

Final output:

```text
uname=7.0.0-38-generic
ordinary_exact app_cookie=37394 child_cookie=37395 meta=Meta { application_cookie: 37394, destination_ip: 158400715, destination_port: 409, pad: 0 }
direct_rejected child_cookie=37397
syncookie_exact delta=1 app_cookie=37398 child_cookie=37399 meta=Meta { application_cookie: 37398, destination_ip: 175177931, destination_port: 410, pad: 0 }
fastopen_exact delta=1 app_cookie=37402 child_cookie=37403 meta=Meta { application_cookie: 37402, destination_ip: 208732363, destination_port: 412, pad: 0 }
refused_cleanup app_cookie=37404
counters=[5, 4, 8, 8, 2, 2, 2, 1, 0, 1, 4, 1]
VERDICT=WORKS
```

Counter order: captured connect armed; active established; ACK option space
reserved; ACK option written; ordinary/syncookie `Ready`; Fast Open
`Pending`; Fast Open `Pending -> Ready`; direct rejection; store error; the
direct connection's expected no-option load; active storage removed by
post-handshake parse/delete; refused socket storage observed at
`TCP_CLOSE` before automatic destruction.

`TcpExt SyncookiesRecv` and `TcpExt TCPFastOpenPassive` each increased by one
for the recorded cases. The Fast Open case read the exact data sent in the
SYN. There were zero store errors. The VM's stock Linux 7.0 settings were
`tcp_ecn=2`, `tcp_ecn_option=2`, `tcp_timestamps=1` and `tcp_sack=1`.

The first revision of this spike wrote the same 18-byte metadata in the SYN.
That default-only case worked, but review falsified it for Fast Open option
space and syncookies. It is superseded by the executed final-ACK mechanism.

## Kernel and API grounding

- `connect4` runs before `tcp_v4_connect` allocates the ephemeral port
  (`net/ipv4/tcp_ipv4.c:206-218`, `:301-308 @ v7.0`).
- After port allocation, `tcp_connect` invokes
  `BPF_SOCK_OPS_TCP_CONNECT_CB` before it builds the SYN
  (`net/ipv4/tcp_output.c:4300-4305 @ v7.0`). This is the feasible owner of
  callback arming and the active metadata copy; `Connect4` does not arm
  SockOps callbacks.
- The active-established callback runs before Linux sends the ACK that
  concludes the handshake (`net/ipv4/tcp_input.c:6753-6768`, `:6994-7022 @
  v7.0`).
- Linux builds SYN options, including Fast Open, before BPF option reservation
  (`net/ipv4/tcp_output.c:959-1054 @ v7.0`). The replacement writes nothing
  on the SYN. The stock guest does not negotiate outgoing AccECN
  (`tcp_ipv4.c:3508-3511`); on the concluding ACK the ordinary kernel option
  is the 12-byte timestamp, leaving 28 of 40 option bytes for the 18-byte
  correlation option (`tcp_output.c:1148-1232`).
- `BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB` exposes the ACK that concludes the
  handshake; Linux's selftest loads the current ACK specifically when a
  syncookie leaves no saved SYN
  (`tools/testing/selftests/bpf/progs/test_tcp_hdr_options.c:458-478 @ v7.0`).
- A Fast Open child is created and receives its passive-established callback
  from the SYN (`net/ipv4/tcp_fastopen.c:326-371 @ v7.0`), then its final ACK
  changes the socket from `TCP_SYN_RECV` to `TCP_ESTABLISHED` and calls
  `sk_state_change` (`net/ipv4/tcp_input.c:7118-7291`). The parse callback is
  therefore the exact `Pending -> Ready` producer.
- SockOps state callbacks name `TCP_CLOSE`; the probe executed that cleanup on
  a refused connection. `BPF_MAP_TYPE_SK_STORAGE` is capacity-free
  (`max_entries = 0`) and follows socket lifetime. The existing v20 probe
  independently exercised its BTF-backed Aya load, userspace fd lookup,
  lifetime, state callbacks, syncookies and Fast Open at scale on Linux 7.0
  (`spike/findings-v20.md`).

## Exact design implication

Use the handshake-completing ACK as the self-contained carrier, active socket
storage and one process-owned ordinary child map:

- `ACTIVE_CORRELATION` is socket storage created on the exact active socket at
  `TCP_CONNECT_CB`, after local-port allocation. Active establishment moves it
  to emit phase. The first later non-SYN peer packet deletes it; if none
  arrives it dies with the socket. Application file release does not remove
  it from a still-live orphaned TCP socket, so immediate release cannot
  suppress ACK retransmission metadata.
- `CHILD_CORRELATION`, keyed by the exact accepted child cookie, contains
  `Pending` or `Ready { application_cookie, original_destination }`. Ordinary
  and syncookie final ACKs create `Ready`; a Fast Open SYN creates `Pending`
  and its final ACK changes it to `Ready`. Intake accept consumes `Ready`
  once. `sock_release` deletes any row for a destroyed child.

The option ABI remains exactly 18 bytes: experimental kind `254`, length
`18`, magic/version `0x4f56`, application cookie as BE `u64`, original IPv4
address in network order and original port as BE `u16`. It never leaves the
guest-local connection to `workload_addr:GUEST_TCP_INTAKE_PORT`.

The active storage has no `max_entries` capacity and cannot alias or outlive
its socket. With Aya 0.13.1 its create/reuse pin is a pre-attach staging
operation only: the running program and teardown have no bpffs pin. The child
map's exact capacity is 8,194: at most 4,097 children can occupy
the intake accept queue for a `listen(4096)` backlog, at most 4,096 accepted
Fast Open children can be held under `GUEST_TCP_PENDING_MAX`, and one spare
entry covers the classification boundary. Its entries are consumed at
classification or deleted by exact child `sock_release`. No collection can
accumulate across sequential connections, so correlation introduces no
admission bound below the accepted pending-flow quota.

A Fast Open child accepted while still `Pending` counts immediately against
`GUEST_TCP_PENDING_MAX`. Its final-ACK callback changes the row and submits a
best-effort internal wake; the event wait scans every pending child before it
blocks and after every poll wake, with the existing fixed one-second owner
audit deadline as the idle fallback. A dropped wake therefore delays but
cannot lose classification. A child that closes while pending is removed by `sock_release` and the
owner drops its held descriptor. This adds no timer, retry, adoption protocol
or host-facing event.

## Limits

- The run used the Lima 7.0 kernel rather than a Cloud Hypervisor guest. It
  exercised the same real cgroup/SockOps/TCP paths without touching a physical
  host kernel. The post-approval Tier-3 scenario remains in the existing
  disposable guest-kit lane.
- The probe populated the accepted origin map from userspace instead of
  executing production `Connect4`. Linux source pins the order, and the Tier-3
  scenario must exercise the composed producer.
- The probe observes the child-cookie map directly and uses a state callback
  only to prove that refused active storage reaches `TCP_CLOSE`; production
  cleanup is the storage's socket lifetime. It does not implement the
  production owner's pending-child descriptor table, best-effort wake or
  one-second audit. Those are existing owner/event-loop primitives whose exact
  correlation use requires Tier-3 coverage.
- The option-space conclusion is for the accepted ordinary IPv4 TCP socket on
  the stock pinned guest configuration. The intake does not configure TCP-AO,
  TCP-MD5 or MPTCP; those are not evidence supplied by this spike.

## Gate

Replace the pending SYN/accepted-child-SK_STORAGE proposal with this exact
final-ACK, active-socket-storage and child-cookie contract, then return it to the same
independent DESIGN reviewer. Do not implement or modify DISTILL until the
reviewed exact decision is approved.
