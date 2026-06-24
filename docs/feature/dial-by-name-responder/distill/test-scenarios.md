# Test scenarios — `dial-by-name-responder`

**Wave**: DISTILL | **Mode**: PROPOSE | **Designer**: Quinn | **Date**: 2026-06-25

**Scope**: executable spec for GH #243 (the in-agent node-local DNS
responder — the THIRD reader of the `service_backends` observation
surface). Covers US-DBN-2 (walking skeleton A→B), US-DBN-3
(bidirectional ping-pong demo), US-DBN-4 (empty-candidate NXDOMAIN
honesty), and the pure `answer_for` / `wire.rs` / `MeshServiceName` /
`NameIndex` seams the socket loop cannot reach. US-DBN-1 (the spike) is
PROMOTED already (`spike/wave-decisions.md`) and is not re-tested here.

**Strategy**: Tier 1 (DST/in-process pure, `tests/acceptance/`, default
lane) + Tier 3 (real-kernel Lima, `tests/integration/`, gated
`#![cfg(feature="integration-tests")]`). **No `.feature` files** — per
`.claude/rules/testing.md` § "No `.feature` files anywhere", this
document is the GIVEN/WHEN/THEN SSOT; DELIVER's RED phase translates each
scenario into a Rust `#[test]`/`#[tokio::test]` body or a proptest. There
is **no Tier 2** here — the socket loop is irreducibly Tier-3 (no
`BPF_PROG_TEST_RUN` surface; DDN-4). Proptest is the Tier-1 tool for the
pure seams (`MeshServiceName` round-trip, `answer_for`, the `wire.rs`
codec).

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

**Production code under test → scenario mapping** lives in §
"Production code → scenario mapping (mutation testing scope)" below.

**Pinned-signature discipline (CLAUDE.md "Implement to the design").**
Every type/fn/variant these scenarios name is EXACTLY the DESIGN-pinned
surface (`feature-delta.md` § "Pinned signatures", ADR-0072 §
Components). DISTILL invents NO API. The one open surface decision —
**OQ-1**, the `SpiffeId` → `<job>` accessor — is NAMED here, not picked
(see § "What these scenarios do NOT cover").

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

---

## Scenario index

| ID | Title | Tags | Tier | US trace |
|---|---|---|---|---|
| S-DBN-NAME-01 | Mesh service name round-trips through Display / FromStr / serde | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-NAME-02 | Mesh service name parse is case-insensitive, canonical form is lowercase | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-NAME-03 | Suffix grammar accepts `<job>.svc.overdrive.local`, rejects wrong / missing suffix | `@in-memory` `@error_path` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-NAME-04 | Over-long label and empty / malformed `<job>` are rejected with a typed `IdParseError` | `@property` `@in-memory` `@error_path` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-ANSWER-01 | `A` with ≥1 running-and-healthy backend yields `Records` of exactly the running-and-healthy IPv4 set | `@property` `@in-memory` `@kpi` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-ANSWER-02 | `A` with 0 running-and-healthy backends yields `NxDomain` | `@property` `@in-memory` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 |
| S-DBN-ANSWER-03 | `AAAA` on a live name yields `NoData`; `AAAA` on an empty name yields `NxDomain` | `@property` `@in-memory` `@error_path` `@dbn-US-2` `@dbn-US-4` | Tier 1 | US-DBN-2, US-DBN-4 |
| S-DBN-ANSWER-04 | An unhealthy-only name yields `NxDomain` (the healthy gate is load-bearing) | `@property` `@in-memory` `@error_path` `@kpi` `@dbn-US-4` | Tier 1 | US-DBN-4 |
| S-DBN-ANSWER-05 | An unknown name yields `NxDomain` | `@in-memory` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 |
| S-DBN-WIRE-01 | Answered records survive a deterministic encode→decode round-trip | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-WIRE-02 | AAAA on a live name encodes NODATA with a 1-second negative-TTL SOA | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-WIRE-03 | A name with no running-and-healthy backend encodes NXDOMAIN with a 1-second negative-TTL SOA | `@property` `@in-memory` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 |
| S-DBN-WIRE-04 | SOA SERIAL is derived from the injected Clock (deterministic per `Clock` reading) | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-IDX-01 | Index seeds from List at probe; a name becomes resolvable once a running-and-healthy row exists | `@property` `@in-memory` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-IDX-02 | A backend going unhealthy drops the name from the index | `@property` `@in-memory` `@error_path` `@dbn-US-4` | Tier 1 | US-DBN-4 |
| S-DBN-IDX-03 | A `Lagged` subscription event triggers a relist that recovers the index | `@property` `@in-memory` `@error_path` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-IDX-04 | The answered addr is drawn ONLY from the same `service_backends` rows (single source) | `@property` `@in-memory` `@kpi` `@dbn-US-2` | Tier 1 | US-DBN-2 |
| S-DBN-WS | Walking skeleton — a deployed workload resolves its peer by name and the hop is mTLS'd | `@walking_skeleton` `@driving_adapter` `@real-io` `@kpi` `@dbn-US-2` | Tier 3 | US-DBN-2 |
| S-DBN-SINGLE-SRC | Byte-consistency oracle — the answered addr is the addr `MtlsResolve` classifies `Mesh` | `@real-io` `@kpi` `@dbn-US-2` | Tier 3 | US-DBN-2 |
| S-DBN-PINGPONG | Bidirectional ping-pong demo — two services dial each other by name, counters advance | `@walking_skeleton` `@real-io` `@edd` `@kpi` `@dbn-US-3` | Tier 3 | US-DBN-3 |
| S-DBN-NXDOMAIN-01 | Querying before running-and-healthy yields NXDOMAIN, never a stale addr | `@real-io` `@error_path` `@kpi` `@dbn-US-4` | Tier 3 | US-DBN-4 |
| S-DBN-NXDOMAIN-02 | After all backends stop, the name stops resolving (no stale addr) | `@real-io` `@error_path` `@kpi` `@dbn-US-4` | Tier 3 | US-DBN-4 |
| S-DBN-NXDOMAIN-03 | An unknown name yields NXDOMAIN through `getent` | `@real-io` `@error_path` `@dbn-US-4` | Tier 3 | US-DBN-4 |
| S-DBN-BIND-01 | Wildcard `0.0.0.0:53` + `IP_PKTINFO` coexists with systemd-resolved; replies source-pinned | `@boot` `@real-io` `@dbn-US-2` | Tier 3 | US-DBN-2 |
| S-DBN-BIND-02 | Per-gateway-addr fallback re-derives the bound socket set from `NetSlotAllocator` on the converge tick | `@boot` `@real-io` `@error_path` `@dbn-US-2` | Tier 3 | US-DBN-2 |
| S-DBN-BIND-03 | Boot refuses (`health.startup.refused`) on an unbindable port or an unreadable store | `@boot` `@real-io` `@error_path` `@dbn-US-2` | Tier 3 | US-DBN-2 |

**Counts**: 26 scenarios (16 Tier 1, 10 Tier 3).
**Error-path coverage**: S-DBN-NAME-03, S-DBN-NAME-04, S-DBN-ANSWER-02,
S-DBN-ANSWER-03, S-DBN-ANSWER-04, S-DBN-ANSWER-05, S-DBN-WIRE-03,
S-DBN-IDX-02, S-DBN-IDX-03, S-DBN-NXDOMAIN-01, S-DBN-NXDOMAIN-02,
S-DBN-NXDOMAIN-03, S-DBN-BIND-02, S-DBN-BIND-03 = **14 of 26 = 54%**.
Target (≥40%) met — the responder's whole posture is fail-honest (the
empty-candidate / NXDOMAIN behaviour is the load-bearing US-DBN-4 leg),
so the error surface is deliberately deep.

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

- **Budget**: 5 s (resolution should land sub-second once the backend is
  running-and-healthy; the budget absorbs index-watch propagation).
- **Cadence**: 200 ms poll (mirrors the keystone's `poll_until`).
- **Failure message** names the two equally-likely culprits: `getent
  <name> did not resolve to the running-and-healthy backend within 5s —
  the responder source-pin (ipi_spec_dst) or the by_name index
  running-and-healthy gate regressed`.

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

### S-DBN-ANSWER-01 — `A` with ≥1 running-and-healthy backend yields `Records` of exactly the running-and-healthy IPv4 set

**Tags**: `@property` `@in-memory` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (the v1 DNS answer contract, `A` row) · **KPI K-DBN-1**
**Driving port**: `answer_for(name, qtype, &index)` (the pure DST seam, the mutation-gate target — DDN-4)
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/dns_answer_for.rs` (NEW; the `answer.rs` proptest home)
**Production code guarded**: `dns_responder::answer::answer_for` `Records` arm (`crates/overdrive-control-plane/src/dns_responder/answer.rs`)

#### Spec

```
PROPERTY: for every name N and every non-empty set B of running-and-healthy
          IPv4 SocketAddrV4 backends indexed under N (and arbitrary other
          names/backends in the index),
GIVEN a NameIndex whose by_name[N] == B (the running-and-healthy set)
WHEN answer_for(N, RecordType::A, &index) is called
THEN it returns NameAnswer::Records(addrs)
  AND addrs (as a set) equals exactly B
  AND no addr outside B appears (single-source: the answer set is drawn
       ONLY from the index's running-and-healthy rows for N)
```

**This is THE mutation-gate target (DDN-4).** A mutant that returns an
empty `Records`, includes an out-of-index addr, or matches the wrong
name flips this property. Pinned `@example`: `N = server`,
`B = {10.x.y.2:8080}` (the US-DBN-2 happy-path example). Mandate-8
universe-equivalent: the `addrs` set is the full observable; set-equality
against `B` is the fail-closed guard (an extra/orphan addr fails exactly
as a `strict=True` state-delta would — per the project policy's Rust
Mandate-8 mapping).

---

### S-DBN-ANSWER-02 — `A` with 0 running-and-healthy backends yields `NxDomain`

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (the v1 DNS answer contract, empty-candidate `A` row) · **KPI K-DBN-2**
**Driving port**: `answer_for`
**Test surface**: Tier 1 — `dns_answer_for.rs`
**Production code guarded**: `answer_for` `NxDomain` arm (empty-set branch)

#### Spec

```
PROPERTY: for every name N absent from the index OR present with an empty
          running-and-healthy set,
GIVEN a NameIndex where by_name.get(N) is None or yields the empty set
WHEN answer_for(N, RecordType::A, &index) is called
THEN it returns NameAnswer::NxDomain
  AND NEVER NameAnswer::Records (no stale/cached/guessed addr — the
       fail-honest contract; declared-but-not-running, all-stopped, and
       unknown all collapse to NxDomain in v1)
```

**Negative-testing note**: this is the relaxed-precondition twin of
S-DBN-ANSWER-01 — removing "the set is non-empty" must change the answer
from `Records` to `NxDomain`, never leave a stale `Records`.

---

### S-DBN-ANSWER-03 — `AAAA` on a live name yields `NoData`; `AAAA` on an empty name yields `NxDomain`

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-2` `@dbn-US-4`
**US trace**: US-DBN-2 (`AAAA` live → NODATA) + US-DBN-4 (`AAAA` empty → NXDOMAIN)
**Driving port**: `answer_for`
**Test surface**: Tier 1 — `dns_answer_for.rs`
**Production code guarded**: `answer_for` `NoData` arm + the qtype dispatch (`RecordType::AAAA`)

#### Spec

```
PROPERTY 1 (live name): for every name N with a non-empty
            running-and-healthy set,
GIVEN by_name[N] is non-empty
WHEN answer_for(N, RecordType::AAAA, &index) is called
THEN it returns NameAnswer::NoData
  AND NEVER NxDomain (the name IS resolvable — it just has no IPv6 record
       in the v1 IPv4 substrate) AND NEVER a fabricated v6 addr

PROPERTY 2 (empty name): for every name N with an empty / absent set,
GIVEN by_name.get(N) is None or empty
WHEN answer_for(N, RecordType::AAAA, &index) is called
THEN it returns NameAnswer::NxDomain (no resolvable backend at all)
```

**Pins the v1 contract table's `AAAA` column** (`feature-delta.md` § "The
v1 DNS answer contract"). The spike PROVED the NODATA-on-live case
(`spike/findings.md` "AAAA → NODATA"); the empty-name AAAA → NXDOMAIN
case is the Slice-01 build concern the spike did not exercise.

---

### S-DBN-ANSWER-04 — An unhealthy-only name yields `NxDomain` (the healthy gate is load-bearing)

**Tags**: `@property` `@in-memory` `@error_path` `@kpi` `@dbn-US-4`
**US trace**: US-DBN-4 · **KPI K-DBN-2 / K-DBN-4** (the running-AND-healthy gate, DDN-2)
**Driving port**: `answer_for` over a `NameIndex` built from rows where `Backend.healthy == false`
**Test surface**: Tier 1 — `crates/overdrive-control-plane/tests/acceptance/dns_name_index.rs` (NEW; the `NameIndex` build path is exercised here, then `answer_for` reads it)
**Production code guarded**: the `NameIndex` `Backend.healthy == true` gate (DDN-2; `name_index.rs`), `answer_for` `NxDomain` arm

#### Spec

```
PROPERTY: for every name N whose ONLY service_backends rows carry
          Backend.healthy == false (running but unhealthy / not-ready),
GIVEN a NameIndex built (via the List-then-Watch path) from those rows
WHEN answer_for(N, RecordType::A, &index) is called
THEN it returns NameAnswer::NxDomain (the contract — the healthy gate
     kept the unhealthy-only name out of the answer set)
  AND a DIFFERENT name M with a running-AND-healthy backend in the same
      index still resolves (answer_for(M, A) → Records) — proving the
      healthy gate dropped only N, not the whole index
```

**Universe** (port-exposed): the `answer_for` return for N (NxDomain) and
for M (Records). The test NEVER asserts on the `by_name: BTreeMap`
internals — whether the gate drops the row at build time or filters it at
read time is a DELIVER detail. The observable contract is `answer_for`'s
return; a co-resident healthy name M resolving proves the index did not
collapse to empty.

**Why this is load-bearing** (DDN-2 verbatim): `MtlsResolve` classifies a
healthy backend `Mesh` but an unhealthy backend `MeshUnreachable`
(`mtls_resolve_adapter.rs:124-135`). Answering an unhealthy addr would
point the dialer at a backend the intercept path REFUSES — violating
byte-consistency. The healthy gate (DDN-2) is the mandatory mechanism
that keeps every answered addr in the `Mesh` set; the scenario observes
it through `answer_for`'s return (NxDomain for the unhealthy-only name,
Records for a co-resident healthy name), not the index internals. This
scenario is the structural guard against a mutant that drops the healthy
filter.

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

### S-DBN-IDX-01 — Index seeds from List at probe; a name becomes resolvable once a running-and-healthy row exists

**Tags**: `@property` `@in-memory` `@dbn-US-2`
**US trace**: US-DBN-2 (the List-then-Watch contract — DDN-3)
**Driving port**: `NameIndex` build via `ObservationStore::all_service_backends_rows()` (List at probe) + `subscribe_all_events()` drain
**Test surface**: Tier 1 — `dns_name_index.rs` (driven by `SimObservationStore`, in-memory)
**Production code guarded**: `name_index.rs` List-seed + Watch-drain (mirrors `ServiceBackendsResolve` probe + `spawn_drain`, `mtls_resolve_adapter.rs:307-471`)

#### Spec

```
PROPERTY: for every name N and every running-and-healthy ServiceBackendRow
          R whose Backend.alloc SVID job segment == N's <job>,
GIVEN a SimObservationStore seeded (before probe) with R
  AND a NameIndex probed against it (List-then-Watch)
WHEN the index is queried for N (via answer_for A)
THEN N resolves to R's Backend.addr (Records contains it)

  AND (watch half): GIVEN the index probed against an EMPTY store
       WHEN a running-and-healthy row R for N is written AFTER probe
       AND the watch drain processes the SubscriptionEvent::Row
       THEN N becomes resolvable (Records contains R's addr)
```

**Universe** (port-exposed): the `answer_for` result for N (the index is
exercised through its public read, never by asserting on the
`by_name: BTreeMap` field directly — that internal structure is a DELIVER
detail). **Validates the OQ-1 mapping consumer** — the index groups rows
by their SVID `<job>` segment; whether the accessor is
`SpiffeId::job_segment()` or a local parse helper is OQ-1 (DELIVER pins
it), but the *behaviour* (rows grouped by `<job>`) is asserted here.

---

### S-DBN-IDX-02 — A backend going unhealthy drops the name from the index

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-4`
**US trace**: US-DBN-4 (the healthy gate over the watch path)
**Driving port**: `NameIndex` watch drain processing a row whose `Backend.healthy` flips to `false`
**Test surface**: Tier 1 — `dns_name_index.rs`
**Production code guarded**: `name_index.rs` healthy-gate applied on the watch path (DDN-2), index re-derivation on row update

#### Spec

```
PROPERTY: for every name N initially resolvable via a healthy row R,
GIVEN N resolves to R.addr
WHEN a fresh ServiceBackendRow for the same backend is written with
     Backend.healthy == false (and the watch drain processes it)
THEN N stops resolving (answer_for(N, A) → NxDomain)
  AND the index never retains the now-unhealthy addr
       (no stale addr after an unhealthy transition — the single source
        of liveness truth is the running-and-healthy filter, US-DBN-4)
```

**Chained narrative** (Pillar 2): the `Given` reuses S-DBN-IDX-01's
`Given + When` (a name made resolvable), then transitions it
unhealthy — the tests read as the lifecycle of one name.

---

### S-DBN-IDX-03 — A `Lagged` subscription event triggers a relist that recovers the index

**Tags**: `@property` `@in-memory` `@error_path` `@dbn-US-2`
**US trace**: US-DBN-2 (the relist-on-`Lagged` recovery — DDN-3)
**Driving port**: `NameIndex` watch drain receiving `SubscriptionEvent::Lagged { missed }`
**Test surface**: Tier 1 — `dns_name_index.rs`
**Production code guarded**: `name_index.rs` `Lagged` arm → relist (mirrors `ServiceBackendsResolve` relist-on-`Lagged`, `mtls_resolve_adapter.rs`)

#### Spec

```
PROPERTY: for every store state S (a set of running-and-healthy rows),
GIVEN a NameIndex that has fallen behind (its drain receives
       SubscriptionEvent::Lagged { missed })
WHEN the index processes the Lagged event
THEN it re-Lists via all_service_backends_rows()
  AND after the relist, the index reflects S exactly
       (every name in S is resolvable; no name absent from S resolves)
```

**Why this matters**: a `Lagged` event means the subscription dropped
rows; without a relist the index would silently miss a name forever
(answering NXDOMAIN for a live name — a stale-negative bug). This is the
recovery property the `ServiceBackendsResolve` sibling already enforces;
the responder MUST inherit it.

---

### S-DBN-IDX-04 — The answered addr is drawn ONLY from the same `service_backends` rows (single source)

**Tags**: `@property` `@in-memory` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 · **KPI K-DBN-4** (single-source consistency, in-memory half)
**Driving port**: `NameIndex` + `answer_for` over a `SimObservationStore`
**Test surface**: Tier 1 — `dns_name_index.rs`
**Production code guarded**: `name_index.rs` (no second source — the index reads ONLY `service_backends` rows; it holds no separate cache)

#### Spec

```
PROPERTY: for every store state S of running-and-healthy rows,
GIVEN a NameIndex probed against a SimObservationStore holding exactly S
WHEN answer_for(N, A, &index) returns Records(addrs) for any N
THEN every addr in addrs is the Backend.addr of some row in S
       (the answer set ⊆ the service_backends rows — no fabricated addr,
        no cached snapshot that outlived S)
  AND removing all rows for N from S and re-deriving the index makes N
      resolve to NxDomain (no stale retention)
```

**This is the in-memory half of the K-DBN-4 single-source oracle.** The
Tier-3 half (S-DBN-SINGLE-SRC) feeds the answered addr into the real
`MtlsResolve.resolve`; this Tier-1 half proves the index introduces no
second source of backend truth (`mtls_resolve_adapter.rs` provably
untouched — the responder is a sibling reader, DDN-1).

---

### S-DBN-WS — Walking skeleton: a deployed workload resolves its peer by name and the hop is mTLS'd

**Tags**: `@walking_skeleton` `@driving_adapter` `@real-io` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (Slice 01) · **KPI K-DBN-1**
**Driving port**: `getaddrinfo`/`getent` (resolution) → `overdrive deploy` (`POST /v1/jobs`) → `overdrive serve` (`run_server_with_obs_and_driver`)
**Test surface**: Tier 3 — `crates/overdrive-control-plane/tests/integration/dns_responder_walking_skeleton.rs` (NEW; mirrors `canonical_address_inbound_walking_skeleton.rs` boot/deploy/netns shape)
**Production code guarded**: `run_server_with_obs_and_driver` responder-construct + `responder.probe()` + spawn (lib.rs ~1893-1957, DDN-6); `DnsResponder::{new, probe, serve}`; the wildcard `IP_PKTINFO` socket loop; `NameIndex` over the real `LocalObservationStore`; the existing production intercept path (`start_alloc` inbound nft-TPROXY)

#### Spec

```
GIVEN a Lima VM as root (else SKIP — K1) with uname -r recorded
  AND the production composition root booted in-process via
       run_server_with_obs_and_driver (real EbpfDataplane, NO
       dataplane_override, mtls_identity_override = Some(TestPki)) —
       the SAME boot shape as the canonical-address keystone — so the
       mtls_worker.is_some() block is active and the DnsResponder is
       constructed + probed + spawned (DDN-6)
  AND a "server" Service deployed through the in-process deploy submit
       handler (POST /v1/jobs) — a real TCP server bound on
       0.0.0.0:<SERVICE_PORT> inside its production-provisioned netns
  AND the server reached Running-AND-HEALTHY with a service_backends addr
WHEN a "client" workload (deployed through the same handler, OR a dial
     thread entered into a client netns per the keystone's dial_in_netns)
     resolves "server.svc.overdrive.local" via getaddrinfo / getent
     (NOT dig — K2) from inside its netns, then connects to the resolved
     addr:<SERVICE_PORT>
THEN getent resolves "server.svc.overdrive.local" to the server's
     running-and-healthy service_backends addr (within the K2 5s budget)
  AND the subsequent connection is captured by the PRODUCTION-installed
     inbound nft-TPROXY rule, mTLS terminates on leg-C, and the peer wire
     carries TLS 1.3 application_data records (0x17) with ZERO payload
     cleartext on the peer leg (the keystone's transitive round-trip litmus)
```

**Framing (what's NEW vs what's REUSED)**: the responder's UNIQUE
addition under test is the `getent`/`getaddrinfo` resolution STEP — the
new surface that turns a name into the running-and-healthy addr. The
intercept + mTLS leg (the inbound nft-TPROXY capture, the leg-C
handshake, the TLS 1.3 `0x17` round-trip) is REUSED verbatim from the
proven canonical-address keystone and is NOT re-tested here; it appears
in the `Then` because the intercept LANDING is itself the proof the
resolved addr was correct (US-DBN-2 AC — a wrong addr would not be
captured by the workload's own inbound rule). So the mTLS assertion stays
load-bearing: it is the observable that the name answer pointed at the
right backend.

**Litmus (the vertical-slice gate, CLAUDE.md)**: remove the production
responder spawn in `run_server` (DDN-6) and this test goes RED — `getent`
times out (nothing answers on the gateway) and the dial never starts. NO
test binds `:53`, installs a `resolv.conf`, or supplies an addr that
production does not itself bind/install/supply. The dial-by-name addition
over the canonical-address keystone is the `getaddrinfo`/`getent`
resolution STEP before the dial; everything downstream (the intercept,
the mTLS, the round-trip) is the already-proven keystone path.

**Walking-skeleton litmus (Dim 5 user-centricity)**: the title describes
a user goal ("a deployed workload resolves its peer by name and the hop
is mTLS'd"), the `Then` describes a user observation (the name resolves;
the wire is encrypted), and a non-technical stakeholder confirms "yes —
an ordinary workload finds its peer by name and the connection is
secure." It is demo-able.

---

### S-DBN-SINGLE-SRC — Byte-consistency oracle: the answered addr is the addr `MtlsResolve` classifies `Mesh`

**Tags**: `@real-io` `@kpi` `@dbn-US-2`
**US trace**: US-DBN-2 (the `answered_backend_addr` shared artifact) · **KPI K-DBN-4**
**Driving port**: `getaddrinfo`/`getent` (the answered addr) → `MtlsResolve.resolve` (the oracle)
**Test surface**: Tier 3 — `dns_responder_walking_skeleton.rs` (a second `#[tokio::test]` in the same file, sharing the boot fixture)
**Production code guarded**: `DnsResponder` answer path + the existing `MtlsResolve.resolve` classification (`mtls_resolve_adapter.rs:124-135`) — asserting they agree on the SAME flow; the intercept struct is read, never modified

#### Spec

```
GIVEN the S-DBN-WS boot fixture (server deployed, Running-AND-HEALTHY)
WHEN "server.svc.overdrive.local" is resolved via getaddrinfo to addr A
  AND A (as a SocketAddrV4 at SERVICE_PORT) is fed into the production
      MtlsResolve.resolve for the same flow
THEN resolve(A) recognizes A
  AND classifies it Mesh (NOT MeshUnreachable)
  AND A is byte-identical to the addr the responder answered
       (one source, three readers — the name answer and the intercept
        read the same service_backends row's Backend.addr; DDN-1 / D-TME-10)
```

**The K-DBN-4 single-source oracle made executable** (`feature-delta.md`
§ Shared-artifact registry, `answered_backend_addr` row). An unhealthy
backend would classify `MeshUnreachable`, so the only way `resolve`
returns `Mesh` for the answered addr is if the responder's healthy gate
(DDN-2) kept them in lockstep. This is the cross-reader equivalence test
the `development.md` § "Trait definitions specify behavior" discipline
asks for — except here the two readers are the responder and the
intercept over a shared row, not two adapters of one trait.

---

### S-DBN-PINGPONG — Bidirectional ping-pong demo: two services dial each other by name, counters advance

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
     (getaddrinfo) and calls B — B's counter increments and its date refreshes
  AND within ~15s, B resolves "a.svc.overdrive.local" and calls A —
     A's counter increments and its date refreshes
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
THEN getent reports the name does not resolve (NXDOMAIN — name-not-found)
  AND NO address is returned (no stale, cached, unhealthy, or guessed addr)
  AND once the server reaches Running-AND-HEALTHY, a re-query resolves
       (the negative answer's 1s SOA TTL lets the retry land promptly — DDN-8)
```

**Example-based sad path** (Mandate 11): one named example per failure
mode (the failure mode here = "queried before running-and-healthy"). No
PBT at Tier 3.

---

### S-DBN-NXDOMAIN-02 — After all backends stop, the name stops resolving (no stale addr)

**Tags**: `@real-io` `@error_path` `@kpi` `@dbn-US-4`
**US trace**: US-DBN-4 · **KPI K-DBN-2**
**Driving port**: `overdrive job stop` (`POST /v1/jobs/{id}/stop`) → `getaddrinfo`/`getent`
**Test surface**: Tier 3 — `dns_responder_nxdomain.rs`
**Production code guarded**: `DnsResponder` serve loop reacting to the index dropping the stopped backend (the watch path → `NxDomain`); the production stop path (`run_server_stop` precedent, keystone:635)

#### Spec

```
GIVEN a "server" Service deployed and Running-AND-HEALTHY, resolving
       "server.svc.overdrive.local" to addr A (the keystone's deploy +
       wait-Running shape, then a getent confirming A)
WHEN the server is stopped through the production stop path
     (POST /v1/jobs/server/stop) and converges to Terminated
     (poll the obs row to Terminated, per the keystone's server_stopped)
  AND a deployed client re-queries "server.svc.overdrive.local" via getent
THEN getent reports the name does not resolve (NXDOMAIN)
  AND it NEVER returns the stale addr A (the index dropped the stopped
       backend; no second source of liveness truth)
```

**Chained narrative** (Pillar 2): the `Given` reuses S-DBN-WS's
`Given + When` (a name resolved to A) — then the lifecycle continues into
the stop. The two Tier-3 scenarios read as one name's life.

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
| `getaddrinfo`/`getent` (glibc stub resolver — the CONSUMING adapter) | S-DBN-WS, S-DBN-NXDOMAIN-01/02/03, S-DBN-BIND-01/02 | The acceptance signal (K2). NEVER `dig` alone. |
| `ObservationStore` reader (`all_service_backends_rows` List + `subscribe_all_events` Watch) | S-DBN-WS (real `LocalObservationStore`), S-DBN-IDX-01/02/03/04 (SimObservationStore, Tier 1) | The third sibling reader over the SAME rows (DDN-1). Tier-3 exercises real redb reads; Tier-1 the watch/relist logic. |
| `Clock` (SOA SERIAL source) | S-DBN-WIRE-04 (Tier 1, injected reading) | Injected `Arc<dyn Clock>`; never wall-clock. |
| `NetSlotAllocator` (`snapshot()` + `responder_addr_for_slot`, fallback source) | S-DBN-BIND-02 | Concrete host state (not a port trait); read ONLY on the per-addr fallback path. |
| `MtlsResolve.resolve` (the oracle the answered addr is fed into) | S-DBN-SINGLE-SRC | Read, never modified (the intercept struct is provably untouched, DDN-1). |
| `EbpfDataplane` + inbound nft-TPROXY intercept (the downstream hop) | S-DBN-WS, S-DBN-PINGPONG | Reused from the canonical-address keystone; the dial-by-name addition is the resolution step before it. |
| `overdrive deploy` submit handler (`POST /v1/jobs`) | S-DBN-WS, S-DBN-PINGPONG, S-DBN-NXDOMAIN-* | The driving adapter; in-process per the keystone precedent. |

**Empty rows**: none. Every driven adapter the responder adds or consumes
has a `@real-io` scenario. **CM-A** adapter integration coverage: PASS.
The pure DNS codec (`wire.rs`) and pure `answer_for` are NOT adapters —
they are the Tier-1 seams (proptested at S-DBN-WIRE-* / S-DBN-ANSWER-*),
which is the correct treatment per DDN-4 (no port trait, no Sim adapter).

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
| `answer_for` `Records` arm (the running-and-healthy set → A) | S-DBN-ANSWER-01, S-DBN-IDX-04 | YES — a mutant returning empty / an out-of-index addr / matching the wrong name flips set-equality (THE primary mutation target, DDN-4) |
| `answer_for` `NxDomain` arm (empty set) | S-DBN-ANSWER-02/05, S-DBN-NXDOMAIN-* | YES — a mutant returning `Records` on an empty set flips the fail-honest property |
| `answer_for` `NoData` arm (`AAAA` on a live name) | S-DBN-ANSWER-03 | YES — a mutant returning `NxDomain` for AAAA-on-live, or a fabricated v6 addr, flips the contract |
| `NameIndex` `Backend.healthy == true` gate (DDN-2) | S-DBN-ANSWER-04, S-DBN-IDX-02, S-DBN-SINGLE-SRC | YES — a mutant dropping the healthy filter answers a `MeshUnreachable` addr, flipping S-DBN-SINGLE-SRC's `Mesh` assertion |
| `NameIndex` List-seed at probe | S-DBN-IDX-01, S-DBN-WS | YES — a mutant skipping the List leaves names unresolvable |
| `NameIndex` Watch drain + `<job>`-grouping | S-DBN-IDX-01/02 | YES — a mutant grouping by the wrong segment (OQ-1 accessor) makes names resolve to the wrong addr or not at all |
| `NameIndex` relist-on-`Lagged` | S-DBN-IDX-03 | YES — a mutant ignoring `Lagged` leaves the index stale (a live name answers NXDOMAIN) |
| `wire.rs` `Records` encode (A records) | S-DBN-WIRE-01, S-DBN-WS | YES — a mutant emitting the wrong RDATA / count fails the hickory round-trip |
| `wire.rs` `NoData` encode (NOERROR + SOA, MINIMUM=1) | S-DBN-WIRE-02 | YES — a mutant emitting NXDOMAIN or omitting the SOA / wrong MINIMUM flips the header/SOA assertion |
| `wire.rs` `NxDomain` encode (NXDOMAIN + SOA, MINIMUM=1) | S-DBN-WIRE-03 | YES — a mutant emitting NOERROR or wrong MINIMUM flips it |
| `wire.rs` SOA SERIAL derivation from `Clock` | S-DBN-WIRE-04 | YES — a mutant reading wall-clock / a constant flips the deterministic-per-`Clock` property |
| `responder.rs` wildcard bind (`SO_REUSEADDR` + `IP_PKTINFO`) | S-DBN-BIND-01, S-DBN-WS | YES (Tier 3) — without `IP_PKTINFO`, `getent` rejects the reply and the test goes RED |
| `responder.rs` per-addr fallback + re-derive-on-converge re-bind | S-DBN-BIND-02 | YES (Tier 3) — a mutant binding the wrong derived gateway / not tracking slot changes flips per-netns resolution |
| `responder.rs` `ipi_spec_dst` source-pin on the send path | S-DBN-BIND-01, S-DBN-WS | YES (Tier 3) — the `getent`-not-`dig` litmus; a missing pin RED on `getent` |
| `run_server` responder construct + probe + spawn (DDN-6) | S-DBN-WS, S-DBN-BIND-03 | YES — deleting the spawn RED's S-DBN-WS (`getent` times out — the litmus); a probe that doesn't refuse-boot flips S-DBN-BIND-03 |
| `DnsResponderError` variant → `health.startup.refused` reason mapping | S-DBN-BIND-03 | YES — a mutant flattening to one reason (or `Internal(String)`) flips the cause-distinct refusal assertion |

**Empty rows**: zero. Every production code path the responder adds has
at least one scenario (Tier 1 for the pure seams, Tier 3 for the socket
+ boot). **Mutation invocation** (per `.claude/rules/testing.md`):
`cargo xtask lima run -- cargo xtask mutants --diff origin/main --features
integration-tests --package overdrive-control-plane --file <files-touched>`
for the `dns_responder/*` files; `--package overdrive-core --file
crates/overdrive-core/src/id.rs` for `MeshServiceName`/`NameAnswer`.
Per-step runs scope to the step's files; the pre-PR run covers the full
per-package diff. The `answer_for` arms are THE primary kill targets
(DDN-4).

---

## What these scenarios do NOT cover (explicit deferrals)

- **OQ-1 — the `SpiffeId` → `<job>` accessor signature.** CONFIRMED-ABSENT
  this DISTILL pass: `SpiffeId` (`crates/overdrive-core/src/id.rs:267-282`)
  exposes only `as_str()`, `trust_domain()`, `path()` (and the
  `for_allocation` constructor) — there is NO job-segment accessor. The
  mapping is verified (DDN-2: the `WorkloadId` segment of the SVID path =
  `<job>`), but the exact accessor — a new
  `SpiffeId::job_segment() -> Option<&str>` on the newtype vs a parse
  helper local to `name_index.rs` — is **the one open surface decision the
  crafter MUST pin in DELIVER**, per CLAUDE.md "Implement to the design —
  never invent API surface." **DISTILL does NOT pick a signature.**
  S-DBN-IDX-01 asserts the *behaviour* (rows grouped by `<job>`); DELIVER
  pins the accessor that achieves it and surfaces it as a decision, not an
  improvisation.
- **`NameAnswer` module placement** (`id.rs` vs a small `dns` module in
  `overdrive-core`) — this is the DESIGN's own intentional latitude, NOT a
  DISTILL gap: ADR-0072 § Components writes the location as "overdrive-core
  (id or a small dns module)". DELIVER picks one and documents the choice;
  it does NOT invent surface — only the module placement of the
  already-pinned `NameAnswer` type/variants is open.
- **VIP path** (`<job>.svc.overdrive.local → fdc2::/16` VIP + XDP
  `SERVICE_MAP`) — **#61** (depends on #167). Headless v1 (D-TME-10)
  avoids it.
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
| 2 | Error-path coverage ≥ 40% | PASS (14/26 = 54%) |
| 3 | Business-language purity in scenario titles | PASS — titles use domain terms (`mesh service name`, `resolve by name`, `running-and-healthy`, `stale addr`, `NXDOMAIN`); DNS/socket terms appear only in step bodies + production-code lines, not titles |
| 4 | Walking-skeleton user-centric framing (Dim 5) | PASS — S-DBN-WS title = a user goal ("a deployed workload resolves its peer by name and the hop is mTLS'd"); `Then` = user observations (name resolves, wire encrypted) |
| 5 | Every Then asserts observable behaviour (Dim 7) | PASS — Tier 1 asserts on `answer_for` return values / decoded wire bytes / `answer_for` results (never the `by_name` field directly); Tier 3 asserts on `getent` output, the round-trip, `ss -ulnp`, boot exit/refusal — all port-exposed |
| 6 | Story-to-scenario traceability (Dim 8 Check A) | PASS — US-DBN-2/3/4 each have ≥1 scenario via `@dbn-US-N`; US-DBN-1 (spike) already PROMOTED, not re-tested |
| 7 | WS strategy declared | PASS — Architecture-of-Reference defaults + the project policy (`docs/architecture/atdd-infrastructure-policy.md`, rows appended for dial-by-name in the feature-delta DISTILL block); driving = real `serve`+`deploy`, driven-internal `ObservationStore` = real, the socket = irreducibly Tier-3 real (NO Sim, DDN-4) |
| 8 | WS uses real adapters (not InMemory) | PASS — S-DBN-WS / S-DBN-PINGPONG exercise real `run_server` + real `EbpfDataplane` + real kernel UDP + real `getent` |
| 9 | Every driven adapter has a `@real-io` scenario (Dim 9c) | PASS — see adapter coverage table |
| 10 | Scenarios named for user value, not technical operations | PASS — titles describe outcomes (`resolves its peer by name`, `counters advance`, `stops resolving`, `NXDOMAIN never a stale addr`) |
| 11 | Driving port named explicitly per scenario | PASS — every scenario has a "Driving port" line |
| 12 | pytest-bdd `.feature` files exist | N/A (Rust project — no `.feature` files per `.claude/rules/testing.md`; this doc is the GIVEN/WHEN/THEN SSOT) |
| 13 | pytest fixtures isolated per environment | N/A (Rust project) |
| 14 | `@property` scenarios at the right layer (Mandate 9) | PASS — every `@property` is Tier 1 (S-DBN-NAME-*, S-DBN-ANSWER-*, S-DBN-WIRE-*, S-DBN-IDX-*); Tier-3 sad paths (S-DBN-NXDOMAIN-*, S-DBN-BIND-02) are example-based (Mandate 11) |
| 15 | conftest.py shared fixtures isolated | N/A (Rust project — Tier-3 fixtures isolate via `TempDir`, root-gate SKIP, RAII netns teardown per the keystone) |

**Adapted-for-Rust items**: 12, 13, 15 are pytest-specific and N/A. The
Rust equivalents (`integration-tests` feature gating, per-test
`tempfile::TempDir`, RAII netns/socket teardown, the leftover-netns
cleanup discipline of `.claude/rules/debugging.md`) are governed by
`.claude/rules/testing.md` and are DELIVER's responsibility.

---

## DISTILL Review (nw-acceptance-designer-reviewer)

**Reviewer:** Sentinel (nw-acceptance-designer-reviewer) · **Date:** 2026-06-25
**Verdict:** **APPROVED** — 0 blocking · 3 suggestions (non-blocking) · 4 nitpicks · 3 praise

Reviewed `test-scenarios.md` (26 scenarios) + `red-classification.md` against
the ADR-0072 / feature-delta DESIGN contract, the eight critique dimensions,
the three design mandates, and `.claude/rules/testing.md` DISTILL discipline.
Style: Radical Candor + Conventional Comments.

### Scores (0-10)

| Dimension | Score | Note |
|---|---|---|
| Happy-path bias / error depth | 9 | 14/26 = 54% error-path, all genuine negative modes (NXDOMAIN, unhealthy-only, Lagged-recovery, refuse-boot, EADDRINUSE-fallback). No relabeled happy paths. |
| GWT format compliance | 9 | Every scenario carries GIVEN/WHEN/THEN or PROPERTY; single behaviour each; chained narrative (Pillar 2) explicit on IDX-01→02, WS→NXDOMAIN-02. |
| Business-language purity (titles) | 8 | Titles use domain vocabulary (`resolve by name`, `running-and-healthy`, `stale addr`, `NXDOMAIN`); DNS/socket terms confined to step bodies + production-code lines. |
| Coverage completeness | 10 | Every cell of the v1-answer-contract table pinned (A-hit ANSWER-01, A-empty→NXDOMAIN ANSWER-02, AAAA-live→NODATA ANSWER-03, AAAA-empty→NXDOMAIN ANSWER-03). US-DBN-2/3/4 each ≥1 scenario; US-DBN-1 spike PROMOTED, not re-tested. |
| Walking-skeleton user-centricity (Dim 5) | 9 | S-DBN-WS / S-DBN-PINGPONG titles = user goals; THENs = user observations (name resolves, counters advance, wire encrypted); stakeholder-confirmable. |
| Priority validation | 9 | Tests target the right problem — `answer_for` (DDN-4 mutation gate) + the source-pin + the healthy gate are the load-bearing seams; the fail-honest NXDOMAIN posture is the US-DBN-4 leg. |
| Observable-behaviour assertions (Dim 7) | 10 | Every THEN asserts a port-exposed observable — `answer_for` return, hickory-decoded wire bytes, `getent` output, `ss -ulnp`, boot exit/refusal, `resolve` classification. ZERO assertions on the `by_name: BTreeMap` internal (explicitly disclaimed at ANSWER-04, IDX-01). |
| Traceability coverage (Dim 8) | 10 | Check A: US-DBN-2/3/4 each tagged `@dbn-US-N` with ≥1 scenario. Inherited-commitments table maps DDN-1..8 + D-TME-9/10/11 to scenarios without inversion. |
| WS boundary proof (Dim 9) | 9 | Real `run_server` + real `EbpfDataplane` + real kernel UDP + real `getent`; the irreducibly-Tier-3 socket correctly has NO Sim (DDN-4); adapter-coverage table has zero empty rows. |

### Mandate compliance

- **CM-A (hexagonal boundary): PASS.** Driving ports are `getaddrinfo`/`getent`,
  `overdrive deploy` (`POST /v1/jobs`), `overdrive serve` (`run_server`), and the
  pure `answer_for` seam. No scenario tests an internal component directly; the
  `answer_for`/`wire.rs` Tier-1 seams are correctly the pure-function ports (no
  port trait, DDN-4), not a boundary violation.
- **CM-B (business language): PASS.** Step bodies delegate to the named driving
  ports; assertions check business outcomes (resolves, NXDOMAIN, Mesh
  classification, encrypted wire), never raw socket internals.
- **CM-C (user journey): PASS.** S-DBN-WS and S-DBN-PINGPONG are complete user
  journeys with business value (an unmodified workload reaches its peer by name
  and the hop is secure); the Tier-1 seams are focused boundary scenarios.

### Implement-to-design verification (CLAUDE.md "Implement to the design")

- Every type/fn/variant a scenario names matches the DESIGN-pinned surface
  exactly: `MeshServiceName::{new, as_str, SUFFIX}`, `NameAnswer::{Records,
  NoData, NxDomain}`, `answer_for(name, qtype: hickory_proto::rr::RecordType,
  &index)`, `DnsResponder::{new, probe, serve}`, `DnsResponderError::{Bind,
  ListSeed, Probe, Socket}` (NO `Internal(String)` — verified at BIND-03 +
  the mutation table). No invented surface.
- **OQ-1 genuinely left open (CONFIRMED).** `SpiffeId` exposes only
  `as_str`/`trust_domain`/`path` (+`for_allocation`) — no job-segment accessor.
  S-DBN-IDX-01 asserts only the `<job>`-grouping *behaviour* ("rows grouped by
  `<job>`"), explicitly disclaiming the accessor signature as a DELIVER decision.
  DISTILL picks no signature. Correct.
- **`NameAnswer` module placement** correctly identified as DESIGN's own latitude
  (`id.rs` vs a small `dns` module), not a DISTILL gap.

### Spot-checks performed (all confirmed)

- `getent`-not-`dig` litmus: every `dig` mention in `test-scenarios.md` is a
  prohibition ("NOT dig", "never dig alone"); zero `dig`-only assertions across
  S-DBN-WS, S-DBN-NXDOMAIN-*, S-DBN-BIND-*. Grounded in `spike/findings.md` edge
  case 2 + the source-pinning proof.
- Vertical-slice litmus (S-DBN-WS): the "delete the `run_server` spawn → `getent`
  times out" guard is genuine; no test binds `:53`, installs a `resolv.conf`, or
  supplies an addr production doesn't. This is the deferred slice from the
  transparent-mtls arc whose precedent failure was a hand-installed production
  call site — this spec does NOT repeat it.
- Tier-3 boot shape mirrors the keystone exactly
  (`canonical_address_inbound_walking_skeleton.rs`): `run_server_with_obs_and_driver`,
  `is_root()` SKIP, `run_server_deploy`, `dial_in_netns`.
- DDN-2 healthy-gate grounding: `mtls_resolve_adapter.rs` confirms healthy →
  `Mesh`, `Backend.healthy == false` → `MeshUnreachable`. S-DBN-SINGLE-SRC's
  oracle is sound.
- Mutation traceability: every production path has ≥1 killing scenario; the
  claimed mutation-killable signals are real. `answer_for` is correctly THE
  primary kill target.
- `@property` tier discipline: every `@property` is Tier 1; Tier-3 sad paths are
  example-based (Mandate 11). No `@property` leaks into Tier 3.
- EDD honesty: S-DBN-PINGPONG graduates to `E05`, honest `pending` mirroring E04
  until #227/#75 lands; no fabricated capture.

### RED-classification soundness (`red-classification.md`)

- Every scenario's expected RED reason is `MISSING_FUNCTIONALITY` (not
  IMPORT_ERROR/FIXTURE_BROKEN/etc.), each row naming the `todo!` that produces it.
- Tier-3 `#[should_panic(expected = "RED scaffold")]` vs `#[ignore]` choice is
  correct per `.claude/rules/testing.md` § "What about `#[ignore]`?".
- "Classification runs in DELIVER, not DISTILL" rationale is sound (the
  `hickory-proto` workspace dep is DELIVER's wiring step).
- The S-DBN-WS / S-DBN-PINGPONG "timeline note" is internally consistent.

### Findings (priority-ordered)

**Suggestions (non-blocking):**

1. `suggestion: (non-blocking)` **S-DBN-WS asserts both resolution + mTLS in one
   THEN.** Defensible (the mTLS landing is the observable that the name pointed at
   the right backend), but in DELIVER assert the `getent` resolution *first and
   separately* so a resolution failure surfaces before the mTLS assertion — keeping
   the K2 two-equally-likely-culprits message honest. The spec ordering already
   implies this; it's a DELIVER implementation note.
2. `suggestion: (non-blocking)` **S-DBN-BIND-02's converge-tick add/drop lifecycle
   is the densest single scenario** (fallback trigger AND reconciler-Bar-1 converge).
   Consider whether DELIVER splits the "fallback fires on EADDRINUSE" assertion from
   the "bound set tracks the live slot set across converge ticks" assertion into two
   `#[test]` bodies for cleaner failure-localisation. Non-blocking — one Tier-3
   example per failure mode (Mandate 11) is satisfied either way.
3. `suggestion: (non-blocking)` **S-DBN-WIRE-04 (SOA SERIAL from `Clock`)** asserts
   "distinct T past the SERIAL granularity → distinct SERIALs"; the granularity is
   left to DELIVER. Recommend DELIVER pin the granularity in the `@example` so the
   property is not vacuously satisfiable by a coarse granularity mapping all test T
   to one SERIAL.

**Nitpicks (non-blocking):**

4. `nitpick (non-blocking):` Both DISTILL artifacts carried a stray tool-artifact
   tail (`</content>` + `</invoke>` at this file's former lines 1182-1183;
   `</content>` at `red-classification.md`). **Stripped during this review.**
5. `nitpick (non-blocking):` Self-review checklist item 15's Rust-equivalent note
   is a DELIVER responsibility — fine to keep, flagged as an assertion about
   DELIVER, not a DISTILL-verified fact.
6. `nitpick (non-blocking):` The "negative-testing note (Hebert ch.6)" and similar
   literary citations are good colour but unverifiable in-band; harmless framing.
7. `nitpick (non-blocking):` K3 leaves the client-program form open (staged Rust
   bin OR `python3 -c`); both satisfy the AC, but pinning one in DELIVER avoids a
   phantom-path class (already flagged via `ping_pong_command_path`).

**Praise:**

8. `praise:` The `getent`-not-`dig` litmus is enforced *structurally* — propagated
   from the spike finding into K2, every name-path Tier-3 THEN, the failure-message
   text, and the mutation table — making a `dig`-only assertion unrepresentable in
   the suite, not merely discouraged.
9. `praise:` The single-source oracle (S-DBN-SINGLE-SRC, K-DBN-4) is a genuinely
   strong cross-reader equivalence test: feeding the answered addr into the
   production `MtlsResolve.resolve` and asserting `Mesh` (not `MeshUnreachable`)
   ties the responder's healthy gate to the intercept's classification through a
   shared row — exactly the byte-consistency invariant D-TME-10 needs, observable
   at the port, never via a shared struct.
10. `praise:` Honest treatment of the irreducibly-Tier-3 socket: the doc refuses to
    invent a `SimNameResponder` (DDN-4), correctly identifying a Sim would "simulate
    exactly the part the spike proved lies," and routes the DST seam through the pure
    `answer_for` + proptested `wire.rs`. Consistently applied across the WS-strategy,
    adapter-coverage, and policy-rows sections.

**Handoff:** Approved for DELIVER's RED phase. The one open surface (OQ-1, the
`SpiffeId` → `<job>` accessor) is correctly named and left for the crafter to
pin per CLAUDE.md, not improvised.
