# Competitive Research — unconnected-udp-sendmsg4 DIVERGE

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Wave:** DIVERGE (Phase 2) · **Research depth:** Comprehensive
· **Confidence:** High (kernel UAPI + Cilium docs + kernel-commit primary
sources, fresh-verified 2026-06-05)

> **Provenance note.** Source verification was performed directly by Flux
> (kernel.org / docs.ebpf.io UAPI, docs.cilium.io, LWN, and the torvalds/
> linux commit tree) rather than dispatched to `nw-researcher` as a
> separate sub-agent — the Task tool is unavailable inside a sub-agent
> context (the same constraint recorded in the sibling
> `udp-service-support/diverge/review.yaml`). Every load-bearing claim
> below carries a primary-source URL and an access date; the kernel-version
> and reply-path-mechanics claims (the hard viability gate and the crux)
> cross-reference ≥ 2 independent sources each.

---

## How existing products serve the validated job

The job (job-analysis.md § 2): *a same-host client's datagram to a service
VIP must reach a healthy backend, and the reply must appear to come from
the VIP — regardless of whether the client connected first.* Below: how
real systems serve this for the **unconnected**-UDP case specifically.

---

### Competitor 1 — Cilium (the dominant reference; same primitive family as ADR-0053)

**What it does.** Cilium's kube-proxy-replacement socket-LB intercepts
**four** BPF cgroup hooks for service translation: `connect4/6`,
`sendmsg4/6`, `recvmsg4/6`, and `getpeername4/6`. Per the
[kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/)
(verified 2026-06-05):

> *"upon `connect` (TCP, connected UDP), `sendmsg` (UDP), or `recvmsg`
> (UDP) system calls, the destination IP is checked for an existing
> service IP and one of the service backends is selected as a target."*

**The load-bearing finding for #200 — Cilium uses `recvmsg4` AND it is
not optional.** For unconnected UDP, Cilium runs **both** `sendmsg4`
(request: VIP→backend destination rewrite) **and** `recvmsg4` (reply:
backend→VIP source rewrite). Per the docs synthesis (verified 2026-06-05):
*"when a backend pod sends a UDP reply, the [recvmsg] hook translates the
source address from the backend pod IP back to the service VIP … This
ensures clients receive responses appearing to originate from the service
endpoint."* The reason is structural: **UDP is connectionless, so there is
no per-connection conntrack state to fix up the reverse direction** — the
translation must be installed on both the request and the reply path
*independently*.

**Where it fails the job (for Overdrive directly).** Nothing about the
mechanism fails the job — it is the proven solution. The only
"failure" is scope: Cilium's full four-hook set (incl. `getpeername4`,
IPv6) is more than Phase 1 single-node IPv4 needs. Overdrive can adopt the
`sendmsg4` + `recvmsg4` pair and defer `getpeername4` / IPv6.

**Key assumption Cilium makes about user behaviour.** That clients use the
unconnected-UDP idiom (they do — DNS resolvers dominate UDP service
traffic) and that those clients **validate the reply source** (they do —
see Competitor 4), which is precisely why `recvmsg4` is mandatory rather
than nice-to-have.

Sources: [Cilium "Kubernetes Without kube-proxy"](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/)
(accessed 2026-06-05, High); cross-referenced with the same doc's
cgroup-hook list and the upstream kernel commit (Competitor 3).

---

### Competitor 2 — The Linux kernel `cgroup_sock_addr` UAPI itself (the primitive's own design intent)

**What it does.** The kernel's own design of the `cgroup_sock_addr`
program type splits UDP coverage exactly along the connected/unconnected
line. Per Andrey Ignatov's (Meta) original "Hooks for sys_sendmsg" series
([LWN 755902](https://lwn.net/Articles/755902/), kernel **4.18**, verified
2026-06-05):

> *"This makes UDP support complete: connected UDP is handled by
> `sys_connect` hooks, unconnected by `sys_sendmsg` ones."*

The series states the sendmsg hooks "override source IP (including the
case when it's set via `cmsg(3)`) and destination IP:port for unconnected
UDP (slow path)."

**Kernel since-version & UAPI field availability (the hard viability gate
— all comfortably below Overdrive's 5.10 LTS floor):**

| Attach type | SEC name | Introduced | Notes |
|---|---|---|---|
| `BPF_CGROUP_INET4_CONNECT` | `cgroup/connect4` | **4.17** | The locked ADR-0053 path. |
| `BPF_CGROUP_UDP4_SENDMSG` | `cgroup/sendmsg4` | **4.18** | Andrey Ignatov / Meta (LWN 755902). The unconnected-UDP request-path hook. |
| `BPF_CGROUP_UDP4_RECVMSG` | `cgroup/recvmsg4` | **4.20** | Daniel Borkmann's "fix unconnected udp hooks" (`983695fa6765`, backported to 4.19 stable). The reply-path source-rewrite hook. |
| `BPF_CGROUP_INET4_GETPEERNAME` | `cgroup/getpeername4` | 5.8 | Later; out of scope here. |

**`bpf_sock_addr` context-field availability for sendmsg4 (verified):**
- `user_ip4` — **writable** (4-byte write); per
  [docs.ebpf.io BPF_PROG_TYPE_CGROUP_SOCK_ADDR](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_CGROUP_SOCK_ADDR/)
  this field's 4-byte write is *"only valid for `BPF_CGROUP_UDP4_SENDMSG`"*
  among the UDP hooks (verified 2026-06-05). This is the field the
  destination rewrite writes.
- `user_port` — **writable**, network byte order (the low-16-NBO-in-u32
  hazard in `.claude/rules/development.md` applies identically to sendmsg4
  as to connect4).
- `protocol` — populated with the IANA L4 number (IPPROTO_UDP=17 for a UDP
  socket). This is the **same zero-translation proto source** ADR-0053
  Amendment 2 pinned for connect4; it slots into `LocalServiceKey.proto`
  with no translation table. (Robustness: `type` = SOCK_DGRAM=2 is the
  documented fallback, same clause ADR-0053 § Amd 2 carries.)
- For **recvmsg4**: the BPF program is called *"whenever a non-NULL
  `msg->msg_name` was passed, independent of `sk->sk_state` being
  `TCP_ESTABLISHED` or not"* (Borkmann, `983695fa6765`) — i.e. it fires for
  the unconnected `recvfrom`/`recvmsg` path, which is exactly where the
  reply source must be rewritten. `user_ip4`/`user_port` carry the
  *source* sockaddr the application will see; rewriting them back to the
  VIP is the "reverse translation."

**Where it fails the job — sendmsg4 ALONE.** The kernel UAPI is explicit
that sendmsg4 handles the *request* path only. It does NOT touch the reply.
A sendmsg4-only implementation delivers the query to the backend but leaves
the reply sourced from the backend IP — which the canonical client rejects
(Competitor 3, Competitor 4). The kernel's own answer to "is recvmsg
needed?" is yes: the recvmsg hooks were added precisely because sendmsg
alone was insufficient.

Sources: [LWN 755902 "BPF hooks for sys_sendmsg"](https://lwn.net/Articles/755902/)
(accessed 2026-06-05, High); [docs.ebpf.io BPF_PROG_TYPE_CGROUP_SOCK_ADDR](https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_CGROUP_SOCK_ADDR/)
(accessed 2026-06-05, High); [docs.kernel.org libbpf program_types](https://docs.kernel.org/bpf/libbpf/program_types.html)
(SEC-name mapping, accessed 2026-06-05, High).

---

### Competitor 3 — Kernel commit `983695fa6765` ("bpf: fix unconnected udp hooks") — the decisive reply-path evidence

This is the single most decisive source for the crux question (*is
recvmsg4 load-bearing, or can the kernel auto-handle the reply source on
the same host?*).

**What it shows.** Per the [commit](https://github.com/torvalds/linux/commit/983695fa6765)
(Daniel Borkmann, verified 2026-06-05), the commit message demonstrates
the **exact failure mode** with a real DNS client: after a sendmsg4 hook
rewrites a DNS query's destination to a backend (`8.8.8.8:53`), `nslookup`
receives the reply *from* `8.8.8.8:53` but **expects it from the original
service IP** (`147.75.207.207:53`) — and **rejects it**. The fix adds
`BPF_CGROUP_UDP4_RECVMSG` / `UDP6_RECVMSG` so a program can *"replace the
current `sockaddr_in{,6}` with the original service IP"* on receive, making
the hooks *"more transparent to the application."*

**Why this kills the "maybe recvmsg4 is unnecessary" hypothesis (the #200
flag).** The kernel maintainers found that sendmsg-only was a real,
shipped bug for unconnected UDP, and the named victim was a DNS resolver —
Overdrive's exact canonical client. There is **no kernel auto-rewrite** of
the reply source for the unconnected sendmsg-rewritten path; the source
arrives as the backend's, and the client's source-validation logic
discards it. recvmsg4 is the documented, upstream answer. **This converts
"#200 should consider recvmsg4" from a hedge into a confirmed
requirement** for any client that validates its reply source.

**Same-host nuance (important for Overdrive's specific scope).** The
commit's example is a *remote* backend (8.8.8.8). For Overdrive's
single-node case the backend is on the *same host* (loopback / host veth),
so the backend's reply genuinely originates from the backend's real IP on
the same machine. The kernel does not synthesise a VIP source for it; the
reply sockaddr the client's `recvfrom` sees is the backend's, not the VIP.
So the same-host case has the **same** source-mismatch as the remote case —
recvmsg4 is needed identically. (The only scenario where it would NOT be
needed is if the backend were bound to the VIP itself and replied from it
— but in the LOCAL_BACKEND_MAP model the backend binds its own real
address, by construction.)

Sources: [torvalds/linux commit `983695fa6765`](https://github.com/torvalds/linux/commit/983695fa6765)
(accessed 2026-06-05, High); [PATCH 4.19 stable backport](https://lore.kernel.org/lkml/20190702080128.014501200@linuxfoundation.org/)
(accessed 2026-06-05, High — confirms it shipped to 4.19/4.20).

---

### Competitor 4 — DNS resolvers (glibc / musl / `dig` / systemd-resolved) — the client whose behaviour is the constraint

**What they do (the unconnected idiom).** The canonical stub resolvers
send queries with **unconnected** UDP sockets — `sendto(VIP:53, query)` /
`recvfrom()` per query, no `connect()`. (glibc `res_send` historically
uses unconnected sockets for the multi-server case; `dig`/BIND tools
likewise.) This is the precise idiom that bypasses connect4.

**Where the connect4 path fails them.** ADR-0053's connect4 only fires on
`connect(2)`. A resolver that never connects is never intercepted → the
datagram leaves with `dst = VIP`, the kernel routes the VIP nowhere useful
on a single-node host, and the query is lost. This is the operator-visible
gap #200 names: a same-host DNS UDP service is unreachable from the common
resolver.

**Where they fail a sendmsg4-ONLY fix (the reply-source-validation
behaviour).** DNS clients **validate the source of the reply** as an
anti-spoofing measure — a reply whose source address/port does not match
the destination they sent to is discarded (this is standard stub-resolver
hardening; source-address validation is the documented defense against
off-path DNS cache poisoning). So a sendmsg4-only fix that delivers the
query but lets the reply arrive from the backend IP produces a resolver
that *times out* — strictly worse than a clean failure, because it looks
like a flaky service. This is the same behaviour the kernel commit
demonstrated with `nslookup`.

**Non-obvious sub-case (systemd-resolved).** systemd-resolved is a *stub
resolver that owns UDP 5353/53 on many hosts* — it both consumes DNS
upstream (unconnected) and, in the Lima VM, **occupies UDP 5353**, which is
the documented fixture hazard in `.claude/rules/debugging.md` § 11. This is
relevant context for Tier-3 testing of any sendmsg4 option (the test
fixture must not collide with systemd-resolved's port), and it is itself a
non-cgroup "same-host DNS LB" pattern (a userspace stub that forwards) —
see Competitor 5.

Sources: [Mathy Vanhoef — "Recvfrom Problems & Forging ICMP Unreachable"](https://www.mathyvanhoef.com/2011/10/recvfrom-problems-forging-icmp.html)
(reply-source-validation behaviour, accessed 2026-06-05, Medium-High);
[The Closed Resolver Project (arXiv 2006.05277)](https://arxiv.org/pdf/2006.05277)
(source-address validation as anti-spoofing, accessed 2026-06-05, High);
`.claude/rules/debugging.md` § 11 (systemd-resolved UDP 5353 fixture
hazard, in-repo).

---

### Competitor 5 — `BPF_PROG_TYPE_SK_LOOKUP` (the NON-OBVIOUS alternative — different category, same job)

This is the structurally-different alternative the dispatch asked to
re-examine: the connect4 ADR rejected SK_LOOKUP (Alt A) on **return-path**
grounds specific to **TCP** (the SYN-ACK source mismatch). Does that
rejection transfer to the **unconnected-UDP** case, where there is no
handshake?

**What it does.** SK_LOOKUP runs *"when transport layer is looking up a
… socket for a packet (UDP)"* — *"at the last possible point on the receive
path"* — and selects the destination socket via `bpf_sk_assign` **without
rewriting the wire-visible destination address**. Per
[kernel.org prog_sk_lookup.rst](https://docs.kernel.org/bpf/prog_sk_lookup.html)
(verified, same doc cited by the connect4 research): *"The destination
address is NOT modified; the program selects which socket receives the
packet."* The application sees the **VIP** via `getsockname(2)`. Kernel
floor: **5.9** (below Overdrive's 5.10). Attaches **per-netns**, not
per-cgroup.

**Does the connect4 SK_LOOKUP rejection transfer to unconnected UDP?
Partially — and the part that *doesn't* transfer is real.** The connect4
rejection (ADR-0053 Alt A) hinged on the **TCP** SYN-ACK return path: with
SK_LOOKUP the backend's stack constructs the SYN-ACK with `src=backend`,
the client expects `src=VIP`, and the packet is dropped. **For inbound
delivery, SK_LOOKUP is actually cleaner** — it delivers the datagram to the
backend's listening socket with the VIP intact (no destination rewrite,
no sendmsg4 needed). **BUT the reply-path problem does NOT disappear** — it
*moves*. When the backend `sendto`s its reply, its kernel uses the
backend's real IP as the source (the backend's socket is bound to its own
address, not the VIP); the client receives a reply from the backend IP and
discards it on source validation — the **same O2 failure** as sendmsg4-
only. SK_LOOKUP redirects *inbound socket selection*; it does **not**
rewrite the *outbound reply source*. So SK_LOOKUP for unconnected UDP
would still need a reply-path fixup (a cgroup `sendmsg4`/`getsockname`-side
rewrite on the backend, or accepting the limitation) — it does **not**
give a free reply path.

**Where it fails the job (for Overdrive's scope).** Two real problems:
(1) **No reply-path source rewrite** — the inbound win is offset by the
unsolved reply source (above). (2) **Phase-2 netns retirement** — SK_LOOKUP
is per-netns; once Phase 2 per-workload netns lands, a host-netns
SK_LOOKUP program cannot redirect to a socket in the workload's netns (the
finding 10A in the connect4 research). It is a stepping-stone, like the
connect4 research flagged for the TCP case. (3) **Divergence from the
shipped same-host primitive** — Overdrive already shipped the cgroup
connect4 path + LOCAL_BACKEND_MAP; SK_LOOKUP is a *different map shape*
(socket-fd registry, not address rewrite) and a *different attach model*
(per-netns), so it does not reuse the connect4 surface at all (worst-case
O4 — marginal surface).

**Honest verdict on the non-obvious alternative.** SK_LOOKUP's
return-path rejection does **not** cleanly transfer (no TCP handshake to
break on the inbound leg) — so it deserves a live slot in the option study
rather than a copy-paste rejection. But it does **not** solve the reply
source for free, and it does not reuse the shipped connect4 surface. It is
a genuine, non-strawman contender that the taste matrix will score on its
merits.

Sources: [kernel.org prog_sk_lookup.rst](https://docs.kernel.org/bpf/prog_sk_lookup.html)
(accessed 2026-06-05, High); [LWN 825103 "Socket lookup with BPF" (Sitnicki/Cloudflare)](https://lwn.net/Articles/825103/)
(accessed 2026-06-05, High); `docs/research/dataplane/same-host-backend-delivery-architecture.md`
§ Q1B/Q4B/Q10 (in-repo prior research, the connect4 SK_LOOKUP analysis
this re-examines).

---

### Competitor 6 — IPVS / kube-proxy iptables (the "what we deliberately don't do" reference)

**What they do.** IPVS keys virtual services on `{protocol, addr, port}`
natively and NATs UDP at the netfilter layer; kube-proxy iptables mode
emits `-p udp -j DNAT` rules. Both handle unconnected UDP transparently
*because* they operate on packets at the netfilter hook, not on the
syscall — there is no connect-vs-unconnected distinction at that layer.
IPVS maintains UDP "connection" entries (a timeout-based pseudo-conntrack)
so the reply is reverse-NATed automatically.

**Where it fails the job (for Overdrive).** It is the **architecturally
disqualified** option. Vision principle 2 ("eBPF is the dataplane; no
userspace proxies in the data path") and ADR-0053 Alt F reject iptables/
IPVS for the same-host case on the explicit grounds that "Overdrive's whole
dataplane premise is eBPF, not iptables … the cost is permanently coupling
the platform to a deprecated kernel subsystem." Adopting netfilter NAT for
the one unconnected-UDP case re-introduces exactly the subsystem the whole
stack was built to obviate. It is included here as the honest floor
reference (it *would* solve the job) and as the DVF-near-disqualified
option the matrix must still score, not as a recommendation.

Sources: `docs/research/dataplane/service-map-l4-proto-keying-research.md`
§ Q2b (IPVS `{protocol,addr,port}` keying + UDP pseudo-conntrack, in-repo,
cross-referencing kernel UAPI `ip_vs_service_user`); ADR-0053 Alternatives
§ F (in-repo, the eBPF-not-iptables rejection); vision.md principle 2
(in-repo SSOT).

---

## Synthesis — what the evidence establishes for the option study

1. **The connect4/sendmsg4 split is the kernel's OWN design** (LWN 755902:
   "connected UDP by connect, unconnected by sendmsg") and **Cilium's
   shipped shape** — adopting sendmsg4 for #200 is exact parity with both
   the primitive's design intent and the dominant production reference.
   This is the strongest Desirability/Feasibility signal.
2. **recvmsg4 is load-bearing, not optional, for the canonical client.**
   Kernel commit `983695fa6765` demonstrates a DNS client (`nslookup`)
   rejecting the backend-sourced reply; the fix IS recvmsg4. Cilium ships
   recvmsg4 for exactly this. The #200 "consider recvmsg4 (verify; may be
   out of scope)" hedge is **resolved by the evidence: it is required**
   for any reply-source-validating client (which DNS resolvers are). This
   is the central discriminator (O2) between the sendmsg4-only and
   sendmsg4+recvmsg4 options.
3. **Kernel viability is not in question.** sendmsg4 (4.18), recvmsg4
   (4.20), `bpf_sock_addr.protocol`/`user_ip4`/`user_port`
   populated/writable for these contexts — all below the 5.10 floor. No
   kernel-floor bump for any cgroup-hook option. (SK_LOOKUP at 5.9 is also
   below the floor.)
4. **The SK_LOOKUP rejection does NOT cleanly transfer from the connect4
   (TCP) case** — there's no handshake to break on the inbound leg — so
   SK_LOOKUP is a live contender, but it does **not** solve the reply
   source for free and does **not** reuse the shipped connect4 surface.
5. **iptables/IPVS is architecturally disqualified** (vision principle 2;
   ADR-0053 Alt F) — included as the honest floor, scored, near-eliminated
   on DVF Viability.
6. **No Tier-2 backstop** for `cgroup_sock_addr` (ENOTSUPP ≤ 6.8) means
   every cgroup-hook option's correctness — *including the reply-path
   source rewrite* — is a **Tier-3-only** gate (real `sendto`/`recvfrom`
   through the cgroup, `tcpdump`/`bpftool` evidence). This is O5 and a real
   cost the matrix weighs; it is identical for sendmsg4 and recvmsg4 (same
   program type), so it does not by itself separate Option 1 from Option 2.

---

## Gate check (Phase 2 — G2)

- [x] **3+ real competitors named** — Cilium, the kernel `cgroup_sock_addr`
  UAPI, kernel commit `983695fa6765`, DNS resolvers (glibc/musl/dig/
  systemd-resolved), SK_LOOKUP, IPVS/kube-proxy = **6** distinct
  references with cited behaviours.
- [x] **≥ 1 non-obvious alternative (different category, same job)** —
  `BPF_PROG_TYPE_SK_LOOKUP` (a per-netns socket-selection primitive, a
  different category from connect-time address rewrite) is examined on its
  merits, with the connect4-rejection-transfer question worked through
  honestly. Secondary non-obvious references: systemd-resolved stub-forward
  pattern; IPVS UDP pseudo-conntrack.
- [x] **No generic market claims** — every claim cites a named product/
  kernel-artifact, a specific behaviour, and a source URL with access date.
  The crux (recvmsg4 load-bearing) is grounded in a primary kernel commit,
  not opinion.
- [x] **Fresh kernel-UAPI verification** — sendmsg4/recvmsg4 since-versions,
  field writability, and reply-path mechanics verified 2026-06-05 against
  kernel.org / docs.ebpf.io / LWN / the torvalds commit tree.

**G2: PASS.**
