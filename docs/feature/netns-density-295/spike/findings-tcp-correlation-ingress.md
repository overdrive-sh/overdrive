# TCP correlation Fast Open final-ACK ingress — native findings

> **Selected mechanism superseded 2026-10-23.** This cgroup-ingress path
> remains valid fallback evidence, but the user-approved production mechanism
> is the smaller handshake-only `fentry/tcp_check_req` candidate documented in
> [`findings-tcp-correlation-candidate.md`](findings-tcp-correlation-candidate.md).
> Production carries no private intake cgroup, ingress value lock or steady
> established-intake dispatch.

## Verdict

**WORKS in the required disposable Cloud Hypervisor guest on Metal:** x86_64,
Linux `7.0.0-38-generic`, two vCPUs, launched through the production
`CloudHypervisorVmm`/GuestKit fixture. The physical host supplied KVM and
vhost-vsock only; all BPF links, maps, cgroups, sysctls and sockets existed
inside the disposable guest. A second run reproduced the mechanism in the
repository Lima VM on aarch64 Linux `7.0.0-38-generic`.

A private TCP-intake-socket-cgroup `cgroup_skb/ingress` program sees a real
data-in-SYN Fast Open child's final
ACK under the exact full child socket cookie while that child is still
`TCP_SYN_RECV`. It parses the existing 18-byte guest-local correlation option
and records a candidate. After ACK validation, the child's SockOps state
callback promotes that candidate to `Ready` or `Rejected` on the
`SYN_RECV -> ESTABLISHED` transition.

The producing client descriptor remained open until `Ready`. The probe sent no
post-handshake application payload and did not close either endpoint before
observing `Ready`. A separate live, open, uncorrelated Fast Open producer
reached `Rejected` through the same ingress hook. No raw packet or seeded child
state was used.

This replaces only the falsified Fast Open consumer in
[`findings-tcp-correlation-option.md`](findings-tcp-correlation-option.md).
That probe's ordinary/syncookie final-ACK SockOps consumer, active-socket
storage, option producer and staging-pin evidence remain valid.

## Falsified premise

The STEP 11-06 production-composition run failed with the child established but
its correlation row still `Pending`:

```text
the naturally released final ACK did not wake the held Pending child:
child_entries=1, tcp_state=Some(1), socket_fds=9220,
before_owner=9219, active=true
```

Evidence:
`.context/netns-density-295/11-06/in-guest-correlation-tfo-qdisc-restored-red.log:473-493`.
The ordinary socket and the production `Connect4`, active storage, Fast Open
SYN, accepted child and owner-held descriptor were all live. Only the asserted
Fast Open `Pending -> Ready` producer failed.

Linux source explains the deterministic failure:

- `bpf_skops_parse_hdr` returns without invoking the SockOps parser while the
  socket is `TCP_SYN_RECV` (`vendors/linux/net/ipv4/tcp_input.c:147-179`).
- For a Fast Open child, `tcp_rcv_state_process` validates and processes the
  final ACK while the child is still `TCP_SYN_RECV`; only afterward does it call
  `tcp_rcv_synrecv_state_fastopen`, set `TCP_ESTABLISHED` and notify state
  change (`tcp_input.c:7228-7289`). There is no second passive-established or
  parse callback for that ACK.
- The earlier standalone probe closed its producer immediately after
  `MSG_FASTOPEN` and before waiting for `Ready`
  (`spike-scratch/netns-density-295-tcp-correlation-option/probe/src/main.rs:310-326`).
  The subsequent FIN/later established packet could invoke the parse callback,
  so that run did not prove that the final ACK produced `Ready`.

## Replacement source path

Linux invokes cgroup socket ingress before the child TCP state machine:

1. Established-hash receive selects the Fast Open child and calls
   `tcp_filter(child, skb)` before locking the child and entering
   `tcp_v4_do_rcv` (`vendors/linux/net/ipv4/tcp_ipv4.c:2292-2319`).
2. `tcp_filter` calls `sk_filter_trim_cap`, which invokes
   `BPF_CGROUP_RUN_PROG_INET_INGRESS(child, skb)`
   (`vendors/linux/include/net/tcp.h:1639-1645`;
   `vendors/linux/net/core/filter.c:134-154`).
3. The cgroup runner temporarily assigns `skb->sk = child` before executing the
   program and restores it afterward
   (`vendors/linux/kernel/bpf/cgroup.c:1561-1622`). Therefore
   `bpf_get_socket_cookie(skb)` is the exact full child cookie.
4. Only after that hook returns does Fast Open validate the final ACK. Its
   `tcp_set_state(TCP_ESTABLISHED)` invokes the requested SockOps state callback
   with old/new states before storing the new state
   (`tcp_input.c:7228-7289`; `net/ipv4/tcp.c:2997-3065`). The callback therefore
   promotes only a candidate carried by a kernel-accepted final ACK.

The probe attaches ingress only to a private cgroup. It temporarily moves the
process there to create the listener, then returns to its parent before
creating clients. Linux records the listener's cgroup and copies it to every
accepted child (`vendors/linux/net/core/sock.c:2538`); cgroup ingress selection
uses that socket cgroup (`vendors/linux/include/linux/bpf-cgroup.h:180-198`).
Unrelated guest sockets never execute the program. The program reads
`skb->sk->state` first and returns unless it is `TCP_SYN_RECV`, so established
intake data avoids the cookie helper and child-map lookup.

The hook is read-only on the packet and always returns allow. It reads only the
IPv4 and TCP headers/options. A cheap child-map lookup gates every transition:
a non-classifiable cookie returns. On the candidate path it parses, looks the
same cookie up again, acquires that existing value's `bpf_spin_lock`, rechecks
the state and mutates the value in place. A valid option writes
`CandidateReady`; a missing or malformed option writes `CandidateRejected`.
The post-validation state callback takes the same lock, rechecks and mutates a
candidate in place to `Ready` or `Rejected`. Neither callback calls map update
or insert after passive establishment.

The child map is the same 8,194-entry hash and the same 24-byte value: its old
padding word becomes the lock (`lock: u32, state: u32, meta: 16 bytes`). It is
pre-created with minimal BTF so the verifier recognizes the lock, reuses the
same private staging directory as active storage, and is unlinked before
program load/attach/READY. No runtime or additional map exists.

Linux hash deletion removes the element from its bucket under the bucket lock
and frees it through the RCU-safe element path
(`vendors/linux/kernel/bpf/hashtab.c:1494-1527`); lookups return the
RCU-protected element pointer (`:710-759`). Therefore a writer that obtained a
pointer before deletion may finish mutating only the now-unreachable old
value. Because candidate/promotion paths never call update/insert, they cannot
recreate the key. Terminal state is monotonic under the per-value lock, and
classification or `sock_release` deletion is final.
Userspace reads child state with `BPF_F_LOCK`; Linux copies the value while
holding the same in-value lock and masks the lock word in the returned copy
(`vendors/linux/kernel/bpf/syscall.c:308-359`). The native probe uses that flag
for every semantic state read.

## Executed probe

Sources:

- `spike-scratch/netns-density-295-tcp-correlation-ingress/bpf/src/main.rs`
- `spike-scratch/netns-density-295-tcp-correlation-ingress/probe/src/main.rs`
- `spike-scratch/netns-density-295-tcp-correlation-ingress/run.sh`
- `spike-scratch/netns-density-295-tcp-correlation-ingress/run-ch.sh`
- `spike-scratch/netns-density-295-tcp-correlation-ingress/runner/src/main.rs`

Required disposable-guest command:

```text
cargo xtask metal run -- \
  bash spike-scratch/netns-density-295-tcp-correlation-ingress/run-ch.sh
```

It builds the probe statically, stages it and the BPF object as GuestKit image
extras, boots a fresh scripted-peer Cloud Hypervisor guest, executes the probe
there, powers the guest off and removes its work directory. The recorded guest
console is
`.context/netns-density-295/11-06/tcp-correlation-ingress-ch-native.log`.

Secondary Lima command:

```text
cargo xtask lima run -- sudo --preserve-env=PATH \
  bash spike-scratch/netns-density-295-tcp-correlation-ingress/run.sh
```

Secondary recorded output:
`.context/netns-density-295/11-06/tcp-correlation-ingress-native.log`.

Relevant disposable-guest output:

```text
Linux version 7.0.0-38-generic ... x86_64 ...
DMI: Cloud Hypervisor cloud-hypervisor
uname=7.0.0-38-generic
ordinary_exact app_cookie=1 child_cookie=2 ...
direct_rejected child_cookie=4
syncookie_exact delta=1 app_cookie=5 child_cookie=6 ...
fastopen_wait_open client_fd=19 child_cookie=8
fastopen_direct_rejected_open client_fd=19 child_cookie=10
fastopen_wait_open client_fd=19 child_cookie=12
terminal_delete_monotonic child_cookie=14
pending_release_final child_cookie=16
fastopen_exact delta=1 app_cookie=11 child_cookie=12 ...
counters=[7, 6, 18, 18, 3, 4, 2, 2, 0, 1, 5, 2, 2, 1, 28]
VERDICT=WORKS
```

`TCPFastOpenPassive` increased for the recorded captured case. `Ready` was
observed while the producer fd remained valid. The direct Fast Open producer
had no origin entry, remained open through observation, and reached
`Rejected`. Ordinary and forced-syncookie paths remained exact. Store errors
were zero. A private PRIO/NETEM class then delayed real Fast Open final ACKs
while normal sends and closes raced the handoff: 128 queued one-byte sends plus
later terminal packets left the exact `Ready` metadata unchanged; one-time
userspace deletion stayed absent after 32 more sends; reset-closing a real
accepted pre-terminal child, then releasing its delayed ACK/data, left its row
absent. Twelve post-terminal packets reached the ingress program and were
counted as no-ops. No raw packet or seeded state was used.
The scratch `correlation_release` cgroup-sock program only mirrors the existing
production `SockRelease` program's child-cookie deletion; it is not a proposed
eighth program.

## Design implication

Retain the root-cgroup SockOps `TcpCorrelation` program for active option
production and ordinary/syncookie passive consumption. Add one
private-intake-socket-cgroup `TcpCorrelationIngress` cgroup-skb program for Fast Open candidate
capture; the already-present SockOps program promotes the candidate on the
post-validation state callback. Both attach before `Connect4` and READY. The
ingress program shares the existing `CFG`, `CHILD_CORRELATION` and private wake
map. `CHILD_CORRELATION` gains the in-value lock and shares the active map's
pre-load BTF/pin/reuse/unlink staging. This adds no map, capacity, host wire,
persistence, payload path, sockmap, parking cell, timer or retry protocol.

The child ABI becomes exactly `Pending | CandidateReady { metadata } |
CandidateRejected | Ready { metadata } | Rejected`. Candidates remain
nonterminal to userspace. The state callback replaces a candidate with its
terminal state and submits the best-effort private correlation-change wake.
The owner emits only after the terminal `Ready` and the accepted fd reports
`TCP_ESTABLISHED`; an early scan continues holding it. `Rejected` resets/closes
the child and emits no owner event, flow or control message. `sock_release`
remains the exact cleanup complement if the child ends first. Ingress and the
state callback never insert after the passive callback, so a terminal state or
deleted key cannot regress or reappear.

The existing one parameterized guest-kit scenario is corrected rather than
expanded: its real Fast Open producer must stay open and send no
post-handshake payload until `OutboundCaptured`, and its direct Fast Open case
must fail closed. No new lane, cell, public test support or abstract owner state
is required.

## Limits

- The probe populated the origin map directly; the failing production
  guest-kit run independently proves that production `Connect4`, the active
  option producer, the real Fast Open child and `KernelGuestOwnerIo` reached
  the isolated `Pending` state. Together the two runs isolate and prove the
  replacement consumer without manufacturing child state.
- The required proof ran in a disposable Cloud Hypervisor guest. It did not
  attach a program, change a sysctl or create a cgroup in the physical host
  kernel. The separate Lima run is corroborating evidence only.
- The scratch program uses a visible `Rejected` row for observation. Production
  consumes that private state immediately or through the existing audit scan;
  it is not an owner event or model state.
- Correctness receipts do not measure the seventh program's per-packet CPU,
  throughput or tail-latency cost. Only intake-listener/child packets execute
  it. Established packets pay BPF dispatch and one socket-state read/branch;
  only `TCP_SYN_RECV` pays cookie acquisition and child-map work.
