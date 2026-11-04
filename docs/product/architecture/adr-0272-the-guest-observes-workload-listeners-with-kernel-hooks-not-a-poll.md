# ADR-0272: The guest observes workload listeners with kernel hooks, not a poll

## Status

**User-approved, 2026-11-03 ("add it. and proceed with recommendations"); the corrected
contract below (feature delta, D13 items 1 and 7 to 9) was ACCEPTED in contract shape by the
user on 2026-11-03. Not yet Accepted as a decision: it remains "user-approved, pending the
V41 boot-cost measurement and the independent re-review"** (cold start is the platform goal;
V41 is a contract-shaping budget item, and a material B-minus-A delta returns D13 to DESIGN;
`.claude/rules/design.md` § "Surface material design decisions").
Spikes V34, V35 and V37 are discharged (held) and V36 held; V39 is partial (the
operator-example guest kernel config was not located). Findings:
`docs/feature/service-allocation-readiness-and-ending/spike/findings-listen-hooks.md`, kernel
`7.0.0-38-generic`. Not implemented. GH #318, GH #334. Spike table: feature delta § Spike table.
Related:
[ADR-0268](adr-0268-the-guest-reports-each-workload-listener-it-opens-over-the-beacon-channel.md)
(how the observation crosses to the host), [ADR-0150](adr-0150-guest-kernel-aya-adaptation-controlled-by-overdrive-init.md)
(guest-side aya programs controlled by `overdrive-init`), ADR-0206 (guest kit).

## Context

Nothing in the guest observes the workload's `listen()` or `bind()`. The guest already
loads seven aya programs from `overdrive-init` before `READY`, attached `Single` at the
cgroup2 root, so they see every socket of the workload (the workload runs as a process
group, with no cgroup of its own): a sockops program, an `fentry` program, four
`cgroup/sock_addr` programs and a `sock_release` `CgroupSock` program
(`overdrive-init/src/transport/kernel.rs:41-42,162-173,300-346`;
`transport/mod.rs:419-436`). The sockops program ignores the listen op
(`tcp_correlation.rs:269-359`).

Source read at `vendors/linux` `v7.0` (sibling workspace checkout; this worktree's
submodule is not initialised, so the reviewer re-verifies in a v7.0 checkout):

- `net/ipv4/af_inet.c:199-233`: `__inet_listen_sk` runs the sockops op
  `BPF_SOCK_OPS_TCP_LISTEN_CB` (`include/uapi/linux/bpf.h:7082`) at `:230`, only when the
  socket enters `TCP_LISTEN` (`:211`), after `inet_csk_listen_start` succeeded (`:226`),
  unconditionally (`include/net/tcp.h:2822-2845`); IPv6 stream sockets use `inet_listen`
  (`net/ipv6/af_inet6.c:689`).
- `net/ipv4/af_inet.c:549-557`: `__inet_bind` runs `CGROUP_INET4_POST_BIND` after `get_port`
  succeeds, unless the bind came from BPF; `net/ipv6/af_inet6.c:406` is the v6 twin.
  `inet_autobind` (`af_inet.c:182-189`) does not run it. The program runs in the socket's
  cgroup (`kernel/bpf/cgroup.c:1642-1645`) and reads the port as `skc_num`
  (`net/core/filter.c:10250-10258`).

## Decision

**TCP listeners are observed by a `BPF_SOCK_OPS_TCP_LISTEN_CB` arm merged into the guest's
existing sockops program; UDP listeners by two `CgroupSock` programs, post-bind4 and
post-bind6. All attach at the cgroup2 root before `READY`. A bind to a loopback address is
not a listener report.**

- The guest does not hold `workload_addr`, so the loopback exclusion is the strongest
  address filter available; a bind to any other address (wildcard or specific, either
  family) is reported (V34, V35).
- Merge, not a second sockops program: aya attaches through `bpf_link_create`, which always
  adds `BPF_F_ALLOW_MULTI` (`kernel/bpf/cgroup.c:1502-1505 @ v7.0`), so both shapes work.
  The merged arm verified at +112 instructions (750 of 1,000,000) with correlation
  behaviour unchanged (V37); one program means no further attach.
- UDP needs two programs because `src_ip4` and `src_ip6` are readable per attach type
  (`net/core/filter.c:8976-8989 @ v7.0`); both return 1 (a 0 fails `bind()`,
  `af_inet.c:549-557 @ v7.0`) and ignore non-UDP binds.
- No guest state about the declared listeners; the host intersects (ADR-0268).
- `IPV6_V6ONLY` is not observable on either hook (V34, V35), so an `[::]` report carries no
  claim about IPv4 reachability; the contract and its exposure are the feature delta's D13
  item 8.

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
- Residual exposures, stated: an IPv6 wildcard listener with `IPV6_V6ONLY` is counted as
  open although IPv4 peers cannot reach it (the hooks cannot see the option; measured, V34
  and V35); the retired probe would have failed in that case. Overdrive does not support IPv6
  yet; the dual-stack rule (an `AF_INET6` wildcard report satisfies an IPv4-declared listener)
  and this exposure are recorded against GH #308 (user ruling 2026-11-03). A UDP socket that receives
  without ever calling `bind()` (autobind), or binds with `IP_BIND_ADDRESS_NO_PORT`, is never
  reported, which is correct for a server and means such a workload must use the opt-out.
- The guest kernel is the operator's (BYO) until #259, and the repo has no guest
  `.config`; the hooks need `CONFIG_BPF_SYSCALL` and `CONFIG_CGROUP_BPF`, which the
  existing guest programs already need, plus sockops and `CgroupSock` program support
  (V39: present on the metal fixture kernel; the operator-example guest kernel config was not
  located and no `IKCONFIG` is embedded). The appliance kernel image config must assert them
  (ADR-0068 §7).
- A report the guest cannot enqueue (event ring full) is dropped and counted; the Service
  then does not become `Stable` and the startup deadline stops it: the fail-closed
  direction. The ring is sized generously by the crafter and its drop counter stays visible;
  only TCP listen transitions and explicit UDP binds produce events. A level map so a full ring
  cannot lose a report (P-10) was rejected by the user on 2026-11-03; a lost report is a stated,
  bounded exposure (stopped at the startup deadline, not hung). Feature delta D13 item 9.
