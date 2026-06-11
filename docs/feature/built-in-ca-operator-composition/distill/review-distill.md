# DISTILL Wave Review - built-in-ca-operator-composition

review_id: `accept_rev_2026-06-09T15:51:52Z`
reviewer: `acceptance-designer (review mode)`
approval_status: `approved`

## Scope

Reviewed the current DISTILL artifacts and executable scaffolds for
`built-in-ca-operator-composition`:

- `docs/feature/built-in-ca-operator-composition/feature-delta.md`
- `docs/feature/built-in-ca-operator-composition/design/wave-decisions.md`
- `docs/feature/built-in-ca-operator-composition/distill/test-scenarios.md`
- `docs/feature/built-in-ca-operator-composition/distill/red-classification.md`
- `crates/overdrive-core/tests/acceptance/svid_lifecycle_rotation.rs`
- `crates/overdrive-core/tests/acceptance.rs`
- `crates/overdrive-control-plane/tests/integration.rs`
- `crates/overdrive-control-plane/tests/integration/built_in_ca_operator_composition/*.rs`
- `verification/expectations/E03-ca-full-chain-verifies/{README.md,runner.sh}`

## Strengths

- Scenario coverage is broad and traceable: 18 scenarios map to D-OC-1..8, the
  three DELIVER slices, and EDD expectations D01/O04/O05/E03.
- Error/edge coverage meets the DISTILL bar: 10/18 scenarios are sad-path or edge
  cases (56%), including no-op rotation, inclusive boundary, rotate-vs-restart
  separation, wrong/tampered/absent KEK, no silent re-mint, no certificate/key
  leak, and the pathLen=0 negative anchor.
- The previous observable-behavior issue is resolved: S-OC-04 now proves
  TTL-derived threshold behavior only through the emitted action list, not by
  inspecting a private threshold constant.
- The previous GWT issue is resolved: the three independent refusal causes are
  split into S-OC-08a/b/c, with S-OC-08d retaining the pairwise-distinct stderr
  contract.
- Adapter coverage is explicit: `RcgenCa`, `SystemdCredsKeyring`,
  `LocalIntentStore`, and `LocalObservationStore` each have at least one real-I/O
  Tier-3 scenario.
- The O05 vs E03 separation is correctly enforced: `issued_certificates` is
  operator metadata only, while E03 requires exported PEMs plus `openssl verify`.

## Issues Identified

None blocking. The two high-severity findings from the earlier review have been
remediated in both the scenario document and the Rust scaffolds.

## Gate Checks

| Gate | Result | Evidence |
|---|---|---|
| Happy-path bias | PASS | 10/18 error or edge scenarios, 56%. |
| GWT format | PASS | S-OC-08 is split into one refusal action per scenario plus a separate distinctness scenario. |
| Business/domain language | PASS | Technical terms are platform domain/port names required to specify this infrastructure feature. |
| Coverage completeness | PASS | D-OC-1..8 and EDD D01/O04/O05/E03 are mapped in the scenario index and feature-delta. |
| Observable assertions | PASS | S-OC-04 asserts emitted `IssueSvid` decisions at TTL-derived boundaries, not private threshold state. |
| Traceability | PASS | Scenario, scaffold, adapter, driving-port, and EDD mappings are present; `c4-diagrams.md` is correctly marked read. |
| Walking skeleton / real-I/O boundary | PASS | CLI scenarios use real subprocesses in Lima; E03 uses exported PEMs plus `openssl`. |
| E03 guardrail | PASS | Current runner is still a two-check pending shape, but DISTILL explicitly forbids `satisfied` until sub-claims 1-3 are enforced, including S-OC-15. |
| RED scaffold convention | PASS | L1 rotation scaffolds use `#[should_panic(expected = "RED scaffold")]`; Lima-only integration scaffolds use slice-specific `#[ignore]`. |

## Verification

Ran:

```bash
cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance -E 'test(svid_lifecycle_rotation)'
cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests --no-run
```

Results:

- `overdrive-core` focused scaffold run: 5 tests run, 5 passed via the expected
  RED panic convention.
- `overdrive-control-plane` integration binary compile check: GREEN.

## Decision

`approved`.

The DISTILL package is ready for DELIVER. E03 remains intentionally pending until
Slice ③ extends the runner/export hook to enforce all three sub-claims; that is
correctly documented as a DELIVER obligation, not a DISTILL blocker.
