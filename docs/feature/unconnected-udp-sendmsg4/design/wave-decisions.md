# DESIGN Decisions — unconnected-udp-sendmsg4

> **Location note.** DISCUSS decisions live in `../feature-delta.md`
> (DD1–DD7, § Locked decisions). DIVERGE decisions live under `../diverge/`.
> This file holds the DESIGN-wave decisions summary. The canonical SSOT is
> **ADR-0053 revision 2026-06-05** + `brief.md` § "Unconnected-UDP sendmsg4
> extension" + `c4-diagrams.md` § "Unconnected-UDP sendmsg4 + recvmsg4";
> this file does not supersede them.

**Architect:** Morgan. **Date:** 2026-06-05. **Mode:** GUIDE (framing pass
complete; all decisions user-locked). **Density:** lean / Tier-1.
**Paradigm:** object-oriented (project CLAUDE.md).

## Key decisions (DDD-N)

| ID | Decision | Verdict |
|---|---|---|
| **DDD-1** | Reverse store = a second BPF map `REVERSE_LOCAL_MAP` (`BPF_MAP_TYPE_HASH`), written **ordered (reverse-first)** by `register_local_backend`. NOT a reverse scan, NOT conntrack (UDP is stateless). | LOCKED |
| **DDD-2** | Reverse key = `(backend_ip, backend_port, proto)` reusing the existing `BackendKey` newtype. `backend_ip`-alone rejected (ambiguous when two services share a backend IP on different ports). | LOCKED |
| **DDD-3** | Reverse-miss = rewrite-to-sentinel `192.0.2.1` (RFC 5737) + counted miss reason. recvmsg4 **CANNOT deny** — verifier restricts it to `[1,1]` (research Q1). Strictly stronger than Cilium's pass-through-leak. | LOCKED |
| **DDD-3a** | AC reframing US-01/US-03/K2/K5 wire-layer → application-sockaddr-layer (back-prop REQUIRED). | LOCKED |
| **DDD-4** | Option 3 — ONE shared `#[inline(always)]` `build_local_service_key` helper (service-key construction + `user_port` low-16-NBO handling only) across connect4 + sendmsg4 + recvmsg4; each hook does its OWN map lookup and rewrite direction (connect4/sendmsg4 → `LOCAL_BACKEND_MAP` forward dest-rewrite; recvmsg4 → `REVERSE_LOCAL_MAP` reverse source-rewrite). ONE attach orchestration + ONE probe set. REFACTORS shipped connect4 (behavior-preserving, Tier-3-reverified, no Tier-2 backstop). **User override of Morgan's Option-2.** | LOCKED |
| **DDD-5a** | `register_local_backend` writes BOTH maps reverse-first; **NO new trait method**; contract rustdoc amended (reverse postcondition + observable invariant + per-proto edge case). | LOCKED |
| **DDD-5b/c** | Probe attaches both new hooks + round-trips a `REVERSE_LOCAL_MAP` sentinel; the `attach()` syscall **IS** the below-floor preflight (4.18/4.20, both <5.10); `#[from]`-routed error variant(s), never `Internal(String)`; NO `/proc`/`uname` parse. | LOCKED |
| **DDD-5d** | `SimDataplane` reply mirror `BTreeMap<BackendKey, Ipv4Addr>` under the SAME mutex acquisition as `local_backends`; `reply_source_for()` test accessor; models the observable contract only (does NOT shape production). | LOCKED |
| **DDD-5e** | `user_port` low-16-NBO idiom copied verbatim into the shared helper; recvmsg4 writable fields confirmed = `user_ip4`/`user_port` (`msg_src_ip4` is sendmsg-only). | LOCKED |

## Architecture summary

Style: unchanged — ports-and-adapters over the shipped `Dataplane` port
trait (`overdrive-core` defines the trait; `EbpfDataplane` is the host
adapter, `SimDataplane` the sim adapter). This feature adds two
kernel-driven adapters (the sendmsg4 + recvmsg4 BPF programs) over one new
driven port surface (the `REVERSE_LOCAL_MAP`), and extends the existing
`register_local_backend` adapter method to fill it. No new container, no
new service, no topology change. The same-host cgroup path stays disjoint
from the XDP wire path (the sibling-journey distinction).

**Earned Trust:** every new dependency is probed. The two new hooks are
attach-probed (the attach syscall is the below-floor preflight); the new
map is sentinel-round-tripped; the composition root refuses to start
(`health.startup.refused`) on any probe failure. The reverse-miss sentinel
+ counter is the Earned-Trust answer to "what if the map lies / is evicted"
on the reply hot path — it fails clean and observably rather than leaking.

## Reuse Analysis (HARD GATE — full table in feature-delta § DESIGN)

**CREATE NEW (6):** `REVERSE_LOCAL_MAP` map, `ReverseLocalMapHandle`,
`cgroup_sendmsg4_service`, `cgroup_recvmsg4_service`, `build_local_service_key`
shared key-build helper, `REVERSE_LOCAL_MISS_COUNTER`.

**EXTEND:** `cgroup_connect4_service` (refactor to shared helper — the one
item DISCUSS called UNCHANGED, now EXTEND per D4), `register_local_backend`/
`deregister_local_backend` (trait + both adapters), `EbpfDataplane`
probe/boot, `DataplaneError`/`DataplaneBootError`.

**REUSE:** `BackendKey`, `Proto`, `LOCAL_BACKEND_MAP` (+ handle),
`Action::RegisterLocalBackend`, hydrator classifier, `cgroup_attach_path`.

## Technology stack

No new third-party dependency. `aya` (existing BPF loader, MIT/Apache-2.0);
`BPF_MAP_TYPE_HASH` (reverse map), `BPF_MAP_TYPE_PERCPU_ARRAY` (miss
counter) — both kernel-native. Kernel floors 4.18 (sendmsg4) / 4.20
(recvmsg4), both below the shipped 5.10 LTS floor — no matrix bump.
Sentinel `192.0.2.1` (RFC 5737). Enforcement: existing `dst-lint`
crate-class gate + `BackendKey`/`Proto` newtype proptest roundtrips; no new
architecture-test tool warranted. No external API → no consumer-driven
contract test.

## Constraints (carried from DISCUSS)

All honored: kernel floors below 5.10 (no bump); proto zero-translation
(`bpf_sock_addr.protocol`); `user_port` low-16-NBO hazard (one shared site
per D5e); no Tier-2 backstop for `cgroup_sock_addr` (Tier-3-only correctness
+ Tier-1 Sim equivalence); ordered (reverse-first) reverse write, no conntrack (D1);
fixture avoids systemd-resolved UDP 5353; single-cut migration, no shim
(the reverse map is hydrator-repopulated from intent on boot); no agent
GitHub-issue creation (#200 covers this).

## Upstream (back-prop) changes

Recorded in `upstream-changes.md` (this directory): (a) US-01/US-03/K2/K5
AC reframing wire → application-sockaddr layer (DDD-3a); (b) DISCUSS
K4/DD6 "0 connect4 changes / pure addition" → connect4 EXTEND (DDD-4).

## Tier mapping (for DISTILL/DELIVER)

- **Tier 1 (DST, default lane):** `SimDataplane` reply-path equivalence
  invariant — reply source = VIP for the declared frontend; a forward-only
  Sim mutation turns it RED (J-PLAT-004, US-02 / K3).
- **Tier 2:** NONE — `BPF_PROG_TEST_RUN` returns ENOTSUPP for
  `cgroup_sock_addr` ≤ 6.8. The structural defense below Tier-3 is the
  Tier-1 invariant above.
- **Tier 3 (Lima, integration lane):** real `dig`/`sendto` unconnected
  round-trip (US-01 / K1); `recvfrom`-sockaddr source = VIP assertion (K2);
  `bpftool map dump` shows both forward + reverse entries after one
  registration; forced reverse-miss → sentinel + counter, no backend IP at
  the app sockaddr (US-03 / K5); below-floor attach refusal (US-03);
  connect4 round-trip re-run against the helper-backed connect4 (D4 risk
  mitigation).

## Open questions (to DELIVER / Tier-3)

1. **Sentinel resolver-rejection** — confirm `dig`/`getaddrinfo`/musl
   cleanly reject a `192.0.2.1`-sourced reply (research Gap 2). Mechanism is
   sound regardless of value; swap the address if not. Surfaced for user
   awareness; NOT a tracking issue (`feedback_no_unilateral_gh_issues`).
2. **Research Gap 1** (non-blocking) — exact verifier `[1,1]` file:line +
   v5.10 `udp_recvmsg` call site; optional crafter pin in a local checkout.

## Handoff

DESIGN baseline ready for DISTILL (acceptance-designer: the BDD scenarios +
the Tier-1 equivalence invariant + the Tier-3 round-trip/forced-miss/
below-floor acceptances + K1–K5) and the DEVOPS/platform-architect handoff
(K1–K5 only; Tier-3 instrumentation). No external integrations → no
contract-test annotation. **Per-wave architect review DEFERRED** to the
mandatory consolidated review at end of DISTILL (not self-invoked here).
