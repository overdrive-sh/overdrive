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

---

# Addendum — UI-1 adjudication (2026-06-05): reverse-miss handling under cgroup-ancestor attach

**Date**: 2026-06-05 | **Researcher**: nw-researcher (Nova) | **Confidence**: High |
**New sources**: 3 (kernel cgroup_sockopt attach-hierarchy doc; Cilium kube-proxy-free
docs cgroup-root attach; re-read of Cilium `bpf/bpf_sock.c` `cil_sock4_recvmsg` /
`__sock4_xlate_rev` miss path).

> **This addendum ADDS to the findings above; it does not rewrite them.** Q1
> (recvmsg4 cannot deny — `[1,1]`), Q2 (writable fields are `user_ip4`/`user_port`),
> and Q4 (the hook fires post-skb-populate, wire-layer is XDP's domain) all stand
> **unchanged and re-confirmed**. What this addendum corrects is the **Q5
> recommendation** ("sentinel-on-miss is strictly stronger than Cilium's
> pass-through") and the **D3 decision built on it** — both of which omitted the
> attach-scope fact that makes a reverse-map miss mean something different from what
> Q5 assumed.

## The decision being adjudicated

During DELIVER (step 01-03, commit `e71ad780`, Tier-3-verified GREEN), the crafter
found ADR-0053 rev 2026-06-05 § D3 — *"on a `REVERSE_LOCAL_MAP` miss, rewrite the
reply source to a non-backend sentinel `192.0.2.1` + counted miss; strictly stronger
than Cilium's pass-through-leak"* — **unworkable**, and changed it to **"rewrite
source→VIP on a HIT; pure NO-OP on a MISS (counter bump only, source left intact)."**

The adjudication question: **is the crafter's correction right, and is the prior
doc's Q5 recommendation (sentinel-on-miss "strictly stronger than Cilium's
pass-through") wrong?**

**Verdict, stated first: the crafter is CORRECT. The Q5 recommendation is WRONG on
the one axis it claimed superiority, and D3-as-locked is unworkable.** The error is
not in any of Q1–Q4's verified mechanics; it is a single unexamined premise — *that
a `REVERSE_LOCAL_MAP` miss denotes "a service reply whose reverse entry was lost."*
Under the actual attach scope, a miss overwhelmingly denotes *"this datagram is not
a service reply at all,"* and sentinel-rewriting it corrupts unrelated traffic.

---

## Findings

### A-Q1 — Attach scope: does recvmsg4 fire on EVERY unconnected UDP recv in the subtree, or only service-VIP datagrams?

**Hypothesis (the crux):** a `BPF_CGROUP_UDP4_RECVMSG` program attached at a cgroup
ancestor (`overdrive.slice`, which contains the control plane AND every workload)
fires for **every** unconnected-UDP `recvmsg`/`recvfrom` issued by **any** process in
the subtree — not only datagrams whose source is a service backend.

**Predicted finding:** yes, all unconnected UDP recv in the subtree. **Falsifier:**
the hook is scoped to specific addresses or specific sockets (per-socket attach), so
it would fire only on the sockets the platform tagged.

**Finding: HYPOTHESIS CONFIRMED. cgroup_sock_addr programs are attached to a cgroup,
not a socket, and the kernel invokes them on the relevant syscall for EVERY task in
that cgroup and all descendants.** **[VERIFIED-PRIMARY]**

**Evidence 1 (kernel cgroup-BPF attach hierarchy):** the kernel BPF cgroup
documentation states that a program attached to a cgroup is *"called every time [a]
process executes"* the relevant syscall, and that programs *"execute for all
processes within that cgroup and its descendants"*; when programs attach at multiple
cgroup levels they run bottom-up (child → parent) over the same syscall invocation.
There is no per-address or per-socket scoping in the attach model — the unit of
attachment is the cgroup, and the trigger is "any task in the subtree performs the
syscall." Source: [kernel.org BPF cgroup sockopt / cgroup-BPF attach semantics](https://docs.kernel.org/bpf/prog_cgroup_sockopt.html),
accessed 2026-06-05. **[VERIFIED-PRIMARY]**

**Evidence 2 (Cilium production, independent corroboration):** Cilium attaches its
socket-LB cgroup programs (the connect/sendmsg/recvmsg family) to the **cgroup v2
root** (`/run/cilium/cgroupv2`), explicitly so the hooks *"apply globally to all
sockets across all pods … fire for all socket operations system-wide, not per
individual socket."* Source: [Cilium kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/),
accessed 2026-06-05. This is the same shape Overdrive uses — one LB cgroup ancestor
covering every process that might issue (or receive a reply to) a VIP datagram. The
ADR itself states this design intent: *"one host netns, one LB cgroup ancestor for
every process that might issue a VIP connect"* (ADR-0053 § "Relationship to prior
ADRs", `feedback_phase1_single_node_scope` line). **[VERIFIED-PRIMARY]**

**Evidence 3 (the shipped program confirms the same reading):** the implemented
`cgroup_recvmsg4_service` (`crates/overdrive-bpf/src/programs/cgroup_recvmsg4_service.rs:107-122`)
documents exactly this in-code: *"recvmsg4 is attached at a cgroup ANCESTOR and
therefore fires on EVERY unconnected UDP `recvmsg`/`recvfrom` from any descendant —
service replies AND all unrelated UDP (DNS clients, the backends' own recvs, etc.).
The map lookup is the discriminator."* **[VERIFIED-PRIMARY — the code under audit.]**

**Consequence (the reframe of "miss"):** because the hook fires on all unconnected
UDP in the subtree, a `REVERSE_LOCAL_MAP` lookup keyed on the datagram's *source*
identity misses for the overwhelming majority of datagrams — every DNS client
reading an upstream answer, **the backend's own `recvfrom` of the inbound query**,
every unrelated same-host UDP exchange. A miss therefore means **"the source of this
datagram is not a registered backend identity"** — i.e. *not a service reply* — NOT
"a service reply whose reverse entry was lost." Q5 assumed the latter; the attach
scope makes it the former. **[VERIFIED-PRIMARY for the mechanism; INFERRED for "the
overwhelming majority", which follows directly from "fires on all subtree UDP".]**

---

### A-Q2 — Consequence of sentinel-on-miss: does it corrupt non-service traffic?

**Hypothesis:** given A-Q1, rewriting source→sentinel `192.0.2.1` on every miss
corrupts the sender address that every *non-service* unconnected-UDP datagram's
application reads. The canonical break: a backend resolver doing its own `recvfrom`
of an inbound query would see source `192.0.2.1` instead of the real client, and
reply to `192.0.2.1` — the wrong peer.

**Predicted finding:** yes — sentinel-on-miss mangles every unrelated UDP recv in
the cgroup; no-op-on-miss (leave the real source intact) is the only correct
behavior. **Falsifier:** some property of the miss path that excludes non-service
traffic (e.g. the lookup key being VIP-derived rather than source-derived, so
non-service datagrams never reach the rewrite).

**Finding: HYPOTHESIS CONFIRMED. Sentinel-on-miss corrupts every non-service
unconnected-UDP recv in the subtree. This is precisely the breakage the crafter
observed (it broke the unconnected round-trip AND the connected-UDP K4 path until
caught).** **[VERIFIED-PRIMARY]**

**Evidence (the lookup is keyed on the datagram source, so non-service datagrams DO
reach the rewrite):** the shipped program builds its reverse key from the
kernel-populated source sockaddr — *"the kernel has already populated the source
sockaddr (`user_ip4`/`user_port`) with the BACKEND identity the datagram arrived
from"* — and the `ReverseLocalKey` is `{ backend_ip_host, backend_port_host, proto }`
(`cgroup_recvmsg4_service.rs:84-102`). For a non-service datagram the source is some
arbitrary peer, the lookup misses, and a sentinel-rewrite branch would then overwrite
that arbitrary peer's address in `user_ip4` — exactly the field the receiving app
reads via `recvfrom`/`msg_name`. The in-code rationale states the failure verbatim:
*"Rewriting the source on a miss would corrupt every unrelated UDP recv in the cgroup
(e.g. a backend reading a query would have its sender address mangled, so its reply
would target the wrong peer)."* (`cgroup_recvmsg4_service.rs:114-122`).
**[VERIFIED-PRIMARY — the code path + the falsifier ruled out: the key IS
source-derived, so non-service traffic reaches the branch.]**

**Cross-reference (UI-1 finding, independent observation):** the crafter's
`upstream-issues.md` § UI-1 records the same break empirically — *"Rewriting those
sources to the sentinel mangles the sender address every non-service datagram's app
reads — it broke the unconnected round-trip AND the connected-UDP K4 path until
caught."* This is a Tier-3 observation, independent of the source-code reasoning
above; the two agree. **[VERIFIED-PRIMARY — Tier-3 evidence + code path.]**

**Conclusion:** **no-op-on-miss (leave the real source intact, bump a counter for
observability) is the only correct behavior** for a hook that fires on all subtree
UDP. Any source rewrite on a miss is a correctness bug, not a "strictly stronger"
hardening.

---

### A-Q3 — What does Cilium actually do, and was Q5 wrong?

**Hypothesis:** Cilium `cil_sock4_recvmsg` / `__sock4_xlate_rev` does HIT → rewrite
source→service-IP, MISS → `SYS_PROCEED` with no rewrite (pass-through). And — the
re-evaluation — Cilium's pass-through is **CORRECT** (a miss = non-service traffic
whose real source must be preserved), making no-op-on-miss **Cilium-aligned** and
Q5's "strictly stronger sentinel" framing **WRONG**.

**Predicted finding:** Cilium passes through unchanged on miss; this is correct, not
a "leak" to be improved on; Q5 mislabeled it. **Falsifier:** Cilium sentinel-rewrites
on miss (then Q5's "Cilium leaks" premise would hold), or Cilium's miss genuinely
denotes a lost-reverse-entry service reply (then "leak" would be the right word).

**Finding: HYPOTHESIS CONFIRMED. Cilium passes through unchanged on miss, and this is
the CORRECT behavior — not a leak. Q5's "strictly stronger than Cilium's
pass-through-leak" conclusion is WRONG.** **[VERIFIED-PRIMARY]**

**Evidence (re-read of Cilium `bpf/bpf_sock.c`, `main`, accessed 2026-06-05):**

- `cil_sock4_recvmsg` calls `__sock4_xlate_rev(ctx, ctx)` and then
  **unconditionally `return SYS_PROCEED;`** (the function's sole return). **[VERIFIED-PRIMARY]**
- Inside `__sock4_xlate_rev`, on a `cilium_lb4_reverse_sk` map **hit** the code
  rewrites `ctx->user_ip4 = val->address` (source → service IP) and returns 0; on a
  **miss** it returns `-ENXIO` **and leaves `user_ip4` unchanged** — the entry point
  discards the `-ENXIO` and proceeds. The source the app reads is the **real**
  source, untouched. **[VERIFIED-PRIMARY]**
- `#define SYS_REJECT 0` / `#define SYS_PROCEED 1` (`bpf_sock.c` lines 20-21).
  **[VERIFIED-PRIMARY]**

**Why pass-through is correct, not a leak (the Q5 mislabel):** Cilium attaches at the
cgroup root (A-Q1, Evidence 2), so — exactly as in Overdrive — `cil_sock4_recvmsg`
fires on *all* unconnected UDP, and a reverse-SK miss is *non-service traffic*.
Leaving the real source intact is the **only** correct action: the datagram's true
source is what its receiving app must see. Q5 reasoned as if the miss were a service
reply whose VIP-mapping was lost (in which case the unrewritten backend source would
indeed be an undesirable "leak"). But under the attach scope, the miss is not a
service reply at all — so there is no backend source to "leak," and "pass-through"
is preserving a *correct* address, not exposing a wrong one. **The word "leak" in Q5
and in D3 is a category error.** Cilium is not making a defensible-but-weaker choice;
it is making the *correct* choice, and Overdrive's no-op-on-miss is **Cilium-aligned,
not Cilium-exceeding.** **[VERIFIED-PRIMARY for the mechanism; INFERRED for the
correctness judgment, which follows from A-Q1 + the source.]**

---

### A-Q4 — No-leak guarantee (K5) under reverse-first dual-write, and the residual sentinel role

**Hypothesis:** with the D1 reverse-first dual-write, every registered backend has a
reverse entry before its forward entry is visible, so a **genuine service reply
ALWAYS hits** → is always rewritten to the VIP → no backend-IP ever reaches the
client app. The only miss is non-service traffic. K5's no-leak guarantee therefore
holds via "always-hit-on-service-reply," not via sentinel-on-miss.

**Predicted finding:** K5 holds by always-hit; the sentinel has no role on the miss
path. **Falsifier:** a service-reply path that can miss (then the no-leak guarantee
would have a hole that a HIT-path fail-safe must close).

**Finding: HYPOTHESIS CONFIRMED. K5's no-leak guarantee holds via the always-hit
property of the reverse-first dual-write, NOT via sentinel-on-miss. The sentinel has
no correct role on the miss path.** **[VERIFIED-PRIMARY for the dual-write ordering;
INFERRED for the end-to-end guarantee.]**

**Evidence (D1/D5a ordering, ADR-0053 rev 2026-06-05):** the observable invariant
amended into the `register_local_backend` contract is *"observers never see a forward
`LOCAL_BACKEND_MAP` entry without its corresponding reverse `REVERSE_LOCAL_MAP` entry
— the reverse write commits first … so any visible forward entry implies a visible
reverse entry"* (ADR-0053 § D5a). A datagram is a service reply only because a client
forward-translated through `LOCAL_BACKEND_MAP` to reach the backend — which means the
forward entry was visible, which (by the invariant) means the reverse entry was
visible. So a genuine service reply's source **is** a registered backend identity and
**hits** the reverse map → rewritten to VIP. **No backend IP reaches the client app
on the service path.** **[VERIFIED-PRIMARY for the ordering invariant; INFERRED for
"every service reply hits", which is the contrapositive of the invariant.]**

**Residual sentinel role — is there a HIT-path corruption case worth handling?** The
only way a service reply could miss is if its reverse entry were **evicted or
tampered** after the forward entry was used (map pressure; external `bpftool`
write). This is the should-never-happen-under-dual-write case. Two observations:

1. It is **not distinguishable at the recvmsg4 layer** from ordinary non-service
   traffic — both present as "source not in `REVERSE_LOCAL_MAP`." recvmsg4 has no
   second signal to tell "evicted service reply" from "DNS client's upstream answer."
   So a sentinel-on-miss fail-safe **cannot** be scoped to only the evicted-service
   case; it would necessarily also fire on all the non-service traffic (A-Q2's
   corruption). There is no correct sentinel branch on the miss path. **[INFERRED
   from A-Q1 + A-Q2 — the miss path carries no discriminator.]**
2. The honest mitigation for the eviction case is **prevention + observability**, not
   a recvmsg4 rewrite: size `REVERSE_LOCAL_MAP` to match `LOCAL_BACKEND_MAP` (the ADR
   sets both to 4096, so co-eviction is structurally avoided), and let the
   `REVERSE_LOCAL_MISS_COUNTER` make any anomaly visible. A non-zero miss counter on a
   single-node box that should have only service replies is the diagnostic; the fix
   is upstream (re-register), not a per-datagram sentinel. **[INFERRED.]**

**Verdict on the sentinel:** **it is should-never-happen-under-dual-write AND not
special-handleable at the recvmsg4 layer, so it is not worth a code path.** Retaining
`SENTINEL_SOURCE_HOST` as a dead documentation constant (as the implementation does,
`cgroup_recvmsg4_service.rs:60-65`) is the correct disposition — it records the
rejected design without executing it. The honest move under
`.claude/rules/development.md` § "Deletion discipline" would be to delete the unused
constant; retaining it as documentation is a defensible judgment call the crafter
flagged for the S-03-01 re-scope (not a research blocker).

---

## UI-1 verdict

**The crafter's correction is CORRECT.** "Rewrite source→VIP on HIT; pure no-op on
MISS (counter bump only)" is the right behavior — it is Cilium-aligned, it is the
only behavior that does not corrupt the non-service unconnected UDP the cgroup-ancestor
hook unavoidably fires on, and it preserves K5's no-leak guarantee via the
reverse-first dual-write's always-hit property. ADR-0053 § D3 as locked
(sentinel-on-miss) is **unworkable** and must be amended.

**The prior doc's Q5 recommendation is WRONG**, specifically on the claim that
sentinel-on-miss is *"strictly stronger than Cilium's pass-through."* The single
defective premise: Q5's option table (rows b/c) and its rationale §1 both assumed a
miss = "a service reply with a lost reverse entry," so pass-through looked like a
backend-IP "leak." Under the verified attach scope (A-Q1), a miss = "not a service
reply," pass-through preserves a *correct* source, and the sentinel *corrupts* it.
Q5 had the right mechanics (Q1–Q4) but reasoned about the wrong population of
datagrams. See the **Q5 correction** section below for the precise scope of the
correction.

---

## Corrected D3 contract (one paragraph, for the architect's ADR amendment)

> **D3 (corrected) — reverse-miss handling: rewrite-on-HIT, pure no-op-on-MISS.**
> `cgroup/recvmsg4` is attached at a cgroup ancestor and therefore fires on every
> unconnected-UDP `recvmsg`/`recvfrom` from any descendant — service replies and all
> unrelated same-host UDP alike. The `REVERSE_LOCAL_MAP` lookup, keyed on the
> datagram's source identity, is the *discriminator*: a **HIT** means the source is a
> registered backend identity (this is a service reply), and recvmsg4 rewrites the
> source `user_ip4` the application reads to the VIP. A **MISS** means the source is
> not a registered backend (this is not a service reply — a DNS client's upstream
> answer, a backend's own `recvfrom` of an inbound query, any unrelated UDP), and
> recvmsg4 performs a **pure no-op**: it leaves the real source address intact and
> increments `REVERSE_LOCAL_MISS_COUNTER` for observability only. recvmsg4 cannot deny
> (`[1,1]`); every path returns 1. The K5 no-leak guarantee is preserved by the D1
> reverse-first dual-write, not by the miss path: every registered backend has a
> visible reverse entry before its forward entry is usable, so a genuine service reply
> *always* hits and is *always* rewritten to the VIP — no backend IP ever reaches the
> client application. The sentinel `192.0.2.1` is **NOT** written on the miss path; a
> source rewrite on a miss would corrupt every non-service datagram's sender address
> (the bug observed and fixed in DELIVER step 01-03). This is Cilium-aligned behavior
> (`cil_sock4_recvmsg` returns `SYS_PROCEED` and `__sock4_xlate_rev` leaves the source
> unchanged on a reverse-SK miss), not weaker than it.

---

## S-03-01 re-scope (for the acceptance-designer)

S-03-01 was *"reverse miss → source is sentinel `192.0.2.1`, never the backend IP."*
That assertion is now **false** and must be replaced. The re-scoped slice-03
assertions should be:

1. **Non-service unconnected UDP is unaffected by recvmsg4.** A same-host
   unconnected-UDP exchange whose source is NOT a registered backend (e.g. a plain
   client/server pair, or a backend reading an inbound query) reads its **real**
   sender address from `recvfrom`/`msg_name` — recvmsg4 leaves it byte-for-byte
   intact. (This is the regression the corrected behavior fixes; it is the
   load-bearing new assertion.)
2. **A service reply always hits → VIP-sourced.** Under the reverse-first dual-write,
   a genuine service reply's source is always a registered backend identity, so it
   always hits and the app reads the **VIP** as the source — there is no
   backend-IP-leak path on the service reply. (Restates the HIT assertion S-03-01
   shared with slice 01/02; unchanged.)
3. **The miss counter is observable, behaviorally inert.**
   `REVERSE_LOCAL_MISS_COUNTER` increments on every non-service recv (it will be
   large and non-zero in any realistic subtree), and its incrementing has **no**
   effect on the source the app reads. Assert the counter moves on non-service recv
   AND that the source is untouched on the same recv — the two together pin "counted
   but inert."

S-03-02 / S-03-03 (the other hardening scenarios) and the K5 / US-03 wording should
be reviewed for the same "sentinel-on-miss" assumption and reframed to the
always-hit / no-op-on-miss reality. The K5 no-leak guarantee is **preserved** — only
its *mechanism* changes (from "sentinel on miss" to "always-hit-on-service-reply via
dual-write"); the operator-facing promise ("no backend IP ever reaches the client
application's `recvfrom`") is intact and is now backed by a stronger structural
argument (the dual-write invariant) rather than a per-datagram fail-safe.

---

## Q5 correction (explicit, for the prior section above)

The original **Q5** ("Honest achievable guarantee + recommended miss-handling") is
corrected as follows. Its verified mechanics — recvmsg4 cannot deny (Q1), writable
fields are `user_ip4`/`user_port` (Q2), the wire layer is XDP's not recvmsg4's
(Q4) — **all stand**. The defective parts are:

- **Q5 option table, row (b) "rewrite to sentinel + proceed" labeled "Achievable:
  YES … No backend IP reaches the app sockaddr":** CORRECTED. On the cgroup-ancestor
  attach, row (b) is *achievable but incorrect* — it rewrites the source of **every
  non-service datagram** in the subtree, not just a lost service reply. It does not
  "avoid a leak"; it *introduces corruption*. Row (b) must be struck as the
  recommendation.
- **Q5 option table, row (c) "pass-through (Cilium's default) … Leak surface:
  Backend IP does reach the app sockaddr":** CORRECTED. The "leak" label is a category
  error. On a miss the source is *not* a backend (the datagram is not a service
  reply), so pass-through preserves the *correct* source. Row (c) — no-op-on-miss — is
  the **correct recommendation**, not the leaky fallback.
- **Q5 "Recommendation: Option (b) — rewrite-to-sentinel + counted miss" and its
  rationale §1 "strictly dominates Cilium's (c) on the no-backend-IP-leak axis":**
  CORRECTED to **Option (c) — pure no-op-on-miss + counted miss.** There is no axis on
  which (b) dominates (c); (b) is a correctness regression. Cilium's pass-through is
  the right answer, and Overdrive matches it.
- **Q5 rationale §3 "reverse miss is a should-never-happen path … sentinel + counter
  meets [the bar] and pass-through does not":** CORRECTED. The premise conflated two
  distinct miss populations. The *should-never-happen* miss (an evicted service
  reply) is real but **rare and indistinguishable at the recvmsg4 layer** from the
  *routine and overwhelmingly common* miss (non-service traffic). Because the two
  cannot be told apart on the miss path, no sentinel branch can be scoped to only the
  rare case; pass-through is correct for the common case and the rare case is handled
  by prevention (map sizing) + observability (the counter), not by a rewrite.
- **Knowledge Gap 2 (resolver behavior on a `0.0.0.0`/`192.0.2.1`-sourced reply):**
  now **moot for the miss path** — no sentinel is written on a miss, so no resolver
  ever sees a sentinel-sourced reply on that path. The gap only mattered under the
  rejected sentinel-on-miss design.

The corrected one-line Q5 takeaway: **on a reverse-map miss, recvmsg4 does nothing
(no-op) and counts the miss; the no-leak guarantee comes from the reverse-first
dual-write making every service reply hit, never from a miss-path sentinel. This is
Cilium-aligned, not Cilium-exceeding.**

---

## Residual Tier-3 open question

One genuine open item remains, narrower than the original Gap 2 and Tier-3-shaped:

> **Does `REVERSE_LOCAL_MISS_COUNTER` carry a usable operational signal, or is it pure
> noise?** Because the counter increments on *all* non-service unconnected UDP in the
> subtree (DNS clients, backend inbound-query recvs, unrelated same-host UDP), its
> absolute value is dominated by non-service traffic and is NOT a "service reply
> failed to translate" alarm. The Tier-3 question for the acceptance-designer /
> DEVOPS is whether the counter is worth surfacing at all (it cannot distinguish the
> should-never-happen evicted-service-reply case from routine non-service misses), or
> whether the honest observability for the eviction case lives elsewhere (e.g. a
> control-plane reconciler comparing forward-vs-reverse map cardinality, or a
> `bpftool map dump` differential). This is a metric-semantics decision, not a
> correctness blocker — the no-op-on-miss behavior is correct regardless of whether
> the counter is kept, demoted, or replaced. **No GitHub issue created** (per
> `feedback_no_unilateral_gh_issues`); surfaced here as a DELIVER/DEVOPS open question
> for user direction.

A second, lower item: per `.claude/rules/development.md` § "Deletion discipline," the
now-unused `SENTINEL_SOURCE_HOST` constant (`cgroup_recvmsg4_service.rs:65`) is dead
code retained as documentation. Whether to delete it or keep it as a rejected-design
marker is a code-hygiene judgment for the S-03-01 re-scope, not a research finding.

---

## Addendum source analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| kernel.org BPF cgroup attach/invocation semantics (`prog_cgroup_sockopt.html`) | docs.kernel.org | High (1.0) | Official kernel doc | 2026-06-05 | Y (Cilium cgroup-root, shipped program) |
| Cilium kube-proxy-free docs — cgroup-root attach scope | docs.cilium.io | High (1.0) | Production reference doc | 2026-06-05 | Y (kernel attach doc, ADR design intent) |
| Cilium `bpf/bpf_sock.c` (`cil_sock4_recvmsg` SYS_PROCEED; `__sock4_xlate_rev` `-ENXIO` no-rewrite miss; SYS_PROCEED/SYS_REJECT) | github.com/cilium/cilium | High (1.0) | Production source | 2026-06-05 | Y (kernel attach doc) |
| Shipped `cgroup_recvmsg4_service.rs` (attach-scope rationale, source-keyed lookup, no-op-on-miss) | overdrive repo | High (1.0) | Code under audit | 2026-06-05 | Y (UI-1 Tier-3 finding) |
| `upstream-issues.md` § UI-1 (Tier-3-observed corruption from sentinel-on-miss) | overdrive repo | High (1.0) | DELIVER back-prop log | 2026-06-05 | Y (code path) |

Reputation: High 5/5 (100%). **Independence check:** the attach-scope crux (A-Q1) is
corroborated by three independent sources — the kernel doc (the authoritative
semantics), Cilium (an independent production consumer attaching at the cgroup root
for the same reason), and the shipped program's own rationale. The Cilium miss-path
behavior (A-Q3) is read directly from upstream source and cross-checked against the
kernel return-code rule from the original Q1. None cite each other.

## Addendum metadata

New sources examined: 3 web + 2 repo | New cross-refs: A-Q1 triangulated across 3
independent primaries (kernel doc, Cilium docs, shipped code); A-Q3 verified directly
against Cilium source | Confidence: High (every load-bearing claim VERIFIED-PRIMARY;
the few INFERRED labels are clearly flagged and follow deductively from the primaries)
| Verdict: crafter CORRECT; Q5 WRONG on the "strictly stronger than Cilium" claim;
D3 amendment + S-03-01 re-scope specified above.
