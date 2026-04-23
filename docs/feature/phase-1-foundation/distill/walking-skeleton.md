# Walking Skeleton — phase-1-foundation

**Strategy**: C (real local adapters; no paid externals; no mocks). See
`wave-decisions.md` DWD-01.

## User goal the skeleton proves

Ana, the Overdrive platform engineer, can clone the repository onto a
laptop, run a single command, and see a distributed-systems test harness
boot a simulated multi-node cluster, exercise invariants that *would
catch real bugs*, report green in under a minute, and — when something
goes wrong — print a seed she can paste back to reproduce the failure
bit-for-bit. That is the whole claim of whitepaper §21 in one engineer
session.

## Demo script for a non-technical stakeholder

1. "Ana has cloned the project. She runs `cargo xtask dst`."
2. "The harness boots three simulated nodes on real redb plus all
   deterministic simulator adapters."
3. "Ninety-plus-some invariants run in under three seconds."
4. "The summary line says zero failures. The seed for this run is
   printed right there."
5. "Ana copies the seed and runs `cargo xtask dst --seed <N>`. The exact
   same trajectory replays."
6. "We deliberately break one adapter. Ana runs `cargo xtask dst` again.
   The harness prints the invariant name, the seed, the exact simulated
   tick, and a one-line command she can paste to reproduce it."
7. "Ana runs the printed command. Same failure at the same tick on the
   same host. No flake. No guessing. That is the claim made real."

Every noun in the demo script names an observable user outcome. No
layer, no protocol, no internal component is named. The stakeholder can
confirm "yes, that is what engineers need" without reading any Rust.

## The three walking-skeleton scenarios

**WS-1 — Clean-clone green run** (`test-scenarios.md` §1.1, tags
`@walking_skeleton @real-io @adapter-integration @driving_port @us-06
@kpi K1`). Enters through the `cargo xtask dst` subprocess. Exercises
real redb-backed LocalStore in a `tempfile::TempDir`. Every Sim adapter
composes. Returns green. Prints the seed on the first line.

**WS-2 — Seed reproduces trajectory** (§1.2, `@walking_skeleton
@driving_port @us-06 @kpi K3`). Enters through the `cargo xtask dst
--seed <N>` subprocess twice in sequence. Proves that the seed captured
in WS-1 is the *whole* determinism input — the harness produces the
same ordered invariant results and the same per-invariant tick numbers
across two runs.

**WS-3 — Red run produces a usable reproduction command** (§1.3,
`@walking_skeleton @driving_port @error-path @canary @us-06 @kpi K6`).
A deliberately planted bug in a Sim adapter (the canary scenario)
causes a real invariant failure. The subprocess exits non-zero. The
failure block names the failing invariant, the seed, the simulated
tick, the turmoil host, and a reproduction command embedding the same
seed and narrowing to the failing invariant via `--only`. This is the
scenario that proves the *feedback-loop* promise — not just that
invariants *can* pass, but that a failure is actionable.

## Why three and not one

Two alternative WS shapes were considered and rejected:

- **Single WS covering green + red + reproduction in one scenario.**
  Rejected. Violates "one scenario, one behavior" (bdd-methodology
  Rule 1). A single WS that tries to assert on both the green path and
  the red path has multiple `When` actions and cannot be split cleanly.
- **Green WS only.** Rejected. That scenario proves wiring, not
  value. The *value* the engineer needs is reproduction on failure; a
  green-only WS lets the reproduction claim ship with zero coverage —
  exactly the Dim-5 trap.

Three WS scenarios cover the three distinct engineer outcomes — "it
works," "it is deterministic," "failures are actionable" — and each
maps to exactly one AC cluster in US-06.

## Strategy-C litmus test

> "If I deleted the real redb adapter, would this WS still pass?"

**Answer**: No. WS-1 and WS-3 both instantiate a real `LocalStore`
backed by redb on a `TempDir`. Deleting the redb dependency breaks
compile; stubbing it with an in-memory fake would change the
snapshot bytes (rkyv framing over redb pages is what makes the
snapshot canonical) and fail WS-1's snapshot-related invariant. The
walking skeleton tests the wiring of the production adapter, not a
convenient stand-in. ✅

> "Could the WS pass without the `cargo xtask dst` subprocess
> wrapper?"

**Answer**: No. WS-1, WS-2, and WS-3 all enter through the
subprocess and assert on observable subprocess outcomes (exit code,
stdout format, artifact files). Calling the Rust `dst()` function
directly would skip the artifact-writing and first-line-seed logic
that the user actually sees. Testing through the subprocess is the
Mandate-1 driving-port rule applied to a CLI-shaped port. ✅

## What is NOT part of the walking skeleton

- Real Cloud Hypervisor, real eBPF, real Corrosion, real Raft. All of
  these are Phase 2+ per the architecture brief §1.
- Real LLM provider. `SimLlm` transcript replay is the production
  adapter for simulation.
- External HTTP services, paid APIs, cloud credentials. Per DWD-01,
  the walking skeleton deliberately has no `@requires_external`
  markers.
- Performance thresholds. K1's <60s bound is a wall-clock guardrail,
  not a performance benchmark. K4's cold-start and RSS figures are
  Phase 2+ per DWD-02.

## Traceability

- Journey: `docs/product/journeys/trust-the-sim.yaml` (Steps 1, 2, 4)
- User stories: US-06 (primary), US-03 (LocalStore dependency), US-04
  (SimObservationStore dependency), US-05 (Sim adapters)
- KPIs: K1, K3, K6 (WS-1 / WS-2 / WS-3 respectively)
- Shared artifacts: `dst_seed`, `invariant_name`, `snapshot_bytes`
  (see `discuss/shared-artifacts-registry.md`)
