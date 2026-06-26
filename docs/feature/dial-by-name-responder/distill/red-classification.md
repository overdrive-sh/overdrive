# RED-classification PLAN — `dial-by-name-responder`

**Wave**: DISTILL — **RE-DISTILL REV-2 (stable-frontend)** (the PLAN) → DELIVER's RED phase (the actual run).
**Designer**: Quinn | **Date**: 2026-06-25

> **REV-2 re-distill notice.** Revised for the ratified ADR-0072 REV-2
> stable-frontend contract (commit `8e22f499`). The scenario set grew from 26
> to 39 (13 NEW: `FrontendAddrAllocator` (4), the re-keyed `MtlsResolve` +
> fail-closed + ordered-drain + DST-equivalence (7), the stable-across-cycle +
> Tier-3-churn ACs (2)); 13 IDX/ANSWER/WS/SINGLE-SRC/NXDOMAIN scenarios were
> re-distilled (answer the stable `F`, withhold-not-release); 13 PRESERVED
> (NAME/WIRE/ANSWER-05/NXDOMAIN-03/BIND). The expected RED reason for EVERY
> still-RED-scaffolded scenario remains `MISSING_FUNCTIONALITY`.

Per ADR-025 D2, the pre-DELIVER **fail-for-the-right-reason gate**
becomes DELIVER's RED-phase entry/exit gate. DISTILL authors the
scenarios (`test-scenarios.md`) and this PLAN; **DISTILL does NOT run the
classification** — the scaffolds + the `hickory-proto` workspace dep do
not exist yet (see § "Why the classification runs in DELIVER, not
DISTILL"). DELIVER's RED phase materialises the scaffolds (per the
Scaffold MANIFEST in `feature-delta.md` § "Wave: DISTILL / [REF] Scaffold
MANIFEST"), adds the dep, and runs the gate.

## Expected RED failure mode for EVERY scaffolded test: `MISSING_FUNCTIONALITY`

Every scenario, once scaffolded, MUST fail with `MISSING_FUNCTIONALITY` —
the production behaviour is unimplemented — NOT `IMPORT_ERROR` /
`FIXTURE_BROKEN` / `SETUP_FAILURE` / `WRONG_ASSERTION`.

In Rust terms (per `.claude/rules/testing.md` § "RED scaffolds"), the
correct RED shape is:

- **Test-side**: `#[should_panic(expected = "RED scaffold")]` on the
  `#[test]`/`#[tokio::test]` body, with a `panic!("Not yet implemented --
  RED scaffold (<scenario-id> / <one-line spec>)")` body. The Red Gate
  Snapshot classifies a `panic!`/`AssertionError`-shaped failure as RED.
- **Production-side**: the `dns_responder/*` module fns and the
  `MeshServiceName`/`NameAnswer`/`answer_for` surfaces carry
  `todo!("RED scaffold: <one-line spec>")` bodies, gated with
  `#[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice 01")]`.

The gate FAILS (and DELIVER must fix the test, not the production code)
if any scenario fails as:

- `IMPORT_ERROR` / unresolved type (e.g. `MeshServiceName` not yet
  declared) — that is a missing-scaffold bug, not genuine RED. The
  scaffold MANIFEST exists to prevent exactly this.
- `FIXTURE_BROKEN` / `SETUP_FAILURE` (e.g. the Lima fixture refuses to
  boot, the netns setup errors, the `hickory-proto` dep is absent so the
  test binary does not compile) — that is infrastructure, not
  missing-functionality.
- `WRONG_ASSERTION` / `OBSERVABLE_NOT_AT_PORT` (e.g. a scenario asserting
  on the `by_name: BTreeMap` field directly instead of through
  `answer_for`) — that is a Universe-shape bug; the scenarios are written
  to assert only through port-exposed surfaces, so this should not arise.

## One-line expected classification per scenario

| Scenario | Tier | REV-2 | Expected RED reason | Scaffold that produces it |
|---|---|---|---|---|
| S-DBN-NAME-01 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `MeshServiceName::new`/`Display`/`FromStr`/serde — COMMITTED (`b39fe4d2`); the test is GREEN already, not RED-scaffolded |
| S-DBN-NAME-02 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `MeshServiceName::new` case-fold — COMMITTED; GREEN |
| S-DBN-NAME-03 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `MeshServiceName::new` suffix grammar — COMMITTED; GREEN |
| S-DBN-NAME-04 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `MeshServiceName::new` label-limit validation — COMMITTED (`4f030771`/`15d86342`); GREEN |
| S-DBN-FRONTEND-01 | 1 | NEW (01-04) | `MISSING_FUNCTIONALITY` | `FrontendAddrAllocator::assign` + `WORKLOAD_FRONTEND_BASE` const `todo!` → membership/disjointness panics |
| S-DBN-FRONTEND-02 | 1 | NEW (01-04) | `MISSING_FUNCTIONALITY` | `FrontendAddrAllocator::assign` idempotency `todo!` |
| S-DBN-FRONTEND-03 | 1 | NEW (01-04) | `MISSING_FUNCTIONALITY` | `FrontendAddrAllocator::release` (deletion-only, no health input) `todo!` |
| S-DBN-FRONTEND-04 | 1 | NEW (01-04) | `MISSING_FUNCTIONALITY` | `FrontendAddrAllocator` smallest-free scan + reclaim `todo!` |
| S-DBN-ANSWER-01 | 1 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `answer_for` `Records` arm `todo!` (now `vec![F]`, the stable frontend addr) |
| S-DBN-ANSWER-02 | 1 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `answer_for` `NxDomain` (withheld) arm `todo!` |
| S-DBN-ANSWER-03 | 1 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `answer_for` `NoData`/qtype-dispatch arm `todo!` |
| S-DBN-ANSWER-04 | 1 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `NameIndex` healthy-gate-as-withhold + `answer_for` `todo!` |
| S-DBN-ANSWER-05 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `answer_for` lookup-miss arm `todo!` |
| S-DBN-WIRE-01 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `wire::encode` `Records` path — COMMITTED (`04fa3d18`); GREEN (addr-agnostic) |
| S-DBN-WIRE-02 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `wire::encode` `NoData`+SOA path — COMMITTED; GREEN |
| S-DBN-WIRE-03 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `wire::encode` `NxDomain`+SOA path — COMMITTED; GREEN |
| S-DBN-WIRE-04 | 1 | PRESERVED | `MISSING_FUNCTIONALITY` | `wire::encode` SOA-SERIAL-from-`Clock` — COMMITTED; GREEN |
| S-DBN-IDX-01 | 1 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `NameIndex` List-seed + watch + `<job>`→stable-`F` mapping `todo!` |
| S-DBN-IDX-02 | 1 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `NameIndex` healthy-gate-as-withhold on watch + allocator-retains-`F` `todo!` |
| S-DBN-IDX-03 | 1 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `NameIndex` relist-on-`Lagged` `todo!` (addr-agnostic relist) |
| S-DBN-IDX-04 | 1 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `NameIndex` single-source-of-`F` read `todo!` |
| S-DBN-REKEY-01 | 1 | NEW (02-00) | `MISSING_FUNCTIONALITY` | `BackendIndex.by_frontend` + `classify` hit arm (first-by-`Ord` healthy → `Mesh`) `todo!` |
| S-DBN-REKEY-02 | 1 | NEW (02-00) | `MISSING_FUNCTIONALITY` | `classify` `by_frontend`-hit-but-zero-healthy → `MeshUnreachable` `todo!` |
| S-DBN-REKEY-03 | 1 | NEW (02-00) | `MISSING_FUNCTIONALITY` | `FrontendKey = (SocketAddrV4, Proto)` proto-discrimination `todo!` |
| S-DBN-REKEY-04 | 1 | NEW (02-00) | `MISSING_FUNCTIONALITY` | `classify` `by_addr` fall-through preserved (additive) `todo!` |
| S-DBN-FAILCLOSED-01 | 1 | NEW (02-00) | `MISSING_FUNCTIONALITY` | `classify` fail-closed-on-frontend-subnet-miss arm (`∈ 10.98.0.0/16` → `MeshUnreachable`) `todo!` |
| S-DBN-COHERENCE-01 | 1 | NEW (02-00) | `MISSING_FUNCTIONALITY` | byte-identity-of-`F`-via-the-ONE-`FrontendAddrAllocator` invariant (Property 1) + fail-closed-regardless-of-inter-drain-timing (Property 2) `todo!` — RECONCILED to two-drains-one-allocator (no single shared drain; the temporal ordering barrier is superseded, FAILCLOSED-01 holds the security half) |
| S-DBN-EQUIV-01 | 1 | NEW (02-00) | `MISSING_FUNCTIONALITY` | re-keyed `classify` trajectory vs an INDEPENDENT reference oracle + determinism `todo!` — RECONCILED: `BackendIndex` is a single struct (no sim/host pair), so two-implementation equivalence is vacuous; the oracle is the genuine enforcement |
| S-DBN-WS | 3 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `DnsResponder::{new,probe,serve}` + `FrontendAddrAllocator` + re-keyed `MtlsResolve` `todo!` + no `run_server` spawn → `getent` times out (the litmus). Test-side `#[should_panic(expected = "RED scaffold")]` until the boot wiring lands. **Timeline note**: on the RED-phase Lima-root run the test panics on the missing `DnsResponder::serve` / `classify` re-key implementation (the `todo!("RED scaffold")`) — that IS the expected RED classification. The "delete the production spawn / `by_frontend` arm → `getent` times out / fail-close" litmus is a POST-GREEN regression guard, NOT part of the RED classification. |
| S-DBN-WS-STABLE | 3 | NEW (02-02) | `MISSING_FUNCTIONALITY` | `FrontendAddrAllocator::assign` idempotency across an alloc cycle + re-keyed re-resolve `todo!` → `getent` not stable / no live backend |
| S-DBN-CHURN | 3 | NEW (02-02) | `MISSING_FUNCTIONALITY` | the pump-task + `TCP_USER_TIMEOUT` churn surface `todo!` → no prompt reset (the in-flight dial does not fail fast). Distinct from `sock_destroy` (NOT used; #61 scope). |
| S-DBN-SINGLE-SRC | 3 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `DnsResponder` answer path + re-keyed `resolve` `todo!` → no answered `F` to feed `resolve`, no translation |
| S-DBN-PINGPONG | 3 | RE-DISTILL | `MISSING_FUNCTIONALITY` | responder serve loop + re-key `todo!` + `examples/dial-by-name-responder/{a,b}.toml` absent → no resolution. **Note**: EDD evidence stays honest `pending` (#227/#75); the `#[test]` itself is RED-scaffolded. **Timeline note**: same as S-DBN-WS — the RED-phase panic is on the missing `serve`/`classify` impl; the "delete spawn → times out" litmus is a POST-GREEN guard. |
| S-DBN-NXDOMAIN-01 | 3 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `answer_for` `NxDomain` arm + serve loop `todo!` |
| S-DBN-NXDOMAIN-02 | 3 | RE-DISTILL | `MISSING_FUNCTIONALITY` | `NameIndex` withhold-on-zero-healthy + allocator-retains-`F` + serve loop `todo!` |
| S-DBN-NXDOMAIN-03 | 3 | PRESERVED | `MISSING_FUNCTIONALITY` | `answer_for` miss + serve loop `todo!` |
| S-DBN-BIND-01 | 3 | PRESERVED | `MISSING_FUNCTIONALITY` | `responder.rs` wildcard bind + `IP_PKTINFO` `todo!` |
| S-DBN-BIND-02 | 3 | PRESERVED | `MISSING_FUNCTIONALITY` | `responder.rs` per-addr fallback `todo!` |
| S-DBN-BIND-03 | 3 | PRESERVED | `MISSING_FUNCTIONALITY` | `run_server` responder-probe refuse-boot wiring + `DnsResponderError` `todo!` |

> **PRESERVED-and-COMMITTED note** (01-01/01-02): S-DBN-NAME-01..04 and
> S-DBN-WIRE-01..04 guard production code that is ALREADY COMMITTED
> (`MeshServiceName` `b39fe4d2`/`4f030771`/`15d86342`; `NameAnswer` + the
> `hickory-proto` wire codec `04fa3d18`). Their tests are GREEN today, not
> RED-scaffolded — the `MISSING_FUNCTIONALITY` column is the *historical*
> expected-RED reason from the original DISTILL pass (retained for the
> traceability record). DELIVER does NOT re-scaffold them; it confirms they
> stay GREEN as the addr-agnostic substrate REV-2 reuses. The genuinely-RED
> scaffolds this RE-DISTILL adds are the FRONTEND/REKEY/FAILCLOSED/COHERENCE/
> EQUIV (Tier 1) + WS-STABLE/CHURN (Tier 3) NEW scenarios, plus the
> re-distilled IDX/ANSWER/WS/SINGLE-SRC/NXDOMAIN bodies against the NEW
> `name_index`→`F` / re-keyed-`classify` production code.

## Tier-3 caveat — `#[ignore]` vs `#[should_panic]`

The Tier-3 scenarios (S-DBN-WS, S-DBN-WS-STABLE, S-DBN-CHURN,
S-DBN-SINGLE-SRC, S-DBN-PINGPONG, S-DBN-NXDOMAIN-*, S-DBN-BIND-*) require
root + a real kernel under Lima.
On a non-root / non-Lima run they SKIP cleanly (the keystone's `is_root()`
gate), which is NOT a RED signal — it is "the test cannot run here." Per
`.claude/rules/testing.md` § "What about `#[ignore]`?", `#[ignore]` is for
tests blocked on an EXTERNAL resource the implementation cannot
synthesize. For dial-by-name the blocker during RED is "the production
code does not exist yet," so the Tier-3 bodies use
`#[should_panic(expected = "RED scaffold")]` (the production code is
missing), NOT `#[ignore]` — they go RED on a real Lima root run and GREEN
once Slice 01 lands. The root-gate SKIP is orthogonal (it gates
execution, not RED-ness): a non-root run neither passes nor fails the
gate; the DELIVER classification run MUST be a Lima-root run.

## Why the classification runs in DELIVER, not DISTILL

The Tier-1 `answer_for`/`name_index`/`frontend_addr_allocator` and the
EXTENDED `mtls_resolve_adapter` (`by_frontend`/`classify` re-key) scaffolds
NAME types (`NameIndex`, `FrontendAddrAllocator`, `FrontendKey`,
`by_frontend`) that do NOT yet exist in `crates/` (confirmed absent this
pass), and the socket loop NAMES `nix` `recvmsg`/`sendmsg` types. A
compilable RED scaffold therefore REQUIRES the `nix` `socket`/`uio`
features (the `hickory-proto` workspace dep is now COMMITTED, `04fa3d18` —
the 01-02 wire codec landed it, so it is no longer a DELIVER-add). **DISTILL
does NOT land any `crates/` file** — materialising the half-built
`dns_responder/*` modules + the `mtls_resolve_adapter` re-key mid-DISTILL
would perturb the workspace build and is out of scope (the SCOPE DECISION).
DELIVER's RED phase:

1. Adds the `nix` `socket`/`uio` features (`hickory-proto.workspace = true`
   is already present from `04fa3d18`).
2. Materialises the scaffolds per the MANIFEST (`feature-delta.md` §
   Scaffold MANIFEST — REV-2 updated), each with the `todo!("RED scaffold:
   …")` / `#[should_panic(expected = "RED scaffold")]` markers, INCLUDING
   the NEW `dns_responder/frontend_addr_allocator.rs` and the EXTENDED
   `mtls_resolve_adapter.rs` (`by_frontend` map + three-way `classify` arm).
3. Runs the suite (Tier 1 directly; Tier 3 under `cargo xtask lima run --
   cargo nextest run -p overdrive-control-plane --features
   integration-tests` as root).
4. Confirms every failure classifies `MISSING_FUNCTIONALITY` per the
   table above. Any `IMPORT_ERROR` / `FIXTURE_BROKEN` / `SETUP_FAILURE` is
   a scaffold bug to fix BEFORE the GREEN phase begins.
5. Records the actual classification result (this file's table is the
   PLAN; DELIVER appends the OBSERVED column).

The `bpf-build` prereq (`overdrive-dataplane build.rs` hard-fails without
`target/bpf/overdrive_bpf.o`) applies: any RED run touching nextest-affected
on `overdrive-control-plane` needs `cargo xtask bpf-build` first (per the
project memory note). That is a DELIVER inner-loop concern, not a DISTILL
one.
