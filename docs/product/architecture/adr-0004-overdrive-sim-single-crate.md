# ADR-0004 — Single `overdrive-sim` crate, not split

## Status

Accepted. 2026-04-21.

## Context

The Phase 1 walking skeleton needs:

1. Sim adapters for seven ports: `Clock`, `Transport`, `Entropy`,
   `Dataplane`, `Driver`, `Llm`, `ObservationStore`.
2. The turmoil harness that composes them (plus the real `LocalStore`)
   into a 3-node simulated cluster.
3. An invariant catalogue (enum + evaluator traits) for the initial six
   invariants.
4. Supporting infrastructure: seeded RNG, summary formatter,
   reproduction-command emitter.

How should these be packaged? Three options:

- **Option A — One crate (`overdrive-sim`)** holding all Sim impls, harness,
  invariants.
- **Option B — Three crates (`overdrive-sim-traits`,
  `overdrive-sim-impls`, `overdrive-sim-harness`)** — strict separation
  of contract, impl, and runner.
- **Option C — Per-adapter crates (`overdrive-sim-clock`,
  `overdrive-sim-transport`, …)** — maximal modularity.

Factors to weigh:

- **Compile time.** `turmoil` (0.6) pulls in a small but non-trivial
  dependency subtree. A single crate compiles once per workspace build.
  N crates each re-declaring the `turmoil` workspace dep incur N link
  steps (though not N compilations, thanks to Cargo's shared target dir).
- **Dependency boundary enforcement.** Production crates must never
  accidentally depend on `turmoil` or `StdRng` — that would put sim
  infrastructure on the production compile path. A single `overdrive-sim`
  crate draws one obvious boundary: "nothing that is not adapter-sim
  should depend on this."
- **Turmoil coupling.** Every Sim impl needs some coupling to turmoil
  primitives (the Sim runtime's RNG, tick scheduler, network). Splitting
  impls from harness means the impls crate also depends on turmoil —
  which defeats the "harness owns turmoil" intuition.
- **User surface.** Phase 1 has exactly two consumers of `overdrive-sim`:
  the turmoil harness (internal to the crate) and `xtask dst` (which
  invokes `cargo test --package overdrive-sim --features dst`). Nothing
  outside the crate references individual Sim impls directly.
- **Semver surface.** Splitting into three crates triples the semver
  surface. The sim impls will evolve together (adding Sim methods in
  lockstep with port-trait extensions — the whole point of US-05 scenario
  "Ana needs a new time-shaped method"). Three lockstep crates are a
  coordination tax.

## Decision

**One crate: `overdrive-sim`**, class `adapter-sim`. It hosts:

- `src/adapters/{clock.rs, transport.rs, entropy.rs, dataplane.rs,
  driver.rs, llm.rs, observation_store.rs}` — one file per Sim impl.
- `src/invariants.rs` — the `Invariant` enum + evaluator traits.
- `src/harness.rs` — the turmoil wiring (Sim::Builder, host registration,
  seed management, summary formatter).
- `src/lib.rs` — re-exports.

Tests:

- `tests/dst/*.rs` — each file is one DST scenario run by the harness.
  Each scenario test pulls the harness, sets up the topology, runs
  invariants, and asserts. This is where `xtask dst` ultimately lands
  via `cargo test --package overdrive-sim`.

Future consideration: *if* an external consumer of Overdrive ever wants
one individual Sim impl (e.g. a consumer using only `SimClock` for their
own DST), the split-out is a mechanical move. The single-crate shape
is not load-bearing against future reuse.

## Alternatives considered

### Option A — Three crates (traits / impls / harness)

**Rejected.** The "traits" crate would be empty — port traits already
live in `overdrive-core`; `overdrive-sim-traits` would be a confusing
second home. The impls crate cannot be turmoil-free because Sim impls
need turmoil primitives. The harness would then depend on impls, and
impls and harness would be changed in lockstep on every new invariant.
Three crates for no semantic benefit.

### Option B — Per-adapter crate

**Rejected.** Nine crates (seven adapters + invariants + harness) for a
single walking-skeleton feature is actively anti-scope. Each adapter is
40–150 lines of code; the crate overhead per adapter is larger than
the adapter itself.

### Option C — One crate (chosen)

See Decision above.

## Consequences

### Positive

- Minimum compile-time cost — turmoil compiles once.
- Minimum coordination cost — one Cargo.toml, one version, one lint class.
- Maximum dependency-boundary clarity: "`overdrive-sim` is the one crate
  with turmoil + StdRng" is a one-sentence rule.
- Test organisation (`tests/dst/*.rs`) maps 1:1 with the invariant
  catalogue, making each DST invariant independently runnable via
  `cargo test --package overdrive-sim --test dst -- <name>`.

### Negative

- A consumer wanting only `SimClock` must pull the whole crate. No known
  consumer today; acceptable.
- The crate will grow. If it exceeds a pain threshold (probably around
  15+ adapters or a thick set of invariant-catalogue sub-domains), a
  superseding ADR can split then, with evidence.

### Neutral

- Splitting later is mechanical (Rust module → crate extraction is well
  supported by the language).

## References

- `docs/feature/phase-1-foundation/discuss/wave-decisions.md` (question 4)
- `docs/feature/phase-1-foundation/discuss/user-stories.md` US-06
- `docs/whitepaper.md` §21
- `.claude/rules/testing.md` Tier 1 (DST)
