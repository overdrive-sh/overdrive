# Story Map — phase-1-foundation

## User: Overdrive platform engineer (Ana), distributed-systems SRE background
## Goal: Run `cargo xtask dst` on a clean clone and trust that a red run is reproducible from the printed seed.

## Backbone

The user activities, left-to-right in chronological order over the lifetime of the feature:

| 1. Model the domain | 2. Persist intent | 3. Observe reality | 4. Inject nondeterminism | 5. Simulate distributed behaviour |
|---|---|---|---|---|
| Define identifier newtypes and enforce completeness | Provide IntentStore trait and LocalStore (redb) with snapshot round-trip | Provide ObservationStore trait and an in-memory LWW sim impl | Provide injectable traits for every source of nondeterminism + CI lint gate | Run turmoil harness with real stores + all Sim* impls and assert invariants |

## Ribs (tasks under each activity)

### 1. Model the domain

- 1.1 Core identifier newtypes (JobId, NodeId, AllocationId) with FromStr/Display/serde/validating constructors *(Walking Skeleton)*
- 1.2 Identity and correlation newtypes (SpiffeId, CorrelationKey, InvestigationId, PolicyId, CertSerial, Region)
- 1.3 Content-addressed newtypes (ContentHash, SchematicId) with deterministic-hash discipline

### 2. Persist intent

- 2.1 IntentStore trait (get/put/delete/watch/txn) + export_snapshot/bootstrap_from *(Walking Skeleton)*
- 2.2 LocalStore implementation backed by real redb *(Walking Skeleton)*
- 2.3 Snapshot round-trip is bit-identical (export→bootstrap→re-export) *(Walking Skeleton)*
- 2.4 Watch prefix subscription delivers events

### 3. Observe reality

- 3.1 ObservationStore trait (read/write/subscribe) *(Walking Skeleton)*
- 3.2 SimObservationStore in-memory LWW with injectable gossip delay *(Walking Skeleton)*
- 3.3 LWW convergence under reordering is deterministic *(Walking Skeleton)*
- 3.4 Type-level separation: IntentStore and ObservationStore are distinct traits on distinct types

### 4. Inject nondeterminism

- 4.1 Clock / Transport / Entropy traits + real and sim implementations *(Walking Skeleton)*
- 4.2 Dataplane / Driver / Llm traits + real and sim implementations *(Walking Skeleton)*
- 4.3 CI lint gate (`cargo xtask dst-lint`) scans core crates for banned APIs *(Walking Skeleton)*
- 4.4 Lint gate error messages name the trait to use and link to `development.md`

### 5. Simulate distributed behaviour

- 5.1 `cargo xtask dst` harness boots a 3-node simulated cluster *(Walking Skeleton)*
- 5.2 Invariant catalogue: `single_leader`, `intent_never_crosses_into_observation`, `snapshot_roundtrip_bit_identical`, `lww_converges`, `replay_equivalent_empty_workflow` *(Walking Skeleton)*
- 5.3 Seeded reproduction: `--seed N` produces bit-identical trajectory *(Walking Skeleton)*
- 5.4 Failure output includes invariant name, seed, tick, reproduction command *(Walking Skeleton)*

## Walking Skeleton

The thinnest end-to-end slice that connects ALL five activities. If any activity has nothing above the skeleton line, the engineer cannot reach `cargo xtask dst` green:

| 1. Model | 2. Intent | 3. Observation | 4. Nondet | 5. Sim |
|---|---|---|---|---|
| 1.1 | 2.1 + 2.2 + 2.3 | 3.1 + 3.2 + 3.3 | 4.1 + 4.2 + 4.3 | 5.1 + 5.2 + 5.3 + 5.4 |

This bundle IS the walking skeleton for the entire Overdrive project. Everything Phase 2 onward assumes these primitives exist.

## Release slices (elephant carpaccio)

Each slice ≤1 day of focused work, ships demonstrable end-to-end value, carries a learning hypothesis.

### Slice 1 — Core identifier newtypes (WS — row 1 of the skeleton)

**Outcome**: Engineer can import `JobId`, `NodeId`, `AllocationId` from `overdrive-core`, construct them from user input, round-trip them through `Display`/`FromStr`/`serde_json`, and get a validating error on malformed input.

**Target KPI**: 100% newtype completeness for the three initial identifiers (proptest round-trip passes).

**Hypothesis**: "If `FromStr → Display → FromStr` is not lossless for any identifier, our newtype contract is broken and every downstream content-hash and DST reproduction inherits the bug."

**Delivers**: a foundation other slices can depend on. US-01.

### Slice 2 — Extended identifier newtypes (row 1 of the skeleton)

**Outcome**: Every whitepaper-referenced identifier (SpiffeId, CorrelationKey, InvestigationId, PolicyId, CertSerial, Region, ContentHash, SchematicId) is a newtype with the same completeness as Slice 1.

**Target KPI**: 100% newtype completeness across all eleven identifiers. Zero `String`-as-identifier in `overdrive-core` public API (enforced by code inspection and documented as a rule).

**Hypothesis**: "If SpiffeId is a `String` alias with a `normalize_spiffe_id` helper, the typed-identity claim in whitepaper §8 is not code, it's convention."

**Delivers**: US-02.

### Slice 3 — IntentStore trait + LocalStore on real redb (WS — row 2)

**Outcome**: Engineer has an `IntentStore` trait with a concrete `LocalStore` backed by real redb on disk. `export_snapshot` produces rkyv-archived bytes; `bootstrap_from` replays them; round-trip is bit-identical.

**Target KPI**: snapshot round-trip produces bit-identical bytes; LocalStore cold start < 50ms; RAM footprint < 30MB under empty-store conditions (whitepaper §4 "~30MB RAM" enforced).

**Hypothesis**: "If snapshot round-trip is lossy, the non-destructive single→HA migration story in `commercial.md` is broken and control-plane density cannot be the commercial margin driver it needs to be."

**Delivers**: US-03.

### Slice 4 — ObservationStore trait + SimObservationStore LWW (WS — row 3)

**Outcome**: Engineer has an `ObservationStore` trait distinct from `IntentStore` (enforced by types), a `SimObservationStore` implementing last-write-wins CRDT semantics in memory, with injectable gossip delay and partition.

**Target KPI**: under arbitrary write reordering from N peers, all peers converge to the same final state within a bounded number of ticks; zero cross-type confusions (a `JobSpec` cannot be persisted into `ObservationStore` — the compiler rejects it).

**Hypothesis**: "If LWW semantics produce different outcomes under reordering, DST loses determinism and the whitepaper's §4 Intent/Observation split is unprovable as code."

**Delivers**: US-04.

### Slice 5 — Nondeterminism traits + CI lint gate (WS — row 4)

**Outcome**: Every source of nondeterminism in whitepaper §21 has a trait (`Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `Llm`) with a real and a sim implementation. `cargo xtask dst-lint` blocks merges that smuggle banned APIs into core crates, with error messages pointing at the right trait and the right rule.

**Target KPI**: zero `Instant::now()` / `rand::random()` / `tokio::net::*` in core crates (enforced by lint). Lint gate has 100% coverage of the banned-API list in `development.md`.

**Hypothesis**: "If the lint gate misses a real `Instant::now()` in a core crate, the DST claim is performative — the harness cannot catch a bug routed around an un-intercepted side channel."

**Delivers**: US-05.

### Slice 6 — turmoil DST harness green (WS — row 5)

**Outcome**: `cargo xtask dst` on a clean clone boots a 3-node simulated cluster using `LocalStore` + `SimObservationStore` + all Sim* traits, runs a catalogue of invariants (`single_leader`, `intent_never_crosses_into_observation`, `snapshot_roundtrip_bit_identical`, `lww_converges`, `replay_equivalent_empty_workflow`), prints the seed on every run, reproduces bit-for-bit on `--seed N` and prints an exact reproduction command on failure.

**Target KPI**: DST wall-clock < 60s on an M-class developer laptop for the default invariant set. Same-seed reproduction is bit-identical across two sequential runs on the same toolchain.

**Hypothesis**: "If invariants can't be expressed as `assert_always!`/`assert_eventually!` against the harness, the testing model from whitepaper §21 needs revision before we build anything else."

**Delivers**: US-06.

## Priority Rationale

Ordering by outcome impact and dependencies. Every slice in Release 1 is on the walking skeleton — they are all required to reach `cargo xtask dst` green. The order within Release 1 is the dependency graph:

| Priority | Slice | Depends on | Why this order |
|---|---|---|---|
| 1 | Slice 1 — Core newtypes | — | Foundation for every other slice. If newtypes do not round-trip, the whole harness produces nondeterministic hashes. |
| 2 | Slice 2 — Extended newtypes | Slice 1 (pattern) | Same discipline, broader coverage. Independent of store work; can run in parallel once Slice 1 lands. |
| 3 | Slice 3 — IntentStore + LocalStore | Slice 1 | DST harness in Slice 6 needs a real intent store to run against. Snapshot round-trip is the commercial proof-point. |
| 4 | Slice 4 — ObservationStore + SimObservationStore | Slice 1 | Needed for the intent-vs-observation invariant in Slice 6. Independent of Slice 3 — both can run in parallel. |
| 5 | Slice 5 — Nondet traits + lint gate | Slice 1 | The six traits have to exist before the harness can compose them; the lint gate has to exist before the DST claim is more than aspirational. |
| 6 | Slice 6 — DST harness green | Slices 3, 4, 5 | Pulls everything together. This slice is **the acceptance gate** for the whole feature. |

All six ship in Release 1. There is no Release 2 for this feature — the walking skeleton is the deliverable.

## Scope Assessment: PASS

- **Story count**: 6 stories. Within the ≤10 ceiling.
- **Bounded contexts**: 2 (`overdrive-core`, `xtask`) plus a new `overdrive-sim` crate. Within the ≤3 ceiling.
- **Walking-skeleton integration points**: 5 activities glued by the harness in Slice 6 — still well under the >5 oversized signal (the harness *is* the integration, it doesn't fan out further).
- **Estimated effort**: 4–6 focused days (each slice ≤1 day, Slices 1–2 and 3–4 parallelisable).
- **Independent outcomes**: none worth shipping separately — without the DST harness green, no slice delivers commercial or technical value on its own. This is a single cohesive deliverable, which is the correct shape for a walking skeleton.

**Verdict**: right-sized. Proceed to user story crafting.

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial story map for phase-1-foundation. |
