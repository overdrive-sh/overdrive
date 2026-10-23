# TCP correlation Fast Open handshake candidate — native findings

## Verdict

**WORKS and selected.** One C/CO-RE `fentry/tcp_check_req` program records the
pre-validation Fast Open candidate, and the existing Rust/Aya SockOps state
callback promotes it after validation. The mechanism passed in a fresh
disposable x86_64 Cloud Hypervisor guest on Metal through production GuestKit,
Linux `7.0.0-38-generic`, Aya `0.13.1`. All runtime BPF programs/maps, qdiscs,
sockets and sysctls existed inside the guest; the physical host supplied the
build, KVM and vhost-vsock only.

Evidence:

- `spike-scratch/netns-density-295-tcp-correlation-scope-validation/core-trace-native.log`
- log SHA-256:
  `679f659450ae2a0d4a2c112dbe64dc07c28a3e47cded4475209cc93df00fb293`
- C/CO-RE source SHA-256:
  `ce0f92d0c7f6d55c5747aeec961414192ef66e2675c673c3b3d7148eccf6d920`
- Rust SockOps source SHA-256:
  `dfa0be81606ae3aa59bac5f8dfcc6bc24619328f5ab61b03c4ee4dba0691cd22`
- probe SHA-256:
  `2f48c006afc231de9d54c869a8ddb231e70d29ea00877c9d63cb034aa94486f7`
- guest runner SHA-256:
  `3daa31d162935919eb95fb4f5ad0da4de18f260f93876445f23da72e6010a87d`

The log ends `VERDICT=WORKS`. The real data-in-SYN Fast Open producer remained
open and sent no FIN, close or post-handshake application data before `Ready`.
A live direct Fast Open producer reached `Rejected`. Ordinary and forced-
syncookie identity, terminal/delete monotonicity and preterminal release also
passed. The candidate counter stayed exactly zero across established data.

## Why the hook is exact

Linux makes the complete `doff`-sized TCP header linear before child
processing (`vendors/linux/net/ipv4/tcp_ipv4.c:2168-2188`). The selected child
reaches `tcp_rcv_state_process` with its socket lock held
(`tcp_ipv4.c:1851-1907`; `net/ipv4/tcp_minisocks.c:992-1017`). A Fast Open
child calls `tcp_check_req(..., fastopen=true, ...)` under that lock before
`tcp_validate_incoming` and `tcp_ack`
(`net/ipv4/tcp_input.c:7228-7266`). Only the accepted path calls
`tcp_set_state(TCP_ESTABLISHED)` (`:7268-7289`), whose opted-in SockOps state
callback runs under the same lock before the state store
(`net/ipv4/tcp.c:3034-3064`). Established data takes
`tcp_rcv_established`, so it never invokes this candidate.

The fentry receives the real `struct sock *` and `struct sk_buff *`. It returns
on `fastopen == false`, obtains the exact child cookie for Fast Open, and
returns unless the existing child row is nonterminal. `BPF_CORE_READ(skb,
data)` relocates the real kernel field; `bpf_probe_read_kernel` copies only the
20-byte base TCP header and bounded 18-byte windows within the maximum 60-byte
header. Exact option kind, length and magic gate metadata decoding. No
application payload is read and the program cannot change the packet or kernel
function return.

## Monotonicity and deletion finality

The same ordinary preallocated 8,194-entry child map and lock-free 24-byte
value are reused:

```text
u32 state; u32 pad; CorrelationMeta meta
```

The candidate and existing SockOps promoter run under the child lock and
whole-row replace with `BPF_EXIST`. Linux replaces a complete hash element
under the bucket lock and publishes it through RCU
(`vendors/linux/kernel/bpf/hashtab.c:1166-1264`). The preallocated map keeps a
per-CPU extra element for an existing-key replacement even when full
(`:1070-1115`).

Owner consumption and `sock_release` may race those writers. If deletion wins,
the later `BPF_EXIST` returns `ENOENT` and cannot recreate the key. If
replacement wins, the later deletion removes it. No post-passive path inserts
a child row. Invalid ACKs may replace one charged candidate; only a later
validated state transition makes it terminal. This preserves terminal
monotonicity, one-time consumption, exact cleanup and admission below the
combined 4,096 owner bound without an in-value lock or another map.

## Native cases

The executed matrix recorded:

```text
scope_control ordinary_socket_bypassed_trace_candidate=true
ordinary_exact ...
direct_rejected ...
syncookie_exact ...
fastopen_wait_open ...
fastopen_direct_rejected_open ...
terminal_delete_monotonic ...
pending_release_final ...
fastopen_exact ...
VERDICT=WORKS
```

The PRIO/NETEM fixture delayed real final ACKs. After terminal state and after
one-time deletion, ordinary established sends could neither regress metadata
nor recreate the row. Reset-closing a preterminal accepted child before delayed
traffic likewise left the row absent. No raw packet, seeded child row, new map
or test-only production branch was used.

## Build and loader boundary proved

The successful candidate is C/CO-RE, compiled with Clang 21 against a
`bpftool`-generated `vmlinux.h`, standard libbpf headers and the x86 target
macro. Aya `0.13.1` loaded the separate object and reused the child map through
the same temporary pin path as the Rust SockOps object. Both pins and their
directory were removed before program load/attach. No Aya upgrade was used.

An earlier Rust dynptr object failed in Aya's object linker because its
hand-written extern did not express the kfunc relocation. That failure applies
only to that prototype; it is not an Aya-version conclusion. The selected
standard-helper path uses `BPF_CORE_READ` plus `bpf_probe_read_kernel` and
loaded successfully through Aya `0.13.1`.

## Scope and practical limits

The fentry is global and dispatches for each system `tcp_check_req`. Ordinary
calls return at the Fast Open gate; unrelated Fast Open calls return after
cookie/map lookup; only an exact nonterminal correlation row pays header reads
and `BPF_EXIST` replacement. It does not run on established data. No numeric
CPU, throughput or tail-latency result was measured.

The receipt is x86_64 Linux `7.0.0-38-generic`. CO-RE relocates kernel fields,
but fentry still requires a compatible `tcp_check_req` BTF prototype. The
production design therefore fails load before READY on missing/incompatible
BTF, symbol, prototype or relocation. The build path supports an arm64 target
macro structurally; this spike supplies no native aarch64 receipt.

## Disposition

This handshake-only mechanism supersedes both broader cgroup-ingress
candidates. The private-intake-cgroup ingress remains valid fallback evidence,
but it adds private-cgroup lifecycle, an ingress-only value lock and dispatch
on established intake packets. Production follows the accepted
`TCP-CORRELATION` contract in the feature delta and ADRs, not the scratch
source.
