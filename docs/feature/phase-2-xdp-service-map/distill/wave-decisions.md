# DISTILL Wave Decisions — phase-2-xdp-service-map

**Wave**: DISTILL (acceptance-designer)
**Owner**: Quinn (Atlas)
**Date**: 2026-05-05
**Status**: COMPLETE — handoff-ready for DELIVER (`/nw-deliver`)
**Mode**: lean (no upstream-issue surface; user-ratified open
questions Q-Sig + Q1..Q7 + three drifts inherited from DESIGN).

---

## Lean shape — what ran, what was skipped, why

This wave runs in the **project's four-tier Rust testing model**, NOT
the skill default of pytest-bdd + `.feature` files. Per the orchestrator
brief and `.claude/rules/testing.md`:

- **No `.feature` files anywhere.** BDD scenarios in
  `test-scenarios.md` are **specification only** (Gherkin markdown
  blocks). The executable form is Rust `#[test]` / `#[tokio::test]`
  bodies under `crates/<crate>/tests/integration/<scenario>.rs`,
  `crates/overdrive-bpf/tests/integration/<scenario>.rs`, and
  `crates/overdrive-sim/src/invariants/`.
- **No pytest, no conftest.py, no cucumber-rs** — the project blocks
  these via pre-tool hooks.
- **Four-tier model maps onto Mandate 6** — Tier 1 (DST) covers
  concurrency / ordering / partition; Tier 2 (`BPF_PROG_TEST_RUN`)
  covers program-level kernel correctness on curated input; Tier 3
  (Lima / `ubuntu-latest`) covers real-kernel integration; Tier 4
  (`veristat` + `xdp-bench`) covers verifier-budget + perf regression.

---

## Wave-decision reconciliation gate result

Both `discuss/wave-decisions.md` and `design/wave-decisions.md` were
read side-by-side. **Zero contradictions detected**. The seven
user-ratified open questions (Q-Sig + Q1..Q7) plus the three drifts
(Drift 1 — typed Action emission; Drift 2 —
`service_hydration_results` table; Drift 3 — three-map split with
typed-distinct outer keys) all flow through DESIGN coherently to
DISTILL. None are re-litigated.

The single piece of forward motion DISTILL adds is the **test-tier
mapping per ASR + per scenario** locked below as DWD-2 — the design
flagged this as advisory in `architecture.md` § 13 and explicitly
deferred to DISTILL.

---

## Decisions

### DWD-1 — Walking-skeleton strategy: inherited, not new

**Decision**: NO new walking skeleton in this feature. Phase 1's
`submit-a-job.yaml` walking skeleton already shipped (the Phase 1
feature delta closed that loop end-to-end through the
`Dataplane::update_service` port stub). Phase 2.1 added the
`EbpfDataplane` adapter behind the same port. Phase 2.2 fills the
empty body of `EbpfDataplane::update_service` and adds the
`ServiceMapHydrator` reconciler that drives it.

**Mapping to test strategies**:

- **Tier 1 DST** — uses `SimDataplane` + `SimObservationStore`
  (Strategy A — InMemory). Property-shaped invariants live in
  `crates/overdrive-sim/src/invariants/service_map_hydrator.rs`.
- **Tier 3 real-kernel** — uses real eBPF programs loaded into the
  Lima VM (developer macOS) or `ubuntu-latest` runner (CI). Real
  veth pair, real `BPF_PROG_TEST_RUN` for Tier 2, real `xdp-bench`
  for Tier 4. (Strategy C — real local).

The two compose: every fault class in `.claude/rules/testing.md`
§ "Fault injection catalogue" has a Tier 1 DST counterpart and a
Tier 3 real-kernel counterpart; neither is a substitute for the
other. Bug classes partition.

**Why no new WS scenario**: a walking skeleton's litmus test is "can
a non-technical stakeholder confirm 'yes, that is what users need'?".
For Phase 2.2 the answer was already shipped at `/v1/jobs` POST →
allocation → workload-running → eventually-routed-via-VIP. This
feature does not introduce a new user-observable goal; it fills the
body of a hexagonal port that the Phase 1 walking skeleton already
exercised. Adding a "WS: customer can route traffic to a multi-backend
service" scenario today would be Testing Theater — the user-facing
flow does not change, only the internal kernel-side mechanism does.

**Inherited WS scenarios (read-only references)**:

- `phase-1-first-workload`'s Phase 1 walking skeleton —
  `customer-submits-a-job-and-watches-it-run` — closes the
  user-observable loop. Phase 2.2 does not regress it; the
  `ReconcilerIsPure` and existing job-lifecycle invariants continue
  to pass with the new hydrator added to the catalogue.

### DWD-2 — Tier mapping per ASR (locked)

Per architecture.md § 13 the test file inventory was advisory. DISTILL
locks it to the table below. Each path is the canonical home for the
scenarios that exercise its ASR; cross-references in
`test-scenarios.md` cite the path verbatim.

| ASR | Quality attribute | Tier | File location (LOCKED) |
|---|---|---|---|
| ASR-2.2-01 | Reliability — zero-drop atomic SERVICE_MAP swap | Tier 3 | `crates/overdrive-dataplane/tests/integration/atomic_swap.rs` |
| ASR-2.2-02 | Reliability — ≤ 1 % Maglev incidental disruption | Tier 1 (DST proptest, primary) + Tier 3 (real-veth confirm) | Tier 1: `crates/overdrive-sim/tests/integration/maglev_churn.rs`; Tier 3: `crates/overdrive-dataplane/tests/integration/maglev_real.rs` |
| ASR-2.2-03 | Maintainability — verifier ≤ 20 % delta | Tier 4 | `cargo xtask verifier-regress` baselines under `perf-baseline/main/verifier-budget/` |
| ASR-2.2-04 | Correctness — hydrator ESR closure | Tier 1 (DST invariants) | `crates/overdrive-sim/src/invariants/service_map_hydrator.rs` (two named invariants `HydratorEventuallyConverges` + `HydratorIdempotentSteadyState`) |
| Endianness lockstep (§ 11) | Correctness — wire-vs-host byte order | Tier 2 + userspace proptest | Tier 2: `crates/overdrive-bpf/tests/integration/reverse_key_roundtrip.rs`; userspace proptest: mod-tests inside `crates/overdrive-dataplane/src/maps/reverse_nat_map_handle.rs` |

### DWD-3 — File-path inventory

The full Rust test file inventory the DELIVER wave will produce, by
crate:

**`crates/overdrive-bpf/tests/integration/`** (Tier 2,
`BPF_PROG_TEST_RUN`):

```
xdp_pass_test_run.rs          # EXISTING (Phase 2.1) — keep
xdp_service_map_lookup.rs     # NEW — S-2.2-04, S-2.2-05, S-2.2-08
tc_reverse_nat.rs             # NEW — S-2.2-12, S-2.2-13
sanity_prologue_drops.rs      # NEW — S-2.2-15..S-2.2-18
reverse_key_roundtrip.rs      # NEW — S-2.2-14 (endianness)
```

**`crates/overdrive-dataplane/tests/integration/`** (Tier 3,
real veth + Lima / `ubuntu-latest`):

```
build_rs_artifact_check.rs    # EXISTING (Phase 2.1) — keep
veth_attach.rs                # NEW — S-2.2-01, S-2.2-02, S-2.2-03 (US-01)
service_map_forward.rs        # NEW — S-2.2-06, S-2.2-07 (US-02)
atomic_swap.rs                # NEW — S-2.2-09, S-2.2-10, S-2.2-11 (US-03; ASR-2.2-01)
maglev_real.rs                # NEW — Tier 3 confirm of ASR-2.2-02
reverse_nat_e2e.rs            # NEW — S-2.2-19, S-2.2-20 (US-05)
sanity_mixed_batch.rs         # NEW — S-2.2-22 (US-06 mixed-batch)
```

**`crates/overdrive-control-plane/tests/integration/`** (Tier 1
DST + control-plane integration):

```
service_map_hydrator_dispatch.rs  # NEW — S-2.2-26, S-2.2-27, S-2.2-28 (US-08)
```

**`crates/overdrive-sim/`** (Tier 1 DST):

```
src/invariants/service_map_hydrator.rs   # NEW module — defines
                                          # HydratorEventuallyConverges
                                          # + HydratorIdempotentSteadyState
                                          # invariants
src/invariants/mod.rs                    # EXTEND — add the two new
                                          # `Invariant` enum variants
tests/integration/maglev_churn.rs        # NEW — DST proptest of
                                          # ASR-2.2-02 (≤ 1 % churn)
```

**`crates/overdrive-core/tests/`** (proptest for newtypes):

```
maglev_table_size.rs           # NEW — newtype roundtrip + prime list
drop_class.rs                  # NEW — newtype roundtrip + variant set
service_vip.rs                 # NEW — newtype roundtrip
service_id.rs                  # NEW — newtype roundtrip
backend_id.rs                  # NEW — newtype roundtrip
fingerprint.rs                 # NEW — content-hash determinism
```

**`xtask/`** (Tier 4 perf gates + self-test):

```
src/main.rs                    # EXTEND — fill the verifier-regress
                               # and xdp-perf stubs (Slice 07)
tests/perf_gate_self_test.rs   # NEW — synthetic input proves the
                               # gate logic itself returns non-zero
                               # on > 5 % regression (US-07 K7)
```

### DWD-4 — RED scaffold strategy

Per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
failing commits" — DISTILL produces:

1. **Production module scaffolds** — files exist, functions exist,
   bodies are `panic!("Not yet implemented -- RED scaffold")` or
   `todo!("RED scaffold: <one-line gherkin>")`. These compile but
   panic if invoked. Pre-existing tests that touch them via generic
   harnesses (DST `SimInvariant` enum, action-shim exhaustive match)
   will panic — that is correct, not a regression. DELIVER turns RED
   to GREEN one scaffold at a time per the carpaccio slice plan.
2. **Integration test bodies** — created as `#[test]` / `#[tokio::test]`
   functions that `panic!("RED scaffold: S-2.2-NN — <gherkin one-liner>")`
   so the test is RED, not BROKEN. The function compiles; it panics
   when run. DELIVER reaches GREEN by replacing the body, not by
   adding the test.
3. **DST invariant scaffolds** — added to the existing `Invariant`
   enum in `crates/overdrive-sim/src/invariants/mod.rs` as additive
   variants. The evaluator body in
   `crates/overdrive-sim/src/invariants/service_map_hydrator.rs`
   panics; the harness's existing exhaustive `match` over `Invariant`
   forces the new variants to be considered, which is exactly the
   scaffold contract.
4. **Commit hygiene** — DISTILL's commit lands the scaffolds normally
   (no RED panic is triggered until DELIVER's first integration test
   actually runs). DELIVER's RED-to-GREEN commits use
   `git commit --no-verify` per the project rule when the panic IS
   fired by the gates; this is per-commit, not per-PR, and the
   rationale is called out in the commit message.

### DWD-5 — `Action::DataplaneUpdateService` variant (RED scaffold)

The new variant lands in `crates/overdrive-core/src/reconciler.rs` as
an additive `Action` enum variant. The action shim's exhaustive match
in `crates/overdrive-control-plane/src/action_shim.rs` is extended
with one new arm whose body is `todo!("RED scaffold: dispatch
DataplaneUpdateService — see S-2.2-26..28")`. The variant declaration
itself does NOT panic — it is data; only the consumer panics until
DELIVER fills the dispatcher.

The shape is fixed at DESIGN time per `architecture.md` § 7 (locked
variant body): four fields — `service_id`, `vip`, `backends`,
`correlation`. DISTILL does not re-litigate; the scaffold mirrors the
DESIGN snippet verbatim.

### DWD-6 — Adapter coverage table

Every driven adapter (driven port) the feature touches has at least
one Tier 3 `@real-io @adapter-integration` scenario that exercises
real I/O. `SimDataplane` + `SimObservationStore` cannot catch wiring
bugs, kernel verifier rejection, packet rate / latency, kTLS offload,
NIC driver behaviour, BPF map format mismatches, or libbpf-sys
binding drift. Synthetic test-doubles miss these; only real adapters
catch them.

| Driven adapter | BPF map / observation table | Real-I/O scenario | Tier |
|---|---|---|---|
| `EbpfDataplane::update_service` | `SERVICE_MAP` (HASH_OF_MAPS outer) | S-2.2-09 (atomic swap), S-2.2-06 (forward) | Tier 3 |
| `EbpfDataplane::update_service` | `BACKEND_MAP` (HASH) | S-2.2-09 (orphan GC) | Tier 3 |
| `EbpfDataplane::update_service` | `MAGLEV_MAP` (HASH_OF_MAPS outer) | S-2.2-21 (Maglev real veth) | Tier 3 |
| `EbpfDataplane::update_service` | `REVERSE_NAT_MAP` (HASH) | S-2.2-19 (real-TCP nc) | Tier 3 |
| `EbpfDataplane::update_service` | `DROP_COUNTER` (PERCPU_ARRAY) | S-2.2-22 (mixed-batch counters) | Tier 3 |
| `Dataplane` port (driving) | `service_hydration_results` write | S-2.2-26 (hydrator dispatch) | Tier 1 (DST + sim, primary) |
| `LocalObservationStore` | `service_hydration_results` row | S-2.2-29 (LWW domination on conflict) | Tier 1 + crate-local trait conformance |
| Loader (aya `Bpf::load`) | XDP attach to real iface | S-2.2-01 (veth attach) | Tier 3 |
| Loader (aya `Bpf::load`) | Native-mode fallback warning | S-2.2-02 (generic-mode warn) | Tier 3 |
| Loader (aya `Bpf::load`) | Iface resolution | S-2.2-03 (`IfaceNotFound` error) | Tier 3 |

Every BPF map gets at least one `@real-io` scenario. Every observation
row written by the action shim has a Tier 1 DST scenario AND its
LWW behaviour is exercised by the existing
`SimObservationLwwConverges` invariant (which the new table inherits
for free as long as the schema follows the additive-only single-
writer-in-Phase-2 model per architecture.md § 12).

### DWD-7 — Scenario tagging convention

Every scenario in `test-scenarios.md` carries one or more of:

- **`@US-NN`** — primary user story (e.g. `@US-01`).
- **`@K-N`** — outcome KPI (e.g. `@K3`).
- **`@ASR-2.2-NN`** — quality-attribute scenario (e.g. `@ASR-2.2-01`).
- **`@slice-NN`** — carpaccio slice (e.g. `@slice-03`).
- **`@walking_skeleton`** — inherits from Phase 1's WS (no new WS in
  this feature; tag absent on every scenario in this feature; the
  Phase 1 inheritance is a documentation pointer, not a tag here).
- **`@real-io @adapter-integration`** — touches real eBPF / real
  veth / real `BPF_PROG_TEST_RUN`. Tier 3 / Tier 2.
- **`@in-memory`** — uses `SimDataplane` + `SimObservationStore` only.
  Tier 1 DST. Cannot use this tag on a scenario that asserts on
  kernel verifier behaviour, real packet rates, or real BPF map
  format.
- **`@property`** — tagged for DELIVER's crafter to implement as a
  property-based test with proptest generators (not a single-example
  assertion). Maglev determinism, fingerprint determinism, newtype
  roundtrip, endianness roundtrip all carry this tag.
- **`@kpi`** — verifies emission of an outcome-KPI signal. K1's
  structured-warning assertion + K7's perf-gate self-test carry this.
- **`@pending`** — DELIVER consumes scenarios one-at-a-time per
  `nw-test-design-mandates` "one scenario at a time"; every scenario
  ships RED-scaffolded with `@pending` and DELIVER drops the tag as
  it picks the scenario up. Phase 1's first scenario (S-2.2-01) is
  the only one launched without `@pending` — it is the one DELIVER
  starts on.

### DWD-8 — Scope boundaries respected (informational)

DISTILL did NOT:

- Add Cargo.toml dependencies (DELIVER's responsibility).
- Add aya-rs map declarations (DELIVER fills bodies; DISTILL ships
  module skeletons + RED panics).
- Touch `dst-lint` allow-lists (no new banned imports introduced).
- Load `nw-roadmap-design` (that's DELIVER).
- Invoke the reviewer (orchestrator decides).
- Add the `integration-tests = []` feature to any crate (already
  present in every workspace member per
  `.claude/rules/testing.md` § "Workspace convention" — verified
  via `xtask::mutants::tests::every_workspace_member_declares_
  integration_tests_feature`).

Verification: every crate touched by DISTILL scaffolds already has
`integration-tests = []` declared.

---

## Cross-cutting concerns surfaced during DISTILL

**None blocking.** The eight user stories' embedded BDD scenarios
distill cleanly into 30 `S-2.2-NN` scenarios across 4 tiers (28 in
`test-scenarios.md` + 2 newtype-proptest-equivalents inherited by the
existing newtype-roundtrip discipline). Error-path coverage is
13/30 = 43.3 % (above the 40 % mandate). Every `@K-N` is exercised
by ≥ 1 scenario. Every `@ASR-2.2-NN` is exercised by ≥ 2 scenarios
across 2 tiers (Mandate 6 — bug classes partition).

The single piece of ambiguity surfaced (and resolved inline in
`test-scenarios.md`) is the **Q1 K3-vs-K6 measurement source for
zero-drop**: K3 measures via `xdp-trafficgen` send vs sink receive
accounting (the Slice 03-time signal); K6 measures via
`DROP_COUNTER` PERCPU_ARRAY (the Slice 06-time signal). Both
scenarios refer to the same underlying behaviour; DISTILL's tag
discipline (`@K3` vs `@K6`) keeps them traceably distinct. No
upstream issue raised.

---

## Definition of Done (project-adapted)

Following `nw-acceptance-designer.md` § DoD adapted to the four-tier
Rust testing model:

- [x] **All acceptance scenarios written with passing step
  definitions** — `test-scenarios.md` ships 30 scenarios; RED
  scaffolds compile and panic with named messages; first scenario
  (S-2.2-01) is unmarked-`@pending` and ready for DELIVER's first
  GREEN pass.
- [x] **Test pyramid complete** — Tier 1 (DST) + Tier 2
  (`BPF_PROG_TEST_RUN`) + Tier 3 (real veth) + Tier 4 (`veristat` +
  `xdp-bench`) all enumerated; per-scenario tier mapping locked in
  DWD-2.
- [ ] **Peer review approved** — orchestrator decides if `/nw-review`
  runs. DISTILL ships handoff-ready; reviewer approval is a
  downstream gate.
- [x] **Tests run in CI/CD pipeline** — every test family routes
  through `cargo nextest run --features integration-tests` (Tier 1 +
  unit), `cargo xtask integration-test vm` (Tier 3), `cargo xtask
  verifier-regress` + `cargo xtask xdp-perf` (Tier 4) per
  `.claude/rules/testing.md` § "CI topology". All four lanes are
  already wired by Phase 2.1; this feature only adds new tests to
  the existing infrastructure.
- [x] **Story demonstrable to stakeholders from acceptance tests** —
  the eight stories' embedded BDD scenarios in
  `discuss/user-stories.md` distill directly into named
  `S-2.2-NN` scenarios; each scenario's title is operator-or-
  platform-engineer-observable behaviour (not internal mechanics).

---

## Handoff package for DELIVER

- `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md` —
  30 named `S-2.2-NN` scenarios, tier mapping, file paths,
  Gherkin-as-spec markdown blocks, tags.
- `docs/feature/phase-2-xdp-service-map/distill/wave-decisions.md` —
  this file.
- `docs/feature/phase-2-xdp-service-map/distill/walking-skeleton.md` —
  inheritance documentation (Phase 1 + Phase 2.1).
- `docs/feature/phase-2-xdp-service-map/distill/acceptance-review.md`
  — self-review per the skill's checklist (project-adapted).
- **RED scaffolds** under
  `crates/overdrive-bpf/src/`, `crates/overdrive-dataplane/src/`,
  `crates/overdrive-control-plane/src/`, `crates/overdrive-core/src/`,
  `crates/overdrive-sim/src/invariants/`. Every module file declared
  in DESIGN's § 9 layout exists as a stub.
- **Integration-test scaffolds** matching DWD-3's file-path inventory
  — empty `#[test]` bodies that `panic!("RED scaffold: S-2.2-NN —
  <gherkin>")`.

DELIVER consumes one scenario at a time per the `@pending` discipline;
S-2.2-01 (`Real-iface XDP attach`) is the unmarked starting scenario.

---

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial DISTILL wave decisions for `phase-2-xdp-service-map`. WS strategy locked as inherited from Phase 1; tier mapping locked per ASR; file-path inventory locked; RED scaffold strategy aligned to project four-tier Rust model; zero contradictions surfaced in DISCUSS↔DESIGN reconciliation pass. — Quinn (Atlas). |
