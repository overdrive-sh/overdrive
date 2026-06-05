# Test Scenarios — unconnected-udp-sendmsg4 (SPECIFICATION ONLY)

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Waves consumed:** DISCUSS (approved) + DESIGN (approved, ADR-0053 rev 2026-06-05)
· **Density:** lean + ask-intelligent

> **This file is a SPECIFICATION, never parsed or executed.** Per
> `.claude/rules/testing.md` § Testing: NO `.feature` files, no cucumber-rs,
> no pytest-bdd. The GIVEN/WHEN/THEN blocks below are the spec the crafter
> translates into Rust `#[test]` / `#[tokio::test]` functions. The
> executable RED scaffolds live under `crates/*/tests/` and `crates/*/src/`
> (see `red-classification.md` for the file list).

---

## Reconciliation note (READ FIRST — app-sockaddr reframe is binding)

Per DESIGN **DDD-3a** + `design/upstream-changes.md` Change A (CA-2), the
DISCUSS wire-layer ACs for US-01/US-03 and KPIs K2/K5 are **superseded** by
**application-sockaddr-layer** ACs. Every scenario below asserts the
reframed contract:

- **K2 / US-01 reply identity** — *"the source address the client
  application reads from `recvfrom` / `msg_name` is the VIP (`10.96.0.10`),
  not the backend IP."* Asserted at the **app sockaddr layer** (the client's
  `recvfrom` return), **NOT** via `tcpdump -i lo` (which correctly shows the
  backend source on every round-trip — recvmsg4 fires post-dequeue and never
  touches the wire; research Q4).
- **K5 / US-03 no-leak via always-hit (NOT sentinel-on-miss)** — *"no backend
  IP ever reaches the client application's `recvfrom` sockaddr."* The mechanism
  is the **D1 reverse-first dual-write**: every registered backend has a visible
  reverse entry before its forward entry is usable, so a genuine service reply's
  source is always a registered backend identity → always **HITS** → is always
  rewritten to the VIP. A `REVERSE_LOCAL_MAP` **miss is non-service traffic**
  (recvmsg4 attaches at a cgroup *ancestor* and fires on EVERY unconnected-UDP
  recv from any descendant — a backend's own `recvfrom` of an inbound query, a
  DNS client's upstream answer, any unrelated same-host UDP), and on a miss
  recvmsg4 is a **pure no-op** (real source left intact, miss counter bumped
  only). recvmsg4 **cannot deny** (verifier `[1,1]`, research Q1) — every path
  returns 1. There is **no sentinel rewrite on the miss path** — rewriting the
  source on a miss would corrupt every non-service datagram's sender address
  (Tier-3-observed and fixed in DELIVER step 01-03). Corrected per DDD-3 /
  feature-delta CA-3 / research addendum "UI-1 adjudication (2026-06-05)";
  supersedes the earlier "sentinel on miss" wording.
- **K4 / connect4** — connect4 is **EXTEND**, not "0 changes" (CA-1 / DDD-4):
  its key-build + NBO is refactored to call the shared `build_local_service_key`
  helper. Net-new behavior 0; diff non-zero; Tier-3-reverified.

There are **NO `tcpdump`/wire-source assertions** for the recvmsg4 reply path
anywhere below. `bpftool map dump` (map-presence) and `recvfrom`-sockaddr
(app-source) are the Tier-3 evidence surfaces.

---

## Four-tier mapping (explicit — the no-Tier-2-backstop reality)

| Tier | Applies to this feature? | Why |
|---|---|---|
| **Tier 1 — DST (default lane)** | **YES — load-bearing** | `SimDataplane` reply-path equivalence invariant (`reply-source-rewrite-lockstep`). The structural defense BELOW Tier-3 for the J-PLAT-004 reply-source identity. Slice 02 / US-02 / K3. |
| **Tier 2 — `BPF_PROG_TEST_RUN`** | **NONE — structurally absent** | `BPF_PROG_TEST_RUN` returns ENOTSUPP for `cgroup_sock_addr` on kernel ≤ 6.8 (`cg_sock_addr_verifier_ops.test_run` is null). The two new programs (sendmsg4, recvmsg4) get **NO Tier-2 triptych** — do not scaffold one. |
| **Tier 3 — real kernel (Lima, integration lane)** | **YES — THE GATE** | Real unconnected `sendto`/`recvfrom` round-trip through `overdrive.slice`; `bpftool map dump` both maps; `recvfrom`-sockaddr source = VIP / sentinel; below-floor attach refusal. Slices 01 + 03. |
| **Tier 4 — verifier/perf** | not in DISTILL scope | The new programs land a verifier-budget baseline in DELIVER; no DISTILL scenario. |

Tier-1 (default lane) + Tier-3 (integration lane) **meet at the shared
backend identity** `(backend_ip, backend_port, proto)` — the two-pronged pin
mirroring `ReverseNatLockstep`'s shape, retargeted to the cgroup reply path.

---

## Tag legend

`@walking_skeleton` `@US-N` `@kpi-KN` `@tier1-dst` `@in-memory` `@tier3`
`@real-io` `@error` `@property` `@driving_adapter`

The WS scenario is `@walking_skeleton @tier3 @real-io @driving_adapter`. The
driving adapter is `overdrive deploy <spec>` + the unconnected
`sendto`/`recvfrom` round-trip through the real cgroup.

---

# Slice 01 — Unconnected-UDP same-host round-trip (WALKING SKELETON)

US-01 · J-OPS-004, J-PLAT-004 · Tier 3 (Lima)

## Scenario S-01-01 — A same-host resolver reaches a UDP service and reads a VIP-sourced reply (WALKING SKELETON)

`@walking_skeleton @US-01 @kpi-K1 @kpi-K2 @tier3 @real-io @driving_adapter @property`

```
Given Ana has declared a same-host DNS-shape UDP service on VIP 10.96.0.10:53
  with one local backend, deployed via `overdrive deploy dns-resolver.toml`
  And the local backend is a stub UDP responder bound off the
  systemd-resolved-owned ports (NOT 53, NOT 5353)
When a same-host client sends an unconnected datagram to 10.96.0.10:53
  via `sendto` (no prior `connect`) and reads the reply via `recvfrom`
Then the client receives a correct answer from the backend
  And the source address the client reads from `recvfrom` is 10.96.0.10
      (the VIP) — NOT the backend IP
```

Notes: the production driving path is `overdrive deploy` → hydrator emits
`RegisterLocalBackend` → `register_local_backend` dual-writes both maps →
sendmsg4 forward-rewrites VIP→backend → recvmsg4 reverse-rewrites the reply
source backend→VIP. `@property`: for ANY same-host unconnected round-trip to
the declared frontend, the `recvfrom` source is the VIP — but per Mandate 9
this is **example-pinned** at Tier 3 (one canonical round-trip), NOT
PBT-generated. App-sockaddr assertion, NOT `tcpdump`.

## Scenario S-01-02 — Forward and reverse map entries are present together after one registration

`@US-01 @kpi-K2 @tier3 @real-io`

```
Given Ana has deployed the same-host UDP service (the S-01-01 Given)
When the platform registers the local backend (one register_local_backend)
Then `bpftool map dump LOCAL_BACKEND_MAP` shows (10.96.0.10, 53, udp) -> backend
  And `bpftool map dump REVERSE_LOCAL_MAP` shows (backend_ip, backend_port,
      udp) -> 10.96.0.10
  And both entries are present after the SAME single registration
```

Notes: reuses S-01-01's `Given` (Pillar 2 chained narrative — `register` is
S-01-01's `When`). The "ordered (reverse-first)" guarantee (DDD-1, F-2) means
no observer sees a forward entry without its reverse; the Tier-3 observable is
"both present after one register", asserted via `local_backend_map_entries()`
+ the new `reverse_local_map_entries()` accessor (the `bpftool`-equivalent
dump surface, mirroring `local_backend_proto_connect.rs`).

## Scenario S-01-03 — Second unconnected query reuses the same mapping (stateless)

`@US-01 @tier3 @real-io @error`

```
Given the same deployed service and a completed first round-trip (S-01-01)
When the client sends a SECOND unconnected `sendto` to 10.96.0.10:53
  immediately after, for a different query name
Then the same LOCAL_BACKEND_MAP / REVERSE_LOCAL_MAP entries serve it
  And the second reply's `recvfrom` source is again 10.96.0.10 (the VIP)
  And no per-flow state was created between the two queries (stateless)
```

Notes: chains S-01-01's `Given + When`. Pins the "no conntrack, UDP is
stateless" contract (DD7 / DDD-1). UDP is connectionless: both `sendto` calls
target the SAME `10.96.0.10:53` VIP:port, so the kernel creates NO per-flow
state between them — the same `(vip, port, proto) -> backend` LOCAL_BACKEND_MAP
entry and the same `BackendKey(backend_ip, backend_port, proto) -> vip`
REVERSE_LOCAL_MAP entry serve both queries by point-lookup. There is no
flow-table slot allocation and no connection tracking; no connection lifecycle
is being modeled here, only repeated stateless datagram delivery. Counts toward
the ≥40% error/edge ratio (boundary: repeated-use edge case).

---

# Slice 02 — Sim≡kernel reply-path equivalence lockstep

US-02 · J-PLAT-004 (primary), J-OPS-004 · Tier 1 (DST, default lane) + Tier 3 meet

## Scenario S-02-01 — The reply-path source identity is asserted in the Sim layer

`@US-02 @kpi-K3 @tier1-dst @in-memory @property`

```
Given a SimDataplane registered with the unconnected-UDP service from US-01
  via register_local_backend(vip=10.96.0.10, vip_port=53, backend, Udp)
When the reply-path equivalence invariant evaluates the declared frontend
Then `reply_source_for(BackendKey(backend_ip, backend_port, udp))` returns
      the VIP 10.96.0.10 — the Sim reply source equals the VIP, not the backend
  And every forward local_backend entry has a matching reply-mirror entry
      mapping the backend identity back to its VIP (lockstep)
  And the invariant runs on the per-PR critical path (default lane)
```

Notes: this is the load-bearing Tier-1 structural defense (no Tier-2
backstop). Mirrors `ReverseNatLockstep`'s shape (N services × M backends,
install / shrink / restore / per-proto co-resident purge) retargeted to the
`register_local_backend` dual-write → reply mirror. `@property`: layer 1-2 →
PBT-full permitted, but the invariant body is the DST `evaluate_*` shape
(install-walk-assert), not a Hypothesis `@given` — it's a `RuleBasedStateMachine`-
free DST evaluator per the existing `reverse_nat_lockstep.rs` precedent.

## Scenario S-02-02 — A forward-only regression fails the Tier-1 invariant loudly

`@US-02 @kpi-K3 @tier1-dst @in-memory @error`

```
Given the reply-path rewrite is removed from the Sim adapter (a mutation:
  register_local_backend writes the forward local_backend entry but NOT
  the reply mirror)
When the reply-path equivalence invariant evaluates the declared frontend
Then the invariant turns RED — `reply_source_for(...)` returns None (or the
      backend IP), failing the "reply source == VIP" assertion
  And the regression is caught at PR time, not in production
```

Notes: the mutation this slice exists to kill (forward-only / asymmetric —
the #163 class). DELIVER verifies via a mutation-test target OR a deliberate
RED scaffold, NOT inspection (AC pins "verified by mutation, not inspection").
Error-path scenario.

## Scenario S-02-03 — The kernel reply rewrite is pinned at Tier 3, meeting Tier 1 at the backend identity

`@US-02 @kpi-K3 @tier3 @real-io`

```
Given the deployed service from US-01 and the real recvmsg4 hook
When a real unconnected round-trip completes (the S-01-01 path)
Then the `recvfrom` source the app reads is the VIP (the kernel reply rewrite)
  And REVERSE_LOCAL_MAP's (backend_ip, backend_port, udp) -> vip entry
      matches the Tier-1 reply mirror's entry for the same backend identity
  And removing the kernel reply rewrite would fail this Tier-3 acceptance
```

Notes: the Tier-3 prong of the two-pronged pin. Re-uses S-01-01's round-trip
(chained); the NEW assertion is the meet-at-backend-identity equivalence
(Tier-1 `reply_source_for` == Tier-3 `reverse_local_map_entries` for the same
`BackendKey`). App-sockaddr source, NOT `tcpdump`.

---

# Slice 03 — Reply-path error hardening (no-op-on-miss, no backend-IP leak, below-floor refusal)

US-03 · J-OPS-004 (primary), J-PLAT-004 · Tier 3 (Lima)

## Scenario S-03-01 — Non-service unconnected UDP reads its real source; a service reply is VIP-sourced; the miss counter is observable but inert

`@US-03 @kpi-K5 @tier3 @real-io @error`

```
Given a same-host UDP exchange whose source is NOT a registered backend
  (a plain client/server pair, or a backend reading an inbound query) AND
  a deployed service whose backend IS registered (forward + reverse present)
When an unconnected datagram from the NON-registered source traverses recvmsg4
  via the client's `recvfrom`
Then the source address the app reads from `recvfrom`/`msg_name` is the REAL
      sender address — recvmsg4 leaves it byte-for-byte intact (no rewrite)
  And the REVERSE_LOCAL_MISS_COUNTER increments on that non-service recv
      (observable), yet the source the app read on that same recv is untouched
      (the counter is behaviorally inert — counted but no source rewrite)
When a genuine service reply (source IS the registered backend identity)
  traverses recvmsg4
Then it always HITS the REVERSE_LOCAL_MAP and the app reads the VIP 10.96.0.10
      as the source — there is NO backend-IP-leak path on the service reply
  And recvmsg4 does NOT drop any datagram (it cannot — verifier [1,1])
```

Notes: THE corrected K5 (DDD-3 / CA-3 / UI-1) — app-sockaddr, NOT a
`tcpdump`/wire assertion. recvmsg4 attaches at a cgroup *ancestor* and fires on
EVERY unconnected-UDP recv from any descendant, so a `REVERSE_LOCAL_MAP` miss
means "this datagram is NOT a service reply" (a backend's own inbound-query
`recvfrom`, any unrelated UDP) — NOT "a service reply with a lost reverse
entry". The three corrected assertions: **(a)** non-service unconnected UDP
reads its REAL sender source (recvmsg4 is a pure no-op on a miss — the
load-bearing new assertion, the regression the correction fixes); **(b)** a
genuine service reply ALWAYS hits → VIP-sourced (no-leak via the D1
reverse-first dual-write's always-hit property, not a sentinel); **(c)** the
`REVERSE_LOCAL_MISS_COUNTER` increments on non-service recv AND the source is
untouched on that same recv (assert both together to pin "counted but inert").
There is **NO sentinel `192.0.2.1` rewrite on the miss path** — rewriting the
source on a miss would corrupt every non-service datagram's sender address
(Tier-3-observed, fixed DELIVER step 01-03, commit `e71ad780`). No-op-on-miss
is Cilium-aligned (`cil_sock4_recvmsg` returns `SYS_PROCEED` and
`__sock4_xlate_rev` leaves the source unchanged on a reverse-SK miss).

## Scenario S-03-02 — A below-floor kernel refuses observably at attach/preflight

`@US-03 @kpi-K5 @tier3 @real-io @error`

```
Given a host whose kernel predates recvmsg4 (< 4.20)
When the platform attaches the same-host UDP hooks (the Earned-Trust probe)
Then the `attach()` syscall fails and the composition root refuses to start
      with a structured `health.startup.refused` event
  And the platform does NOT deliver a forward-only half-working service
  And the failure routes through a `#[from]`-typed DataplaneBootError
      variant, never a flattened Internal(String)
```

Notes: the `attach()` syscall IS the below-floor preflight (DDD-5b/c) — NO
`/proc`/`uname` parse (avoids the `unwrap_or_default` boundary-read footgun).
On the 5.10+ matrix this is informational; the scenario pins the refusal
SHAPE. Mirrors ADR-0028/ADR-0034 cgroup-preflight refusal precedent. Since a
real <4.20 kernel is not on the Lima matrix, DELIVER asserts the refusal shape
via the typed-error path + the probe's structured-refusal contract (the
below-floor branch is the attach-failure branch).

## Scenario S-03-03 — The Tier-3 fixture binds off systemd-resolved's ports and asserts a clean bind

`@US-03 @tier3 @real-io @error`

```
Given the Tier-3 stub resolver fixture in the Lima VM
When it binds its UDP backend socket
Then it binds off the systemd-resolved-owned UDP 5353 (and :53)
  And a clean `bind` is asserted — an `EADDRINUSE` fails the test loudly,
      it is NOT swallowed with `.ok()` / `let _`
```

Notes: codifies `.claude/rules/debugging.md` § 11 + § 8 (no `let _` on
fallible setup). The stub resolver binds an ephemeral / non-collision port;
the fixture asserts the bind result rather than absorbing it. This is a
fixture-discipline scenario protecting every other Tier-3 test in the slice.

---

## Error/edge-path ratio (Mandate: ≥ 40%)

| Scenario | Class |
|---|---|
| S-01-01 | happy (WS) |
| S-01-02 | happy (map presence) |
| S-01-03 | **edge** (stateless reuse) |
| S-02-01 | happy (Tier-1 pin) |
| S-02-02 | **error** (forward-only mutation) |
| S-02-03 | happy (Tier-3 meet) |
| S-03-01 | **error** (non-service no-op + miss-counter inert) |
| S-03-02 | **error** (below-floor refusal) |
| S-03-03 | **error** (fixture collision) |

Error/edge: 5 of 9 = **56%** (≥ 40% ✓).

---

## Adapter / driven-component coverage

| Driven adapter / surface | Scenario(s) | Tier |
|---|---|---|
| `cgroup/sendmsg4` program (forward VIP→backend) | S-01-01, S-01-03 | @tier3 @real-io |
| `cgroup/recvmsg4` program (reply backend→VIP) | S-01-01, S-02-03, S-03-01 | @tier3 @real-io |
| `REVERSE_LOCAL_MAP` (kernel map + handle) | S-01-02, S-02-03, S-03-01 | @tier3 @real-io |
| `register_local_backend` dual-write (host adapter) | S-01-02, S-02-03 | @tier3 @real-io |
| `register_local_backend` reply mirror (sim adapter) | S-02-01, S-02-02 | @tier1-dst @in-memory |
| `REVERSE_LOCAL_MISS_COUNTER` | S-03-01 | @tier3 @real-io |
| `EbpfDataplane::probe` (attach both hooks; below-floor preflight) | S-03-02 | @tier3 @real-io |
| `cgroup/connect4` (EXTEND — helper refactor, behavior-preserving) | shipped `local_backend_proto_connect.rs` re-run (D4 risk mitigation) | @tier3 @real-io |
| Driving adapter `overdrive deploy` + unconnected round-trip | S-01-01 | @tier3 @real-io @driving_adapter |

Every NEW driven adapter has a `@real-io @tier3` scenario; the reply-path
identity additionally has the `@tier1-dst` equivalence invariant (the
no-Tier-2-backstop structural defense).
