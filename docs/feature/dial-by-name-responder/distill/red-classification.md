# RED-classification PLAN — `dial-by-name-responder`

**Wave**: DISTILL (the PLAN) → DELIVER's RED phase (the actual run).
**Designer**: Quinn | **Date**: 2026-06-25

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

| Scenario | Tier | Expected RED reason | Scaffold that produces it |
|---|---|---|---|
| S-DBN-NAME-01 | 1 | `MISSING_FUNCTIONALITY` | `MeshServiceName::new`/`Display`/`FromStr`/serde `todo!` → round-trip panics |
| S-DBN-NAME-02 | 1 | `MISSING_FUNCTIONALITY` | `MeshServiceName::new` case-fold `todo!` |
| S-DBN-NAME-03 | 1 | `MISSING_FUNCTIONALITY` | `MeshServiceName::new` suffix grammar `todo!` (accept/reject unimplemented) |
| S-DBN-NAME-04 | 1 | `MISSING_FUNCTIONALITY` | `MeshServiceName::new` label-limit validation `todo!` |
| S-DBN-ANSWER-01 | 1 | `MISSING_FUNCTIONALITY` | `answer_for` `Records` arm `todo!` |
| S-DBN-ANSWER-02 | 1 | `MISSING_FUNCTIONALITY` | `answer_for` `NxDomain` (empty-set) arm `todo!` |
| S-DBN-ANSWER-03 | 1 | `MISSING_FUNCTIONALITY` | `answer_for` `NoData`/qtype-dispatch arm `todo!` |
| S-DBN-ANSWER-04 | 1 | `MISSING_FUNCTIONALITY` | `NameIndex` healthy-gate + `answer_for` `todo!` |
| S-DBN-ANSWER-05 | 1 | `MISSING_FUNCTIONALITY` | `answer_for` lookup-miss arm `todo!` |
| S-DBN-WIRE-01 | 1 | `MISSING_FUNCTIONALITY` | `wire::encode` `Records` path `todo!` |
| S-DBN-WIRE-02 | 1 | `MISSING_FUNCTIONALITY` | `wire::encode` `NoData`+SOA path `todo!` |
| S-DBN-WIRE-03 | 1 | `MISSING_FUNCTIONALITY` | `wire::encode` `NxDomain`+SOA path `todo!` |
| S-DBN-WIRE-04 | 1 | `MISSING_FUNCTIONALITY` | `wire::encode` SOA-SERIAL-from-`Clock` `todo!` |
| S-DBN-IDX-01 | 1 | `MISSING_FUNCTIONALITY` | `NameIndex` List-seed + watch `todo!` |
| S-DBN-IDX-02 | 1 | `MISSING_FUNCTIONALITY` | `NameIndex` healthy-gate on watch `todo!` |
| S-DBN-IDX-03 | 1 | `MISSING_FUNCTIONALITY` | `NameIndex` relist-on-`Lagged` `todo!` |
| S-DBN-IDX-04 | 1 | `MISSING_FUNCTIONALITY` | `NameIndex` single-source read `todo!` |
| S-DBN-WS | 3 | `MISSING_FUNCTIONALITY` | `DnsResponder::{new,probe,serve}` `todo!` + no `run_server` spawn → `getent` times out (the litmus). Test-side `#[should_panic(expected = "RED scaffold")]` until the boot wiring lands. **Timeline note**: on the RED-phase Lima-root run the test panics on the missing `DnsResponder::serve` implementation (the `todo!("RED scaffold")`) — that IS the expected RED classification. The "delete the production spawn → `getent` times out" litmus is a POST-GREEN regression guard (run after the responder is implemented), NOT part of the RED classification. |
| S-DBN-SINGLE-SRC | 3 | `MISSING_FUNCTIONALITY` | `DnsResponder` answer path `todo!` → no answered addr to feed `resolve` |
| S-DBN-PINGPONG | 3 | `MISSING_FUNCTIONALITY` | responder serve loop `todo!` + `examples/dial-by-name-responder/{a,b}.toml` absent → no resolution. **Note**: EDD evidence stays honest `pending` (#227/#75); the `#[test]` itself is RED-scaffolded. **Timeline note**: on the RED-phase Lima-root run the test panics on the missing `DnsResponder::serve` implementation (the `todo!("RED scaffold")`) — that IS the expected RED classification. The "delete the production spawn → `getent` times out" litmus is a POST-GREEN regression guard (run after the responder is implemented), NOT part of the RED classification. |
| S-DBN-NXDOMAIN-01 | 3 | `MISSING_FUNCTIONALITY` | `answer_for` `NxDomain` arm + serve loop `todo!` |
| S-DBN-NXDOMAIN-02 | 3 | `MISSING_FUNCTIONALITY` | `NameIndex` watch-drop + serve loop `todo!` |
| S-DBN-NXDOMAIN-03 | 3 | `MISSING_FUNCTIONALITY` | `answer_for` miss + serve loop `todo!` |
| S-DBN-BIND-01 | 3 | `MISSING_FUNCTIONALITY` | `responder.rs` wildcard bind + `IP_PKTINFO` `todo!` |
| S-DBN-BIND-02 | 3 | `MISSING_FUNCTIONALITY` | `responder.rs` per-addr fallback `todo!` |
| S-DBN-BIND-03 | 3 | `MISSING_FUNCTIONALITY` | `run_server` responder-probe refuse-boot wiring + `DnsResponderError` `todo!` |

## Tier-3 caveat — `#[ignore]` vs `#[should_panic]`

The Tier-3 scenarios (S-DBN-WS, S-DBN-SINGLE-SRC, S-DBN-PINGPONG,
S-DBN-NXDOMAIN-*, S-DBN-BIND-*) require root + a real kernel under Lima.
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

The `answer_for` and `wire.rs` scaffolds NAME `hickory_proto::rr::RecordType`
and `hickory_proto` `Message`/`RData`/`SOA` types. A compilable RED
scaffold therefore REQUIRES the `hickory-proto` workspace dependency,
which is DELIVER's wiring step (ADR-0072 § Components: "`hickory-proto`
workspace dep — ADD"). **DISTILL does NOT add the dependency** —
materialising a half-built module + a new workspace dep mid-DISTILL would
perturb the workspace build for everyone and is out of scope (the SCOPE
DECISION). DELIVER's RED phase:

1. Adds `hickory-proto.workspace = true` (root `Cargo.toml`
   `[workspace.dependencies]`) + the `nix` `socket`/`uio` features.
2. Materialises the scaffolds per the MANIFEST (`feature-delta.md` §
   Scaffold MANIFEST), each with the `todo!("RED scaffold: …")` /
   `#[should_panic(expected = "RED scaffold")]` markers.
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
