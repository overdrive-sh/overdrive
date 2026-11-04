# ADR-0272: The guest observes workload listeners with kernel hooks, not a poll

## Status

User-approved, 2026-11-03; hook scope (PR-2) accepted in contract shape 2026-11-04
(direction approved 2026-11-04 pending V42; V42 held 2026-11-04, kernel
`7.0.0-38-generic`, [findings](../../feature/service-allocation-readiness-and-ending/spike/findings-workload-cgroup.md));
not yet Accepted (remaining gates: model check, design review round 4). Not implemented.
GH #318, GH #334. Contract
and status: [feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md) (D13).
Related:
[ADR-0268](adr-0268-the-guest-reports-each-workload-listener-it-opens-over-the-beacon-channel.md)
(how the observation crosses to the host), [ADR-0150](adr-0150-guest-kernel-aya-adaptation-controlled-by-overdrive-init.md)
(guest-side aya programs controlled by `overdrive-init`), ADR-0206 (guest kit).

## Context

Nothing in the guest observes the workload's `listen()` or `bind()`. The guest already
loads seven aya programs from `overdrive-init` before `READY`, attached `Single` at the
cgroup2 root, so they see every socket on the guest (the workload runs as a process
group, with no cgroup of its own): a sockops program, an `fentry` program, four
`cgroup/sock_addr` programs and a `sock_release` `CgroupSock` program
(`overdrive-init/src/transport/kernel.rs:41-42,162-173,300-346`;
`transport/mod.rs:419-436`). The sockops program ignores the listen op
(`tcp_correlation.rs:269-359`). Init also binds its own TCP intake and 1,024 UDP slots
before `READY`; hooks that see them report sockets the host never uses.

Kernel source at `vendors/linux` `v7.0`:

- `net/ipv4/af_inet.c:199-233`: `__inet_listen_sk` runs the sockops op
  `BPF_SOCK_OPS_TCP_LISTEN_CB` (`include/uapi/linux/bpf.h:7082`) at `:230`, only when the
  socket enters `TCP_LISTEN` (`:211`), after `inet_csk_listen_start` succeeded (`:226`),
  unconditionally (`include/net/tcp.h:2822-2845`); IPv6 stream sockets use `inet_listen`
  (`net/ipv6/af_inet6.c:689`).
- `net/ipv4/af_inet.c:549-557`: `__inet_bind` runs `CGROUP_INET4_POST_BIND` after `get_port`
  succeeds, unless the bind came from BPF; `net/ipv6/af_inet6.c:406` is the v6 twin.
  `inet_autobind` (`af_inet.c:182-189`) does not run it. The program runs in the socket's
  cgroup (`kernel/bpf/cgroup.c:1639-1646`) and reads the port as `skc_num`
  (`net/core/filter.c:10250-10258`).

## Decision

**TCP listeners are observed by a `BPF_SOCK_OPS_TCP_LISTEN_CB` arm in a sockops program;
UDP listeners by two `CgroupSock` programs, post-bind4 and post-bind6. All attach before
`READY`. A bind to a loopback address is not a listener report.** Init creates a workload
cgroup before `READY`, attaches the three listen programs there and starts the workload
inside it, so init's own transport sockets are invisible by construction. The TCP arm is a
separate small sockops program in the same object, not merged into the root-attached sockops
program (feature delta, D13 items 1 and 7).

- The guest does not hold `workload_addr`, so the loopback exclusion (`127.0.0.0/8`, `::1`,
  `::ffff:127.0.0.0/104`) is the strongest address filter available; a bind to any other
  address (wildcard or specific, either family) is reported (V34, V35).
- aya attaches through `bpf_link_create`, which always adds `BPF_F_ALLOW_MULTI`
  (`kernel/bpf/cgroup.c:1506-1507 @ v7.0`), so a second sockops program coexists with the
  first; the existing correlation behaviour is unchanged (V37).
- Constraints pinned by V42 (the contract is in the feature delta, D13 item 1, P-a to P-d):
  - The three listen programs live in the guest object `overdrive-init` already loads;
    a second `Ebpf::load` is not permitted (it costs +20.4 ms p50, aya parses kernel BTF in
    `EbpfLoader::new()`).
  - The root programs remain `BPF_F_ALLOW_MULTI` link attachments, so they stay effective in
    the workload cgroup: a parent's programs are inherited only when flagged
    `BPF_F_ALLOW_MULTI` or the child has none of that attach type
    (`compute_effective_progs`, `kernel/bpf/cgroup.c:432-490`, `:443-447`, `:458-484`); a
    legacy `Single` attach without the flag would drop the root sockops program
    (`:459 @ v7.0`).
  - The workload is created directly in the workload cgroup, with no window in which it
    runs elsewhere (`CLONE_INTO_CGROUP`).
  - Reports cover only sockets created by tasks in the workload cgroup; a workload that
    leaves it is never reported and never `Stable` (fails closed).
- UDP needs two programs because `src_ip4` and `src_ip6` are readable per attach type
  (`net/core/filter.c:8976-8989 @ v7.0`); both return 1 (a 0 fails `bind()`,
  `af_inet.c:549-557 @ v7.0`) and ignore non-UDP binds.
- No guest state about the declared listeners; the host intersects (ADR-0268).
- `IPV6_V6ONLY` is not observable on either hook (V34, V35), so an `[::]` report carries no
  claim about IPv4 reachability.

## Alternatives considered

- **`sock_diag` / `/proc/net` polling in an in-guest agent.** The floor moves into the
  guest, and a listener opened and closed between polls is missed. A poll loses to an
  event the kernel already raises.
- **`fentry` on `inet_listen`.** Needs BTF and a trampoline; the sockops arm needs
  neither and the program already exists.
- **`cgroup/bind4` (pre-bind) instead of post-bind.** Fires before `get_port`, so a bind
  that fails with `EADDRINUSE` would be reported; post-bind runs only after the port is
  held.
- **Post-bind for TCP too.** A TCP socket that binds is not listening until `listen()`;
  the report would be early. Rejected.
- **The workload signals readiness (Unikraft style).** Needs every workload modified;
  retained as the producer a non-Linux guest could use against the same wire.

## Consequences

- An event, not a poll: no guest-side timer exists.
- Cold start: attaching the listen programs before `READY` adds 0.36 ms at p50 (V41); the
  PR-2 shape (programs in the existing object, attached to the workload cgroup, workload
  placed at creation) measures **+0.88 ms at p50** (paired p90 +1.66 ms, max +2.49 ms) to a
  guest that already spends about 117 ms on BPF work there (V42). Keeping the programs in
  the object init already loads is what keeps it small; a second `Ebpf::load` costs +20.4 ms. The attach cannot move after `EXEC`: attaching after
  the workload ran missed every listener (0 of 50).
- A report the guest cannot enqueue (event ring full) is dropped and counted, and the workload
  is not slowed. The Service then does not become `Stable` and the startup deadline stops it:
  the fail-closed direction, a stated and bounded exposure (no level map). The ring is sized
  generously and its drop counter stays visible; only TCP listen transitions and explicit UDP
  binds produce events, and a workload opens a handful of listeners.
- Residual exposures, stated: an IPv6 wildcard listener with `IPV6_V6ONLY` is counted as
  open although IPv4 peers cannot reach it (the hooks cannot see the option); the retired
  probe would have failed in that case. Overdrive does not support IPv6 yet; the dual-stack
  rule and this exposure are recorded against GH #308. A UDP socket that receives without
  ever calling `bind()` (autobind), or binds with `IP_BIND_ADDRESS_NO_PORT`, is never
  reported, which is correct for a server and means such a workload must use the opt-out.
- The guest kernel is the operator's (BYO) until #259, and the repo has no guest
  `.config`; the hooks need `CONFIG_BPF_SYSCALL` and `CONFIG_CGROUP_BPF`, which the
  existing guest programs already need, plus sockops and `CgroupSock` program support
  (present on the metal fixture kernel; the operator-example guest kernel config was not
  located and no `IKCONFIG` is embedded). The appliance kernel image config must assert them
  (ADR-0068 §7).
