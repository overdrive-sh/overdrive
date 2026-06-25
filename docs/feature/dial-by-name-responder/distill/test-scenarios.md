# Test scenarios — `dial-by-name-responder`

**Wave**: DISTILL — **RE-DISTILL REV-2 (stable-frontend / ClusterIP split)** | **Mode**: PROPOSE | **Designer**: Quinn | **Date**: 2026-06-25

> **REV-2 re-distill notice.** This spec is revised to the **ratified
> ADR-0072 REV-2 stable-frontend contract** (commit `8e22f499`). The
> superseded REV-1 contract (DNS answers a *volatile per-instance backend
> addr*) is REPLACED: **DNS now answers a STABLE per-`<job>` IPv4 frontend
> addr `F` in `10.98.0.0/16`** and the already-live dataplane (nft-TPROXY +
> per-connection `MtlsResolve`, ADR-0071 Path-A) owns backend churn. See
> ADR-0072 § "Changed Assumptions (REV-2)" + feature-delta §§ "Frontend-key
> contract" / "Frontend lifecycle contract" / "Frontend-subnet coherence
> contract" for the contract this spec distills. PRESERVED unchanged:
> S-DBN-NAME-01..04 (01-01, `MeshServiceName`) and S-DBN-WIRE-01..04 (01-02,
> the codec) — both are addr-agnostic (`Records(Vec<SocketAddrV4>)` holds
> whatever IPv4 addr it is handed; ADR-0072 / research SQ3) and COMMITTED.

**Scope**: executable spec for GH #243 (the in-agent node-local DNS
responder — the THIRD reader of the `service_backends` observation
surface, REV-2 stable-frontend split). Covers US-DBN-2 (walking skeleton
A→B), US-DBN-3 (bidirectional ping-pong demo), US-DBN-4 (empty-candidate
NXDOMAIN honesty), and the pure `MeshServiceName` / `wire.rs` /
`answer_for` / `NameIndex` / `FrontendAddrAllocator` / re-keyed-`MtlsResolve`
seams. US-DBN-1 (the spike) is PROMOTED already (`spike/wave-decisions.md`)
and is not re-tested here; BLOCKER-1 (frontend-subnet capture) is RESOLVED →
WORKS (`spike/findings-blocker1-frontend-addr-capture.md`).

**Strategy**: Tier 1 (DST/in-process pure, `tests/acceptance/`, default
lane) + Tier 3 (real-kernel Lima, `tests/integration/`, gated
`#![cfg(feature="integration-tests")]`). **No `.feature` files** — per
`.claude/rules/testing.md` § "No `.feature` files anywhere", this
document is the GIVEN/WHEN/THEN SSOT; DELIVER's RED phase translates each
scenario into a Rust `#[test]`/`#[tokio::test]` body or a proptest. There
is **no Tier 2** here — the socket loop is irreducibly Tier-3 (no
`BPF_PROG_TEST_RUN` surface; DDN-4). Proptest is the Tier-1 tool for the
pure seams (`MeshServiceName` round-trip, `answer_for`, the `wire.rs`
codec, the `FrontendAddrAllocator` assign/release, the re-keyed `classify`).

**Driving ports** (entry points exercised by these scenarios):

- **`getaddrinfo`/`getent` (glibc stub resolver)** — the workload's
  actual name-resolution path. The Tier-3 acceptance SIGNAL. **NEVER
  `dig @gw` alone** — the spike proved `dig` is lenient and masks a
  missing `ipi_spec_dst` source-pin (`spike/findings.md` edge case 2;
  ADR-0072 § "Design constraints inherited from the spike" #1).
- **`overdrive deploy <SPEC>`** — the in-process deploy submit handler
  (`POST /v1/jobs`), the operator workload verb. Every Tier-3 scenario
  deploys through it (mirroring
  `canonical_address_inbound_walking_skeleton.rs::run_server_deploy`).
- **`overdrive serve`** — the composition root
  (`run_server_with_obs_and_driver`, `lib.rs` ~1893-1957), the boot
  entry point. Where `DnsResponder::{new, probe, serve}` is constructed,
  probed, and spawned (DDN-6).
- **`answer_for(name, qtype, &index)`** — the pure DST seam (the
  mutation-gate target, DDN-4). The driving "port" for the Tier-1
  contract scenarios; the socket loop is its irreducibly-Tier-3 shell.
  **REV-2: the answered `Records` holds the stable per-`<job>` frontend
  addr `F`, not a backend addr.** The `answer_for` SIGNATURE is UNCHANGED
  (`answer_for(name, qtype: hickory_proto::rr::RecordType, &NameIndex) ->
  NameAnswer`); only what the `NameIndex` *maps to* changes (a stable `F`,
  not a backend-addr set).
- **`FrontendAddrAllocator::{assign, release, snapshot}`** — the NEW pure
  per-`<job>` stable-frontend allocator (REV-2 1a-A, step 01-04). The
  driving port for the Tier-1 allocator scenarios; idempotent per `<job>`,
  carving from `WORKLOAD_FRONTEND_BASE = 10.98.0.0/16`.
- **`MtlsResolve.resolve(orig_dst, proto)` → re-keyed `classify`** — the
  EXTENDED translation seam (REV-2 1b-A, step 02-00). `by_frontend:
  BTreeMap<FrontendKey, ServiceId>` translates `(F, listener.port, proto)`
  → a current running-AND-healthy backend; the three-way `classify` arm
  (hit → translate-to-`Mesh`; frontend-subnet miss → `MeshUnreachable`
  fail-closed; else `by_addr` fall-through) is the driving port for the
  re-key + fail-closed scenarios.

**Production code under test → scenario mapping** lives in §
"Production code → scenario mapping (mutation testing scope)" below.

**Pinned-signature discipline (CLAUDE.md "Implement to the design").**
Every type/fn/variant these scenarios name is EXACTLY the DESIGN-pinned
surface (`feature-delta.md` § "Pinned signatures (FINAL — 1a-A / 1b-A
ratified)", ADR-0072 § Components + § "Changed Assumptions (REV-2)").
DISTILL invents NO API. The open surface decisions NAMED (not picked) here:
**OQ-1** (the `SpiffeId` → `<job>` accessor) and two DELIVER details the
REV-2 contract explicitly leaves open — the `FrontendKey` newtype-vs-tuple
shape and the `orig_dst.ip() ∈ 10.98.0.0/16` CIDR-membership accessor (see
§ "What these scenarios do NOT cover").

---

## Scenario tag glossary

| Tag | Meaning |
|---|---|
| `@walking_skeleton` | The US-DBN-2 / US-DBN-3 e2e gate; Tier 3 Lima through `serve` + `deploy`. |
| `@driving_adapter` | Exercises the `overdrive deploy` submit handler (`POST /v1/jobs`) and/or the `getaddrinfo`/`getent` resolution path. |
| `@real-io` | Real kernel UDP socket + `IP_PKTINFO`, real `getaddrinfo`/`getent`, real netns/veth, real `EbpfDataplane` + intercept. |
| `@in-memory` | `SimObservationStore` / in-process pure; no real I/O. Tier 1. |
| `@property` | Universal invariant; proptest proves over arbitrary inputs (Tier 1-2 only, Mandate 9). |
| `@error_path` | Negative / failure-mode scenario. |
| `@boot` | Exercises the `run_server` boot composition (`DnsResponder::probe` / refuse-boot). |
| `@kpi` | Anchored to a K-DBN-* KPI contract. |
| `@edd` | Graduates to a `verification/expectations/` EDD expectation. |
| `@dbn-US-N` | US-DBN-N AC traceability. |
| `@frontend` | Exercises the REV-2 stable per-`<job>` frontend addr `F` (allocator, the answered addr, or the `by_frontend` translation). |
| `@fail_closed` | Exercises the fail-closed-on-frontend-subnet-miss `classify` arm (Finding-3 — a `10.98.0.0/16` miss → `MeshUnreachable`, NO cleartext). |
| `@churn` | Exercises backend churn (alloc cycle or mid-connection death) against the stable `F`. |

---

## Scenario index

| ID | Title | Tags | Tier | US trace | REV-2 status |
|---|---|---|---|---|---|
| S-DBN-NAME-01 | Mesh service name round-trips through Display / FromStr / serde | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 | PRESERVED |
| S-DBN-NAME-02 | Mesh service name parse is case-insensitive, canonical form is lowercase | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 | PRESERVED |
| S-DBN-NAME-03 | Suffix grammar accepts `<job>.svc.overdrive.local`, rejects wrong / missing suffix | `@in-memory` `@error_path` `@dbn-US-2` | Tier 1 | US-DBN-2 | PRESERVED |
| S-DBN-NAME-04 | Over-long label and empty / malformed `<job>` are rejected with a typed `IdParseError` | `@property` `@in-memory` `@error_path` `@dbn-US-2` | Tier 1 | US-DBN-2 | PRESERVED |
| S-DBN-FRONTEND-01 | Each `<job>` is assigned a stable frontend addr in `10.98.0.0/16`, disjoint from the workload and VIP subnets | `@property` `@in-memory` `@frontend` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (01-04) |
| S-DBN-FRONTEND-02 | The frontend addr is retained across an alloc cycle (idempotent `assign`), and across a zero-healthy window | `@property` `@in-memory` `@frontend` `@churn` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (01-04) |
| S-DBN-FRONTEND-03 | A frontend addr is released ONLY on logical-workload deletion, never on a transient zero-healthy state | `@property` `@in-memory` `@frontend` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 | NEW (01-04) |
| S-DBN-FRONTEND-04 | Distinct `<job>`s get collision-free distinct frontend addrs; the block is reclaimed on release | `@property` `@in-memory` `@frontend` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (01-04) |
| S-DBN-ANSWER-01 | `A` for a resolvable `<job>` yields `Records` of exactly the stable frontend addr | `@property` `@in-memory` `@frontend` `@kpi` `@dbn-US-2` | Tier 1 | US-DBN-2 | RE-DISTILL |
| S-DBN-ANSWER-02 | `A` for a `<job>` with 0 running-and-healthy backends yields `NxDomain` (withheld) | `@property` `@in-memory` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 | RE-DISTILL |
| S-DBN-ANSWER-03 | `AAAA` on a resolvable `<job>` yields `NoData`; on a withheld `<job>` yields `NxDomain` | `@property` `@in-memory` `@error_path` `@dbn-US-2` `@dbn-US-4` | Tier 1 | US-DBN-2, US-DBN-4 | RE-DISTILL |
| S-DBN-ANSWER-04 | An unhealthy-only `<job>` is withheld → `NxDomain` (the healthy gate governs resolvability) | `@property` `@in-memory` `@error_path` `@kpi` `@dbn-US-4` | Tier 1 | US-DBN-4 | RE-DISTILL |
| S-DBN-ANSWER-05 | An unknown name yields `NxDomain` | `@in-memory` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 | PRESERVED |
| S-DBN-WIRE-01 | Answered records survive a deterministic encode→decode round-trip | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 | PRESERVED |
| S-DBN-WIRE-02 | AAAA on a live name encodes NODATA with a 1-second negative-TTL SOA | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 | PRESERVED |
| S-DBN-WIRE-03 | A withheld `<job>` encodes NXDOMAIN with a 1-second negative-TTL SOA | `@property` `@in-memory` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 | PRESERVED |
| S-DBN-WIRE-04 | SOA SERIAL is derived from the injected Clock (deterministic per `Clock` reading) | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 | PRESERVED |
| S-DBN-IDX-01 | Index seeds from List at probe; a `<job>` becomes resolvable to its stable `F` once a running-and-healthy row exists | `@property` `@in-memory` `@frontend` `@dbn-US-2` | Tier 1 | US-DBN-2 | RE-DISTILL |
| S-DBN-IDX-02 | A `<job>` going zero-healthy on the watch path is withheld (NXDOMAIN); the stable `F` is NOT released | `@property` `@in-memory` `@frontend` `@churn` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 | RE-DISTILL |
| S-DBN-IDX-03 | A `Lagged` subscription event triggers a relist that recovers the index | `@property` `@in-memory` `@error_path` `@dbn-US-2` | Tier 1 | US-DBN-2 | RE-DISTILL |
| S-DBN-IDX-04 | The answered `F` is the allocator's binding; the index introduces no second source of frontend truth | `@property` `@in-memory` `@frontend` `@kpi` `@dbn-US-2` | Tier 1 | US-DBN-2 | RE-DISTILL |
| S-DBN-REKEY-01 | `resolve(F, Tcp)` translates the frontend key to a current running-and-healthy backend, classified `Mesh` | `@property` `@in-memory` `@frontend` `@kpi` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (02-00) |
| S-DBN-REKEY-02 | A `(F, port, proto)` key with a known service but zero healthy backends classifies `MeshUnreachable` | `@property` `@in-memory` `@frontend` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 | NEW (02-00) |
| S-DBN-REKEY-03 | The frontend key discriminates proto — same `(F, port)` on tcp vs udp resolves to distinct services | `@property` `@in-memory` `@frontend` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (02-00) |
| S-DBN-REKEY-04 | A direct backend-addr dial still resolves via `by_addr` (the re-key is additive, backward-compatible) | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (02-00) |
| S-DBN-FAILCLOSED-01 | A `10.98.0.0/16` miss classifies `MeshUnreachable` (refuse, NO cleartext); a non-frontend miss stays `NonMesh` | `@property` `@in-memory` `@fail_closed` `@error_path` `@kpi` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (02-00) |
| S-DBN-COHERENCE-01 | A single ordered drain updates `by_frontend` before `name_index` exposes `F` (DNS never answers an `F` resolve has not learned) | `@property` `@in-memory` `@frontend` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (02-00) |
| S-DBN-EQUIV-01 | Host and in-memory `MtlsResolve` agree on the re-keyed `classify` over the same call sequence (DST equivalence) | `@property` `@in-memory` `@frontend` `@dbn-US-2` | Tier 1 | US-DBN-2 | NEW (02-00) |
| S-DBN-WS | Walking skeleton — a deployed workload resolves its peer's stable frontend name and the hop is mTLS'd | `@walking_skeleton` `@driving_adapter` `@real-io` `@frontend` `@kpi` `@dbn-US-2` | Tier 3 | US-DBN-2 | RE-DISTILL |
| S-DBN-WS-STABLE | The answered `F` is byte-stable across an alloc cycle; the next connect lands the NEW backend | `@real-io` `@frontend` `@churn` `@kpi` `@dbn-US-2` | Tier 3 | US-DBN-2 | NEW (02-02) |
| S-DBN-CHURN | Cycling a backend mid-connection gives the client a prompt reset bounded by `TCP_USER_TIMEOUT`, never an indefinite hang | `@real-io` `@churn` `@error_path` `@kpi` `@dbn-US-4` | Tier 3 | US-DBN-4 | NEW (02-02) |
| S-DBN-SINGLE-SRC | Single-source oracle — the answered `F` is the addr `MtlsResolve` recognizes and translates to a `Mesh` backend | `@real-io` `@frontend` `@kpi` `@dbn-US-2` | Tier 3 | US-DBN-2 | RE-DISTILL |
| S-DBN-PINGPONG | Bidirectional ping-pong demo — two services dial each other by stable frontend name, counters advance | `@walking_skeleton` `@real-io` `@edd` `@kpi` `@dbn-US-3` | Tier 3 | US-DBN-3 | RE-DISTILL |
| S-DBN-NXDOMAIN-01 | Querying before running-and-healthy yields NXDOMAIN, never a stale addr | `@real-io` `@error_path` `@kpi` `@dbn-US-4` | Tier 3 | US-DBN-4 | RE-DISTILL |
| S-DBN-NXDOMAIN-02 | After all backends stop, the `<job>` is withheld (NXDOMAIN); the stable `F` is NOT released | `@real-io` `@error_path` `@frontend` `@churn` `@kpi` `@dbn-US-4` | Tier 3 | US-DBN-4 | RE-DISTILL |
| S-DBN-NXDOMAIN-03 | An unknown name yields NXDOMAIN through `getent` | `@real-io` `@error_path` `@dbn-US-4` | Tier 3 | US-DBN-4 | PRESERVED |
| S-DBN-BIND-01 | Wildcard `0.0.0.0:53` + `IP_PKTINFO` coexists with systemd-resolved; replies source-pinned | `@boot` `@real-io` `@dbn-US-2` | Tier 3 | US-DBN-2 | PRESERVED |
| S-DBN-BIND-02 | Per-gateway-addr fallback re-derives the bound socket set from `NetSlotAllocator` on the converge tick | `@boot` `@real-io` `@error_path` `@dbn-US-2` | Tier 3 | US-DBN-2 | PRESERVED |
| S-DBN-BIND-03 | Boot refuses (`health.startup.refused`) on an unbindable port or an unreadable store | `@boot` `@real-io` `@error_path` `@dbn-US-2` | Tier 3 | US-DBN-2 | PRESERVED |

**Counts**: 39 scenarios (28 Tier 1, 11 Tier 3).
**Error-path coverage**: S-DBN-NAME-03, S-DBN-NAME-04, S-DBN-FRONTEND-03,
S-DBN-ANSWER-02, S-DBN-ANSWER-03, S-DBN-ANSWER-04, S-DBN-ANSWER-05,
S-DBN-WIRE-03, S-DBN-IDX-02, S-DBN-IDX-03, S-DBN-REKEY-02,
S-DBN-FAILCLOSED-01, S-DBN-CHURN, S-DBN-NXDOMAIN-01, S-DBN-NXDOMAIN-02,
S-DBN-NXDOMAIN-03, S-DBN-BIND-02, S-DBN-BIND-03 = **18 of 39 = 46%**.
Target (≥40%) met — the responder's whole posture is fail-honest (the
empty-candidate / withheld NXDOMAIN behaviour is the load-bearing US-DBN-4
leg) AND the REV-2 fail-closed-on-frontend-subnet-miss arm is a deliberate
negative-path defense, so the error surface stays deep.

**REV-2 scenario-delta summary** (vs the superseded REV-1 26-scenario spec
→ 39 scenarios: 13 PRESERVED + 13 RE-DISTILL + 13 NEW):
- **PRESERVED unchanged (13)**: S-DBN-NAME-01..04, S-DBN-WIRE-01..04 (both
  addr-agnostic, committed 01-01/01-02), S-DBN-ANSWER-05 (unknown→NXDOMAIN),
  S-DBN-NXDOMAIN-03 (unknown→NXDOMAIN e2e), S-DBN-BIND-01/02/03 (the socket
  bind/fallback/refuse-boot — the substrate is addr-agnostic).
- **RE-DISTILLed (13)**: S-DBN-ANSWER-01..04 (answer the stable `F`, not a
  backend set; the healthy gate governs *resolvability*, not *which addr*),
  S-DBN-IDX-01..04 (index maps `<job>` → stable `F`; withhold-not-release),
  S-DBN-WS / S-DBN-SINGLE-SRC / S-DBN-PINGPONG (resolve `F`, intercept `F`,
  translate `F` → live backend), S-DBN-NXDOMAIN-01/02 (withhold; `F` retained).
- **NEW (13)**: S-DBN-FRONTEND-01..04 (the `FrontendAddrAllocator`, 01-04 —
  4); S-DBN-REKEY-01..04 + S-DBN-FAILCLOSED-01 + S-DBN-COHERENCE-01 +
  S-DBN-EQUIV-01 (the re-keyed `MtlsResolve` + fail-closed + ordered-drain +
  DST equivalence, 02-00 — 7); S-DBN-WS-STABLE + S-DBN-CHURN (the
  stable-across-cycle AC + the Tier-3 churn AC, 02-02 — 2).

---

## Tier-3 fixture knobs (inherited from `canonical_address_inbound_walking_skeleton.rs`)

The Tier-3 scenarios (S-DBN-WS, S-DBN-SINGLE-SRC, S-DBN-PINGPONG,
S-DBN-NXDOMAIN-*, S-DBN-BIND-*) mirror the keystone's boot/deploy/netns
shape exactly. These knobs are PINNED here so DELIVER does not re-derive
them.

### K1 — Root gate + kernel record

Every Tier-3 scenario opens with the keystone's `is_root()` SKIP
(`libc::getuid() == 0`; the responder's real UDP `:53` bind +
`IP_PKTINFO` + per-workload netns provision + real `EbpfDataplane` XDP
attach all need `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`). A non-root run SKIPs
cleanly, never fails. `uname -r` is recorded to stderr. **MERGE GATE =
the pinned-6.18 Tier-3 matrix (ADR-0068)** — dev-Lima `7.0.0-22-generic`
(the spike kernel) is necessary-but-not-sufficient; the responder's
exercised surfaces (`IP_PKTINFO`, multi-homed UDP, per-netns
`resolv.conf`, `SO_REUSEADDR` wildcard coexistence) are long-stable but
MUST be re-confirmed on 6.18 (ADR-0072 § DEVOPS/Tier-3 obligation).

### K2 — The resolution probe is `getent`, NOT `dig`

The acceptance SIGNAL for every name-path Tier-3 scenario is
`getent ahosts <name>` / `getent hosts <name>` (or a real
`getaddrinfo()` call) run **from inside the deployed client workload's
netns**, NOT `dig @<gw>`. The spike proved a `dig +short @gw` query
succeeds even when the reply source-pin is missing, while
`getaddrinfo`/glibc rejects a reply whose source ≠ the queried server
(`spike/findings.md` edge case 2). A scenario that asserts on `dig`
alone would PASS a broken responder — the reviewer MUST flag any such
scenario.

- **Budget**: 5 s (resolution should land sub-second once the `<job>` has a
  running-and-healthy backend; the budget absorbs index-watch propagation).
- **Cadence**: 200 ms poll (mirrors the keystone's `poll_until`).
- **REV-2 resolved value**: `getent <job>.svc.overdrive.local` resolves to
  the **stable per-`<job>` frontend addr `F` in `10.98.0.0/16`** (NOT a
  per-instance backend addr). The subsequent connect to `(F, listener.port)`
  is captured by the production egress nft-TPROXY (BLOCKER-1 WORKS — the
  capture is destination-blind, `findings-blocker1-frontend-addr-capture.md`)
  and `MtlsResolve` (re-keyed) translates `(F, port, Tcp)` → the current live
  backend.
- **Failure message** names the equally-likely culprits: `getent <name> did
  not resolve to the stable frontend addr within 5s — the responder
  source-pin (ipi_spec_dst), the FrontendAddrAllocator binding, or the
  name_index withhold-on-zero-healthy gate regressed`.

### K3 — The client workload's resolution + dial program

The client workload program (the thing that calls `getaddrinfo` then
`connect`) is the same staged tiny Rust bin decided for the ping-pong
demo (`feature-delta.md` § Wave-Decisions DISCUSS #5 — the
`coinflip-helper` precedent), OR a `/usr/bin/python3 -c` one-liner that
does `socket.getaddrinfo` + `connect` (the keystone's server-spec
precedent at `canonical_address_inbound_walking_skeleton.rs:712`). The
`[exec].command` MUST point at a **real on-disk binary** present in the
deploy env (no phantom paths — the `dns-resolver.toml` collision class).
DELIVER picks the concrete form; both satisfy the AC. Listener ports
avoid `5353` (`systemd-resolved` owns it) and `53` (the responder owns
it).

---

## Scenarios

### S-DBN-NAME-01 — Mesh service name round-trips through Display / FromStr / serde

**Tags**: `@property` `@in-memory` `@dbn-US-2`
**US trace**: US-DBN-2 (the `mesh_dns_name` shared artifact)
**Driving port**: `MeshServiceName::{new, as_str}` + `Display` / `FromStr` / serde (the newtype's own public surface IS the driving port — pure-function port-to-port)
**Test surface**: Tier 1 — `crates/overdrive-core/tests/acceptance/core_newtype_roundtrip.rs` (extend the existing newtype-roundtrip suite; MANDATORY proptest per `.claude/rules/testing.md` § "Property-based testing → Newtype roundtrip")
**Production code guarded**: `MeshServiceName::new`, `as_str`, `Display`, `FromStr`, `Serialize`/`Deserialize` (`crates/overdrive-core/src/id.rs`)

#### Spec

```
PROPERTY: for every valid <job> label L (a DNS-1123-label-like string,
          1..=LABEL_MAX chars after accounting for the fixed
          ".svc.overdrive.local" suffix),
GIVEN a MeshServiceName N constructed from "<L>.svc.overdrive.local"
WHEN N is rendered via Display and re-parsed via FromStr
THEN the re-parsed value equals N
  AND N.as_str() yields the canonical <job> label (lowercase)
  AND serde_json::to_string(&N) yields the quoted Display form
  AND serde_json::from_str of that quoted form yields N back
       (serde matches Display/FromStr exactly — the mandatory newtype rule)
```

**Universe** (port-exposed): the parsed value, `as_str()` output, the
serde JSON string. **No internal-field assertion** — the test never
names whether `MeshServiceName` stores the `<job>` label or the full
name (that is a DELIVER detail per the pinned-signatures note). Pinned
`@example("server")` preserves a domain-readable canonical case.

---

### S-DBN-NAME-02 — Mesh service name parse is case-insensitive, canonical form is lowercase

**Tags**: `@property` `@in-memory` `@dbn-US-2`
**US trace**: US-DBN-2
**Driving port**: `MeshServiceName::new` / `FromStr` (case-folding entry)
**Test surface**: Tier 1 — `core_newtype_roundtrip.rs`
**Production code guarded**: `MeshServiceName::new` case-folding + `Display` lowercase emission (`id.rs`)

#### Spec

```
PROPERTY: for every valid <job> label L and every case-permutation P of
          "<L>.svc.overdrive.local" (mixed upper/lower in BOTH the label
          and the suffix),
GIVEN P
WHEN MeshServiceName::new(P) is called
THEN it succeeds
  AND the result equals MeshServiceName::new of the all-lowercase form
       (case-insensitive parse, lowercase canonical — matches WorkloadId /
        the validate_label precedent, id.rs:99)
  AND Display emits the lowercase form
```

**Why this matters**: workloads type the name as it appears in their
config; the suffix grammar and the `<job>` label both fold case so
`Server.SVC.Overdrive.Local` and `server.svc.overdrive.local` resolve
identically. This is the `mesh_dns_name` integration risk (name-grammar
drift) the shared-artifact registry flags.

---

### S-DBN-NAME-03 — Suffix grammar accepts `<job>.svc.overdrive.local`, rejects wrong / missing suffix

**Tags**: `@in-memory` `@error_path` `@dbn-US-2`
**US trace**: US-DBN-2 (negative — the bespoke `FromStr` suffix grammar)
**Driving port**: `MeshServiceName::new` / `FromStr`
**Test surface**: Tier 1 — `crates/overdrive-core/tests/acceptance/core_newtype_validation.rs` (extend; rejection asserts the `IdParseError` variant)
**Production code guarded**: `MeshServiceName::new` suffix-grammar branch (the bespoke `FromStr` the design notes `validate_label` alone cannot provide, since it permits `.` — `id.rs:102`), `SUFFIX` const

#### Spec

```
GIVEN the example matrix:
  | input                              | result                       |
  | "server.svc.overdrive.local"       | accepted, <job> = "server"   |
  | "payments-api.svc.overdrive.local" | accepted, <job> = "payments-api" |
  | "server.svc.example.com"           | rejected (wrong suffix)      |
  | "server.svc.overdrive.local.evil"  | rejected (suffix not terminal) |
  | "server"                           | rejected (missing suffix)    |
  | "server.overdrive.local"           | rejected (missing .svc segment) |
  | ".svc.overdrive.local"             | rejected (empty <job> label) |
WHEN MeshServiceName::new(input) is called
THEN accepted inputs yield Ok with as_str() == the expected <job>
  AND rejected inputs yield Err(IdParseError::<variant>)
```

**Note**: which `IdParseError` variant each rejection maps to is a
DELIVER detail (the design pins the public surface — `Result<Self,
IdParseError>` — not the per-case variant). The scenario asserts
`is_err()` + that the error is an `IdParseError`, and pins the
accepted-case `<job>` extraction. DELIVER refines the per-case variant
when it writes `MeshServiceName::new`.

---

### S-DBN-NAME-04 — Over-long label and empty / malformed `<job>` are rejected with a typed `IdParseError`

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-2`
**US trace**: US-DBN-2 (negative — label-limit / character-class)
**Driving port**: `MeshServiceName::new` / `FromStr`
**Test surface**: Tier 1 — `core_newtype_validation.rs`
**Production code guarded**: `MeshServiceName::new` label validation (reusing `validate_label` / `LABEL_MAX` per the Reuse gate, `id.rs:92-120`)

#### Spec

```
PROPERTY: for every <job> label L that violates the DNS-1123-label rules
          (empty, > LABEL_MAX after the suffix budget, starts/ends with
          a non-alphanumeric, or contains an out-of-class character),
GIVEN "<L>.svc.overdrive.local"
WHEN MeshServiceName::new is called
THEN it returns Err(IdParseError::<variant>) — never panics, never
     silently truncates (the "one shared length ceiling" rule:
     MeshServiceName sizes its own ceiling off LABEL_MAX, never a bespoke
     smaller magic number; development.md § "One shared length ceiling")
```

**Negative-testing note** (Hebert ch.6): this property RELAXES the
happy-path assumption "the label is well-formed" to surface any
under-specified accept path. If a malformed label is accepted, the
suffix grammar is under-specified.

---

### S-DBN-FRONTEND-01 — Each `<job>` is assigned a stable frontend addr in `10.98.0.0/16`, disjoint from the workload and VIP subnets

**Tags**: `@property` `@in-memory` `@frontend` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 1a-A, step 01-04 — the stable per-`<job>` frontend) · **KPI K-DBN-1**
**Driving port**: `FrontendAddrAllocator::assign(job)` (the NEW pure allocator's public surface IS the driving port — pure-function port-to-port)
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/dns_frontend_allocator.rs` (NEW; the `frontend_addr_allocator.rs` proptest home)
**Production code guarded**: `FrontendAddrAllocator::assign` + the `WORKLOAD_FRONTEND_BASE = 10.98.0.0/16` const (REV-2 1a-A; `dns_responder/frontend_addr_allocator.rs`)

#### Spec

```
PROPERTY: for every <job> label J,
GIVEN a fresh FrontendAddrAllocator
WHEN assign(J) is called
THEN it returns Ok(F) where F is an Ipv4Addr
  AND F ∈ WORKLOAD_FRONTEND_BASE (10.98.0.0/16)
  AND F ∉ WORKLOAD_SUBNET_BASE (10.99.0.0/16)  — disjoint from per-netns /30s
  AND F ∉ the service-VIP range (10.96.0.0/16, VipRange::default())
       — the collision the spike's 10.96.0.0/16 candidate was REJECTED for
       (ADR-0072 § Collision check)
```

**Why disjointness is load-bearing** (ADR-0072 § "Pinned frontend
subnet"): overlap with `10.99.0.0/16` would make `F` on-link to some
per-alloc `/30` (changing the carrying route from the per-netns default
route BLOCKER-1 validated) AND catch it in the `service_map_hydrator`
mesh-gate membership test (`Backend.addr ∈ 10.99.0.0/16`). Overlap with
`10.96.0.0/16` would put two uncoordinated allocators on one block (the
addressing-collision defect class, `development.md` § "Check-and-act must
be atomic"). The two named consts (`WORKLOAD_SUBNET_BASE` = `10.99.0.0/16`
at `veth_provisioner.rs:307`; `VipRange::default()` = `10.96.0.0/16`) are
real and verified — the property asserts membership against them, not a
magic number. Pinned `@example`: `J = "server"`, asserting `F` is the
allocator's first stable assignment in `10.98.0.0/16`. The pure
`smallest_free`-style slot scan is the mutation-gate target (mirror
`NetSlotAllocator`'s pure scan split).

---

### S-DBN-FRONTEND-02 — The frontend addr is retained across an alloc cycle (idempotent `assign`), and across a zero-healthy window

**Tags**: `@property` `@in-memory` `@frontend` `@churn` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 Finding-2 — `F` per logical workload, the SQ1-elimination contract) · **KPI K-DBN-1**
**Driving port**: `FrontendAddrAllocator::assign(job)` called twice for the same `<job>` (idempotent)
**Test surface**: Tier 1 — `dns_frontend_allocator.rs`
**Production code guarded**: `FrontendAddrAllocator::assign` idempotency (the atomic per-`<job>` claim, `development.md` § "Check-and-act must be atomic" — the `assign` return IS the claim)

#### Spec

```
PROPERTY: for every <job> label J and any sequence of intervening assigns
          of OTHER <job>s,
GIVEN assign(J) returned F the first time
WHEN assign(J) is called again (the alloc-cycle case: a stop → new
     AllocationId → new workload_addr, but the SAME logical <job>)
THEN it returns the SAME F (idempotent per <job>)
  AND F is unchanged regardless of intervening assigns/releases of other
      <job>s (stability is a property of the logical workload, not of the
      current backend instance — Finding-2; § Frontend lifecycle contract)
```

**Chained narrative** (Pillar 2): the `Given` reuses S-DBN-FRONTEND-01's
`Given + When` (a `<job>` assigned `F`), then re-assigns the same `<job>`
— the tests read as the lifecycle of one `<job>`'s frontend addr. **This
is the SQ1-elimination contract at the allocator layer**: a backend cycle
mints a new `AllocationId` → a new per-instance `workload_addr`, but the
per-`<job>` frontend `F` stays byte-stable. The Tier-3 half is
S-DBN-WS-STABLE (the same property observed through `getent`).

---

### S-DBN-FRONTEND-03 — A frontend addr is released ONLY on logical-workload deletion, never on a transient zero-healthy state

**Tags**: `@property` `@in-memory` `@frontend` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (REV-2 Finding-2 — withhold-not-release; the rejected "release on zero backends" path)
**Driving port**: `FrontendAddrAllocator::{assign, release, snapshot}`
**Test surface**: Tier 1 — `dns_frontend_allocator.rs`
**Production code guarded**: `FrontendAddrAllocator::release` (fires ONLY on logical-workload deletion; the allocator holds no health state)

#### Spec

```
PROPERTY: for every <job> label J,
GIVEN assign(J) returned F (the <job> is bound)
WHEN release(J) is NOT called (a transient zero-healthy window — the
     <job> is still declared; the allocator is never told to release)
THEN a subsequent assign(J) STILL returns the SAME F
  AND snapshot() still contains J -> F
       (the allocator carries NO health state — zero-healthy is handled by
        the name_index WITHHOLDING the DNS answer, NEVER by releasing F;
        Finding-2 / § Frontend lifecycle contract)

PROPERTY 2 (the genuine end): WHEN release(J) IS called (logical-workload
     deletion / undeploy)
THEN snapshot() no longer contains J
  AND F is returned to the free block (a later assign of a DIFFERENT <job>
      MAY draw F — the binding genuinely ended)
```

**Negative-testing note** (Hebert ch.6): this property RELAXES the
assumption "zero-healthy means release" — and proves the allocator does
NOT release on that signal (it has no health input at all). Releasing `F`
on a transient zero-healthy state would destroy the stability property and
reintroduce the SQ1 stale-cached-`F` failure — the explicitly-rejected
path (REV-2 Finding-2). The allocator's `<job> → F` binding is orthogonal
to backend health; only the explicit deletion signal drives `release`.

---

### S-DBN-FRONTEND-04 — Distinct `<job>`s get collision-free distinct frontend addrs; the block is reclaimed on release

**Tags**: `@property` `@in-memory` `@frontend` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 1a-A — collision-free per-`<job>` assignment, the rejected 1a-C hash-collision risk)
**Driving port**: `FrontendAddrAllocator::{assign, release}` over a set of distinct `<job>`s
**Test surface**: Tier 1 — `dns_frontend_allocator.rs`
**Production code guarded**: `FrontendAddrAllocator` smallest-free scan (no two live `<job>`s share an `F`; release reclaims the slot)

#### Spec

```
PROPERTY: for every set of distinct <job> labels {J1..Jn} (n within the
          10.98.0.0/16 block capacity),
GIVEN a fresh FrontendAddrAllocator
WHEN each Ji is assign()ed
THEN the assigned addrs {F1..Fn} are pairwise distinct (no collision —
     the rejected 1a-C deterministic-hash birthday-bound risk; 1a-A is an
     allocator precisely to avoid it)
  AND each Fi ∈ 10.98.0.0/16
  AND after release(Jk), a fresh assign of a NEW <job> MAY reuse Fk
      (the block is reclaimed — drop-then-reassign tracks the live set,
       the converge-on-boot rebuild discipline of NetSlotAllocator)
```

**This is the collision-free-addressing guard.** A mutant that hands two
distinct live `<job>`s the same `F` (e.g. a broken smallest-free scan that
does not record the claim) flips the pairwise-distinctness property —
exactly the addressing collision 1a-A's allocator (over the rejected 1a-C
hash) exists to prevent.

---

### S-DBN-ANSWER-01 — `A` for a resolvable `<job>` yields `Records` of exactly the stable frontend addr

**Tags**: `@property` `@in-memory` `@frontend` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (the REV-2 v1 DNS answer contract, `A` row — stable `F`) · **KPI K-DBN-1**
**Driving port**: `answer_for(name, qtype, &index)` (the pure DST seam, the mutation-gate target — DDN-4)
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/dns_answer_for.rs` (NEW; the `answer.rs` proptest home)
**Production code guarded**: `dns_responder::answer::answer_for` `Records` arm (`crates/overdrive-control-plane/src/dns_responder/answer.rs`)

#### Spec

```
PROPERTY: for every <job> name N that is RESOLVABLE (≥1 running-AND-healthy
          backend exists for N, so the name_index holds N -> F where F is
          N's stable frontend addr) — and arbitrary other resolvable names
          in the index,
GIVEN a NameIndex whose by_name[N] == F (the SINGLE stable frontend addr —
       REV-2: NOT a backend-addr set)
WHEN answer_for(N, RecordType::A, &index) is called
THEN it returns NameAnswer::Records(addrs)
  AND addrs == vec![F] (exactly ONE record — the stable frontend addr;
       REV-2 answers a single per-<job> F, not the backend set)
  AND F ∈ 10.98.0.0/16 (the answered addr is a frontend addr, never a
       per-instance backend addr in 10.99.0.0/16)
```

**REV-2 shift** (vs superseded REV-1): REV-1 answered the *set of
running-and-healthy backend addrs*; REV-2 answers the *single stable
per-`<job>` frontend addr* `F`. The `answer_for` SIGNATURE is unchanged;
`Records` now carries `vec![F]`. **This is THE mutation-gate target
(DDN-4).** A mutant that returns an empty `Records`, the wrong addr, or a
backend addr (`∈ 10.99.0.0/16`) instead of the frontend addr flips this
property. Pinned `@example`: `N = server`, `F = 10.98.x.y` (the US-DBN-2
happy-path example). Mandate-8 universe-equivalent: the `addrs` vec is the
full observable; equality against `vec![F]` is the fail-closed guard.

---

### S-DBN-ANSWER-02 — `A` for a `<job>` with 0 running-and-healthy backends yields `NxDomain` (withheld)

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (the REV-2 v1 DNS answer contract, withheld `A` row) · **KPI K-DBN-2**
**Driving port**: `answer_for`
**Test surface**: Tier 1 — `dns_answer_for.rs`
**Production code guarded**: `answer_for` `NxDomain` arm (the withheld branch — the `<job>` has no resolvable `F` right now)

#### Spec

```
PROPERTY: for every <job> name N that is NOT resolvable — N absent from the
          index, OR present but with 0 running-and-healthy backends (so the
          name_index WITHHOLDS the answer),
GIVEN a NameIndex where N has no exposed F (no running-and-healthy backend)
WHEN answer_for(N, RecordType::A, &index) is called
THEN it returns NameAnswer::NxDomain
  AND NEVER NameAnswer::Records (no stale/cached/guessed F — the fail-honest
       contract; declared-but-not-running, all-stopped/zero-healthy, and
       unknown all collapse to NxDomain in v1)
```

**Withhold-not-release note** (REV-2 Finding-2): the `name_index` WITHHOLDS
the answer on zero-healthy, while the `FrontendAddrAllocator` RETAINS `F`
(tested at S-DBN-FRONTEND-03 / S-DBN-IDX-02). At the `answer_for` layer the
observable is identical — NXDOMAIN — whether the `<job>` is transiently
zero-healthy (retained `F`) or deleted (released `F`); v1 does not
distinguish them at the wire (ADR-0072 contract table). This is the
relaxed-precondition twin of S-DBN-ANSWER-01.

---

### S-DBN-ANSWER-03 — `AAAA` on a resolvable `<job>` yields `NoData`; on a withheld `<job>` yields `NxDomain`

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-2` `@dbn-US-4`
**US trace**: US-DBN-2 (`AAAA` resolvable → NODATA) + US-DBN-4 (`AAAA` withheld → NXDOMAIN)
**Driving port**: `answer_for`
**Test surface**: Tier 1 — `dns_answer_for.rs`
**Production code guarded**: `answer_for` `NoData` arm + the qtype dispatch (`RecordType::AAAA`)

#### Spec

```
PROPERTY 1 (resolvable <job>): for every <job> N with a stable F exposed
            (≥1 running-AND-healthy backend),
GIVEN N is resolvable (by_name[N] == F)
WHEN answer_for(N, RecordType::AAAA, &index) is called
THEN it returns NameAnswer::NoData
  AND NEVER NxDomain (the name IS resolvable — it just has no IPv6 record
       in the v1 IPv4 substrate; the frontend addr F is IPv4) AND NEVER a
       fabricated v6 addr

PROPERTY 2 (withheld <job>): for every <job> N with 0 running-and-healthy
            backends (withheld),
GIVEN N has no exposed F
WHEN answer_for(N, RecordType::AAAA, &index) is called
THEN it returns NameAnswer::NxDomain (no resolvable <job> at all)
```

**Pins the v1 contract table's `AAAA` column** (ADR-0072 contract table —
unchanged by REV-2: the AAAA-on-resolvable → NODATA, AAAA-on-withheld →
NXDOMAIN shape is addr-agnostic; the substrate is still IPv4). The spike
PROVED the NODATA-on-live case; the empty-name AAAA → NXDOMAIN case is the
Slice-01 build concern.

---

### S-DBN-ANSWER-04 — An unhealthy-only `<job>` is withheld → `NxDomain` (the healthy gate governs resolvability)

**Tags**: `@property` `@in-memory` `@error_path` `@kpi` `@dbn-US-4`
**US trace**: US-DBN-4 · **KPI K-DBN-2 / K-DBN-4** (the running-AND-healthy gate, REV-2 DDN-2)
**Driving port**: `answer_for` over a `NameIndex` built from rows where `Backend.healthy == false`
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/dns_name_index.rs` (NEW; the `NameIndex` build path is exercised here, then `answer_for` reads it)
**Production code guarded**: the `NameIndex` `Backend.healthy == true` gate governing *resolvability* (REV-2 DDN-2; `name_index.rs`), `answer_for` `NxDomain` arm

#### Spec

```
PROPERTY: for every <job> name N whose ONLY service_backends rows carry
          Backend.healthy == false (running but unhealthy / not-ready),
GIVEN a NameIndex built (via the List-then-Watch path) from those rows
WHEN answer_for(N, RecordType::A, &index) is called
THEN it returns NameAnswer::NxDomain (withheld — the healthy gate kept
     the unhealthy-only <job> from exposing its F)
  AND a DIFFERENT <job> M with a running-AND-healthy backend in the same
      index still resolves to its stable F (answer_for(M, A) ->
      Records(vec![F_M])) — proving the healthy gate withheld only N, not
      the whole index
```

**Universe** (port-exposed): the `answer_for` return for N (NxDomain) and
for M (Records of M's stable F). The test NEVER asserts on the `by_name:
BTreeMap` internals — whether the gate withholds the binding at build time
or filters at read time is a DELIVER detail. The observable contract is
`answer_for`'s return; a co-resident resolvable `<job>` M proves the index
did not collapse to empty.

**Why the healthy gate moved** (REV-2 DDN-2 — the load-bearing shift): in
REV-1 the gate decided *which addr we answer* (only healthy backend addrs);
in REV-2 it decides *whether this `<job>` is resolvable at all* (does it
have ≥1 running-AND-healthy backend → expose its stable `F`; else withhold
→ NXDOMAIN). The healthy backend is now selected by `MtlsResolve` at
*translation* time (S-DBN-REKEY-01/02), not by the responder at *answer*
time. This scenario is the structural guard against a mutant that drops the
healthy filter and exposes a `<job>` whose only backends are unhealthy.

---

### S-DBN-ANSWER-05 — An unknown name yields `NxDomain`

**Tags**: `@in-memory` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (unknown-name case)
**Driving port**: `answer_for`
**Test surface**: Tier 1 — `dns_answer_for.rs`
**Production code guarded**: `answer_for` lookup-miss → `NxDomain`

#### Spec

```
GIVEN a NameIndex that contains "server" but NOT "nonexistent"
WHEN answer_for("nonexistent.svc.overdrive.local", RecordType::A, &index) is called
THEN it returns NameAnswer::NxDomain
  AND querying "server" in the same index still returns Records
       (the miss does not corrupt the hit path)
```

**Single-example** (not a property) — this is the one-off "unknown name"
regression case; the property over absent names is covered by
S-DBN-ANSWER-02.

---

### S-DBN-WIRE-01 — Answered records survive a deterministic encode→decode round-trip

**Tags**: `@property` `@in-memory` `@dbn-US-2`
**US trace**: US-DBN-2 (the `wire.rs` ACL — DDN-5)
**Driving port**: `dns_responder::wire` encode (the separately-proptested encoder, DDN-4)
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/dns_wire.rs` (NEW; the `wire.rs` round-trip home)
**Production code guarded**: `wire.rs` `NameAnswer::Records → Vec<u8>` encode path (`hickory_proto::op::Message` build + `RData::A`)

#### Spec

```
PROPERTY (roundtrip — Hebert symmetric property): for every query name N
          and every non-empty set of SocketAddrV4 addrs,
GIVEN a NameAnswer::Records(addrs) and the original query for N (A)
WHEN wire::encode(N, A, NameAnswer::Records(addrs)) produces bytes
  AND those bytes are decoded by hickory_proto's Message parser
THEN the decoded Message has ResponseCode::NoError
  AND its answer section contains exactly one A record per addr
  AND the decoded A-record addrs (as a set) equal the input addrs
  AND ANCOUNT == addrs.len()
```

**Symmetric/roundtrip property** (Hebert ch.3 "symmetric"). The encoder
is the irreducibly-Tier-1 half of the DNS surface (the socket loop is
Tier-3); proptesting it round-trips through hickory's own decoder
catches name-compression / RDATA-layout bugs the spike's lenient `dig`
path masks.

---

### S-DBN-WIRE-02 — AAAA on a live name encodes NODATA with a 1-second negative-TTL SOA

**Tags**: `@property` `@in-memory` `@dbn-US-2`
**US trace**: US-DBN-2 (the NODATA-SOA shape — DDN-8)
**Driving port**: `wire::encode`
**Test surface**: Tier 1 — `dns_wire.rs`
**Production code guarded**: `wire.rs` `NameAnswer::NoData` encode path (NOERROR header + SOA in authority, MINIMUM=1)

#### Spec

```
PROPERTY: for every query name N (AAAA),
GIVEN a NameAnswer::NoData
WHEN wire::encode(N, AAAA, NameAnswer::NoData) produces bytes decoded by hickory
THEN ResponseCode == NoError
  AND ANCOUNT == 0
  AND the AUTHORITY section carries exactly one SOA record
  AND that SOA's MINIMUM (RFC 2308 negative TTL) == 1
       (DDN-8: a 1s negative TTL so a retrying dialer re-resolves promptly)
```

**Pins DDN-8.** The spike proved AAAA→NODATA on the wire
(`spike/findings.md`); this Tier-1 proptest pins the SOA/MINIMUM detail
the spike did not assert.

---

### S-DBN-WIRE-03 — A name with no running-and-healthy backend encodes NXDOMAIN with a 1-second negative-TTL SOA

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (the NXDOMAIN-SOA shape — DDN-8)
**Driving port**: `wire::encode`
**Test surface**: Tier 1 — `dns_wire.rs`
**Production code guarded**: `wire.rs` `NameAnswer::NxDomain` encode path (NXDOMAIN header + SOA, MINIMUM=1)

#### Spec

```
PROPERTY: for every query name N and qtype Q ∈ {A, AAAA},
GIVEN a NameAnswer::NxDomain
WHEN wire::encode(N, Q, NameAnswer::NxDomain) produces bytes decoded by hickory
THEN ResponseCode == NXDomain
  AND ANCOUNT == 0
  AND the AUTHORITY section carries exactly one SOA record with MINIMUM == 1
       (the negative-TTL SOA so the stub resolver caches the negative
        answer for ~1s and re-resolves once a backend is running-and-healthy)
```

---

### S-DBN-WIRE-04 — SOA SERIAL is derived from the injected Clock

**Tags**: `@property` `@in-memory` `@dbn-US-2`
**US trace**: US-DBN-2 (DDN-8 — SERIAL via `Clock`)
**Driving port**: `wire::encode` with an injected `Arc<dyn Clock>` reading
**Test surface**: Tier 1 — `dns_wire.rs`
**Production code guarded**: `wire.rs` SOA SERIAL derivation from the `Clock` value passed by `DnsResponder`

#### Spec

```
PROPERTY: for every NEGATIVE answer (NoData | NxDomain) and every clock
          reading T (a UnixInstant),
GIVEN the SOA-bearing encode is supplied the clock reading T
WHEN wire::encode renders the SOA
THEN the SOA SERIAL is a deterministic function of T
  AND two encodes with the SAME T produce byte-identical SOA SERIAL fields
  AND two encodes with DISTINCT T (differing past the SERIAL granularity)
      produce distinct SERIALs
```

**Why injected, not wall-clock**: `Clock` is the existing injected port
(`Arc<dyn Clock>` on `AppState`, `config.clock`); the SOA SERIAL MUST NOT
read wall-clock directly (`.claude/rules/development.md` § "Never call
`SystemTime::now()` in core logic"). This keeps the encoder deterministic
and DST-replayable.

---

### S-DBN-IDX-01 — Index seeds from List at probe; a `<job>` becomes resolvable to its stable `F` once a running-and-healthy row exists

**Tags**: `@property` `@in-memory` `@frontend` `@dbn-US-2`
**US trace**: US-DBN-2 (the List-then-Watch contract — DDN-3; REV-2 maps `<job>` → stable `F`)
**Driving port**: `NameIndex` build via `ObservationStore::all_service_backends_rows()` (List at probe) + `subscribe_all_events()` drain, consuming the `FrontendAddrAllocator` binding
**Test surface**: Tier 1 — `dns_name_index.rs` (driven by `SimObservationStore`, in-memory)
**Production code guarded**: `name_index.rs` List-seed + Watch-drain (mirrors `ServiceBackendsResolve` probe + `spawn_drain`, `mtls_resolve_adapter.rs:307-471`); the `<job>` → stable `F` mapping (REV-2 D-DBN-4')

#### Spec

```
PROPERTY: for every <job> name N and every running-and-healthy
          ServiceBackendRow R whose Backend.alloc SVID job segment == N's
          <job>, with the FrontendAddrAllocator binding N -> F,
GIVEN a SimObservationStore seeded (before probe) with R
  AND a NameIndex probed against it (List-then-Watch), fed the N -> F binding
WHEN the index is queried for N (via answer_for A)
THEN N resolves to its STABLE frontend addr F (Records == vec![F]) — NOT
     R's per-instance Backend.addr (the REV-2 shift: the index maps
     <job> -> stable F, not -> backend addrs)

  AND (watch half): GIVEN the index probed against an EMPTY store
       WHEN a running-and-healthy row R for N is written AFTER probe
       AND the watch drain processes the SubscriptionEvent::Row
       THEN N becomes resolvable to its F (Records == vec![F])
```

**Universe** (port-exposed): the `answer_for` result for N (the index is
exercised through its public read, never by asserting on the `by_name:
BTreeMap` field directly — that internal structure is a DELIVER detail).
**Validates the OQ-1 mapping consumer** — the index groups rows by their
SVID `<job>` segment to decide *resolvability*, then exposes the
allocator's stable `F` for that `<job>`; whether the accessor is
`SpiffeId::job_segment()` or a local parse helper is OQ-1 (DELIVER pins
it), but the *behaviour* (a `<job>` with a running-and-healthy row resolves
to its stable `F`) is asserted here.

---

### S-DBN-IDX-02 — A `<job>` going zero-healthy on the watch path is withheld (NXDOMAIN); the stable `F` is NOT released

**Tags**: `@property` `@in-memory` `@frontend` `@churn` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (the healthy gate as the WITHHOLD seam over the watch path — REV-2 Finding-2)
**Driving port**: `NameIndex` watch drain processing a row whose `Backend.healthy` flips to `false`; the `FrontendAddrAllocator` observed across the transition
**Test surface**: Tier 1 — `dns_name_index.rs`
**Production code guarded**: `name_index.rs` healthy-gate as the withhold seam on the watch path (REV-2 DDN-2 / Finding-2); the allocator does NOT release on this transition

#### Spec

```
PROPERTY: for every <job> name N initially resolvable to its stable F via a
          healthy row R,
GIVEN N resolves to F (answer_for(N, A) == Records(vec![F]))
WHEN a fresh ServiceBackendRow for the same backend is written with
     Backend.healthy == false (and the watch drain processes it), so N is
     now zero-healthy
THEN N stops resolving (answer_for(N, A) → NxDomain) — the name_index
     WITHHOLDS the answer
  AND the FrontendAddrAllocator STILL binds N -> the SAME F
     (snapshot() still contains N -> F — withhold-not-release; the stable
      F is retained across the zero-healthy window, Finding-2)
  AND the moment a running-AND-healthy row for N is written again, N
     resolves to the SAME F (no churn of the addr across the window)
```

**Chained narrative** (Pillar 2): the `Given` reuses S-DBN-IDX-01's `Given
+ When` (a `<job>` made resolvable to `F`), then transitions it zero-healthy
— the tests read as the lifecycle of one `<job>`'s resolvability. **The
load-bearing REV-2 distinction**: the answer is withheld (NXDOMAIN) at the
`name_index`, but the addressing binding (`<job> → F`) survives at the
allocator. Asserting BOTH (the withheld answer AND the retained `F`) is
what proves the withhold-not-release contract — a mutant that releases `F`
on zero-healthy passes the NXDOMAIN check but fails the retained-`F` check
(reintroducing the SQ1 stale-cached-`F` failure on recovery).

---

### S-DBN-IDX-03 — A `Lagged` subscription event triggers a relist that recovers the index

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-2`
**US trace**: US-DBN-2 (the relist-on-`Lagged` recovery — DDN-3)
**Driving port**: `NameIndex` watch drain receiving `SubscriptionEvent::Lagged { missed }`
**Test surface**: Tier 1 — `dns_name_index.rs`
**Production code guarded**: `name_index.rs` `Lagged` arm → relist (mirrors `ServiceBackendsResolve` relist-on-`Lagged`, `mtls_resolve_adapter.rs`)

#### Spec

```
PROPERTY: for every store state S (a set of running-and-healthy rows whose
          <job>s are bound F by the allocator),
GIVEN a NameIndex that has fallen behind (its drain receives
       SubscriptionEvent::Lagged { missed })
WHEN the index processes the Lagged event
THEN it re-Lists via all_service_backends_rows()
  AND after the relist, the index reflects S exactly
       (every <job> with a running-and-healthy row in S resolves to its
        stable F; no <job> absent from S resolves)
```

**Why this matters**: a `Lagged` event means the subscription dropped
rows; without a relist the index would silently miss a `<job>` forever
(answering NXDOMAIN for a resolvable `<job>` — a stale-negative bug). This
is the recovery property the `ServiceBackendsResolve` sibling already
enforces; the responder MUST inherit it. The relist logic is addr-agnostic
(it re-derives resolvability from rows regardless of whether the exposed
value is a backend addr or a stable `F`).

---

### S-DBN-IDX-04 — The answered `F` is the allocator's binding; the index introduces no second source of frontend truth (single source)

**Tags**: `@property` `@in-memory` `@frontend` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 · **KPI K-DBN-4** (single-source consistency, in-memory half)
**Driving port**: `NameIndex` + `answer_for` over a `SimObservationStore`, with the `FrontendAddrAllocator` as the sole `<job> → F` source
**Test surface**: Tier 1 — `dns_name_index.rs`
**Production code guarded**: `name_index.rs` (no second source — resolvability is read ONLY from `service_backends` rows; the exposed `F` is read ONLY from the `FrontendAddrAllocator`; the index holds no separate addr cache)

#### Spec

```
PROPERTY: for every store state S of running-and-healthy rows whose <job>s
          are bound by the allocator,
GIVEN a NameIndex probed against a SimObservationStore holding exactly S,
       fed the allocator's <job> -> F bindings
WHEN answer_for(N, A, &index) returns Records(addrs) for any resolvable N
THEN addrs == vec![the allocator's F for N's <job>]
       (the answered F is the ALLOCATOR's binding — no fabricated addr, no
        second source of frontend truth, no cached snapshot that outlived
        the allocator state)
  AND removing all rows for N from S and re-deriving the index makes N
      resolve to NxDomain (withheld — no stale retention at the index),
      WHILE the allocator still binds N -> F (the binding is the allocator's,
      not the index's — single source of frontend truth)
```

**This is the in-memory half of the K-DBN-4 single-source oracle (REV-2
form).** The Tier-3 half (S-DBN-SINGLE-SRC) feeds the answered `F` into the
real re-keyed `MtlsResolve.resolve` and asserts it translates to a `Mesh`
backend. This Tier-1 half proves the index introduces no second source of
the `<job> → F` truth: the answered `F` is exactly the allocator's binding,
and resolvability is exactly the `service_backends` running-and-healthy
fold. There are two single-source claims now — frontend truth (the
allocator) and liveness truth (the rows) — and the index fabricates
neither.

---

### S-DBN-REKEY-01 — `resolve(F, Tcp)` translates the frontend key to a current running-and-healthy backend, classified `Mesh`

**Tags**: `@property` `@in-memory` `@frontend` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 1b-A, step 02-00 — the frontend → live-backend translation) · **KPI K-DBN-4**
**Driving port**: `MtlsResolve.resolve(orig_dst, proto)` → the re-keyed `classify` (the `by_frontend` hit arm)
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/mtls_resolve_rekey.rs` (NEW; the `BackendIndex`/`classify` re-key proptest home)
**Production code guarded**: `BackendIndex.by_frontend` lookup + the `classify` hit arm: `(F, port, proto)` → `ServiceId` → first-by-`Ord` running-and-healthy backend → `Mesh` (`mtls_resolve_adapter.rs`, EXTENDED additively REV-2 1b-A)

#### Spec

```
PROPERTY: for every ServiceId Svc with ≥1 running-AND-healthy backend, a
          frontend key K = (F, listener.port, Proto::Tcp) bound
          by_frontend[K] == Svc, and arbitrary other entries in by_addr,
GIVEN a BackendIndex holding by_frontend[K] == Svc and ≥1 healthy backend
       for Svc
WHEN classify(orig_dst = (F, listener.port), proto = Tcp) is evaluated
THEN it returns Mesh
  AND the selected backend is the FIRST-by-Ord running-AND-healthy backend
      for Svc (BLOCKER-2 pinned: deterministic first-by-Ord — v1 single-
      replica makes the choice degenerate, but the tie-break keeps DST
      replay-equivalence and is mutation-gate-able)
  AND it is NEVER NonMesh (a frontend HIT is always mesh) and NEVER an
      unhealthy backend (the healthy-set selection is REUSED verbatim)
```

**This is the Cilium ClusterIP→backend translation made executable**
(research SQ4). The selection rule (deterministic first-by-`Ord`) is the
mutation-gate target — a mutant that picks last-by-`Ord`, or an unhealthy
backend, flips this property. The re-keyed contract MUST be pinned in the
`MtlsResolve` trait docstring + the S-DBN-EQUIV-01 equivalence test
(`development.md` § "Trait definitions specify behavior"). Pinned
`@example`: `Svc = server`, `K = (10.98.x.y, 8080, Tcp)`, one healthy
backend → `Mesh(that backend)`.

---

### S-DBN-REKEY-02 — A `(F, port, proto)` key with a known service but zero healthy backends classifies `MeshUnreachable`

**Tags**: `@property` `@in-memory` `@frontend` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (REV-2 1b-A — the frontend-hit-but-unhealthy arm; D-DBN-6 translation invariant)
**Driving port**: `MtlsResolve.resolve(orig_dst, proto)` → the `by_frontend` hit arm with an empty healthy set
**Test surface**: Tier 1 — `mtls_resolve_rekey.rs`
**Production code guarded**: `classify` `by_frontend` hit → `MeshUnreachable` when no running-and-healthy backend exists for the matched `ServiceId`

#### Spec

```
PROPERTY: for every ServiceId Svc bound by_frontend[K] == Svc (K = (F,
          port, Tcp)) but whose backend set is currently EMPTY or all
          unhealthy,
GIVEN a BackendIndex holding by_frontend[K] == Svc and 0 healthy backends
       for Svc
WHEN classify((F, port), Tcp) is evaluated
THEN it returns MeshUnreachable (the service is KNOWN — the key matched —
     but has no healthy backend right now: fail-closed, refuse, NO cleartext)
  AND NEVER Mesh (no healthy backend to translate to)
  AND NEVER NonMesh (a frontend HIT is a mesh dial — never cleartext-by-miss)
```

**Negative-testing note** (Hebert ch.6): RELAX S-DBN-REKEY-01's
"≥1 healthy backend" precondition — the frontend KEY still matches, so the
verdict must be `MeshUnreachable` (known service, momentarily no healthy
backend), never a fall-through to `NonMesh`/cleartext. This is distinct
from S-DBN-FAILCLOSED-01 (a frontend-subnet MISS); here the key HITS but
the healthy set is empty. Both fail closed; the difference is hit-vs-miss.

---

### S-DBN-REKEY-03 — The frontend key discriminates proto — same `(F, port)` on tcp vs udp resolves to distinct services

**Tags**: `@property` `@in-memory` `@frontend` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 Finding-1 — the `FrontendKey = (SocketAddrV4, Proto)` collision-safety contract)
**Driving port**: `MtlsResolve` `by_frontend` keyed by `FrontendKey = (SocketAddrV4, Proto)`
**Test surface**: Tier 1 — `mtls_resolve_rekey.rs`
**Production code guarded**: `BackendIndex.by_frontend: BTreeMap<FrontendKey, ServiceId>` where `FrontendKey = (SocketAddrV4, Proto)` — the proto axis is a key field, NOT a bare `SocketAddrV4` (Finding-1)

#### Spec

```
PROPERTY: for one frontend IP F, one port P, and two DISTINCT services
          Svc_tcp and Svc_udp whose listeners are (F, P, Tcp) and (F, P, Udp),
GIVEN by_frontend holds (F, P, Tcp) -> Svc_tcp AND (F, P, Udp) -> Svc_udp
       (one frontend IP fronting N distinct (port, proto) listeners yields
        N distinct entries — Finding-1)
WHEN by_frontend is looked up with key (F, P, Tcp)
THEN it yields Svc_tcp (NOT Svc_udp — the two do not collide on the key)
  AND looking up (F, P, Udp) yields Svc_udp
  AND a bare SocketAddrV4 (F, P) WITHOUT the proto axis CANNOT distinguish
      them (the collision Finding-1 names — the key MUST carry proto)
```

**Pins REV-2 Finding-1.** A bare `SocketAddrV4` key is ip+port only and
collides `tcp/53` with `udp/53`; the `ServiceId` value only disambiguates
AFTER the lookup, so two same-`(F, port)` rows on different protos would
collide on the key BEFORE the value is read. The production row-producing
path is proto-aware and UDP-admissible (`service_map_hydrator.rs:130` + the
C3 guard refusing to default to `Tcp`), so the key carries the proto axis.
**v1 plumbing note** (Mandate 11 / DELIVER): v1 captures TCP only at the
worker layer, so the live `resolve` call site keys `Proto::Tcp`; this
Tier-1 property exercises the index's proto-discrimination at the
data-structure level (both `Tcp` and `Udp` entries) — the future-UDP
capture is the named plumbing step, NOT an index-key change. A mutant that
drops the `Proto` field from the key flips this property by collapsing the
two services onto one key.

---

### S-DBN-REKEY-04 — A direct backend-addr dial still resolves via `by_addr` (the re-key is additive, backward-compatible)

**Tags**: `@property` `@in-memory` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 1b-A — the EXTEND-not-replace verdict; the `by_addr` path preserved)
**Driving port**: `MtlsResolve.resolve(orig_dst, proto)` → the `by_addr` fall-through (a non-frontend, non-frontend-subnet dst)
**Test surface**: Tier 1 — `mtls_resolve_rekey.rs`
**Production code guarded**: `classify` `by_addr` fall-through arm — unchanged by the additive `by_frontend` extension (`mtls_resolve_adapter.rs:207-300`, the existing path)

#### Spec

```
PROPERTY: for every running-and-healthy backend addr B (∈ 10.99.0.0/16, a
          per-instance workload addr, NOT a frontend addr) indexed in
          by_addr,
GIVEN a BackendIndex whose by_addr holds B (and by_frontend does NOT hold B)
WHEN classify(orig_dst = B, proto) is evaluated
THEN it returns the SAME verdict the pre-REV-2 by_addr path returns
     (Mesh for a healthy B; MeshUnreachable for an unhealthy B) — the
     re-key is ADDITIVE, the direct-backend-dial path is preserved
  AND a true non-mesh dst (∉ by_frontend, ∉ by_addr, ∉ 10.98.0.0/16)
     still returns NonMesh (legitimate non-mesh egress, unchanged)
```

**This is the backward-compatibility guard** (REV-2 1b-A "EXTEND in
place"). The `by_frontend` map is additive; the existing `by_addr` lookup
and its `Mesh`/`MeshUnreachable`/`NonMesh` verdicts are unchanged for a
direct backend dial and for genuine non-mesh egress. A mutant that routes
a `by_addr` hit through the frontend path (or breaks the non-mesh
fall-through) flips this property — proving the extension did not regress
the security-critical existing path.

---

### S-DBN-FAILCLOSED-01 — A `10.98.0.0/16` miss classifies `MeshUnreachable` (refuse, NO cleartext); a non-frontend miss stays `NonMesh`

**Tags**: `@property` `@in-memory` `@fail_closed` `@error_path` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 Finding-3 — the fail-closed-on-frontend-subnet-miss arm; D-DBN-7') · **KPI K-DBN-4** (single-source consistency / fail-closed-for-mesh)
**Driving port**: `MtlsResolve.resolve(orig_dst, proto)` → the THREE-way `classify` (the frontend-subnet-miss arm vs the general-miss arm)
**Test surface**: Tier 1 — `mtls_resolve_rekey.rs`
**Production code guarded**: the `classify` fail-closed arm: `else if orig_dst.ip() ∈ WORKLOAD_FRONTEND_BASE (10.98.0.0/16) { MeshUnreachable }` BEFORE the general `by_addr` fall-through (REV-2 Finding-3)

#### Spec

```
PROPERTY 1 (frontend-subnet miss → fail-closed): for every orig_dst whose
            ip ∈ 10.98.0.0/16 that MISSES by_frontend (no matching key) and
            MISSES by_addr,
GIVEN a BackendIndex where (orig_dst, proto) is absent from by_frontend AND
       orig_dst is absent from by_addr, AND orig_dst.ip() ∈ 10.98.0.0/16
WHEN classify(orig_dst, proto) is evaluated
THEN it returns MeshUnreachable (refuse, NO cleartext — a mesh dial that
     arrived before the index was ready (race) OR to a withdrawn <job>;
     Finding-3)
  AND it is NEVER NonMesh (NEVER cleartext PassThrough — the fail-OPEN
     regression the subnet-scoped arm exists to prevent)

PROPERTY 2 (non-frontend miss → today's behaviour): for every orig_dst
            whose ip ∉ 10.98.0.0/16 that misses both maps,
GIVEN orig_dst.ip() ∉ 10.98.0.0/16 and (orig_dst, proto) misses by_frontend
       and orig_dst misses by_addr
WHEN classify(orig_dst, proto) is evaluated
THEN it returns NonMesh (legitimate non-mesh egress → cleartext, by design
     — the live rustdoc requires a GENERAL miss stay NonMesh,
     mtls_resolve_adapter.rs:128-130; the subnet discriminator distinguishes
     the two)
```

**Pins REV-2 Finding-3 — the structural defense that makes the DNS↔resolve
race non-exploitable.** The subnet-membership discriminator is what makes
the fail-closed arm SAFE: `10.98.0.0/16` is DEDICATED to mesh frontends, so
a miss THERE is a mesh dial (early or withdrawn), never legitimate
cleartext; a miss OUTSIDE keeps today's `NonMesh` behaviour verbatim. The
two properties asserted together (subnet-miss → `MeshUnreachable`,
non-subnet-miss → `NonMesh`) are the load-bearing pair — a mutant that
treats the subnet miss as `NonMesh` flips Property 1 (the fail-open
footgun); a mutant that treats EVERY miss as `MeshUnreachable` flips
Property 2 (breaks legitimate non-mesh egress). **Membership-check
primitive is a DELIVER detail** (the `orig_dst.ip() ∈ 10.98.0.0/16`
CIDR-containment accessor — an `Ipv4Net::contains` on the const or a mask
compare; the crafter MUST NOT invent a broader "is this any reserved
subnet" helper — the test is specifically membership in the dial-by-name
frontend block, REV-2 § Frontend-subnet coherence contract).

---

### S-DBN-COHERENCE-01 — A single ordered drain updates `by_frontend` before `name_index` exposes `F` (DNS never answers an `F` resolve has not learned)

**Tags**: `@property` `@in-memory` `@frontend` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 Finding-3 coherence option (b) — the write-time ordering barrier; D-DBN-7' (i))
**Driving port**: the single ordered drain that feeds BOTH projections (`by_frontend` and `name_index`) off the shared `service_backends` rows
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/dns_name_index.rs` (the drain feeds both the index and the re-keyed resolve)
**Production code guarded**: the single ordered drain ordering invariant — `by_frontend` (+ a healthy backend for `F`'s `ServiceId`) is applied BEFORE `name_index` exposes `F` for the `<job>` (REV-2 Finding-3 (i), the "one source, three readers" write-time barrier)

#### Spec

```
PROPERTY: for every batch of running-and-healthy rows that binds <job> -> F
          applied through the single ordered drain,
GIVEN the drain is mid-apply of a batch that will make <job> resolvable to F
WHEN any observation is taken between the start and end of the batch apply
THEN there is NO observable moment where name_index answers F for <job>
     while by_frontend does NOT yet hold the (F, listener.port, proto) entry
     (the ordering invariant: by_frontend is updated FIRST, name_index
      exposes F SECOND — option (b), the write-time barrier)
  AND therefore DNS never answers an F that the re-keyed resolve has not
     already learned (a steady-state-race-free coherence guarantee)
```

**Pins REV-2 Finding-3 (i) — the ordering coherence.** Both projections
derive from ONE ordered apply over the same `service_backends` rows (the
"one source, three readers" model, `mtls_resolve_adapter.rs:56-87`), so a
write-time barrier (resolve updated first) is the natural and stronger
shape than a per-query read gate. The property is DST-assertable as an
ordering invariant on the drain. Combined with S-DBN-FAILCLOSED-01 (the
structural defense for any residual race), this makes the DNS↔resolve race
non-exploitable: ordering closes the steady-state race; fail-closed makes
any residual race a refused-connect-and-retry, never silent cleartext. A
mutant that exposes `F` to `name_index` before binding `by_frontend` flips
this ordering property.

---

### S-DBN-EQUIV-01 — Host and in-memory `MtlsResolve` agree on the re-keyed `classify` over the same call sequence (DST equivalence)

**Tags**: `@property` `@in-memory` `@frontend` `@dbn-US-2`
**US trace**: US-DBN-2 (the `development.md` § "Trait definitions specify behavior" equivalence discipline the REV-2 re-key MANDATES)
**Driving port**: `MtlsResolve.resolve(orig_dst, proto)` driven through the SAME call sequence against two `BackendIndex` constructions
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/mtls_resolve_rekey.rs` (the equivalence harness)
**Production code guarded**: the re-keyed `classify` contract — the three-way arm (`by_frontend` hit → `Mesh`/`MeshUnreachable`; frontend-subnet miss → `MeshUnreachable`; else `by_addr` fall-through) MUST be observably identical regardless of construction path

#### Spec

```
PROPERTY: for every sequence of (orig_dst, proto) classify calls and every
          sequence of row applies/withdrawals,
GIVEN two BackendIndex instances driven through the SAME ordered sequence
       of row applies + frontend bindings (the "host" build path and the
       in-memory test-double build path honouring the same contract)
WHEN the SAME sequence of classify(orig_dst, proto) calls is replayed
     against both
THEN both return the byte-identical verdict (Mesh{backend} / MeshUnreachable
     / NonMesh) at EVERY step
  AND the verdict trajectory is deterministic (seed → bit-identical, the
      DST replay-equivalence property — the first-by-Ord selection rule is
      what makes this hold across builds)
```

**Mandated by the REV-2 contract** (ADR-0072 Finding-3 + feature-delta
§ Frontend-key contract: "the re-keyed contract … MUST be pinned in the
`MtlsResolve` trait docstring + an equivalence test"). This is the
`development.md` § "Trait definitions specify behavior" structural guard —
the re-keyed `classify`'s observable behavior (the three-way arm + the
first-by-`Ord` selection) is the contract; the equivalence test is the
enforcement. A divergence between the two build paths means exactly one of
the contract / the implementations is wrong, and the test isolates which.
**Note**: this exercises the `BackendIndex`/`classify` data structure
in-process at Tier 1 (no real socket); it is the re-key's DST seam, not a
socket test (the socket is irreducibly Tier 3, DDN-4).

---

### S-DBN-WS — Walking skeleton: a deployed workload resolves its peer's stable frontend name and the hop is mTLS'd

**Tags**: `@walking_skeleton` `@driving_adapter` `@real-io` `@frontend` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (Slice 01) · **KPI K-DBN-1**
**Driving port**: `getaddrinfo`/`getent` (resolution) → `overdrive deploy` (`POST /v1/jobs`) → `overdrive serve` (`run_server_with_obs_and_driver`)
**Test surface**: Tier 3 — `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs` (NEW; mirrors `canonical_address_inbound_walking_skeleton.rs` boot/deploy/netns shape)
**Production code guarded**: `run_server_with_obs_and_driver` responder-construct + `FrontendAddrAllocator` construct/share + `responder.probe()` + spawn (lib.rs ~1893-1957, DDN-6 + REV-2 02-01); `DnsResponder::{new, probe, serve}`; the wildcard `IP_PKTINFO` socket loop; `NameIndex` (`<job>` → stable `F`) over the real `LocalObservationStore`; the re-keyed `MtlsResolve` (`by_frontend` translation); the existing production egress nft-TPROXY intercept path capturing the connection to `F` (BLOCKER-1 WORKS)

#### Spec

```
GIVEN a Lima VM as root (else SKIP — K1) with uname -r recorded
  AND the production composition root booted in-process via
       run_server_with_obs_and_driver (real EbpfDataplane, NO
       dataplane_override, mtls_identity_override = Some(TestPki)) —
       the SAME boot shape as the canonical-address keystone — so the
       mtls_worker.is_some() block is active and the DnsResponder +
       FrontendAddrAllocator + re-keyed MtlsResolve are constructed +
       probed + spawned (DDN-6 + REV-2 02-00/02-01)
  AND a "server" Service deployed through the in-process deploy submit
       handler (POST /v1/jobs) — a real TCP server bound on
       0.0.0.0:<SERVICE_PORT> inside its production-provisioned netns
  AND the server reached Running-AND-HEALTHY, so its <job> "server" is bound
       a STABLE frontend addr F ∈ 10.98.0.0/16 (by the FrontendAddrAllocator)
       and exposed by name_index
WHEN a "client" workload (deployed through the same handler, OR a dial
     thread entered into a client netns per the keystone's dial_in_netns)
     resolves "server.svc.overdrive.local" via getaddrinfo / getent
     (NOT dig — K2) from inside its netns, then connects to the resolved
     F:<SERVICE_PORT>
THEN getent resolves "server.svc.overdrive.local" to the STABLE frontend
     addr F ∈ 10.98.0.0/16 (within the K2 5s budget) — NOT a per-instance
     backend addr in 10.99.0.0/16
  AND the subsequent connection to (F, SERVICE_PORT) is captured by the
     PRODUCTION-installed egress nft-TPROXY rule (BLOCKER-1 WORKS — the
     capture is destination-blind, recovers orig_dst = (F, SERVICE_PORT)
     verbatim), the re-keyed MtlsResolve translates (F, SERVICE_PORT, Tcp)
     → the current live backend, mTLS terminates on the leg, and the peer
     wire carries TLS 1.3 application_data records (0x17) with ZERO payload
     cleartext on the peer leg (the keystone's transitive round-trip litmus)
```

**Framing (what's NEW vs what's REUSED)**: the responder's UNIQUE
addition under test is the `getent`/`getaddrinfo` resolution STEP turning
a name into the **stable frontend addr `F`**, PLUS the re-keyed
`MtlsResolve` translating `F` → a live backend. The capture +
handshake + TLS 1.3 `0x17` round-trip datapath (the egress nft-TPROXY,
the leg handshake) is REUSED verbatim from the proven canonical-address
keystone (BLOCKER-1 confirmed the capture is destination-blind and works
for a non-`/30` frontend addr — no test hand-installs the capture). It
appears in the `Then` because the intercept LANDING + the mTLS round-trip
is itself the proof the resolved `F` was translated to the right backend
(US-DBN-2 AC — a wrong `F` would miss `by_frontend` and fail-close, never
reach a backend). So the mTLS assertion stays load-bearing: it is the
observable that the name answer (`F`) + the translation pointed at the
right live backend.

**Litmus (the vertical-slice gate, CLAUDE.md)**: remove the production
responder spawn in `run_server` (DDN-6) and this test goes RED — `getent`
times out (nothing answers on the gateway) and the dial never starts.
Remove the `by_frontend` translation arm (02-00) and the connect to `F`
misses → fail-closed (`MeshUnreachable`) → no backend reached. NO test
binds `:53`, installs a `resolv.conf`, allocates `F`, or hand-installs the
egress capture — production does all of those itself (the capture via the
shipped Path-A datapath, BLOCKER-1).

**Walking-skeleton litmus (Dim 5 user-centricity)**: the title describes a
user goal ("a deployed workload resolves its peer's stable frontend name
and the hop is mTLS'd"), the `Then` describes a user observation (the name
resolves to a stable addr; the wire is encrypted), and a non-technical
stakeholder confirms "yes — an ordinary workload finds its peer by name,
gets a stable address, and the connection is secure." It is demo-able.

---

### S-DBN-WS-STABLE — The answered `F` is byte-stable across an alloc cycle; the next connect lands the NEW backend

**Tags**: `@real-io` `@frontend` `@churn` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (REV-2 02-02 NEW AC — the SQ1-staleness-elimination proof, the whole point of the stable-frontend split) · **KPI K-DBN-1**
**Driving port**: `getaddrinfo`/`getent` (the answered `F`) observed across a backend alloc cycle → `connect` to `F`
**Test surface**: Tier 3 — `dns_responder_walking_skeleton.rs` (a `#[tokio::test]` sharing the S-DBN-WS boot fixture)
**Production code guarded**: the `FrontendAddrAllocator` idempotent `assign` across an alloc cycle (REV-2 Finding-2) + the re-keyed `MtlsResolve` re-resolving the live backend per-connect (the churn half, `mtls_resolve_adapter.rs:521-540`, REUSED)

#### Spec

```
GIVEN the S-DBN-WS boot fixture: "server" deployed and Running-AND-HEALTHY,
       getent "server.svc.overdrive.local" resolves to stable frontend F1
       (recorded), one connect to F1 lands the current backend B1
WHEN the server backend is CYCLED — stopped (its AllocationId ends → its
     per-instance workload_addr is freed) and a new instance starts (a NEW
     AllocationId → a NEW workload_addr B2), the <job> "server" still declared
  AND the new instance reaches Running-AND-HEALTHY
  AND getent "server.svc.overdrive.local" is re-queried from the client netns
THEN getent resolves to the SAME F1 byte-for-byte (the stable frontend addr
     is retained across the alloc cycle — Finding-2; the FrontendAddrAllocator's
     idempotent assign("server") returns the same F)
  AND the subsequent connect to (F1, SERVICE_PORT) lands the NEW backend B2
     (the re-keyed MtlsResolve re-resolved the live backend per-connect),
     mTLS terminates, the peer wire carries TLS 1.3 0x17
  AND at NO point did getent return a per-instance backend addr (neither B1
     nor B2 ∈ 10.99.0.0/16 was ever the resolved value — only the stable F1)
```

**This is THE SQ1-elimination AC** (REV-2 02-02): it proves the
stale-cached-address failure REV-1 had (answering a per-instance backend
addr that goes stale on cycle) is eliminated by the stable-frontend split.
The DNS answer is byte-stable across the cycle; the dataplane re-resolves
the live backend per-connect. **Example-based sad path / single golden
walkthrough** (Mandate 11) — one Tier-3 alloc-cycle example, no PBT (the
PBT half of the stability property is S-DBN-FRONTEND-02 at Tier 1). The
Tier-1 idempotency property (S-DBN-FRONTEND-02) and this Tier-3 observable
(`getent` returns the same `F` across a real cycle) are the two halves of
the same contract.

---

### S-DBN-CHURN — Cycling a backend mid-connection gives the client a prompt reset bounded by `TCP_USER_TIMEOUT`, never an indefinite hang

**Tags**: `@real-io` `@churn` `@error_path` `@kpi` `@dbn-US-4`
**US trace**: US-DBN-4 (REV-2 02-02 NEW Tier-3 churn AC — the active-termination posture: NO `sock_destroy`) · **KPI K-DBN-2**
**Driving port**: `connect` to `F` (an in-flight connection) → the per-connection pump task on the worker proxy legs
**Test surface**: Tier 3 — `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs` (or the churn file; a `#[tokio::test]` sharing the boot fixture)
**Production code guarded**: the per-connection pump task + `TCP_USER_TIMEOUT` / keepalive surfacing backend death (`mtls_intercept_worker.rs:34`) — REV-2 § Active-termination posture: NO `sock_destroy` (the terminating-proxy framing)

#### Spec

```
GIVEN the S-DBN-WS boot fixture: "server" resolved to stable F, a client
       holding an OPEN, in-flight connection through the intercept to the
       current backend B1 (data flowing)
WHEN the backend B1 is CYCLED mid-connection (the instance is stopped while
     the client's connection is still open)
THEN the client's in-flight connection gets a PROMPT reset / error bounded
     by TCP_USER_TIMEOUT (a connection-reset or timeout error surfaced
     within the TCP_USER_TIMEOUT window) — NOT an indefinite hang
  AND NO sock_destroy is used (backend death surfaces through the
     per-connection pump task + TCP_USER_TIMEOUT/keepalive — the
     terminating-proxy posture; sock_destroy is #61 scope, NOT here)
  AND TCP_USER_TIMEOUT is confirmed tuned to a sane bound on the worker
     proxy legs (the in-flight dial fails FAST, the way Cilium's own default
     leaves TCP socket-termination OFF — research SQ5.3)
  AND a SUBSEQUENT fresh connect to F lands the new live backend B2
     (distinct from S-DBN-WS-STABLE: that proves the NEXT dial is live;
      this proves the IN-FLIGHT dial fails fast rather than hangs)
```

**Example-based sad path** (Mandate 11) — one named Tier-3 example for the
mid-connection-churn failure mode; no PBT at Tier 3. **The corrected
active-termination framing** (REV-2 § Active-termination posture): the
responder + intercept is a *terminating proxy* (a userspace pump task in
path), so backend death is surfaced by the pump + `TCP_USER_TIMEOUT`, NOT
by `sock_destroy` (which belongs to #61's future connect-time XDP-VIP-LB
with no userspace proxy in path). The AC is "the in-flight connection
fails FAST (bounded), not hangs" + "the next dial is live" — distinct from
S-DBN-WS-STABLE which proves the addr is stable across the cycle.

---

### S-DBN-SINGLE-SRC — Single-source oracle: the answered `F` is the addr `MtlsResolve` recognizes and translates to a `Mesh` backend

**Tags**: `@real-io` `@frontend` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (the `answered_backend_addr` shared artifact, REV-2 form — the frontend↔backend translation invariant D-DBN-6) · **KPI K-DBN-4**
**Driving port**: `getaddrinfo`/`getent` (the answered `F`) → `MtlsResolve.resolve` (the re-keyed oracle)
**Test surface**: Tier 3 — `dns_responder_walking_skeleton.rs` (a second `#[tokio::test]` in the same file, sharing the boot fixture)
**Production code guarded**: `DnsResponder` answer path (the stable `F`) + the re-keyed `MtlsResolve.resolve` translation (`by_frontend` hit → `Mesh`, REV-2 1b-A) — asserting they agree on the SAME flow; the intercept struct is EXTENDED additively (REV-2 supersedes REV-1's "untouched"), read for the oracle

#### Spec

```
GIVEN the S-DBN-WS boot fixture (server deployed, Running-AND-HEALTHY,
       <job> "server" bound stable frontend F)
WHEN "server.svc.overdrive.local" is resolved via getaddrinfo to addr F
  AND F (as a SocketAddrV4 at SERVICE_PORT) is fed into the production
      re-keyed MtlsResolve.resolve(F, Tcp) for the same flow
THEN resolve(F, Tcp) recognizes F (a by_frontend HIT — NOT a miss → NonMesh)
  AND classifies it Mesh (NOT MeshUnreachable, NOT NonMesh)
  AND TRANSLATES F to a current running-AND-healthy backend (the Mesh
       verdict carries the live backend the connection mTLS-originates to)
  AND F is byte-identical to the addr the responder answered
       (one source: the name answer and the resolve translation read the
        SAME <job> -> F binding from the SAME FrontendAddrAllocator; the
        translation lands a healthy backend from the SAME service_backends
        rows — D-DBN-6 / the REV-2 byte-consistency restatement)
```

**The K-DBN-4 single-source oracle made executable (REV-2 form)**
(`feature-delta.md` § Changed byte-consistency contract). REV-2 RESTATES
the byte-consistency claim: the answered *frontend* `F` is byte-identical
to the addr `MtlsResolve` is re-keyed to *recognize AND translate*, and
the translation always lands a `Mesh` backend (never a miss → `NonMesh`,
never an unhealthy backend → `MeshUnreachable`). The only way `resolve(F)`
returns `Mesh` for the answered `F` is if the `by_frontend` binding and the
healthy-set selection kept them in lockstep — the cross-reader equivalence
the `development.md` § "Trait definitions specify behavior" discipline asks
for, here observed at the port (the `getent` answer + the `resolve`
verdict) over a shared `<job> → F` binding + shared rows.

---

### S-DBN-PINGPONG — Bidirectional ping-pong demo: two services dial each other by stable frontend name, counters advance

**Tags**: `@walking_skeleton` `@real-io` `@edd` `@kpi` `@dbn-US-3`
**US trace**: US-DBN-3 (Slice 02) · **KPI K-DBN-3**
**Driving port**: `overdrive deploy` ×2 (`examples/dial-by-name-responder/{a,b}.toml`) against `overdrive serve`
**Test surface**: Tier 3 — `crates/overdrive-control-plane/tests/integration/dns_responder_ping_pong.rs` (NEW) **AND** a `verification/expectations/` EDD expectation (proposed `E05-dial-by-name-ping-pong-mtls`)
**Production code guarded**: the full responder + intercept loop driven bidirectionally through two real deploys; `examples/dial-by-name-responder/{a,b}.toml`; the staged ping-pong bin

#### Spec

```
GIVEN a Lima VM as root (else SKIP) with the ping-pong bin staged at a
       real on-disk path (K3) and overdrive serve booted (real dataplane)
  AND examples/dial-by-name-responder/a.toml and b.toml exist with the
       [service]/[exec]/[resources]/[[listener]] schema, command pointing
       at the staged bin, listener ports avoiding 5353 and 53
WHEN Sam runs "overdrive deploy examples/dial-by-name-responder/a.toml"
  AND "overdrive deploy examples/dial-by-name-responder/b.toml"
THEN within ~15s of the second deploy, A resolves "b.svc.overdrive.local"
     to b's STABLE frontend addr F_b ∈ 10.98.0.0/16 (getaddrinfo) and
     connects to F_b — the connection is captured + the re-keyed MtlsResolve
     translates F_b → b's live backend — B's counter increments and its date refreshes
  AND within ~15s, B resolves "a.svc.overdrive.local" to a's stable frontend
     F_a and calls A — A's counter increments and its date refreshes
  AND each hop is intercepted + mTLS'd (TLS 1.3 records on the peer leg,
     observable via tcpdump / ss -tie)
  AND both counters continue advancing on a ~10s ±5s cadence over a 60s window
```

**EDD graduation** (`.claude/rules/verification.md`): this is the
operator-surface proof → graduates to a `verification/expectations/`
`O`/`E`-surface expectation `E05-dial-by-name-ping-pong-mtls`, anchored
to this scenario + K-DBN-3, captured black-box against the built
`overdrive` binary under Lima, **different-fox-reviewed** (never
self-stamped). Like sibling `E04`, the capture is **honest `pending`**
until the full-system EDD harness (#227 / #75) lands — surface that as a
dependency, mirror E04's `pending` posture, do NOT fabricate a capture.

**Single-example, not a property** (Mandate 9 / Mandate 11): a Tier-3
walking-skeleton uses ONE golden bidirectional walkthrough; no PBT
machinery. The two `[service].id`s (`a`, `b`) and the ~10s cadence are
the pinned example.

**REV-2 note**: behaviorally UNCHANGED at the demo layer (two services
dial each other by name; counters advance; each hop mTLS'd; the E05 EDD
expectation is unchanged in shape). The resolved value is now a stable
frontend `F` rather than a per-instance backend addr — an internal detail
the operator-visible counter/date advance does not depend on (03-02 in the
REV-2 per-step re-scope: "UNCHANGED behaviorally").

---

### S-DBN-NXDOMAIN-01 — Querying before running-and-healthy yields NXDOMAIN, never a stale addr

**Tags**: `@real-io` `@error_path` `@kpi` `@dbn-US-4`
**US trace**: US-DBN-4 (Slice 03) · **KPI K-DBN-2**
**Driving port**: `getaddrinfo`/`getent` (before the backend is running-and-healthy)
**Test surface**: Tier 3 — `crates/overdrive-control-plane/tests/integration/dns_responder_nxdomain.rs` (NEW)
**Production code guarded**: `DnsResponder` serve loop + `answer_for` `NxDomain` arm over the real `NameIndex`; the running-and-healthy gate end-to-end

#### Spec

```
GIVEN a Lima VM as root (else SKIP), overdrive serve booted
  AND a "server" Service deployed through POST /v1/jobs but NOT yet
       Running-AND-HEALTHY (the alloc is Pending, OR Running-but-unhealthy)
WHEN a deployed client workload queries "server.svc.overdrive.local"
     via getaddrinfo / getent (NOT dig — K2) from inside its netns
THEN getent reports the name does not resolve (NXDOMAIN — name-not-found;
     the name_index WITHHOLDS the answer — no running-and-healthy backend)
  AND NO address is returned (no stale, cached, unhealthy, or guessed addr —
       NOT a stable frontend F either, since F is withheld until the <job>
       has a running-and-healthy backend)
  AND once the server reaches Running-AND-HEALTHY, a re-query resolves to
       the stable frontend addr F ∈ 10.98.0.0/16 (the negative answer's 1s
       SOA TTL lets the retry land promptly — DDN-8)
```

**Example-based sad path** (Mandate 11): one named example per failure
mode (the failure mode here = "queried before running-and-healthy"). No
PBT at Tier 3. **REV-2 note**: NXDOMAIN here is the `name_index`
WITHHOLDING (the `<job>` has no running-and-healthy backend → no exposed
`F`); the recovery resolves to the stable `F` once a backend is healthy.

---

### S-DBN-NXDOMAIN-02 — After all backends stop, the `<job>` is withheld (NXDOMAIN); the stable `F` is NOT released

**Tags**: `@real-io` `@error_path` `@frontend` `@churn` `@kpi` `@dbn-US-4`
**US trace**: US-DBN-4 (REV-2 03-01 minor re-distill — withhold-not-release; Finding-2) · **KPI K-DBN-2**
**Driving port**: `overdrive job stop` (`POST /v1/jobs/{id}/stop`) → `getaddrinfo`/`getent`
**Test surface**: Tier 3 — `dns_responder_nxdomain.rs`
**Production code guarded**: `DnsResponder` serve loop reacting to the index WITHHOLDING the zero-healthy `<job>` (the watch path → `NxDomain`); the `FrontendAddrAllocator` RETAINING the `<job> → F` binding across the stop (Finding-2); the production stop path (`run_server_stop` precedent, keystone:635)

#### Spec

```
GIVEN a "server" Service deployed and Running-AND-HEALTHY, resolving
       "server.svc.overdrive.local" to its stable frontend addr F (the
       keystone's deploy + wait-Running shape, then a getent confirming F)
WHEN the server is stopped through the production stop path
     (POST /v1/jobs/server/stop) and converges to Terminated
     (poll the obs row to Terminated, per the keystone's server_stopped),
     leaving the <job> "server" zero-healthy but STILL DECLARED (not deleted)
  AND a deployed client re-queries "server.svc.overdrive.local" via getent
THEN getent reports the name does not resolve (NXDOMAIN) — the name_index
     WITHHELD the answer (zero running-and-healthy backends)
  AND it NEVER returns the stale F's translated backend, nor any stale addr
       (the index dropped the resolvability; no second source of liveness truth)
  AND (withhold-not-release — Finding-2): re-deploying / recovering the SAME
       <job> "server" to Running-AND-HEALTHY makes getent resolve to the SAME
       F as before the stop (the FrontendAddrAllocator retained F across the
       zero-healthy window — F is per-logical-workload, released ONLY on
       logical-workload deletion)
```

**Chained narrative** (Pillar 2): the `Given` reuses S-DBN-WS's `Given +
When` (a name resolved to `F`) — then the lifecycle continues into the
stop. The two Tier-3 scenarios read as one `<job>`'s life. **REV-2
withhold-not-release** (03-01 minor re-distill, Finding-2): the answer is
withheld at the `name_index` (NXDOMAIN) but the addressing binding survives
at the allocator — so a recovered `<job>` keeps its SAME `F`. This is the
Tier-3 observable of the S-DBN-IDX-02 / S-DBN-FRONTEND-03 Tier-1
withhold-not-release contract. A *withdrawn (deleted)* `<job>` would also
NXDOMAIN AND release its `F` — but the deletion case is the allocator's
`release` (S-DBN-FRONTEND-03 Property 2); this scenario exercises the
transient-stop case where `F` is retained.

---

### S-DBN-NXDOMAIN-03 — An unknown name yields NXDOMAIN through `getent`

**Tags**: `@real-io` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (unknown-name case, end-to-end)
**Driving port**: `getaddrinfo`/`getent`
**Test surface**: Tier 3 — `dns_responder_nxdomain.rs`
**Production code guarded**: `DnsResponder` serve loop + `answer_for` lookup-miss → `NxDomain` over the real socket

#### Spec

```
GIVEN a Lima VM as root (else SKIP), overdrive serve booted, at least one
       unrelated server deployed and Running-AND-HEALTHY
WHEN a deployed client queries "nonexistent.svc.overdrive.local" via getent
THEN getent reports the name does not resolve (NXDOMAIN)
  AND the unrelated server's name still resolves in the same fixture
       (the miss does not break the hit path on the real socket)
```

---

### S-DBN-BIND-01 — Wildcard `0.0.0.0:53` + `IP_PKTINFO` coexists with systemd-resolved; replies source-pinned

**Tags**: `@boot` `@real-io` `@dbn-US-2`
**US trace**: US-DBN-2 (the spike-validated bind shape — DDN-5, `responder_addr` shared artifact)
**Driving port**: `DnsResponder::probe` (the wildcard bind) + `serve` (the `IP_PKTINFO` recv/send loop)
**Test surface**: Tier 3 — `crates/overdrive-control-plane/tests/integration/dns_responder_bind.rs` (NEW)
**Production code guarded**: `responder.rs` wildcard bind (`SO_REUSEADDR` + `IP_PKTINFO`), the `recvmsg`/`sendmsg` `ipi_spec_dst` source-pin (`nix` `socket`/`uio` features)

#### Spec

```
GIVEN a Lima VM as root (else SKIP) where systemd-resolved holds
       127.0.0.53:53 and 127.0.0.54:53 as SPECIFIC-address binds
       (the spike's pre-run environment — ss -ulnp confirms)
  AND >=2 per-workload netns provisioned (each with its gateway addr +
       injected resolv.conf), via the production deploy path
WHEN DnsResponder::probe binds 0.0.0.0:53 (SO_REUSEADDR + IP_PKTINFO)
  AND serve receives queries from BOTH netns' gateways on the one socket
THEN ss -ulnp shows the wildcard 0.0.0.0:53 binder coexisting with
     systemd-resolved's two specific binds (no EADDRINUSE)
  AND getent from EACH netns resolves its query (each reply source-pinned
     to the queried gateway via ipi_spec_dst — the getent path ACCEPTS it,
     which it only does when the source-pin is correct; spike finding #2)
```

**Directly mirrors the spike's proven shape** (`spike/findings.md` "Bind
shape" + "Source-pinning proof"). The litmus is `getent` from each netns,
NOT `dig` — a missing source-pin passes `dig` and fails `getent`.

**Framing (this scenario's own assertions)**: BIND-01 owns the BIND-layer
proofs — the `ss -ulnp` wildcard/systemd-resolved coexistence and the
per-netns `ipi_spec_dst` source-pin (proven via `getent` accepting each
source-pinned reply). The end-to-end dial-by-name acceptance proper (name
→ running-and-healthy addr → intercepted mTLS hop) is S-DBN-WS's concern;
BIND-01 uses `getent` here narrowly as the source-pin oracle, not as the
full resolution-acceptance gate.

---

### S-DBN-BIND-02 — Per-gateway-addr fallback re-derives the bound socket set from `NetSlotAllocator` on the converge tick

**Tags**: `@boot` `@real-io` `@error_path` `@dbn-US-2`
**US trace**: US-DBN-2 (the DDN-5 fallback — `responder_addr` integration risk)
**Driving port**: `DnsResponder::probe`/`serve` fallback path triggered by a wildcard `EADDRINUSE`
**Test surface**: Tier 3 — `dns_responder_bind.rs`
**Production code guarded**: `responder.rs` per-gateway-addr fallback bind, the re-derive-on-converge-tick re-bind lifecycle reading `NetSlotAllocator::snapshot()` + `responder_addr_for_slot` (DDN-5, `veth_provisioner.rs:561,754`)

#### Spec

```
GIVEN a Lima VM as root (else SKIP) where a stand-in process holds a
       WILDCARD 0.0.0.0:53 bind (forcing the wildcard path to EADDRINUSE
       — the appliance-image case the spike did NOT exercise but
       implemented the fallback for)
  AND >=2 allocs assigned slots in the NetSlotAllocator (each with a
       derived gateway addr via responder_addr_for_slot)
WHEN DnsResponder::probe attempts the wildcard bind, gets EADDRINUSE, and
     falls back to per-gateway-addr sockets re-derived from
     net_slot_allocator.snapshot()
THEN one :53 socket is bound per currently-assigned gateway addr
     (the desired set == { responder_addr_for_slot(slot) : slot in snapshot })
  AND getent from each netns resolves (each per-addr socket answers its gateway)
  AND WHEN a new alloc is assigned a slot (a new gateway appears in snapshot)
       on the next converge tick, a new per-addr :53 socket is bound for it
       (add-if-missing); and when an alloc is released, its socket is dropped
       (drop-if-absent) — the bound set tracks the live slot set (reconcilers.md Bar-1 converge)
```

**This is the node-image-coupling insurance** (DDN-5; the spike
implemented the fallback but did not fire it). Example-based sad path —
the `EADDRINUSE` trigger is forced by a stand-in wildcard holder; one
example covers the failure mode.

---

### S-DBN-BIND-03 — Boot refuses (`health.startup.refused`) on an unbindable port or an unreadable store

**Tags**: `@boot` `@real-io` `@error_path` `@dbn-US-2`
**US trace**: US-DBN-2 (the Earned-Trust gate — DDN-6, wire→probe→use)
**Driving port**: `run_server_with_obs_and_driver` (the composition root) → `DnsResponder::probe` returning `Err`
**Test surface**: Tier 3 — `dns_responder_bind.rs`
**Production code guarded**: `run_server` responder-probe call site (`health.startup.refused` warn + boot-refusal mapping, DDN-6); `DnsResponderError::{Bind, ListSeed, Probe, Socket}` (each → a distinct refusal reason, mirroring `MtlsResolveError::{Probe, StoreUnreadable}`)

#### Spec

```
SCENARIO A (unbindable port):
GIVEN a Lima VM as root where BOTH the wildcard 0.0.0.0:53 AND every
       candidate per-gateway-addr :53 are already held (no bindable :53)
WHEN run_server_with_obs_and_driver boots and calls responder.probe()
THEN probe returns Err(DnsResponderError::Bind { addr, source })
  AND the boot REFUSES (run_server returns an error; the process exits non-zero)
  AND a structured health.startup.refused event is observable with a
       reason naming the bind failure (NOT a silent start-then-fail-to-answer)

SCENARIO B (unreadable store at List-seed):
GIVEN an ObservationStore whose all_service_backends_rows() fails at probe
WHEN responder.probe() runs its List-seed
THEN probe returns Err(DnsResponderError::ListSeed { reason })
  AND the boot REFUSES with a health.startup.refused reason naming the List failure
```

**wire→probe→use (DDN-6)**: a responder that binds lazily could start and
THEN fail to answer (the silent-degradation footgun). The Earned-Trust
gate makes an unbindable port / unreadable store a boot refusal. This
mirrors the `MtlsResolve.probe()` → `health.startup.refused` →
`MtlsBoot` block the responder is wired next to (lib.rs ~1904). The
`DnsResponderError` enum carries NO `Internal(String)` — each variant
maps to a distinct refusal reason (`development.md` § "Never flatten a
typed error to `Internal(String)`").

---

## Adapter coverage table (Mandate 6 / adapter integration)

Every driven adapter the responder touches has at least one `@real-io`
scenario. Audit:

| Adapter | `@real-io` scenario(s) | Notes |
|---|---|---|
| UDP `0.0.0.0:53` socket loop (`recvmsg`/`sendmsg` + `IP_PKTINFO`) | S-DBN-WS, S-DBN-BIND-01, S-DBN-BIND-02 | The irreducibly-Tier-3 substrate (DDN-4, no Tier-2 backstop). `getent` source-pin is the litmus. |
| `getaddrinfo`/`getent` (glibc stub resolver — the CONSUMING adapter) | S-DBN-WS, S-DBN-WS-STABLE, S-DBN-NXDOMAIN-01/02/03, S-DBN-BIND-01/02 | The acceptance signal (K2) — resolves to the stable frontend `F`. NEVER `dig` alone. |
| `ObservationStore` reader (`all_service_backends_rows` List + `subscribe_all_events` Watch) | S-DBN-WS (real `LocalObservationStore`), S-DBN-IDX-01/02/03/04, S-DBN-COHERENCE-01 (SimObservationStore, Tier 1) | The third sibling reader over the SAME rows (DDN-1). The single ordered drain feeds BOTH `name_index` and the re-keyed `by_frontend` (Finding-3). Tier-3 exercises real redb reads; Tier-1 the watch/relist/ordering logic. |
| `FrontendAddrAllocator` (`assign`/`release`/`snapshot`, the NEW per-`<job>` stable-frontend source) | S-DBN-FRONTEND-01/02/03/04 (Tier 1 pure), S-DBN-WS / S-DBN-WS-STABLE / S-DBN-NXDOMAIN-02 (Tier 3, real binding) | The NEW REV-2 1a-A allocator. Pure (no port trait) — the Tier-1 scenarios are the proptest seam; Tier-3 observes the binding through `getent` stability across a cycle. |
| re-keyed `MtlsResolve.resolve` (`by_frontend` translation + the three-way `classify`) | S-DBN-REKEY-01/02/03/04, S-DBN-FAILCLOSED-01, S-DBN-EQUIV-01 (Tier 1), S-DBN-SINGLE-SRC, S-DBN-WS, S-DBN-CHURN (Tier 3) | EXTENDED additively (REV-2 1b-A supersedes REV-1's "untouched"). The `BackendIndex`/`classify` is a Tier-1 data-structure seam; the Tier-3 oracle feeds the answered `F` into the live re-keyed `resolve`. |
| `Clock` (SOA SERIAL source) | S-DBN-WIRE-04 (Tier 1, injected reading) | Injected `Arc<dyn Clock>`; never wall-clock. |
| `NetSlotAllocator` (`snapshot()` + `responder_addr_for_slot`, fallback source) | S-DBN-BIND-02 | Concrete host state (not a port trait); read ONLY on the per-addr fallback path. (Distinct from the NEW `FrontendAddrAllocator` — `NetSlotAllocator` stays per-`AllocationId`.) |
| `EbpfDataplane` + egress nft-TPROXY intercept (captures the connection to `F`) | S-DBN-WS, S-DBN-WS-STABLE, S-DBN-CHURN, S-DBN-PINGPONG | Reused verbatim from the canonical-address keystone (BLOCKER-1 WORKS — destination-blind capture of a non-`/30` frontend addr); the dial-by-name addition is the resolution + translation steps around it. |
| per-connection pump task + `TCP_USER_TIMEOUT` (active-termination, NO `sock_destroy`) | S-DBN-CHURN | The terminating-proxy churn surface; backend death bounded by `TCP_USER_TIMEOUT`, NOT `sock_destroy` (#61 scope). |
| `overdrive deploy` submit handler (`POST /v1/jobs`) | S-DBN-WS, S-DBN-WS-STABLE, S-DBN-PINGPONG, S-DBN-NXDOMAIN-* | The driving adapter; in-process per the keystone precedent. |

**Empty rows**: none. Every driven adapter the responder adds or consumes
has a `@real-io` scenario. **CM-A** adapter integration coverage: PASS.
The pure DNS codec (`wire.rs`), the pure `answer_for`, the pure
`FrontendAddrAllocator`, and the `BackendIndex`/`classify` data structure
are NOT adapters — they are the Tier-1 seams (proptested at S-DBN-WIRE-* /
S-DBN-ANSWER-* / S-DBN-FRONTEND-* / S-DBN-REKEY-*), the correct treatment
per DDN-4 (no port trait, no Sim adapter for the irreducibly-Tier-3
socket).

---

## Driving-adapter verification (Mandate 1 / hexagonal boundary)

The user-facing driving surfaces are: (1) the **`getaddrinfo`/`getent`**
stub-resolver path the workload actually uses, (2) the **`overdrive
deploy`** submit handler (`POST /v1/jobs`), and (3) the **`overdrive
serve`** boot composition (`run_server_with_obs_and_driver`).

- **`getent` is THE name-path driving adapter.** S-DBN-WS, the
  S-DBN-NXDOMAIN-* set, and S-DBN-BIND-01/02 all resolve through
  `getaddrinfo`/`getent` from inside a deployed workload's netns — the
  unmodified-workload path (Ana's lens). A scenario that asserts on `dig
  @gw` instead would prove the wire codec works but NOT that the
  source-pin makes glibc accept the reply — the exact gap the spike
  flagged (`spike/findings.md` edge case 2). The reviewer MUST flag any
  name-path Tier-3 scenario that asserts on `dig` alone.
- **`overdrive deploy` (`POST /v1/jobs`)** is exercised in-process via
  the keystone's `run_server_deploy` shim (a real HTTPS request to the
  bound socket — NOT a direct call into application services), mirroring
  `canonical_address_inbound_walking_skeleton.rs:606`.
- **`overdrive serve` (`run_server`)** is the boot driving surface for
  S-DBN-BIND-03 (the Earned-Trust refuse-boot gate) — `overdrive serve`
  is the binary's real entry, and the responder probe is the first
  user-observable boot behaviour for the name layer.

A scaffold that calls `answer_for` directly and skips the socket/`getent`
path is a valid Tier-1 unit test (S-DBN-ANSWER-*), NOT a substitute for
the Tier-3 driving-adapter scenarios. Both are needed: the pure seam
proves the contract logic; the `getent` path proves the wiring + the
source-pin.

---

## Production code → scenario mapping (mutation testing scope)

| Production code path | Guarded by | Mutation-killable signal |
|---|---|---|
| `MeshServiceName::new` suffix grammar (the bespoke `FromStr`) | S-DBN-NAME-01/02/03/04 | YES — a mutant accepting a wrong suffix or skipping the label-limit flips a validation property |
| `MeshServiceName` case-fold + lowercase `Display` | S-DBN-NAME-02 | YES — a mutant skipping `to_ascii_lowercase` flips the case-insensitivity property |
| `answer_for` `Records` arm (the stable frontend `F` → A) | S-DBN-ANSWER-01, S-DBN-IDX-04 | YES — a mutant returning empty / the wrong addr / a backend addr (`∈ 10.99.0.0/16`) instead of `vec![F]` flips the equality (THE primary mutation target, DDN-4) |
| `answer_for` `NxDomain` arm (withheld `<job>`) | S-DBN-ANSWER-02/05, S-DBN-NXDOMAIN-* | YES — a mutant returning `Records` for a withheld `<job>` flips the fail-honest property |
| `answer_for` `NoData` arm (`AAAA` on a resolvable `<job>`) | S-DBN-ANSWER-03 | YES — a mutant returning `NxDomain` for AAAA-on-resolvable, or a fabricated v6 addr, flips the contract |
| `NameIndex` `Backend.healthy == true` gate as the WITHHOLD seam (REV-2 DDN-2 / Finding-2) | S-DBN-ANSWER-04, S-DBN-IDX-02, S-DBN-REKEY-01, S-DBN-SINGLE-SRC | YES — a mutant dropping the healthy filter exposes a `<job>` whose only backends are unhealthy → the connect translates to a `MeshUnreachable` backend, flipping S-DBN-SINGLE-SRC's `Mesh` assertion; the WITHHOLD-not-release distinction is killed by S-DBN-IDX-02's retained-`F` check |
| `NameIndex` `<job>` → stable `F` mapping (REV-2 D-DBN-4') | S-DBN-ANSWER-01, S-DBN-IDX-01/04 | YES — a mutant mapping `<job>` → a backend addr (REV-1 behaviour) instead of the allocator's `F` flips the `F ∈ 10.98.0.0/16` assertion |
| `NameIndex` List-seed at probe | S-DBN-IDX-01, S-DBN-WS | YES — a mutant skipping the List leaves `<job>`s unresolvable |
| `NameIndex` Watch drain + `<job>`-grouping (OQ-1 accessor) | S-DBN-IDX-01/02 | YES — a mutant grouping by the wrong SVID segment makes `<job>`s resolve to the wrong `F` or not at all |
| `NameIndex` relist-on-`Lagged` | S-DBN-IDX-03 | YES — a mutant ignoring `Lagged` leaves the index stale (a resolvable `<job>` answers NXDOMAIN) |
| `FrontendAddrAllocator::assign` (smallest-free scan over `10.98.0.0/16`) | S-DBN-FRONTEND-01/04 | YES — a mutant assigning out-of-block, or two live `<job>`s the same `F`, flips the membership / pairwise-distinctness property |
| `FrontendAddrAllocator::assign` idempotency per `<job>` (Finding-2) | S-DBN-FRONTEND-02, S-DBN-WS-STABLE | YES — a mutant minting a NEW `F` on a re-`assign` flips the byte-stability property (the SQ1 staleness regression) |
| `FrontendAddrAllocator::release` (only on deletion; no health input) | S-DBN-FRONTEND-03, S-DBN-IDX-02, S-DBN-NXDOMAIN-02 | YES — a mutant releasing `F` on a zero-healthy transition flips the retained-`F` check (reintroduces SQ1 stale-cached-`F`) |
| `BackendIndex.by_frontend` lookup + `classify` hit arm → first-by-`Ord` healthy → `Mesh` (REV-2 1b-A, BLOCKER-2) | S-DBN-REKEY-01, S-DBN-SINGLE-SRC, S-DBN-WS | YES — a mutant selecting last-by-`Ord` / an unhealthy backend / returning `NonMesh` on a hit flips the `Mesh` + first-by-`Ord` assertions |
| `classify` `by_frontend` hit but zero-healthy → `MeshUnreachable` | S-DBN-REKEY-02 | YES — a mutant returning `Mesh` (no backend) or `NonMesh` (cleartext) on a known-but-unhealthy service flips the fail-closed-on-hit assertion |
| `FrontendKey = (SocketAddrV4, Proto)` proto discrimination (Finding-1) | S-DBN-REKEY-03 | YES — a mutant dropping the `Proto` field from the key collapses two same-`(F, port)` services onto one key, flipping the proto-discrimination property |
| `classify` `by_addr` fall-through (additive, backward-compatible) | S-DBN-REKEY-04 | YES — a mutant routing a `by_addr` hit through the frontend path / breaking the non-mesh fall-through flips the backward-compat assertion |
| `classify` fail-closed-on-frontend-subnet-miss arm: `∈ 10.98.0.0/16` miss → `MeshUnreachable` (Finding-3) | S-DBN-FAILCLOSED-01 | YES — a mutant treating a `10.98.0.0/16` miss as `NonMesh` (cleartext) flips Property 1 (fail-open footgun); a mutant treating EVERY miss as `MeshUnreachable` flips Property 2 (breaks non-mesh egress) |
| single ordered drain ordering invariant: `by_frontend` before `name_index` (Finding-3 (i)) | S-DBN-COHERENCE-01 | YES — a mutant exposing `F` to `name_index` before binding `by_frontend` flips the ordering property |
| re-keyed `MtlsResolve` `classify` observable equivalence (host vs in-memory, the trait-docstring contract) | S-DBN-EQUIV-01 | YES — a divergence between the two build paths means the contract or an impl is wrong; the test isolates which (DST replay-equivalence) |
| per-connection pump task + `TCP_USER_TIMEOUT` (active-termination, NO `sock_destroy`) | S-DBN-CHURN | YES (Tier 3) — a mutant disabling the timeout / never surfacing backend death flips the prompt-reset bound (the in-flight dial hangs) |
| `wire.rs` `Records` encode (A records — renders whatever IPv4 addr it is handed) | S-DBN-WIRE-01, S-DBN-WS | YES — a mutant emitting the wrong RDATA / count fails the hickory round-trip (the codec is addr-agnostic — REV-2 hands it `F`) |
| `wire.rs` `NoData` encode (NOERROR + SOA, MINIMUM=1) | S-DBN-WIRE-02 | YES — a mutant emitting NXDOMAIN or omitting the SOA / wrong MINIMUM flips the header/SOA assertion |
| `wire.rs` `NxDomain` encode (NXDOMAIN + SOA, MINIMUM=1) | S-DBN-WIRE-03 | YES — a mutant emitting NOERROR or wrong MINIMUM flips it |
| `wire.rs` SOA SERIAL derivation from `Clock` | S-DBN-WIRE-04 | YES — a mutant reading wall-clock / a constant flips the deterministic-per-`Clock` property |
| `responder.rs` wildcard bind (`SO_REUSEADDR` + `IP_PKTINFO`) | S-DBN-BIND-01, S-DBN-WS | YES (Tier 3) — without `IP_PKTINFO`, `getent` rejects the reply and the test goes RED |
| `responder.rs` per-addr fallback + re-derive-on-converge re-bind | S-DBN-BIND-02 | YES (Tier 3) — a mutant binding the wrong derived gateway / not tracking slot changes flips per-netns resolution |
| `responder.rs` `ipi_spec_dst` source-pin on the send path | S-DBN-BIND-01, S-DBN-WS | YES (Tier 3) — the `getent`-not-`dig` litmus; a missing pin RED on `getent` |
| `run_server` responder + `FrontendAddrAllocator` + re-keyed resolve construct + probe + spawn (DDN-6 + REV-2 02-01) | S-DBN-WS, S-DBN-BIND-03 | YES — deleting the spawn RED's S-DBN-WS (`getent` times out — the litmus); a probe that doesn't refuse-boot flips S-DBN-BIND-03 |
| `DnsResponderError` variant → `health.startup.refused` reason mapping | S-DBN-BIND-03 | YES — a mutant flattening to one reason (or `Internal(String)`) flips the cause-distinct refusal assertion |

**Empty rows**: zero. Every production code path the responder adds has
at least one scenario (Tier 1 for the pure seams, Tier 3 for the socket
+ boot + churn). **Mutation invocation** (per `.claude/rules/testing.md`):
`cargo xtask lima run -- cargo xtask mutants --diff origin/main --features
integration-tests --package overdrive-control-plane --file <files-touched>`
for the `dns_responder/*` files, the NEW
`dns_responder/frontend_addr_allocator.rs`, and the EXTENDED
`mtls_resolve_adapter.rs` (the `by_frontend`/`classify` re-key); `--package
overdrive-core --file crates/overdrive-core/src/id.rs` for
`MeshServiceName`/`NameAnswer`. Per-step runs scope to the step's files;
the pre-PR run covers the full per-package diff. THE primary kill targets
(DDN-4 + REV-2): the `answer_for` `Records` arm (now `vec![F]`), the
`classify` three-way arm (hit / fail-closed-subnet-miss / `by_addr`
fall-through), the first-by-`Ord` selection (BLOCKER-2), and the
`FrontendAddrAllocator` idempotency + release-only-on-deletion (Finding-2).

---

## What these scenarios do NOT cover (explicit deferrals)

- **OQ-1 — the `SpiffeId` → `<job>` accessor signature.** CONFIRMED-ABSENT
  this RE-DISTILL pass: `SpiffeId` (`crates/overdrive-core/src/id.rs:269-304`)
  exposes only `as_str()`, `trust_domain()`, `path()` (and the
  `for_allocation` constructor) — there is NO job-segment accessor. The
  mapping is verified (REV-2 D-DBN-4': the `WorkloadId` segment of the SVID
  path = `<job>`), but the exact accessor — a new
  `SpiffeId::job_segment() -> Option<&str>` on the newtype vs a parse
  helper local to `name_index.rs` — is an open surface decision the crafter
  MUST pin in DELIVER, per CLAUDE.md "Implement to the design — never invent
  API surface." **DISTILL does NOT pick a signature.** Under REV-2 the
  accessor is STILL needed (now to key the `FrontendAddrAllocator` by `<job>`
  AND to group running-and-healthy rows under it for the resolve
  translation). S-DBN-IDX-01 / S-DBN-FRONTEND-01 assert the *behaviour*
  (rows grouped by `<job>`; the allocator keyed by `<job>`); DELIVER pins the
  accessor that achieves it and surfaces it as a decision, not an improvisation.
- **`FrontendKey` newtype-vs-tuple shape (REV-2 DELIVER detail).** The
  CONTRACT (`feature-delta.md` § Frontend-key contract) is that the key
  `FrontendKey = (SocketAddrV4, Proto)` DISCRIMINATES proto. Whether it is a
  bare tuple or a named newtype `FrontendKey(SocketAddrV4, Proto)` for
  call-site clarity is an explicit DELIVER latitude; S-DBN-REKEY-03 asserts
  the proto-discrimination *behaviour*, not the type shape. DISTILL does NOT
  pick.
- **The `orig_dst.ip() ∈ 10.98.0.0/16` CIDR-membership accessor (REV-2
  DELIVER detail).** The CONTRACT (Finding-3 / § Frontend-subnet coherence
  contract) is the membership test + the `MeshUnreachable` verdict. Whether
  it is `Ipv4Net::contains` on the `WORKLOAD_FRONTEND_BASE` const or an
  equivalent mask compare is a DELIVER surface detail; the crafter MUST NOT
  invent a broader "is this any reserved subnet" helper — the test is
  specifically membership in the dial-by-name frontend block.
  S-DBN-FAILCLOSED-01 asserts the membership *behaviour*, not the accessor shape.
- **`NameAnswer` module placement** (`id.rs` — already chosen and committed
  per commit `04fa3d18`; `NameAnswer` lives in `overdrive-core::id`). No
  longer open — the addr-agnostic `NameAnswer` is reused unchanged by REV-2.
- **VIP path** (`<job>.svc.overdrive.local → fdc2::/16` VIP + XDP
  `SERVICE_MAP` + `sock_destroy`) — **#61** (depends on #167). REV-2's
  stable IPv4 frontend is VIP-*shaped* but delivered via nft-TPROXY, NOT the
  #61 XDP/`SERVICE_MAP`/`fdc2::/16` machinery (a re-scoping, not a
  contradiction; research Conflict 2). `sock_destroy` belongs to #61 (the
  userspace-proxy-free connect-time LB), NOT here — REV-2 § Active-termination
  posture. The #61 corrected-scope GitHub edit is a user-gated item (the
  orchestrator relays; no agent edits the issue).
- **Relaxed positive `A` TTL (REV-2 named optional, NOT v1-auto-applied).**
  A stable answer makes the positive `A_RECORD_TTL_SECS=1` TTL-moot by
  construction (DNS research sub-q 4); it MAY relax to a longer TTL — a
  named, optional v1 sub-decision (a one-line `wire.rs` edit, re-ratified if
  changed), NOT a contract change to `NameAnswer` and NOT auto-applied. No
  scenario asserts a relaxed positive TTL; the negative-TTL SOA MINIMUM=1
  (S-DBN-WIRE-02/03) is unchanged.
- **Intended-peer / expected-SVID pinning** (verify the resolved peer is
  the INTENDED destination, not merely SOME valid workload) — **#242**
  (split from #178). v1 is authn-only; the responder answers an addr, not
  an expected identity.
- **Declared-but-empty → NODATA** (distinct from unknown → NXDOMAIN)
  requires a declared-service view distinct from the running-and-healthy
  index — a NAMED future refinement (ADR-0072 § Out of scope), explicitly
  NOT v1; no tracking issue (v1 collapses all 0-running-and-healthy cases
  — declared-but-not-running, unhealthy, and unknown — to NXDOMAIN by
  construction).
- **IPv6 / real AAAA records** — widening the `SocketAddrV4`/`Ipv4Addr`
  substrate; out of v1 scope (AAAA is NODATA in v1). No scenario asserts a
  real v6 addr.
- **Cross-node / multi-node name resolution, gossiped name state** — out
  of Phase-2 single-node scope. No scenario crosses nodes.
- **The intercept / mTLS enforcement substrate itself** — shipped by the
  transparent-mtls arc (#26/#236); these scenarios REUSE it (S-DBN-WS,
  S-DBN-PINGPONG) and never re-test or modify it. `mtls_resolve_adapter.rs`
  is provably untouched (DDN-1).
- **6.18 appliance-kernel confirmation** — the Tier-3 verdicts here are
  dev-Lima `7.0.0-22-generic` (the spike kernel); the MERGE GATE is the
  pinned-6.18 Tier-3 matrix (ADR-0068). Re-confirmation on 6.18 is a
  DELIVER/DEVOPS Tier-3 obligation (ADR-0072 § DEVOPS), not a separate
  scenario.

---

## Self-review checklist (per skill spec)

| # | Item | Status |
|---|---|---|
| 1 | All scenarios use GIVEN/WHEN/THEN (or PROPERTY) structure | PASS |
| 2 | Error-path coverage ≥ 40% | PASS (18/39 = 46%) |
| 3 | Business-language purity in scenario titles | PASS — titles use domain terms (`mesh service name`, `resolve by name`, `stable frontend addr`, `running-and-healthy`, `stale addr`, `NXDOMAIN`, `withheld`, `prompt reset`); DNS/socket/index terms (`by_frontend`, `classify`, `IP_PKTINFO`) appear only in step bodies + production-code lines, not titles |
| 4 | Walking-skeleton user-centric framing (Dim 5) | PASS — S-DBN-WS title = a user goal ("a deployed workload resolves its peer's stable frontend name and the hop is mTLS'd"); `Then` = user observations (name resolves to a stable addr, wire encrypted) |
| 5 | Every Then asserts observable behaviour (Dim 7) | PASS — Tier 1 asserts on `answer_for` / `classify` / `assign` return values + decoded wire bytes (never the `by_name`/`by_frontend` `BTreeMap` field directly — disclaimed at ANSWER-04, IDX-01); Tier 3 asserts on `getent` output, the round-trip, `ss -ulnp`, boot exit/refusal, the prompt-reset bound — all port-exposed |
| 6 | Story-to-scenario traceability (Dim 8 Check A) | PASS — US-DBN-2/3/4 each have ≥1 scenario via `@dbn-US-N` (the NEW FRONTEND/REKEY/FAILCLOSED/COHERENCE/EQUIV/WS-STABLE/CHURN scenarios all trace to US-DBN-2/4); US-DBN-1 (spike) already PROMOTED, not re-tested |
| 7 | WS strategy declared | PASS — Architecture-of-Reference defaults + the project policy (`docs/architecture/atdd-infrastructure-policy.md`, rows appended for dial-by-name + the REV-2 frontend ports in the feature-delta DISTILL block); driving = real `serve`+`deploy`+`getent`, driven-internal `ObservationStore` = real, the `FrontendAddrAllocator`/`BackendIndex` = pure (Tier-1 proptest), the socket = irreducibly Tier-3 real (NO Sim, DDN-4) |
| 8 | WS uses real adapters (not InMemory) | PASS — S-DBN-WS / S-DBN-WS-STABLE / S-DBN-CHURN / S-DBN-PINGPONG exercise real `run_server` + real `EbpfDataplane` + real kernel UDP + real `getent` + the real re-keyed `MtlsResolve` |
| 9 | Every driven adapter has a `@real-io` scenario (Dim 9c) | PASS — see adapter coverage table (the NEW `FrontendAddrAllocator` + re-keyed `MtlsResolve` + pump-task adapters all covered) |
| 10 | Scenarios named for user value, not technical operations | PASS — titles describe outcomes (`resolves its peer's stable frontend name`, `counters advance`, `the <job> is withheld`, `prompt reset … never an indefinite hang`, `assigned a stable frontend addr`) |
| 11 | Driving port named explicitly per scenario | PASS — every scenario has a "Driving port" line |
| 12 | pytest-bdd `.feature` files exist | N/A (Rust project — no `.feature` files per `.claude/rules/testing.md`; this doc is the GIVEN/WHEN/THEN SSOT) |
| 13 | pytest fixtures isolated per environment | N/A (Rust project) |
| 14 | `@property` scenarios at the right layer (Mandate 9) | PASS — every `@property` is Tier 1 (S-DBN-NAME-*, S-DBN-FRONTEND-*, S-DBN-ANSWER-*, S-DBN-WIRE-*, S-DBN-IDX-*, S-DBN-REKEY-*, S-DBN-FAILCLOSED-01, S-DBN-COHERENCE-01, S-DBN-EQUIV-01); Tier-3 sad paths (S-DBN-NXDOMAIN-*, S-DBN-BIND-02, S-DBN-CHURN, S-DBN-WS-STABLE) are example-based (Mandate 11) |
| 15 | conftest.py shared fixtures isolated | N/A (Rust project — Tier-3 fixtures isolate via `TempDir`, root-gate SKIP, RAII netns teardown per the keystone) |

**Adapted-for-Rust items**: 12, 13, 15 are pytest-specific and N/A. The
Rust equivalents (`integration-tests` feature gating, per-test
`tempfile::TempDir`, RAII netns/socket teardown, the leftover-netns
cleanup discipline of `.claude/rules/debugging.md`) are governed by
`.claude/rules/testing.md` and are DELIVER's responsibility.

---

## DISTILL Review

### REV-2 re-distill — prior review SUPERSEDED; Final Wave Review Gate pending

The prior `nw-acceptance-designer-reviewer` (Sentinel) **APPROVED** verdict
(2026-06-25) reviewed the **superseded REV-1 26-scenario spec** (DNS answers a
per-instance backend addr). That verdict does NOT carry forward to this REV-2
39-scenario rewrite — the answer contract, the index mapping, the resolve
classification, and 13 NEW scenarios are materially changed. The REV-1 review
text is intentionally NOT preserved verbatim here (it would assert green over a
spec that no longer exists — the "stale evidence treated as live" failure mode
`.claude/rules/verification.md` warns against). Its load-bearing findings that
still apply to REV-2 are folded in below.

**This REV-2 spec MUST be re-reviewed** by the Final Wave Review Gate
(`@nw-acceptance-designer-reviewer` Sentinel + the three sibling reviewers
against the full `feature-delta.md`) before DELIVER handoff. The
orchestrator dispatches that gate; DISTILL does NOT self-stamp a verdict.

### REV-1 findings carried into REV-2 (still apply)

- **`getent`-not-`dig` litmus** — structurally enforced (K2 + every name-path
  Tier-3 THEN + the failure-message text). REV-2 preserves this verbatim
  (S-DBN-WS / S-DBN-NXDOMAIN-* / S-DBN-BIND-01/02 all assert on `getent`).
- **OQ-1 genuinely open** — `SpiffeId` exposes only `as_str`/`trust_domain`/
  `path`/`for_allocation` (re-confirmed `id.rs:269-304` this pass); no
  job-segment accessor. REV-2 STILL needs it (now to key the frontend
  allocator + group rows). DISTILL picks no signature.
- **Vertical-slice litmus** — S-DBN-WS's "delete the `run_server` spawn →
  `getent` times out" guard is preserved; REV-2 ADDS "delete the
  `by_frontend` arm → connect misses → fail-closed, no backend reached." No
  test binds `:53`, installs a `resolv.conf`, allocates `F`, or hand-installs
  the egress capture (BLOCKER-1 confirmed production captures `F`).
- **Tier-3 boot shape mirrors the keystone** (`canonical_address_inbound_
  walking_skeleton.rs`: `run_server_with_obs_and_driver`, `is_root()` SKIP,
  `run_server_deploy`, `dial_in_netns`) — unchanged.
- **`@property` tier discipline** — every `@property` is Tier 1; Tier-3 sad
  paths (incl. the NEW S-DBN-CHURN / S-DBN-WS-STABLE) are example-based
  (Mandate 11). No `@property` leaks into Tier 3.
- **EDD honesty** — S-DBN-PINGPONG → `E05`, honest `pending` mirroring E04
  until #227/#75; no fabricated capture (REV-2 unchanged behaviorally at the
  demo layer).
- **`@dbn-US-N` traceability** — US-DBN-2/3/4 each ≥1 scenario; the 13 NEW
  REV-2 scenarios all trace to US-DBN-2/4.

### REV-2-specific spot-checks (self-verified this pass; the reviewer re-confirms)

- **`NameAnswer` reused unchanged** — `Records(Vec<SocketAddrV4>)` is
  addr-agnostic (committed `04fa3d18`, `id.rs:954`); REV-2 hands it `vec![F]`.
  `MeshServiceName` + the codec are addr-agnostic and PRESERVED.
- **Re-keyed surfaces grounded on live code** — `resolve(orig_dst:
  SocketAddrV4)` + `by_addr: BTreeMap<SocketAddrV4, BTreeMap<ServiceId,
  Backend>>` + the `NonMesh`/`MeshUnreachable` classification + the "a miss is
  `NonMesh`, NOT `MeshUnreachable`" rustdoc (`mtls_resolve_adapter.rs:127-128`,
  the line the fail-closed-on-subnet-miss arm is built to be SAFE against) all
  verified. `by_frontend`/`FrontendKey`/`FrontendAddrAllocator`/
  `WORKLOAD_FRONTEND_BASE` are confirmed ABSENT from `crates/` (NEW DELIVER
  surfaces, not yet built).
- **Subnet consts grounded** — `WORKLOAD_SUBNET_BASE = 10.99.0.0/16`
  (`veth_provisioner.rs:307`, a const `Ipv4Net`); `VipRange::default() =
  10.96.0.0/16` (the collision the spike's candidate was rejected for);
  `WORKLOAD_FRONTEND_BASE = 10.98.0.0/16` disjoint from both. S-DBN-FRONTEND-01
  asserts membership against the real consts.
- **No invented surface** — every type/fn/variant a scenario names is the
  REV-2-pinned surface (`feature-delta.md` § "Pinned signatures (FINAL — 1a-A /
  1b-A ratified)"): `FrontendAddrAllocator::{new, assign, release, snapshot}`,
  `FrontendKey = (SocketAddrV4, Proto)`, `by_frontend: BTreeMap<FrontendKey,
  ServiceId>`, the three-way `classify` arm, `answer_for(name, qtype, &index)`
  (signature unchanged). The three DELIVER-open details (OQ-1 accessor,
  `FrontendKey` newtype-vs-tuple, the CIDR-membership accessor) are NAMED, not
  picked (§ "What these scenarios do NOT cover").

**Handoff:** Pending the Final Wave Review Gate against this REV-2 spec. The
open surfaces (OQ-1 + the two REV-2 DELIVER details) are correctly named and
left for the crafter to pin per CLAUDE.md, not improvised.
