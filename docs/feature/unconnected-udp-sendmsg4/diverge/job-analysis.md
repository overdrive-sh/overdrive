# Job Analysis — unconnected-udp-sendmsg4 DIVERGE

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Wave:** DIVERGE (Phase 1 — JTBD) · **Analyst:** Flux

> **Scope frame.** This is a brownfield DIVERGE on **how to close the
> unconnected-UDP same-host delivery gap** that ADR-0053 Amendment 4
> explicitly left open and tracked as #200. ADR-0053's connect4 decision
> is **locked and shipping** — it is NOT reopened here. The connected-UDP
> + TCP path (`cgroup_connect4_service`, `LOCAL_BACKEND_MAP` keyed
> `(vip, vip_port, proto)`) is the surface this work plugs into.

---

## 1. Raw request (verbatim grounding)

From ADR-0053 Amendment 4 (§ "Out of scope", revision 2026-06-03) and
issue #200:

> ADR-0053's `cgroup_connect4_service` (`BPF_CGROUP_INET4_CONNECT`)
> rewrites VIP→local-backend at `connect()` time. It fires for TCP
> `connect()` and *connected*-UDP `connect()` only. A UDP client that calls
> `sendto(VIP:port, …)` WITHOUT a prior `connect()` is never intercepted —
> the datagram bypasses `LOCAL_BACKEND_MAP`, so the VIP→backend rewrite
> never happens. This is the canonical DNS client pattern (glibc
> getaddrinfo, musl, dig) — `sendto` per query, no connect — which makes a
> same-host DNS UDP service unreachable from the common resolver. Issue
> #200 proposes a `BPF_CGROUP_UDP4_SENDMSG` (`cgroup/sendmsg4`) program
> doing the same `LOCAL_BACKEND_MAP` lookup + `(user_ip4, user_port)`
> rewrite, keyed `(VIP, vip_port, proto=UDP)`. It also flags
> `BPF_CGROUP_UDP4_RECVMSG` as a "consider if reply-path source-address
> correctness requires it."

This raw request **is a proposed solution** (`sendmsg4` program). Per the
JTBD trap, the first move is to extract the job beneath it.

---

## 2. Job extraction — 5 Whys (tactical → physical)

| Layer | Question → Answer |
|---|---|
| **Tactical** | *Add a `sendmsg4` BPF program.* → This is the proposed solution, not the job. Reject as the job (§ first-principles inversion below). |
| **Operational** | *Why a sendmsg4 program?* → Because unconnected-UDP `sendto(VIP)` is not intercepted, so the VIP→backend rewrite never happens for those datagrams. |
| **Operational** | *Why does the rewrite need to happen?* → Because the operator declared a same-host UDP Service on a VIP, and the platform's promise is that traffic addressed to the VIP reaches a healthy backend. |
| **Strategic** | *Why must the platform deliver to a VIP at all?* → Because the operator should describe **intent** (a service on a stable VIP) and trust the dataplane to converge actual delivery to it — independent of *how* the client's syscall path happens to be shaped (connect vs sendto, TCP vs UDP). |
| **Strategic** | *Why must it be independent of the client's syscall shape?* → Because the operator does not control the client. The canonical UDP client (a DNS resolver) is third-party code the operator cannot rewrite to call `connect()` first. A delivery guarantee contingent on the client's socket idiom is not a guarantee. |
| **Physical** | *What is the irreducible function?* → **A datagram a same-host client addresses to a service VIP must be delivered to a healthy backend of that service, and the reply must appear to come from the VIP — regardless of whether the client connected first.** Input (datagram → VIP) → translation (VIP → backend, backend → VIP) → delivery (both directions). |

**Stop condition:** the next "why?" ("why run services?") is a life/business
goal (run workloads at all). The physical-level statement above is the
irreducible function.

---

## 3. First-principles inversion (the JTBD 3-step)

1. **Identify the activity:** "the platform intercepts a `sendto` syscall
   and rewrites its destination."
2. **Reject the activity as the job:** no operator wakes up wanting a
   syscall intercepted. The syscall interception is a *mechanism*. The
   operator wants their DNS service to *answer queries*.
3. **Strip to irreducible function:** what remains if every BPF mechanism
   is removed? **"A client's datagram to the service VIP reaches a healthy
   backend; the reply looks like it came from the VIP."** That is the job.
   `sendmsg4` is one guess at the mechanism; the job is protocol- and
   syscall-idiom-agnostic.

**Disruption check.** Is there a higher-level job that makes this one
unnecessary? Only "the client could connect() first" — but that requires
rewriting third-party resolver code the operator does not own, so it does
NOT subsume the job; it relocates the burden onto someone who cannot
discharge it. (This is itself Option D in the brainstorm — the honest
no-op floor.)

---

## 4. Job statements

**Functional (primary).** When a same-host client sends a datagram to a
service VIP **without first connecting** — the dominant DNS resolver idiom
(`sendto`/`sendmsg` per query) — I, the operator who declared that UDP
service, want the platform to deliver the datagram to a healthy backend
and make the reply appear to originate from the VIP, so the service is
reachable from real clients I do not control.

**Functional (correctness twin, rides J-PLAT-004).** When the dataplane
gains a new same-host delivery hook, I, the dataplane author, want
`SimDataplane` and the real kernel path to be provably equivalent on the
observable `(vip, vip_port, proto)` → backend rewrite **and** on the
reply-path source identity, so a forward-only or asymmetric fix cannot
reach production undetected (the #163 / silent-asymmetry class).

**Emotional.** Relief from the *anxiety of the half-working service*: the
class of defect where `overdrive deploy dns-resolver.toml` succeeds,
`overdrive alloc status` shows Running, the connected-UDP path even
passes a hand-written test — and yet `dig @<vip>` from the box hangs,
because the resolver never called `connect()`. The operator wants delivery
that does not depend on the client's socket idiom.

**Social.** The operator is seen as running a *real* DNS service on
Overdrive — one that `resolv.conf` can point at and `getaddrinfo` actually
resolves through — not a demo that only works with a bespoke client.

---

## 5. Validated jobs this DIVERGE rides (extract/elevate, not duplicate)

This work serves **two existing active jobs** in `docs/product/jobs.yaml`,
along the *same-host UDP-reachability* dimension. The dataplane-reachability
job is **implicit** in J-OPS-003/J-OPS-004 today; this analysis elevates it
explicitly (see § 8 SSOT update — `jobs.yaml` changelog note, no new job
minted).

| Job | Title | Relevance to #200 |
|---|---|---|
| **J-OPS-004** | "Submit a Service-kind workload and trust the wire signal to reflect operator-meaningful liveness…" (operator-trust, `served_by_phase: 1`, active) | The operator-facing outcome: a UDP service the operator declared must be **reachable** from the canonical unconnected-UDP client, and its reply must be sourced from the VIP (never leak the backend IP). #200 is the mechanism that extends J-OPS-004's reachability guarantee to the unconnected-UDP datagram path. A service reported "Stable" that a DNS resolver cannot reach is exactly the operator-trust violation J-OPS-004 exists to prevent. |
| **J-PLAT-004** | "Run a reconciler I wrote against a simulated cluster and know it converges" (dataplane-correctness, `served_by_phase: 2`, active) | The same-host delivery hook(s) and any reply-path rewrite are the surface a DST equivalence invariant pins. The decision determines *whether Sim and the real kernel path can be asserted equivalent on the reverse-path source identity* for unconnected UDP — the structural defense against the #163 asymmetry class recurring on the sendmsg path. |

Why no new job: minting "J-OPS-005: unconnected-UDP reachability" would
fragment J-OPS-004 along the syscall-idiom axis (then owing a job for
`sendmmsg`, for `io_uring` UDP, etc.). Per the udp-service-support D5
precedent (one job spans the protocol/idiom dimension), this rides
J-OPS-004 + J-PLAT-004 and adds a changelog note, not a job.

---

## 6. ODI outcome statements (the decision must move these)

ODI form: `[Direction] + [Metric] + [Object] + [Context]`. These are the
empirical anchor for the taste-evaluation's Desirability and
Speed-as-Trust scores.

| ID | ODI statement | Source | Status |
|---|---|---|---|
| **O1** | Minimize the **likelihood that** a same-host UDP service is unreachable from a client that uses the unconnected `sendto`/`sendmsg` idiom (the DNS-resolver default). | #200; ADR-0053 Amd 4; J-OPS-004 | Under-served (0% reachable today via unconnected path) |
| **O2** | Minimize the **likelihood that** an unconnected-UDP reply reaches the client with a source address ≠ the VIP it sent to (causing the client to discard the reply via source validation). | kernel commit `983695fa6765` (the `nslookup`-rejects-8.8.8.8-reply failure); J-OPS-004 | Under-served (the connect4 path never sees these datagrams) |
| **O3** | Minimize the **likelihood that** a Sim/real-kernel divergence on the unconnected-UDP rewrite **or reply-path source identity** reaches production undetected. | J-PLAT-004; #163 asymmetry class | Under-served (no hook, no equivalence surface today) |
| **O4** | Minimize the **effort required to** add the unconnected-UDP path on top of the existing connect4 surface (program, map, action, trait, hydrator) without re-migrating the connect4 call sites. | ADR-0053 single-cut posture; marginal-surface taste | Under-served (no shared lookup helper / attach orchestration today) |
| **O5** | Minimize the **likelihood of** introducing an untestable correctness gap, given no Tier-2 `BPF_PROG_TEST_RUN` backstop exists for `cgroup_sock_addr` (ENOTSUPP ≤ 6.8) — i.e. how much of the option's correctness is provable below Tier 3. | `.claude/rules/development.md` § sock_addr-no-Tier-2; ADR-0053 § proto-source contract | Under-served (Tier-3-only is the gate; options differ in how much they pin below it) |

**Discriminating outcomes.** O1 is the *table-stakes* outcome — every
non-no-op option must move it (deliver the datagram). The option study
**bites on O2, O3, O4, O5**: O2 (reply-path source identity) separates
sendmsg4-only from sendmsg4+recvmsg4; O4 (marginal surface) separates the
unify-with-connect4 option from the standalone ones; O5 (testability)
separates the cgroup-hook options from the SK_LOOKUP / non-cgroup options
whose verification cost and shape differ. The taste matrix's T1
(Subtraction), T2 (Concept Count), and T4 (Speed-as-Trust → here read as
"trust the delivery is real, not half-working") score against exactly O2,
O4, O5.

---

## 7. Gate check (Phase 1 — G1)

- [x] **Job at strategic/physical level** — physical statement in § 2
  ("a datagram to a VIP reaches a healthy backend; the reply looks like it
  came from the VIP, regardless of whether the client connected first");
  strategic framing in J-OPS-004 / J-PLAT-004. Not tactical, not the
  `sendmsg4`-program feature description.
- [x] **No feature reference inside the job statement** — the job
  statements (§ 4) name no BPF program, map, or hook; the *raw request*
  (§ 1) is feature-shaped, the *job* is not.
- [x] **≥ 3 ODI outcome statements** — 5 produced (O1–O5), each in
  Minimize + likelihood/effort form, no forbidden words (easy/reliable/
  good/effective/manage), no embedded solution.

**G1: PASS.**

---

## 8. SSOT update (jobs.yaml)

`docs/product/jobs.yaml` is updated with a **changelog note** elevating the
same-host dataplane-reachability dimension under J-OPS-004 + J-PLAT-004,
and J-OPS-004's `source` is annotated to span unconnected-UDP reachability.
**No new job is minted** (per § 5 rationale; the udp-service-support D5
precedent). See `docs/product/jobs.yaml` changelog entry dated 2026-06-05
referencing `unconnected-udp-sendmsg4` / #200.
