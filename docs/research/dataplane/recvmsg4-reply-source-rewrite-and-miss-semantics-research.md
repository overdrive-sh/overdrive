# Research: `cgroup/recvmsg4` reply-source rewrite mechanism and reverse-miss semantics

**Date**: 2026-06-05 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 9 (primary kernel source/commit, primary kernel selftest patch, Cilium production source, eBPF reference docs)

> **Decision this informs (D3, feature `unconnected-udp-sendmsg4`, GH #200):**
> When a `BPF_CGROUP_UDP4_RECVMSG` (`cgroup/recvmsg4`) program looks up the reply's
> `backend → VIP` reverse mapping and **MISSES**, what can the program actually do,
> and what is the honest, achievable "no backend-IP leak" guarantee?
>
> The DISCUSS ACs (US-03 / KPI K5) were written in XDP/wire terms — "no
> backend-IP-sourced reply reaches a client," "tcpdump shows NO backend-IP-sourced
> reply left the host." The architect flagged that recvmsg4 is NOT XDP and may be
> unable to make a wire-level guarantee. This doc establishes the real mechanism
> before the AC is reframed.

**Labeling convention:** every load-bearing claim is tagged **[VERIFIED-PRIMARY]**
(kernel source by file/commit, Cilium source by file/function, or a primary
kernel.org/LWN doc) or **[INFERRED]** (reasoned from primary facts but not directly
stated by any source).

---

## Executive Summary

The single most consequential finding flips the framing in the **opposite**
direction from the architect's hedge: **`cgroup/recvmsg4` cannot deny the syscall
at all.** The kernel verifier hard-restricts `BPF_CGROUP_UDP4_RECVMSG` /
`UDP6_RECVMSG` programs to a return-value range of exactly `[1, 1]` — they can
**only return 1 (proceed)**. A program that returns 0 is **rejected at load time**
by the verifier with `"At program exit the register R0 has smin=0 smax=0 should
have been in [1, 1]"`. So "drop on miss" is **not** available at the syscall layer
for recvmsg4. This contradicts the Q1 hypothesis (which predicted return-0 → EPERM)
and is the crux correction for D3. **[VERIFIED-PRIMARY]**

What recvmsg4 *can* do is rewrite the source sockaddr the application observes
through `msg_name`, via the writable `ctx->user_ip4` / `ctx->user_port` fields on
the `bpf_sock_addr` context — the same fields connect4/sendmsg4 use (and the
low-16-NBO-in-`u32` `user_port` hazard applies identically). The hook fires inside
`udp_recvmsg()` **after** the kernel has already populated the `sockaddr_in` from
the received skb's IP/UDP headers, so the rewrite is a pure presentation-layer
edit of what the app's `recvfrom` reports — it changes nothing on the wire.

Cilium — the dominant production reference — confirms the shape: `cil_sock4_recvmsg`
calls `__sock4_xlate_rev` and then **unconditionally `return SYS_PROCEED`**
(`SYS_PROCEED == 1`). On a reverse-NAT **hit** it rewrites `user_ip4`/`user_port`
to the service VIP; on a **miss** the inner function returns `-ENXIO`, the entry
point **ignores it, and proceeds — the datagram is delivered to the app with the
backend's source address unchanged.** Cilium's recvmsg4 leaks the backend source
on a miss because it has no other choice: it cannot drop.

Consequences for D3: on the same-host (loopback) cgroup path, **a wire-level
"no backend-IP-sourced reply left the host" guarantee is not physically meaningful**
— the datagram has already traversed `lo` and is sitting on the socket receive
queue *before* recvmsg4 fires; `tcpdump -i lo` would capture the backend-sourced
reply regardless of what recvmsg4 does. The only honest guarantee recvmsg4 can
make is at the **application sockaddr layer**: on a hit, the app sees the VIP; on a
miss, the achievable fail-safe is to **rewrite the source to a non-backend
sentinel** (the only no-leak option that recvmsg4 can actually execute, since it
cannot deny and pass-through leaks) — and to make the miss observable via a
counter. Given that the chosen D1 design writes the forward and reverse entries
atomically together, a reverse miss is a should-never-happen corruption/eviction
path, not a routine one. **Recommendation: rewrite-to-sentinel + counted miss,
and reframe the AC away from wire/tcpdump language to the recvfrom-sockaddr layer.**

---

## Research Methodology

**Search Strategy**: Primary kernel sources — the recvmsg4 origin commit
`983695fa6765` (full patch), the kernel verifier return-code rules (via the
kselftest migration patch that pins the exact range and error message), and the
`bpf_sock_addr` UAPI context-field write rules (via the isovalent/ebpf-docs
reference, which transcribes kernel UAPI). Production reference: Cilium
`bpf/bpf_sock.c` (`cil_sock4_recvmsg`, `__sock4_xlate_rev`) and its `SYS_PROCEED`/
`SYS_REJECT` defines.

**Source Selection**: Kernel source/commit (High, 1.0) by commit hash and quoted
hunk; kernel selftest patch (High, 1.0 — it IS the kernel's own conformance
spec for the return-code rule); Cilium source (High, 1.0 — dominant production
reference) by function; eBPF reference docs (Medium-High, 0.8 — transcribes UAPI,
cross-referenced against kernel).

**Quality Standards**: Q1 (the crux) is triangulated across three independent
primaries (commit, selftest patch, Cilium's `return SYS_PROCEED`). Every claim is
verified-vs-inferred labeled. Some exact file:line citations could not be pinned
because Bootlin Elixir blocked automated fetch; those are flagged in Knowledge Gaps
with the commit/function that nonetheless establishes the fact.

---

## Findings

### Q1 — Can `cgroup/recvmsg4` return a DENY verdict, and what is the syscall effect?

**Hypothesis (from dispatch):** returning 0 from a recvmsg4 program fails the
`recvmsg(2)`/`recvfrom(2)` syscall (EPERM), i.e. the app never receives the
backend-sourced datagram.

**Finding: HYPOTHESIS FALSIFIED. recvmsg4 programs cannot return 0 at all —
the verifier rejects them at load time. They can ONLY return 1 (proceed).**
**[VERIFIED-PRIMARY]**

**Evidence 1 (kernel selftest — the conformance spec):** The kernel's own
verifier selftests pin the rule. The patch migrating the recvmsg return-code tests
states: *"recvmsg4 and recvmsg6 hooks can only return 1."* The positive tests are
`recvmsg4_good_return_code` / `recvmsg6_good_return_code` with `return 1;`, and the
negative tests expect the verifier to **reject** a program returning 0 with the
exact diagnostic:

> `"At program exit the register R0 has smin=0 smax=0 should have been in [1, 1]"`

This is in explicit contrast to connect/sendmsg, "which remain in the old test
file" — i.e. those attach types retain the `[0, 1]` deny-or-allow range.
**[VERIFIED-PRIMARY]** — Source: [PATCH v1 bpf-next 01/17] "selftests/bpf: Migrate
recvmsg* return code tests to verifier_sock_addr.c", linux-kselftest list.

**Evidence 2 (cross-reference — eBPF reference docs + general cgroup_sock_addr
semantics):** For `cgroup_sock_addr` attach types in the deny-capable set
(connect/sendmsg/bind), `return 0` causes the syscall to fail with **EPERM** and
`return 1` proceeds (the standard `__cgroup_bpf_run_filter_sock_addr` → `-EPERM`
conversion). recvmsg is deliberately excluded from this deny set. **[VERIFIED-PRIMARY
for connect/sendmsg deny semantics; the exclusion of recvmsg is [VERIFIED-PRIMARY]
via Evidence 1.]**

**Evidence 3 (production cross-reference):** Cilium's recvmsg4 entry point returns
`SYS_PROCEED` unconditionally, and `SYS_PROCEED == 1` (see Q3). A production LB that
*wanted* to deny on miss could not, and Cilium's code shape reflects exactly that
constraint. **[VERIFIED-PRIMARY]**

**Why recvmsg cannot deny (mechanism, [INFERRED] from the commit rationale in Q4):**
recvmsg4 fires *after* the datagram has already been dequeued and the sockaddr
populated; the data is already in the application's hands at the kernel layer.
There is no "block the receive" semantics to expose at that point — the hook's only
job is to rewrite the *presented* source address. Denying a receive that has
already happened is not a state the kernel models, which is the structural reason
the verifier restricts the range to `[1,1]`.

**Impact on framing:** "drop on miss" is **NOT** available. The architect's
"recvmsg4 can't make a wire-level guarantee" was directionally right but the
sharper truth is stronger: recvmsg4 can't make *any* drop/deny guarantee at any
layer. The fail-safe must be a **source rewrite**, not a drop.

---

### Q2 — Writable fields: is the source rewrite via `user_ip4` / `user_port`?

**Finding: YES. recvmsg4 rewrites the source sockaddr the app sees through
`msg_name` via the same `bpf_sock_addr` `user_ip4` / `user_port` context fields
connect4/sendmsg4 use. The low-16-NBO-in-`u32` `user_port` hazard applies
identically.** **[VERIFIED-PRIMARY]**

**Evidence:** The `bpf_sock_addr` UAPI context exposes:

- `user_ip4` — *"Allows 1,2,4-byte read and 4-byte write. Stored in network byte
  order."* — valid for INET4 attach types (includes recvmsg4).
- `user_port` — *"Allows 1,2,4-byte read and 4-byte write. Stored in network byte
  order."*
- `msg_src_ip4` — *"valid only for `BPF_CGROUP_UDP4_SENDMSG`"* (so it is NOT the
  recvmsg field; recvmsg's source rewrite is `user_ip4`/`user_port`, which on the
  recvmsg path *are* the message source the app reads from `msg_name`).

Source: isovalent/ebpf-docs `BPF_PROG_TYPE_CGROUP_SOCK_ADDR.md` (transcribes the
kernel `sock_addr_is_valid_access` write rules). **[VERIFIED-PRIMARY]** —
cross-referenced against the commit hunk in Q4, which shows the kernel populating
`sin->sin_addr.s_addr` / `sin->sin_port` from the skb *before* invoking the hook,
confirming `user_ip4`/`user_port` are the recvmsg source-rewrite handles.

**`user_port` NBO hazard (cross-reference to project rule):** `user_port` is a
`u32` carrying the port in the **low 16 bits, network byte order**. Read via
`u16::from_be(ctx.user_port as u16)`; write via `ctx.user_port =
u32::from(host_port.to_be())`. This is the exact hazard already documented in
`.claude/rules/development.md` § "`bpf_sock_addr.user_port` — low-16-NBO in a u32",
and it applies verbatim to the recvmsg4 source-port rewrite. **[VERIFIED-PRIMARY —
project rule + kernel UAPI agree.]**

---

### Q3 — What does Cilium do on the recvmsg4 path (HIT and MISS)?

**Finding: Cilium's `cil_sock4_recvmsg` ALWAYS returns `SYS_PROCEED` (=1). On a
reverse-NAT HIT it rewrites `user_ip4`/`user_port` to the service VIP. On a MISS
(`__sock4_xlate_rev` returns `-ENXIO`) the entry point IGNORES the error and
proceeds — the datagram is delivered to the app with the backend source unchanged
(pass-through; a backend-IP "leak" to the app's `recvfrom`).** **[VERIFIED-PRIMARY]**

**Evidence (Cilium `bpf/bpf_sock.c`, `main` branch):**

Entry point:

```c
__section("cgroup/recvmsg4")
int cil_sock4_recvmsg(struct bpf_sock_addr *ctx)
{
	__sock4_xlate_rev(ctx, ctx);
	return SYS_PROCEED;
}
```

Reverse-translation core:

```c
val = map_lookup_elem(&cilium_lb4_reverse_sk, &key);
if (val) {
	// ... service validation ...
	ctx->user_ip4 = val->address;
	ctx_set_port(ctx, val->port);
	// ... trace notification ...
	return 0;
}
return -ENXIO;
```

- **HIT:** `ctx->user_ip4 = val->address; ctx_set_port(ctx, val->port);` — rewrites
  source to the service VIP/port. **[VERIFIED-PRIMARY]**
- **MISS:** `__sock4_xlate_rev` returns `-ENXIO`, but `cil_sock4_recvmsg` discards
  the return value and unconditionally executes `return SYS_PROCEED`. The app's
  `recvfrom` therefore sees the **backend's** source address. **[VERIFIED-PRIMARY]**

`SYS_PROCEED == 1`, `SYS_REJECT == 0`, both `#define`d in Cilium's `bpf_sock.c`.
**[VERIFIED-PRIMARY]** Note that for the *forward* (connect4/sendmsg4) path Cilium
DOES use `SYS_REJECT` on certain errors (`try_set_retval(err); return SYS_REJECT`
on `-EHOSTUNREACH`/`-ENOMEM`) — which is available there precisely because the
forward attach types are in the deny-capable `[0,1]` range. recvmsg4 cannot, which
is why it has no `SYS_REJECT` path. **[VERIFIED-PRIMARY — corroborates Q1.]**

**Interpretation for D3 ([INFERRED]):** The dominant production reference accepts a
backend-source pass-through on a reverse miss. It does NOT sentinel-rewrite and does
NOT (cannot) drop. This is defensible for Cilium because its forward and reverse
entries are written together and a miss is an internal-consistency failure, not a
routine path — the same situation Overdrive's atomic-dual-write design creates.
Overdrive can choose to be *stricter* than Cilium (sentinel-rewrite on miss to
avoid the leak-to-app), but it cannot choose to be *safer at the wire layer* than
the hook physically allows.

---

### Q4 — Is the backend-sourced reply wire-visible BEFORE recvmsg4 fires?

**Hypothesis:** recvmsg4 fires inside the receiving syscall AFTER the datagram has
been delivered to the socket receive queue, so a backend-sourced datagram has
already traversed `lo` and would appear in `tcpdump -i lo` regardless of what
recvmsg4 does to the app's `recvfrom` sockaddr.

**Finding: HYPOTHESIS CONFIRMED. The hook fires inside `udp_recvmsg()` AFTER the
kernel has dequeued the skb and populated the `sockaddr_in` from the skb's IP/UDP
headers. recvmsg4 edits the already-populated sockaddr; it never touches the wire.**
**[VERIFIED-PRIMARY]**

**Evidence (commit `983695fa6765` "bpf: fix unconnected udp hooks", patched
`net/ipv4/udp.c` `udp_recvmsg()`):** the hook is invoked right after the source
address is written into `sin` from the received skb:

```c
sin->sin_addr.s_addr = ip_hdr(skb)->saddr;
memset(sin->sin_zero, 0, sizeof(sin->sin_zero));
*addr_len = sizeof(*sin);

if (cgroup_bpf_enabled)
	BPF_CGROUP_RUN_PROG_UDP4_RECVMSG_LOCK(sk, (struct sockaddr *)sin);
```

The skb has already been received (it is being read in `udp_recvmsg`); `ip_hdr(skb)
->saddr` is the **backend's** address as it arrived. The hook then rewrites
`sin`/`user_ip4` to the VIP for the application. **[VERIFIED-PRIMARY]**

**Ordering on the same-host loopback path ([INFERRED] from the above + standard
loopback delivery):**

1. Backend `sendto(client_addr)` → the reply datagram is sent **with the backend's
   source IP**.
2. The datagram traverses the loopback path (`lo`) and is placed on the **client
   socket's receive queue**. *This is the point a `tcpdump -i lo` capture observes
   it — sourced from the backend IP.*
3. The client calls `recvfrom`; inside `udp_recvmsg` the kernel populates the
   sockaddr from the skb (backend IP), **then** fires recvmsg4, which rewrites the
   presented source to the VIP.

So the backend-sourced datagram is **wire-visible on `lo` at step 2, strictly
before recvmsg4 runs at step 3.** No recvmsg4 verdict or rewrite can prevent the
step-2 capture. **[VERIFIED-PRIMARY for the hook position; INFERRED for the
loopback step ordering, which follows directly from "the skb is already received
when the hook fires".]**

**Impact:** the DISCUSS US-03/K2/K5 phrasing *"tcpdump shows NO backend-IP-sourced
reply left the host"* / *"no backend-IP-sourced reply reaches a client"* **conflates
two layers**. On the cgroup recvmsg4 path there is no "leaves the host" — client
and backend share the host netns; the datagram never leaves. And the wire/`lo`
layer is **not** recvmsg4's domain — it is XDP's. recvmsg4 governs only the
**application sockaddr layer**. A `tcpdump -i lo` will show a backend-sourced
datagram on **every** round-trip, hit or miss, because the rewrite is downstream of
the capture point. The AC, as written, asserts something recvmsg4 structurally
cannot deliver.

---

### Q5 — Honest achievable guarantee + recommended miss-handling

Enumerating the real options recvmsg4 has on a reverse miss, against the verified
constraints (cannot deny — Q1; can only rewrite `user_ip4`/`user_port` — Q2; wire
already happened — Q4):

| Option | What it does | Achievable? | Leak surface |
|---|---|---|---|
| (a) Return 0 / deny → app gets EPERM | — | **NO. Verifier rejects load (Q1).** | n/a — impossible |
| (b) Rewrite source to a non-backend, non-VIP **sentinel** + proceed | app's `recvfrom` sees a sentinel (e.g. `0.0.0.0` or a reserved marker), never the backend IP | **YES** | No backend IP reaches the **app sockaddr**. (Wire/`lo` still showed backend — unavoidable, Q4.) |
| (c) Pass-through unchanged (Cilium's default) | app's `recvfrom` sees the **backend** IP | YES | Backend IP **does** reach the app sockaddr (the source-validation reject the resolver was failing on). |
| (d) Cilium-aligned default | = (c) | YES | = (c) |

**Honest guarantee statement (the one recvmsg4 can actually back):**

> On a reverse-map **hit**, the application's `recvfrom` source address is rewritten
> to the service VIP, so a source-validating resolver accepts the reply. On a
> reverse-map **miss**, recvmsg4 **rewrites the presented source to a non-backend
> sentinel** so the application never observes the backend IP in `msg_name`, and
> increments an observable miss counter. recvmsg4 **cannot** drop the datagram and
> **cannot** affect what a `tcpdump -i lo` capture shows — the wire-layer
> no-leak property is XDP's domain, not recvmsg4's, and on the same-host loopback
> path the datagram never leaves the host at all.

**Recommendation: Option (b) — rewrite-to-sentinel + counted miss.**

Rationale:
1. It is the **only no-leak-to-app** option that recvmsg4 can execute (since deny
   is impossible and pass-through leaks). It strictly dominates Cilium's (c) on the
   "no backend-IP leak" axis the AC cares about, at near-zero added complexity (one
   conditional rewrite vs. one early `return SYS_PROCEED`).
2. It is **simple and observable**, which the constraint set demands: there is
   **no Tier-2 `BPF_PROG_TEST_RUN` backstop** for `cgroup_sock_addr` (ENOTSUPP
   ≤6.8 — confirmed by the project's existing `.claude/rules` and the DISCUSS
   constraint table), so the chosen behavior is **Tier-3-only verifiable**. A
   sentinel rewrite is trivially assertable from the app side (`recvfrom` returns
   the sentinel) and the miss counter from `bpftool map dump`.
3. The reverse miss is a **should-never-happen** path under the locked D1/D7
   atomic-dual-write design (forward + reverse written as one logical write), so
   this is **corruption/eviction handling**, not a routine branch — the bar is
   "fail clean and diagnosable," which (b) meets and (c) does not.

**Caveat on the sentinel choice ([INFERRED], for DESIGN to pin):** the sentinel
must be a value that (i) is not any backend IP, (ii) is not the VIP (so a miss is
distinguishable from a hit by the resolver's behavior and by an observer), and
(iii) the resolver will *reject* (the desired clean-failure outcome). `0.0.0.0` is
the obvious candidate but DESIGN should confirm the target resolvers treat a
`0.0.0.0`-sourced reply as a clean reject rather than some surprising accept; if
not, a reserved/documentation-range address is the fallback. This is a DESIGN
detail, not a DISCUSS/D3 blocker.

---

## Findings → Recommendation for D3

**The crux, stated first (it changes the question):** recvmsg4 **cannot deny** the
receive — the verifier restricts it to `return 1`. So D3's options collapse from
"drop vs pass vs sentinel" to **"sentinel-rewrite vs pass-through"** (drop is off
the table). **[VERIFIED-PRIMARY — triangulated across the kernel selftest, the
origin commit's hook placement, and Cilium's unconditional `SYS_PROCEED`.]**

**Recommended D3 decision:** On a `REVERSE_LOCAL_MAP` miss, **recvmsg4 rewrites the
reply source to a non-backend sentinel (DESIGN pins the exact value) and increments
an observable miss counter; it does not (cannot) drop.** This is strictly stronger
than the dominant production reference (Cilium pass-through leaks the backend IP to
the app) on the one axis the AC cares about, and it is the maximal no-leak
guarantee physically achievable at the recvmsg4 layer.

**The honest guarantee to put to the user:**

> "No backend IP ever reaches the application's `recvfrom` sockaddr — on a hit it's
> the VIP, on a miss it's a sentinel — and every miss is counted. We **cannot**
> guarantee that a `tcpdump -i lo` shows no backend-sourced datagram, because the
> datagram is on the socket receive queue (and thus capturable on `lo`) before our
> hook runs; that wire-level property belongs to XDP, not to a cgroup recvmsg hook,
> and on the same-host path the reply never leaves the host anyway."

---

## What this means for the US-03 / K5 (and K2) AC wording

The current ACs assert a **wire-layer** property that recvmsg4 structurally cannot
own (Q4). They should be **reframed to the application-sockaddr layer**, which is
recvmsg4's actual domain. Concretely:

- **DROP the `tcpdump`/"left the host"/"on the wire" framing for the recvmsg4
  path.** Phrases like *"a `tcpdump` shows NO backend-IP-sourced reply left the
  host"* (US-03 elevator pitch) and *"no reply ever leaves with the backend IP as
  its source"* (US-01 scenario 2) describe XDP/remote-backend semantics, not the
  same-host cgroup path. On the loopback cgroup path the reply never leaves the
  host, and a `lo` capture will always show the pre-rewrite source. Keeping this
  language guarantees an AC that fails for a reason unrelated to correctness.

- **REWORD K2 / US-01 "VIP-sourced reply"** from a wire assertion to a
  **`recvfrom`-sockaddr assertion**: *"the source address the client application
  reads from `recvfrom`/`msg_name` is the VIP (`10.96.0.10`), not the backend IP."*
  This is what recvmsg4 actually delivers and is directly Tier-3-assertable from the
  client app, with no dependence on a wire capture that would (correctly) show the
  backend source.

- **REWORD K5 / US-03 "no backend-IP-sourced reply reaches a client on a miss"** to
  the sentinel guarantee: *"on a reverse-map miss, the source address the client
  application reads is a non-backend sentinel (never the backend IP), and the miss
  is observable via a counter."* Drop "no backend-IP-sourced reply leaves the host"
  entirely for this path.

- **If a wire-layer no-leak guarantee is genuinely required**, it is an **XDP**
  concern (the connected-UDP REVERSE_NAT_MAP path, explicitly out of scope per the
  feature-delta § Out of scope), not a recvmsg4 deliverable. Note this boundary in
  the AC so a future reader does not re-import wire semantics onto the cgroup hook
  (the same wrong-map-on-wrong-hook trap the sibling-journey decision defused).

These are wording/layer corrections, not scope changes: the *intent* of US-03/K5
("a misconfigured reply path fails clean and never exposes the backend IP to the
client") is fully preserved — it is just pinned to the layer recvmsg4 can honor.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Kernel commit `983695fa6765` "bpf: fix unconnected udp hooks" (full patch) | github.com/torvalds/linux | High (1.0) | Official source | 2026-06-05 | Y (Cilium, ebpf-docs) |
| kselftest patch "Migrate recvmsg* return code tests" (return-range rule) | mail-archive.com/linux-kselftest | High (1.0) | Official kernel spec | 2026-06-05 | Y (Cilium `SYS_PROCEED`, ebpf-docs) |
| Cilium `bpf/bpf_sock.c` (`cil_sock4_recvmsg`, `__sock4_xlate_rev`) | github.com/cilium/cilium | High (1.0) | Production source | 2026-06-05 | Y (kernel commit, selftest) |
| Cilium `SYS_PROCEED`/`SYS_REJECT` defines | github.com/cilium/cilium | High (1.0) | Production source | 2026-06-05 | Y (selftest range) |
| isovalent/ebpf-docs `BPF_PROG_TYPE_CGROUP_SOCK_ADDR.md` (UAPI write rules) | github.com/isovalent | Medium-High (0.8) | Reference (transcribes UAPI) | 2026-06-05 | Y (kernel commit hunk) |
| docs.ebpf.io `BPF_PROG_TYPE_CGROUP_SOCK_ADDR` | docs.ebpf.io | Medium-High (0.8) | Reference | 2026-06-05 | Y |

Reputation: High 4 (67%) | Medium-High 2 (33%) | Avg ≈ 0.93. No excluded-tier
sources cited.

**Independence check:** the three primaries backing Q1 are genuinely independent —
the kernel *selftest* (conformance spec), the kernel *commit* (hook placement), and
*Cilium* (a third-party production consumer that independently observes the
constraint by never emitting `SYS_REJECT` on recvmsg). They do not cite each other.

---

## Knowledge Gaps

### Gap 1: Exact file:line for the verifier `check_return_code` range and the v5.10 `udp_recvmsg` call site

**Issue:** Bootlin Elixir and the raw GitHub blob for `kernel/bpf/verifier.c`
(592 KB) could not be fetched whole by the automated tool (truncation / bot
blocking), so the precise `verifier.c` line where `BPF_CGROUP_UDP4_RECVMSG` is
assigned `tnum_range(1, 1)` and the exact `net/ipv4/udp.c:NNN` of the v5.10
`BPF_CGROUP_RUN_PROG_UDP4_RECVMSG_LOCK` call were not pinned to a line number.
**Attempted:** raw.githubusercontent.com (truncated), codebrowser.dev (truncated),
bootlin.com ident page (JS-rendered, blocked). **Impact on conclusions: none** —
the *fact* (range `[1,1]`, hook fires post-skb-populate) is established by the
selftest's verbatim error string `"should have been in [1, 1]"` and the commit
patch hunk respectively; only the line-number citation is missing.
**Recommendation:** if DESIGN wants the exact citation for the ADR-0053 amendment,
resolve `check_return_code` in a local `linux-5.10` checkout
(`git grep -n 'BPF_CGROUP_UDP4_RECVMSG' kernel/bpf/verifier.c`) and
`grep -n RECVMSG_LOCK net/ipv4/udp.c` — both are one-line lookups in-tree.

### Gap 2: Resolver behavior on a `0.0.0.0`-sourced reply

**Issue:** Whether the specific target resolvers (`dig`, glibc `getaddrinfo`, musl)
treat a sentinel-sourced (`0.0.0.0`) reply as a clean reject vs. some other behavior
was not empirically tested — it is the open sentinel-value question flagged in Q5.
**Attempted:** out of scope for a source-research pass (requires a Tier-3 repro).
**Recommendation:** DESIGN/DELIVER validates the sentinel value against the actual
resolvers in the Tier-3 fixture; the *mechanism* (sentinel rewrite) is sound
regardless of which sentinel value is chosen.

---

## Conflicting Information

No source-level conflicts. One **framing conflict** between the DISCUSS ACs and the
verified mechanism is the entire point of this research and is resolved in
"What this means for the AC wording" above: the ACs assert a wire-layer property
(XDP's domain); the verified recvmsg4 mechanism operates only at the
application-sockaddr layer. The kernel sources, the selftest, and Cilium all agree
on the mechanism; the ACs were simply written in the wrong layer's vocabulary
(inherited from the XDP/connected-UDP sibling path).

---

## Recommendations for Further Research

1. **Pin the two exact file:line citations** (Gap 1) in a local 5.10 checkout for
   the ADR-0053 amendment record — cheap, and makes the ADR's mechanism claims
   fully primary-cited.
2. **Tier-3 sentinel-value validation** (Gap 2) — confirm the chosen sentinel
   produces a clean resolver reject before locking it in DELIVER.

## Full Citations

[1] Daniel Borkmann / Andrey Ignatov. "bpf: fix unconnected udp hooks" (commit
983695fa6765758b3a23a763dd9e735d4e8d8d24). Linux kernel git, torvalds/linux.
2018. https://github.com/torvalds/linux/commit/983695fa6765 (patch:
https://github.com/torvalds/linux/commit/983695fa6765.patch). Accessed 2026-06-05.

[2] "[PATCH v1 bpf-next 01/17] selftests/bpf: Migrate recvmsg* return code tests to
verifier_sock_addr.c". linux-kselftest mailing list.
https://www.mail-archive.com/linux-kselftest@vger.kernel.org/msg12402.html.
Accessed 2026-06-05.

[3] Cilium. "bpf/bpf_sock.c" (`cil_sock4_recvmsg`, `__sock4_xlate_rev`,
`SYS_PROCEED`/`SYS_REJECT`). cilium/cilium, `main`.
https://github.com/cilium/cilium/blob/main/bpf/bpf_sock.c. Accessed 2026-06-05.

[4] isovalent / eBPF Docs. "Program Type 'BPF_PROG_TYPE_CGROUP_SOCK_ADDR'"
(context-field write rules: `user_ip4`, `user_port`, `msg_src_ip4`).
https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_CGROUP_SOCK_ADDR/ and
https://github.com/isovalent/ebpf-docs/blob/master/docs/linux/program-type/BPF_PROG_TYPE_CGROUP_SOCK_ADDR.md.
Accessed 2026-06-05.

## Research Metadata

Examined: 9 sources | Cited: 6 | Cross-refs: Q1 triangulated across 3 independent
primaries | Confidence: High (Q1–Q4 verified-primary; Q5 recommendation is reasoned
from verified primaries with two clearly-flagged DESIGN-detail gaps) | Output:
docs/research/dataplane/recvmsg4-reply-source-rewrite-and-miss-semantics-research.md
