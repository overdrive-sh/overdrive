# Spike findings: listen-hooks for D13 (ADR-0272, ADR-0268)

Feature `service-allocation-readiness-and-ending`, proposed decision D13. Probe: `spike-scratch/listen-hooks/` (self-contained; aya-rs eBPF in `bpf/`, aya 0.13.1 host harness in `probe/`;
nothing under `crates/` touched). Driver: `spike-scratch/listen-hooks/run.sh`. Raw evidence: `spike-scratch/listen-hooks/evidence/` (`full-metal-run-log.txt`, `all-run.txt`).

Item numbering follows the feature-delta renumbering: the task's V32 = **V34** (TCP listen), V33 = **V35** (UDP post_bind), V35 = **V37** (coexistence, verifier, flood),
V37 = **V39** (kernel config). Both labels are given below as "V32/V34" and so on.

## Substrate

Native metal host through `cargo xtask metal run --`, as root, under the lease. Every case ran in a child cgroup `/sys/fs/cgroup/lh-<pid>-<n>-<label>`; the host root cgroup was never used.
Programs were attached with the guest's own loader shape (aya `Single`).

```
uname -r: 7.0.0-38-generic   uname -m: x86_64   systemd-detect-virt: none
fixture kernel == /boot/vmlinuz-7.0.0-38-generic (byte-identical)
```

This is a host run, not an in-guest run. The in-guest load and attach of the same program shapes on this same kernel is already proven by the sibling spike
(`findings-guest-signal.md`: `GS_HOOKS_ATTACHED` on five real guest boots). This spike adds the per-case behaviour, the merged-program verifier and correlation checks, and the flood.

## Verdicts

| Item | Verdict | One line |
|---|---|---|
| V32/V34 TCP listen | **WORKS** (with one precision limit: V6ONLY invisible) | 13 cases, all 13 predictions held; one `TCP_LISTEN_CB` per CLOSE to LISTEN transition, port and address readable |
| V33/V35 UDP post_bind | **WORKS** (same V6ONLY limit) | 13 cases, all 13 predictions held; explicit bind reported, autobind and unbound client not |
| V35/V37 coexistence, verifier, flood | **WORKS** (with a correction to the multi-attach premise) | merged arm verified (+112 insns), correlation unchanged, overflow drops are counted and never block |
| V37/V39 kernel config | **WORKS** on the metal fixture kernel; **PARTIAL** overall | operator-example guest kernel not located |

## V32 / V34: TCP `TCP_LISTEN_CB` (13 cases)

Kernel basis: the op is not gated by a callback flag (`include/net/tcp.h:2822-2845 @ v7.0`) and is called only on the `TCP_CLOSE` to `TCP_LISTEN` transition
(`net/ipv4/af_inet.c:211,230 @ v7.0`). `post_bind4/6` was attached as well, so every TCP `bind()` also produces an `OTHER_POST_BIND` line; only `TCP_LISTEN_CB` lines are listener reports.

| Case | Predicted | Actual |
|---|---|---|
| T1 v4 wildcard | 1 listen event, `0.0.0.0:port` | held: `TCP_LISTEN_CB AF_INET local=0.0.0.0:56087` |
| T2 v4 loopback | address readable | held: `local=127.0.0.1:57159` |
| T3 v4 specific | address readable | held: `local=151.115.99.251:46105` |
| T4 v6 `[::1]` | 1 event | held: `AF_INET6 local=[::1]:34079` |
| T5 v6 wildcard V6ONLY=1 | event; V6ONLY unreadable, rc -22 | held: `local=[::]:56155 getsockopt(V6ONLY)_rc=-22` |
| T6 v6 wildcard V6ONLY=0 | indistinguishable from T5 | held: `local=[::]:35551 getsockopt(V6ONLY)_rc=-22` |
| T7 `listen()` x3 same socket | exactly 1 event | held: 1 `TCP_LISTEN_CB` |
| T8 listen, `shutdown(SHUT_RD)`, listen again | 2 events if the second listen succeeds | held: second listen rc=0, **2** `TCP_LISTEN_CB`, same port |
| T9 forked child listens | 1 event with the child pid | held: `pid=664301` (child), parent pid 664300 |
| T10 `SO_REUSEPORT` x2 | 2 events, same port | held: 2 `TCP_LISTEN_CB` on 49039 |
| T11 listener plus client connect plus accept | 1 event only | held: 1 `TCP_LISTEN_CB` (accepted child is not a listen transition) |
| T12 bind without listen | 0 listen events | held: only `OTHER_POST_BIND` |
| T13 `listen()` with no prior bind | 1 event with the autobound port | held: `TCP_LISTEN_CB local=0.0.0.0:40341`, no `OTHER_POST_BIND` |

Pasted (T5, T8, T13):

```
--- T5 v6 wildcard V6ONLY=1   [wl tcp6_wild_v6only1]
  wl> wl tcp6_wild_v6only1: listening [::]:56155 v6only=true
ACTUAL: wl exit=0; 2 event(s)
  ev> OTHER_POST_BIND  AF_INET6 local=[::]:56155 pid=664296 proto=6 type=1
  ev> TCP_LISTEN_CB    AF_INET6 local=[::]:56155 pid=664296 proto=6 type=1 getsockopt(V6ONLY)_rc=-22
--- T8 ...
ACTUAL: wl exit=0; 3 event(s)
  ev> OTHER_POST_BIND  AF_INET  local=0.0.0.0:47050 pid=664299 proto=6 type=1
  ev> TCP_LISTEN_CB    AF_INET  local=0.0.0.0:47050 pid=664299 proto=6 type=1
  ev> TCP_LISTEN_CB    AF_INET  local=0.0.0.0:47050 pid=664299 proto=6 type=1
--- T13 listen() with no prior bind (autobind)   [wl tcp4_listen_unbound]
ACTUAL: wl exit=0; 1 event(s)
  ev> TCP_LISTEN_CB    AF_INET  local=0.0.0.0:40341 pid=664305 proto=6 type=1
```

Edge cases and consequences:

- Port is host byte order and the address is readable for both families, so a loopback exclusion (`127.0.0.0/8`, `::1`) is implementable in the guest or in the BPF arm.
- **`IPV6_V6ONLY` is not observable.** `bpf_getsockopt(SOL_IPV6, IPV6_V6ONLY)` from the sockops arm returns -EINVAL (-22) on every AF_INET6 case, as predicted from
  `net/core/filter.c:5562-5573 @ v7.0` (`sol_ipv6_sockopt` handles only `IPV6_TCLASS` and `IPV6_AUTOFLOWLABEL`). A `V6ONLY=1` wildcard and a dual-stack wildcard are byte-identical reports.
  A report for `[::]:port` therefore cannot claim IPv4 reachability.
- The same `(protocol, port)` can be reported more than once (T8, T10), so the guest must keep the dedupe D13 already requires (duplicate lines are no-ops).
- A listener on a specific non-workload address (T3) is reported like any other; the guest does not know `workload_addr`, so only the loopback exclusion is available guest-side.
- The reported pid is the pid of the process that called `listen()`, including a forked child. D13 says reports carry no process identity, which stays correct.
- IPv4-mapped IPv6 binds (`::ffff:a.b.c.d`) were not exercised.

## V33 / V35: UDP `CgroupSock` post_bind4 / post_bind6 (13 cases)

aya 0.13.1 exposes the attach types: `CgroupSock` loaded and attached `post_bind4` and `post_bind6` (`bpftool cgroup show`: `cgroup_inet4_post_bind`, `cgroup_inet6_post_bind`).

| Case | Predicted | Actual |
|---|---|---|
| U1 v4 wildcard bind | 1 event | held: `UDP_POST_BIND AF_INET local=0.0.0.0:47101` |
| U2 v4 specific | 1 event | held: `local=151.115.99.251:47102` |
| U3 v4 loopback | address readable | held: `local=127.0.0.1:47103` |
| U4 `bind(0)` | assigned port visible | held: wl reports 42698, event `local=0.0.0.0:42698` |
| U5 autobind via `sendto` | 0 events | held: 0 events (kernel-assigned port 34698 not reported) |
| U6 connected UDP client, no bind | 0 events | held: 0 events |
| U7 `IP_BIND_ADDRESS_NO_PORT` plus `bind(0)` | 0 events | held: 0 events, port still 0 |
| U8 `SO_REUSEPORT` x2 | 2 events | held: 2 events on 47108 |
| U9 second bind to a taken port | first bind only | held: second bind `EADDRINUSE`, 1 event |
| U10 v6 wildcard V6ONLY=1 | 1 event | held: `AF_INET6 local=[::]:47110` |
| U11 v6 wildcard V6ONLY=0 | same shape as U10 | held: `AF_INET6 local=[::]:47111` |
| U12 v6 loopback | `[::1]` | held: `local=[::1]:47112` |
| U13 v6 `bind(0)` | assigned port | held: wl reports 39974, event `local=[::]:39974` |

Pasted (U4, U5, U11):

```
--- U4 v4 bind(port 0)   [wl udp4_port0]
  wl> wl udp4_port0: bind(port 0) -> assigned 42698
ACTUAL: wl exit=0; 1 event(s)
  ev> UDP_POST_BIND    AF_INET  local=0.0.0.0:42698 pid=664309 proto=17 type=2
--- U5 v4 autobind via sendto (no bind)   [wl udp4_autobind_sendto]
  wl> wl udp4_autobind_sendto: sendto without bind rc=1; kernel-assigned local port 34698
ACTUAL: wl exit=0; 0 event(s)
--- U11 v6 wildcard V6ONLY=0   [wl udp6_wild_v6only0 0.0.0.0 47111]
ACTUAL: wl exit=0; 1 event(s)
  ev> UDP_POST_BIND    AF_INET6 local=[::]:47111 pid=664316 proto=17 type=2
```

Edge cases and consequences:

- UDP has the same V6ONLY blindness: U10 and U11 reports are identical. A `bind(0)` that is not `IP_BIND_ADDRESS_NO_PORT` reports the real assigned port, so a UDP server on a kernel-chosen port is visible.
- A UDP socket using `IP_BIND_ADDRESS_NO_PORT` (U7) is never reported: the port is assigned later, outside the post_bind path (`af_inet.c:541-557 @ v7.0`, get_port and post_bind share one branch). A UDP server cannot be a declared listener this way, because it would have no known port at bind time.
- A TCP `bind()` also reaches `post_bind4/6` (`proto=6` lines above). The post_bind programs must return early for `protocol != 17` (IPPROTO_UDP) or the ring carries redundant TCP bind events.
- **Verifier constraints discovered by failing loads (pinned for the crafter, not design surface):**
  - `src_ip4` is readable only from `post_bind4` and `src_ip6` only from `post_bind6`; one shared reader is rejected with `invalid bpf_context access off=40 size=4` (`net/core/filter.c:8976-8989 @ v7.0`). The two programs must be separate (the spike splits them: `post_bind4` 149 insns, `post_bind6` 74 insns).
  - In the sockops arm, copying `ops.local_ip6` as an array through a modified ctx pointer is rejected (`dereference of modified ctx ptr R6 off=48 disallowed`); it must be copied element by element.
- **`post_bind` must return 1.** A program returning 0 makes `bind()` fail and releases the port (`net/ipv4/af_inet.c:549-557 @ v7.0`). The spike returns 1 in every path; no bind failed in 13 + 13 cases.

## V35 / V37: coexistence with the existing sockops shape, verifier budget, flood

### Verifier and correlation regression

```
verified_insns baseline: connect4=225 getpeername4=133 recvmsg4=65 sendmsg4=157 sock_release=41323 tcp_correlation=638
verified_insns listen  : connect4=225 getpeername4=133 listen_only=? post_bind4=149 post_bind6=74 recvmsg4=65 sendmsg4=157 sock_release=41323 tcp_correlation=750
```

The merged `tcp_correlation` (existing arms plus the `TCP_LISTEN_CB` arm with the V6ONLY `bpf_getsockopt` call, 112 more verified instructions) is accepted at 750 of the 1,000,000 limit. The other five copied programs are unchanged.
(`listen_only` shows `?` because that program is loaded only in the multi-attach test. `sock_release=41323` is the copied guest program's own count, unchanged between the two objects.)

Correlation behaviour is unchanged on the same scenario run against both objects (one accepted plain connection):

```
  [baseline] CHILD_CORRELATION while the connection is alive: 1 entr(ies), states=[3] (3 = REJECTED: no TCP option on a plain connect)
  [listen] CHILD_CORRELATION while the connection is alive: 1 entr(ies), states=[3] (3 = REJECTED: no TCP option on a plain connect)
  listen rig events during the regression scenario: 2
```

### Multi-attach: the premise is corrected

The source-reading note predicted that a second `Single` sockops attach replaces the first (`kernel/bpf/cgroup.c:630-634`). That holds for the legacy `BPF_PROG_ATTACH` path only.
aya 0.13.1 on this kernel attaches through `bpf_link_create`, and the kernel adds `BPF_F_ALLOW_MULTI` to every link attach (`kernel/bpf/cgroup.c:1502-1505 @ v7.0`). Measured:

```
M1 Single then Single
PREDICT: raw BPF_PROG_ATTACH would replace (cgroup.c:630-634); but aya attaches via bpf_link_create which forces BPF_F_ALLOW_MULTI (cgroup.c:1505) => both coexist
  first attach result: Ok(())
  second attach result: Ok(())
ID       AttachType      AttachFlags     Name
4127     cgroup_sock_ops multi           tcp_correlation
4128     cgroup_sock_ops multi           listen_only
  ACTUAL: tcp_correlation-arm events delivered=1 (counter seen+1 drops+0); listen_only invocations+1
M2 Multi then Multi
  first attach result: Err("`bpf_link_create` failed")
M3 Single then Multi
  first attach result: Ok(())
  second attach result: Err("`bpf_link_create` failed")
```

- With aya links, `CgroupAttachMode::Single` is effectively multi: two sockops programs at one cgroup both run on one `listen()` (the correlation-arm event was delivered and `listen_only` was invoked).
- `CgroupAttachMode::AllowMultiple` fails with EINVAL: a caller-supplied `BPF_F_ALLOW_MULTI` is outside `BPF_F_LINK_ATTACH_MASK` (`cgroup.c:1474-1484`). It is unusable through aya links and not needed.
- Consequence: merging the arm into the existing sockops program is **allowed and verified**, and a separate second sockops program is **also viable**. The spike did not need either; D13 can keep "an arm in the existing program" (one program, +112 insns, no extra attach) as the lower-surface choice.

### Flood and latency (20,000 distinct sockets held open per run, 3 repetitions)

Configs: `ctl` = cgroup with no BPF; `base` = copied guest programs, no listen arm; `big` = merged arm plus post_bind, 4 MiB ring; `small` = same, 4096-byte ring, **not drained during the run**.

```
### UDP bind x 20000 distinct sockets held open
  [big  ] rep0 ... bind mean/p99/max ns=(1185, 2173, 18959)  ring: seen+20000 drops+0 delivered=20000
          accounting: seen=20000 == delivered+drops=20000  expected_seen=20000 -> OK
  [small] rep0 ... bind mean/p99/max ns=(1222, 2324, 203267)  ring: seen+20000 drops+19927 delivered=73
          accounting: seen=20000 == delivered+drops=20000  expected_seen=20000 -> OK
    ctl    bind      1067     2133      27401
    base   bind      1053     2066      78351
    big    bind      1194     2226      75674
    small  bind      1191     2276      87565

### TCP bind+listen x 20000 distinct sockets held open
  [small] rep0 ... listen=(1053, 1823, 8102)  ring: seen+40000 drops+39927 delivered=73
          accounting: seen=40000 == delivered+drops=40000  expected_seen=40000 -> OK
    ctl    bind    475439  1634494    2548320   | listen      949     1658      23592
    base   bind    473282  1630962    3347279   | listen      992     1743      10846
    big    bind    471687  1629947    2524524   | listen     1078     1879      24694
    small  bind    475718  1634507    2927955   | listen      1064     1819       9106
```

(Mean-of-3 columns are mean, p99, max in ns per syscall.)

- Accounting holds in all 12 flood runs: `seen == delivered + drops` and `seen == expected`. No event is lost silently. A 4096-byte ring delivered 73 of 20,000 events (73 x (48-byte event + 8-byte ringbuf header) = 4,088 of 4,096 bytes) and counted 19,927 drops.
- The workload is not slowed or blocked by overflow: `small` (nearly always full) and `big` (never full) have the same per-call latency within noise (UDP bind 1191 vs 1194 ns; TCP listen 1064 vs 1078 ns). A full ring costs no more than an empty one because the program only counts the failed `output`.
- Added cost of the hooks relative to the copied guest baseline is about 90 to 140 ns per UDP bind (1053 to 1194 ns) and about 90 ns per TCP listen (992 to 1078 ns). TCP `bind()` is 470 to 475 microseconds in every config, dominated by port allocation with 20,000 held sockets, with no difference across configs.
- The test used a drain of zero during the run (worst case). Overflow is the fail-closed direction required by ADR-0268: a dropped report means the port is never reported ready, so the startup probe times out and the allocation fails closed.

## V37 / V39: kernel config

```
uname -r: 7.0.0-38-generic   uname -m: x86_64   systemd-detect-virt: none
--- host kernel config (/boot/config-7.0.0-38-generic)
CONFIG_BPF=y  CONFIG_BPF_SYSCALL=y  CONFIG_BPF_JIT=y  CONFIG_CGROUPS=y  CONFIG_CGROUP_BPF=y
CONFIG_NET_HANDSHAKE=y  CONFIG_INET=y  CONFIG_IPV6=y  CONFIG_DEBUG_INFO_BTF=y
--- bpftool feature probe kernel (filtered)
CONFIG_BPF_SYSCALL is set to y
CONFIG_CGROUP_BPF is set to y
eBPF program_type cgroup_sock is available
eBPF program_type sock_ops is available
eBPF program_type cgroup_sock_addr is available
eBPF map_type percpu_array is available
eBPF map_type ringbuf is available
fixture kernel == /boot/vmlinuz-7.0.0-38-generic (byte-identical)
no IKCFG config found in /srv/vm/overdrive-testing/kernel
```

- Everything D13 needs is present on the metal fixture kernel. `bpf_getsockopt` is available to sockops; the real loads and attaches of `sock_ops`, `post_bind4/6` (and the five other guest program types) succeeded.
- The fixture guest kernel (`/srv/vm/overdrive-testing/kernel`) is byte-identical to the host's `/boot/vmlinuz-7.0.0-38-generic`, so the host `/boot/config-7.0.0-38-generic` is its config. The image carries no embedded `IKCONFIG`, so a guest cannot read its own config from `/proc/config.gz`; the appliance image config assertion (ADR-0068 section 7) is the place to hold the requirement list.
- Not covered: "any guest kernel the operator example uses" was not located in this spike, hence PARTIAL overall.

## Design implications for D13's contract

1. **Merge the TCP arm into the existing sockops program (verified), single program stays.** +112 verified instructions, correlation behaviour unchanged. A second sockops program is also legal because aya attaches through links (always multi); the earlier "Single replaces" worry does not apply. D13 need not pin Single-versus-Multi attach semantics.
2. **UDP needs two `CgroupSock` programs, `post_bind4` and `post_bind6`** (the context fields differ per attach type), not one. Both must **return 1**; returning 0 breaks the workload's `bind()`.
3. **Filter to UDP in the post_bind programs** (`protocol == 17`); TCP binds reach them too and would otherwise add ring noise.
4. **`IPV6_V6ONLY` is invisible** on both hooks. The report for an AF_INET6 wildcard means "a listener exists on `[::]:port`" and must not be read as IPv4 reachability. The report stays `LISTEN <proto> <port>` with no family; the contract text should not promise dual-stack knowledge.
5. **Loopback exclusion is implementable** from both hooks (v4 `127.0.0.0/8`, v6 `::1` readable). Wildcard and specific addresses are indistinguishable from "serves the workload address" without `workload_addr`, which the guest does not hold; D13's "loopback-only is not a report" holds, nothing stronger is available.
6. **Duplicate reports occur** (re-listen after `shutdown`, `SO_REUSEPORT`); the existing "duplicates are no-ops" clause is needed, as written.
7. **UDP `IP_BIND_ADDRESS_NO_PORT` sockets are never reported**, and autobind and unbound clients are not reported: consistent with D13's declared-port model.
8. **Overflow is fail-closed and free for the workload**, with exact accounting. Ring size matters only in the pathological case of many listeners before the declared one is reported; the spike measured 4 MiB (no drops at 20,000 UDP and 40,000 TCP-side events) and 4096 bytes (drops counted), not intermediate sizes, so a minimum size is not established and is left to the crafter.

## Gate recommendation

**PROMOTE** D13's observation half (ADR-0272) with items 1 to 7 above folded into the contract; the operator-example guest-kernel config check (V37/V39) remains the one unproven premise.

## Cleanup

No spike cgroup (`/sys/fs/cgroup/lh-*`), pinned map (`/sys/fs/bpf/lh-*`), spike process or spike BPF program remained on the metal host after the run (`ls -d` and `bpftool prog show` checked).
