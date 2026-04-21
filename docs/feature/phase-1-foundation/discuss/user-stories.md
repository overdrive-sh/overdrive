<!-- markdownlint-disable MD024 -->

# User Stories — phase-1-foundation

Six LeanUX stories, each delivering a single carpaccio slice from `story-map.md`. All stories share the persona and the vision context from `docs/product/vision.md` and `docs/product/jobs.yaml`.

## System Constraints (cross-cutting)

These apply to every story below. Violating any of them is a Definition-of-Ready failure regardless of how well the story otherwise reads.

- **No `.feature` files, anywhere.** Gherkin appears only in `discuss/*.md`, `discuss/*.yaml`, and `distill/test-scenarios.md` as markdown or YAML. The crafter translates to Rust `#[test]` / `#[tokio::test]` in `crates/{crate}/tests/`. (Source: `.claude/rules/testing.md`.)
- **Result alias convention.** Every crate that defines its own error type exposes `pub type Result<T, E = Error> = std::result::Result<T, E>;`. Internal code writes `fn foo(...) -> Result<Foo>`. Cross-crate callers use `overdrive_core::Result<T>`. Binary boundaries (`overdrive-cli`, `xtask`) use `eyre::Result`. (Source: `CLAUDE.md`.)
- **Newtypes are strict by default.** Every identifier MUST be a newtype with `FromStr`, `Display`, `Serialize`, `Deserialize`, and validating constructors returning `Result`. No `normalize_*` helpers — validation lives in the constructor. (Source: `.claude/rules/development.md`.)
- **Hashing uses deterministic serialization.** rkyv-archived bytes for internal hashing; RFC 8785 JCS for JSON hashing; never `serde_json::to_string()`. (Source: `.claude/rules/development.md`.)
- **Core crates are `async`-pure on the I/O boundary.** `Instant::now()`, `SystemTime::now()`, `rand::random()`, `rand::thread_rng()`, `tokio::time::sleep`, `std::thread::sleep`, `tokio::net::{TcpStream, TcpListener, UdpSocket}` are banned in core crates and enforced by `cargo xtask dst-lint`. (Source: `.claude/rules/development.md`, whitepaper §21.)
- **Reconcilers do not perform I/O.** `reconcile(desired, actual, db) → Vec<Action>` is pure over its inputs. External calls route through `Action::HttpCall`. (Applies to stories that define reconciler-relevant primitives — e.g., US-03 and US-04.)
- **IntentStore and ObservationStore are distinct traits on distinct types.** Nothing in the codebase can persist a job spec into the observation store or an allocation heartbeat into the intent store — the compiler rejects it. (Source: whitepaper §4.)

---

## US-01: Core Identifier Newtypes

### Problem

Ana, the Overdrive platform engineer, reads whitepaper §4's promise that "workload identity is cryptographic and typed" and then opens a scaffolded crate where `JobId` is a bare `String`. Every hand-written helper (`normalize_job_id`, `parse_node_id`) is a place the type system is doing nothing and a human is doing the work. The typed-identity claim in the whitepaper is words, not code, until the newtypes exist and are round-trippable through Display / FromStr / serde without loss.

### Who

- Overdrive platform engineer, primary author of control-plane logic | working inside `crates/overdrive-core` | motivated to rely on the type system rather than review discipline for identifier correctness.

### Solution

Introduce `JobId`, `NodeId`, and `AllocationId` as newtypes in `overdrive-core`, each with a validating constructor, `FromStr` (case-insensitive where human-typable), `Display`, serde, and round-trip proptests. No helper functions, no bare `String`s in any `overdrive-core` public API signature.

### Domain Examples

#### 1: Happy Path — Ana parses a job ID from a config file

Ana reads `job-id = "payments-api-v2"` out of a TOML file. She calls `JobId::from_str("payments-api-v2")` and gets `Ok(JobId(...))`. She calls `.to_string()` on it and gets `"payments-api-v2"` back, byte-for-byte. She serializes it with serde_json and gets `"\"payments-api-v2\""`. She deserializes that back and gets the same `JobId` value.

#### 2: Edge Case — Ana parses a valid SPIFFE ID with unusual but legal characters

Ana parses `spiffe://overdrive.local/job/payments-v2/alloc/7f3a9b12` as a `SpiffeId`. The parser accepts it, rejects `spiffe://Overdrive.Local/Job/...` if case-insensitive handling is wrong, and normalises path components through the constructor (never through a separate helper). She rounds-trips it and gets back the canonical lowercase form.

#### 3: Error Boundary — Ana tries to construct a JobId from garbage

Ana accidentally passes `"payments api"` (containing a space) to `JobId::from_str`. The constructor returns `Err(ParseError::InvalidCharacter { position: 8, character: ' ' })`. No `JobId` value is constructed; Ana's calling code branches on the error and reports it to the user. The same error path fires for an empty string, a leading slash, and any other malformed input. There is never a `JobId` in the system whose `Display` output would fail to re-parse.

### UAT Scenarios (BDD)

#### Scenario: Newtype round-trip is lossless across Display, FromStr, and serde

Given a valid identifier value for `JobId`, `NodeId`, or `AllocationId`
When Ana formats it via `Display`, parses the output with `FromStr`, serializes it with `serde_json`, and deserializes the result
Then the final value equals the original
And the serde output equals the `Display` output byte-for-byte

#### Scenario: Malformed input is rejected at the constructor

Given an input string containing a forbidden character
When Ana calls `JobId::from_str` on that string
Then an `Err(ParseError::...)` is returned with a variant naming the specific violation
And no `JobId` instance is constructed

#### Scenario: Empty input is rejected at the constructor

Given the empty string
When Ana calls `JobId::from_str("")`
Then `Err(ParseError::Empty)` is returned
And no `JobId` instance is constructed

#### Scenario: Display output is canonical

Given two `JobId` instances produced from inputs that differ only in unicode whitespace handling
When both are formatted via `Display`
Then the output strings are equal

#### Scenario: rkyv-archived bytes are stable

Given a `JobId` instance
When Ana archives it with `rkyv` on two separate invocations
Then the archived byte slices are equal

### Acceptance Criteria

- [ ] `JobId`, `NodeId`, `AllocationId` exist in `overdrive-core` with validating constructors
- [ ] Each type implements `FromStr`, `Display`, `Serialize`, `Deserialize`, `PartialEq`, `Eq`, `Hash`, `Debug`, `Clone`
- [ ] `FromStr` is case-insensitive where the whitepaper requires case-insensitive parsing
- [ ] `Display` output equals the canonical form `FromStr` accepts
- [ ] `serde_json::to_string(&x)` produces the `Display` output (surrounded by quotes)
- [ ] `rkyv` archival of the same value produces identical bytes across runs
- [ ] A proptest asserts the round-trip for randomised valid inputs
- [ ] No helper function named `normalize_*` exists for any of these types (validation is in the constructor)
- [ ] No `String`-typed identifier remains in `overdrive-core` public API

### Outcome KPIs

- **Who**: Overdrive platform engineer working inside `overdrive-core`
- **Does what**: uses typed identifiers instead of raw strings with normalisation helpers
- **By how much**: 100% of identifiers listed here are newtypes; 0 `normalize_*` helpers; 0 `String`-typed identifiers on the public API
- **Measured by**: static scan in a test that inspects the public API of `overdrive-core`; proptest round-trip for every listed newtype
- **Baseline**: greenfield (partial scaffolding exists in `crates/overdrive-core/src/id.rs` but is incomplete and untested at the newtype-contract level)

### Technical Notes

- Case-insensitive FromStr only where the whitepaper explicitly calls for it (SPIFFE, region codes). SHA-256 content hashes stay case-sensitive — they are not human-typed.
- Validation errors must be structured (a `ParseError` enum with variants), not a single `String` message.
- `Serialize`/`Deserialize` must route through `Display`/`FromStr` — no bespoke serde impl that could diverge from the canonical string form.
- **Depends on**: nothing — Slice 1 is the foundation.

---

## US-02: Extended Identifier Newtypes

### Problem

Ana, reading the whitepaper, sees that the platform talks about `SpiffeId`, `CorrelationKey`, `InvestigationId`, `PolicyId`, `CertSerial`, `Region`, `ContentHash`, and `SchematicId` as first-class concepts. Without them as newtypes, every subsystem — the CA, the gossip layer, the image factory, the SRE agent — starts its own string-parsing. The same three bugs (accepts garbage, drops trailing whitespace, inconsistent case) appear eight times in eight different crates. The structural solution is to land all eight as newtypes at the same time as the core three, so no downstream crate is ever tempted to invent its own.

### Who

- Overdrive platform engineer, working across `overdrive-core` and future subsystem crates | motivated to stop re-parsing identifiers in every crate.

### Solution

Add the remaining eight identifier newtypes to `overdrive-core` using the exact discipline established in US-01: validating constructors, `FromStr`/`Display`/serde round-trip, proptest coverage, no helpers.

### Domain Examples

#### 1: Happy Path — Ana constructs a SpiffeId from a SPIFFE URI

Ana has the string `spiffe://overdrive.local/job/payments/alloc/7f3a9b12` (whitepaper §8 canonical example). She calls `SpiffeId::from_str(...)` and gets back an instance whose `Display` output equals the original. She gets structured accessors for trust domain (`overdrive.local`) and path (`/job/payments/alloc/7f3a9b12`), not string-splitting at call sites.

#### 2: Edge Case — Ana hashes a WASM module and gets a ContentHash

Ana computes the SHA-256 of a WASM bytecode buffer and constructs `ContentHash::from_sha256_bytes([32 bytes])`. She formats it as hex via `Display`, parses it back with `FromStr`, and gets the same value. She tries to construct one from `"sha256:abc"` (too short) and gets `Err(ParseError::WrongLength { expected: 32, got: 2 })`.

#### 3: Error Boundary — Ana tries to register a Region with a mixed-case code

Ana writes `cluster.region = "EU-West-1"` in a Overdrive config. The loader calls `Region::from_str("EU-West-1")`. FromStr is case-insensitive and yields a `Region` whose `Display` output is `"eu-west-1"`. When Ana tries to construct `Region::from_str("eu west 1")` (spaces), she gets `Err(ParseError::InvalidCharacter)`.

### UAT Scenarios (BDD)

#### Scenario: SpiffeId parses and round-trips the whitepaper canonical example

Given the string `"spiffe://overdrive.local/job/payments/alloc/7f3a9b12"`
When Ana calls `SpiffeId::from_str` and then `Display`
Then the resulting string equals the input
And structured accessors expose the trust domain and path segments

#### Scenario: SpiffeId rejects a malformed URI

Given a string without the `spiffe://` prefix
When Ana calls `SpiffeId::from_str`
Then `Err(ParseError::MissingScheme)` is returned

#### Scenario: ContentHash enforces length

Given a hex string shorter than 64 characters
When Ana calls `ContentHash::from_str`
Then `Err(ParseError::WrongLength { expected: 64, got: <n> })` is returned

#### Scenario: Region normalises case on parse

Given the input `"EU-West-1"`
When Ana calls `Region::from_str`
Then an `Ok(Region)` is returned
And the resulting `Region`'s `Display` output is `"eu-west-1"`

#### Scenario: Zero string-typed identifiers remain on the public API

Given the `overdrive-core` public API
When a static-API inspection test runs
Then no exported function signature accepts a `&str` or `String` as an identifier parameter when a matching newtype exists

### Acceptance Criteria

- [ ] `SpiffeId`, `CorrelationKey`, `InvestigationId`, `PolicyId`, `CertSerial`, `Region`, `ContentHash`, `SchematicId` exist in `overdrive-core`
- [ ] Each meets the full US-01 completeness contract (FromStr, Display, serde, validating constructor, proptest)
- [ ] `SpiffeId` exposes trust-domain and path accessors; it does not require string-splitting at call sites
- [ ] `ContentHash` is fixed-length (32 bytes) and round-trips through hex `Display`/`FromStr`
- [ ] `Region` is case-insensitive on parse, lowercase on `Display`
- [ ] `SchematicId` is a SHA-256 content hash over the canonical serialised schematic (rkyv-archived bytes or JCS-canonical JSON; pick one and document it)
- [ ] A static inspection test asserts zero `String`-as-identifier in the public API

### Outcome KPIs

- **Who**: Overdrive platform engineer
- **Does what**: constructs any whitepaper-referenced identifier through a newtype in `overdrive-core`
- **By how much**: 11 newtypes total (3 from US-01 + 8 here) cover 100% of identifiers named in whitepaper §4 + §8 + §11 + §23
- **Measured by**: manual cross-reference between whitepaper identifier list and `overdrive-core` exports; automated static inspection test for `String`-as-identifier
- **Baseline**: partial scaffolding in `crates/overdrive-core/src/id.rs`; no round-trip guarantees yet

### Technical Notes

- Decide (and document) for `SchematicId`: rkyv-archived bytes of the schematic, or RFC 8785 JCS of the JSON form. Either works; inconsistency breaks content-addressing.
- `CorrelationKey` is derived — it is a newtype wrapping `(reconciliation_target, spec_hash, purpose)`. Its `Display` MUST be deterministic over those inputs.
- **Depends on**: US-01 (pattern).

---

## US-03: IntentStore Trait + LocalStore on Real redb

### Problem

The whitepaper (§4) and the commercial model (`commercial.md` — "Control Plane Density") both commit to a control plane that runs in ~30MB RAM in single mode and migrates to HA without downtime. That claim is hollow unless `LocalStore` actually exists, runs on real redb, and round-trips snapshots bit-for-bit. Ana needs the `IntentStore` trait defined once, a `LocalStore` implementation backed by redb on disk (not a mock), and `export_snapshot`/`bootstrap_from` that survive a round-trip without losing a single byte.

### Who

- Overdrive platform engineer implementing control-plane logic | operator running a single-mode control plane (indirect user; the density claim must hold for them).

### Solution

Introduce the `IntentStore` trait in `overdrive-core` with `get/put/delete/watch/txn/export_snapshot/bootstrap_from`. Provide `LocalStore` in a new `overdrive-store-local` crate (or inside `overdrive-core` — crafter's choice) that wraps redb directly. Assert bit-identical snapshot round-trip with a proptest.

### Domain Examples

#### 1: Happy Path — Ana stores a JobSpec and reads it back

Ana constructs a `LocalStore` on a tmpfs path. She writes a `JobSpec` for `job/payments` under a key derived from `JobId`. She calls `get` with the same key and gets the same bytes back (or, with rkyv, an `&Archived<JobSpec>` view). She watches the `jobs/` prefix, writes a second `JobSpec`, and sees a single event on the watch stream.

#### 2: Edge Case — Ana migrates a LocalStore to a fresh instance via snapshot

Ana has a `LocalStore` populated with three `JobSpec` entries. She calls `export_snapshot()` and captures the bytes. She creates a new `LocalStore` on a different path and calls `bootstrap_from(snapshot_bytes)`. She calls `export_snapshot()` on the new store. The two snapshot byte slices are equal. Every entry is readable from the new store. No reconciliation loop ran on empty state.

#### 3: Error Boundary — Ana tries to bootstrap from a corrupted snapshot

Ana has snapshot bytes but flips a single bit before calling `bootstrap_from`. The call returns `Err(IntentStoreError::SnapshotCorrupt { offset: <byte_position> })`. The target `LocalStore` remains empty. No partial state has been written.

### UAT Scenarios (BDD)

#### Scenario: Put then get is consistent within a single store

Given an empty `LocalStore` backed by redb on a tmpfs path
When Ana puts a value at key K and then gets key K
Then the returned bytes equal the put bytes

#### Scenario: Snapshot round-trip is bit-identical

Given a `LocalStore` populated with a known set of entries
When Ana calls `export_snapshot`, constructs a second `LocalStore`, calls `bootstrap_from`, and calls `export_snapshot` on the second store
Then the two snapshot byte slices are equal

#### Scenario: Watch fires on prefix-matching writes

Given a `LocalStore` and a watch subscribed to prefix `jobs/`
When another caller writes to key `jobs/payments`
Then the watch stream yields exactly one event whose key is `jobs/payments`

#### Scenario: bootstrap_from a corrupted snapshot fails clean

Given snapshot bytes that have been flipped in one position
When Ana calls `bootstrap_from` with the corrupted bytes
Then an `IntentStoreError::SnapshotCorrupt` is returned
And the target `LocalStore` contains no entries

#### Scenario: LocalStore cold start stays within the commercial envelope

Given a clean tmpfs path
When Ana constructs a new empty `LocalStore`
Then the constructor returns within 50ms
And the process RSS stays below 30MB while the store is empty

### Acceptance Criteria

- [ ] `IntentStore` trait defined in `overdrive-core::traits` with `get`, `put`, `delete`, `watch`, `txn`, `export_snapshot`, `bootstrap_from`
- [ ] `LocalStore` implementation wraps real redb (not a mock) and implements `IntentStore` end-to-end
- [ ] A proptest asserts bit-identical snapshot round-trip for randomised store contents
- [ ] A test asserts corrupted-snapshot inputs fail with a typed error, leaving the target store empty
- [ ] A benchmark asserts cold start < 50ms on a reference VM
- [ ] A test asserts RSS < 30MB on an empty `LocalStore`
- [ ] `IntentStore` and `ObservationStore` are separate traits on distinct types (typed separation is load-bearing — see System Constraints)
- [ ] No `put(key, value)` shared surface exists that could route a write to the wrong store

### Outcome KPIs

- **Who**: Overdrive platform engineer + single-mode operator
- **Does what**: runs a control plane that starts fast, uses little memory, and survives the HA migration
- **By how much**: cold start < 50ms; RSS < 30MB empty; 100% of snapshots round-trip bit-identical
- **Measured by**: `criterion` bench + RSS probe in tests; proptest for round-trip
- **Baseline**: whitepaper claim "~30MB RAM" for single mode; no current implementation to regress against

### Technical Notes

- redb is the backing store; do not introduce a mock. DST tests compose `LocalStore` with sim traits, but the store itself is real.
- Snapshot format is rkyv-archived bytes with a versioned framing header (for future migration).
- `export_snapshot`/`bootstrap_from` will also be invoked by the future `RaftStore` in HA mode — do not bake assumptions into the framing that would break that.
- **Depends on**: US-01, US-02 (for typed keys).

---

## US-04: ObservationStore Trait + SimObservationStore (in-memory LWW)

### Problem

The whitepaper (§4) draws a hard line between intent (linearizable, Raft) and observation (eventually consistent, Corrosion). That line is defensive — it is the lesson Fly.io learned from years of trying to use Consul-style consensus for everything. Ana needs that line to be a compiler-enforced boundary in Overdrive from day one, not a convention layered on top of a shared KV interface. And the DST harness needs a `SimObservationStore` that implements LWW semantics deterministically under reordering, so invariants like `intent never crosses into observation` and `LWW converges` can be asserted.

### Who

- Overdrive platform engineer writing reconcilers, gateways, and subsystems that read observation data | DST harness (primary consumer in Phase 1).

### Solution

Define `ObservationStore` as a trait distinct from `IntentStore`, on its own Rust types. Provide `SimObservationStore`: an in-memory LWW CRDT using logical timestamps, with injectable gossip delay and partition. Assert convergence under reordering is deterministic given a seed.

### Domain Examples

#### 1: Happy Path — Ana writes an allocation status row and observes it on a peer

Ana has a 3-node cluster running under the DST harness. Node A writes an `alloc_status` row for `alloc/a1b2c3` into its local `SimObservationStore`. Gossip delivers the row to nodes B and C within a few ticks (injectable). Reads on B and C return the same row bytes that A wrote.

#### 2: Edge Case — Ana writes concurrent updates to the same row

Node A writes `alloc_status { state = "running" }` at logical timestamp T1. Node B writes `alloc_status { state = "draining" }` at logical timestamp T2 > T1 for the same `alloc_id`. After gossip converges, all three nodes have the row at T2 with `state = "draining"` — LWW wins deterministically regardless of delivery order.

#### 3: Error Boundary — Ana tries to persist a JobSpec into the ObservationStore

Ana has a `JobSpec` (intent data) in hand. She tries to call `observation_store.write("jobs/payments", &job_spec_bytes)`. The code does not compile: `ObservationStore::write` takes a row shape derived from observation schemas, and `JobSpec` is not one. There is no escape hatch that would accept raw bytes under a caller-supplied key.

### UAT Scenarios (BDD)

#### Scenario: IntentStore and ObservationStore are distinct types at compile time

Given a function that accepts an `&dyn ObservationStore`
When Ana calls it with an `&dyn IntentStore` value
Then the code fails to compile
And the error message distinguishes the two traits

#### Scenario: SimObservationStore converges under reordered gossip

Given a seeded run where two peers write concurrent updates to the same row
When gossip delivers the writes in arbitrary order
Then all peers converge to the same final row after bounded ticks
And the final row's logical timestamp equals the maximum of the input timestamps

#### Scenario: Gossip delay is injectable and deterministic under seed

Given a seed and a configured gossip delay of N ticks
When Ana runs the sim twice with the same seed and same delay
Then every peer sees row updates at identical ticks in both runs

#### Scenario: Intent never crosses into observation (assert_always)

Given a DST run with any workload
When the invariant `intent_never_crosses_into_observation` is evaluated on every tick
Then no row in any `SimObservationStore` belongs to an intent type
And no key in any `IntentStore` carries an observation-class name prefix

#### Scenario: Full-row writes (not diffs) converge correctly

Given two peers each hold a previously-gossiped row
When a third peer writes the full updated row at a higher logical timestamp
Then all peers converge to the third peer's row
And no peer applies a partial-field merge

### Acceptance Criteria

- [ ] `ObservationStore` trait defined in `overdrive-core::traits` with `read`, `write`, `subscribe`, typed over observation-row shapes
- [ ] `IntentStore` and `ObservationStore` are not substitutable: a compile-time test asserts a function taking `&dyn ObservationStore` rejects an `&dyn IntentStore` argument
- [ ] `SimObservationStore` implements LWW with logical timestamps, in-memory storage
- [ ] Gossip delay and partition are injectable via the `SimObservationStore` constructor (or an explicit configuration type)
- [ ] A seeded test asserts identical trajectories across two runs
- [ ] An invariant `intent_never_crosses_into_observation` exists in `overdrive-sim` and is evaluable as `assert_always!`
- [ ] Row writes are full-row; field-diff merges are explicitly not supported in Phase 1

### Outcome KPIs

- **Who**: Overdrive platform engineer writing code that reads observation data
- **Does what**: writes code that cannot confuse intent and observation at compile time
- **By how much**: 100% of accidental intent/observation crossings fail to compile; DST invariant `intent_never_crosses_into_observation` is `assert_always` passing
- **Measured by**: compile-time test; DST harness
- **Baseline**: none — the trait does not exist yet; scaffolding in `crates/overdrive-core/src/traits/observation_store.rs` is partial

### Technical Notes

- `SimObservationStore` is sim-only; the real `CorrosionStore` is a later feature.
- LWW under logical timestamps means `(clock, writer_id)` tuples, not wall-clock — the whitepaper's consistency guardrails require this.
- Gossip delay injection uses the `SimClock` injected at harness boot.
- **Depends on**: US-01, US-02 (for typed row keys).

---

## US-05: Nondeterminism Traits + CI Lint Gate

### Problem

Whitepaper §21 states: "Every source of nondeterminism must be injectable. This is almost impossible to retrofit onto an existing system. Overdrive is designed with DST as a first-class constraint from day one." That claim is performative unless (a) every source of nondeterminism actually has a trait with a real and a sim implementation, and (b) the CI pipeline blocks PRs that smuggle `Instant::now()` or `rand::random()` into a core crate. Ana needs both — and she needs the lint gate's error messages to name the right trait and the right rule, because a gate that blocks a merge without explaining how to fix it trains engineers to bypass it.

### Who

- Overdrive platform engineer writing any new core-crate code | CI (automated enforcement).

### Solution

Ship `Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `Llm` traits, each with a real implementation (`SystemClock`, `TcpTransport`, `OsEntropy`, `EbpfDataplane` stub, `CloudHypervisorDriver` stub, `RigLlm` stub) and a sim implementation. Ship `cargo xtask dst-lint` as a blocking CI step that scans core crates for the banned-API list, with structured error output.

### Domain Examples

#### 1: Happy Path — Ana writes a reconciler that needs current time

Ana writes a function inside `overdrive-core` that needs to know the current simulated time. She takes `clock: &dyn Clock` as a parameter and calls `clock.now()`. The code compiles, DST passes, and the same function under the real system gets `SystemClock::now()` at runtime.

#### 2: Edge Case — Ana needs a new time-shaped method that isn't on the trait yet

Ana is writing a reconciler that wants to check "has it been at least 5 seconds since X." The `Clock` trait has `now()` but not a helper for duration-since. Ana's first instinct is `Instant::now() - some_instant`. She runs `cargo xtask dst-lint` locally and gets the error message. The remediation is clear: extend `Clock` with the method she needs, add a `SimClock` impl and a `SystemClock` impl, use it from her reconciler.

#### 3: Error Boundary — Ana copy-pastes a snippet that uses `std::thread::sleep`

Ana ports code from an old project into a core crate. The snippet contains `std::thread::sleep(Duration::from_millis(10))`. She opens a PR. CI runs `cargo xtask dst-lint`. The step fails with an error pointing at the line, naming `Clock::sleep` as the replacement, and linking to `.claude/rules/development.md`. Ana fixes the code, pushes, and CI goes green.

### UAT Scenarios (BDD)

#### Scenario: Every source of nondeterminism has a trait

Given the trait list `Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `Llm`
When the crate inventory of `overdrive-core::traits` is enumerated
Then every listed trait exists
And each trait has at least one real implementation and at least one sim implementation in a sibling crate

#### Scenario: Lint gate blocks Instant::now() in a core crate

Given a PR that inserts `std::time::Instant::now()` into a source file inside a crate labelled as core
When the CI step `cargo xtask dst-lint` runs
Then the step exits with non-zero status
And the output names the file, line, and column of the banned call
And the output names `Clock` as the replacement trait
And the output links to `.claude/rules/development.md`

#### Scenario: Lint gate blocks raw rand::random() in a core crate

Given a PR that inserts `rand::random::<u64>()` into a core crate
When `cargo xtask dst-lint` runs
Then the step fails and names `Entropy` as the replacement

#### Scenario: Lint gate blocks tokio::net::TcpStream::connect in a core crate

Given a PR that inserts `tokio::net::TcpStream::connect` into a core crate
When `cargo xtask dst-lint` runs
Then the step fails and names `Transport::connect` as the replacement

#### Scenario: Wiring crates may still use real implementations

Given the wiring crate that constructs `SystemClock`, `TcpTransport`, `OsEntropy` uses `Instant::now()` internally
And that crate is not labelled as core
When `cargo xtask dst-lint` runs
Then no violation is reported for that crate

#### Scenario: Lint gate is silent when core crates are clean

Given no core crate uses any banned API
When `cargo xtask dst-lint` runs
Then the step exits with zero status
And the output confirms zero violations

### Acceptance Criteria

- [ ] `Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `Llm` traits exist in `overdrive-core::traits`
- [ ] Each trait has one real implementation and one sim implementation, the latter deterministic under a seed (where relevant)
- [ ] `cargo xtask dst-lint` scans every crate labelled as core (mechanism documented in `development.md`)
- [ ] The banned-API list is a single constant in `xtask`; docs reference the constant rather than restating items
- [ ] Banned APIs at minimum: `std::time::Instant::now`, `std::time::SystemTime::now`, `rand::random`, `rand::thread_rng`, `tokio::time::sleep`, `std::thread::sleep`, `tokio::net::TcpStream`, `tokio::net::TcpListener`, `tokio::net::UdpSocket`
- [ ] Every violation message includes: file path, line, column, banned symbol, replacement trait, link to `development.md`
- [ ] The CI pipeline step runs `cargo xtask dst-lint` and blocks merges on non-zero exit
- [ ] A self-test in `xtask` asserts that each banned symbol is caught against a synthetic source file
- [ ] 0 false positives against wiring crates that legitimately use real impls

### Outcome KPIs

- **Who**: Overdrive platform engineer submitting PRs against core crates
- **Does what**: cannot merge a change that smuggles nondeterminism into a core crate
- **By how much**: 100% of smuggling attempts blocked; 0% false positives on wiring crates
- **Measured by**: CI step result; deliberate regression PR seeded weekly that attempts to smuggle a banned API; inspection of xtask self-test
- **Baseline**: no lint gate exists today; partial trait scaffolding exists in `crates/overdrive-core/src/traits/`

### Technical Notes

- The labelling mechanism for "core crate" must be stable and scriptable (candidate: `package.metadata.overdrive.crate_class = "core"` in each crate's `Cargo.toml`; xtask reads workspace metadata).
- Lint scanning can use `syn` or a ripgrep-style walker; the crafter picks. What matters is that the rules are enforced on every PR, not the implementation choice.
- The lint gate is a Tier-1 tool: it is expected to be fast (< few seconds on a fresh clone).
- **Depends on**: US-03 (for `IntentStore` trait as one of the required traits); US-04 (for `ObservationStore` trait).

---

## US-06: turmoil DST Harness + Core Invariants

### Problem

This is the acceptance gate for the entire Phase 1 feature. The whitepaper's §21 claim — "DST is a first-class constraint from day one" — is either real (engineers run `cargo xtask dst` and see green invariants on a simulated distributed cluster) or it is performance. Ana needs the turmoil-based harness wired up, composing the real `LocalStore` with all `Sim*` traits from US-04 and US-05, running a catalogue of invariants, reproducible from seed, and failing CI on red.

### Who

- Overdrive platform engineer running DST locally and reviewing CI output | CI (runs the harness on every PR).

### Solution

Build `overdrive-sim` crate with the Sim* trait implementations, the turmoil harness, and an invariant catalogue. Build `cargo xtask dst` as the entry point. Wire it into CI. Cover at minimum: `single_leader` (against a stubbed leader-election test topology), `intent_never_crosses_into_observation`, `snapshot_roundtrip_bit_identical`, `sim_observation_lww_converges`, `replay_equivalent_empty_workflow`, `entropy_determinism_under_reseed`.

### Domain Examples

#### 1: Happy Path — Ana runs `cargo xtask dst` on a clean clone

Ana clones the repo and runs `cargo xtask dst`. Cargo compiles. The harness boots a 3-node simulated cluster using real `LocalStore` + `SimObservationStore` + the five other Sim* traits. Invariants run to completion. Summary: `100 scenarios · 0 failures · 2.3s wall-clock`. The seed is printed on the summary line.

#### 2: Edge Case — Ana forces a partition scenario and watches invariants hold

The harness supports scenario scripts. Ana runs a scenario that partitions node-0 from the other two for 10 simulated seconds. The `single_leader` invariant holds throughout: at most one leader exists in any connected region at any tick. After healing, `all_nodes_agree_on_state` eventually fires green.

#### 3: Error Boundary — Ana hits a seed that triggers a real bug

Ana introduces a subtle bug: under certain timing, two leaders can be briefly elected. DST run 37 fails `single_leader`. The failure output prints: the invariant name, the seed, the tick (8743), the turmoil host where it fired, the cause, and a reproduction command with the seed embedded. Ana copies the reproduction command, runs it, and sees the same failure at the same tick. She bisects, finds the bug, fixes it, and re-runs — green.

### UAT Scenarios (BDD)

#### Scenario: Clean-clone `cargo xtask dst` is green within wall-clock budget

Given a clean clone of the repository
When Ana runs `cargo xtask dst`
Then every invariant in the default catalogue runs to completion
And the summary reports zero failures
And wall-clock time stays under 60 seconds on an M-class laptop
And the seed is printed in the summary

#### Scenario: Same-seed reproduction is bit-identical

Given Ana has captured seed S from a previous run on git SHA G and toolchain T
When Ana runs `cargo xtask dst --seed S` on the same G and T
Then the harness produces the same ordered invariant results
And every per-invariant tick number matches the previous run

#### Scenario: A failing invariant prints seed, tick, and reproduction command

Given a bug exists that will cause `single_leader` to fail at some tick
When Ana runs `cargo xtask dst`
Then the failure output includes the invariant name, the seed, the simulated tick, and the turmoil host
And the output contains a reproduction command with the seed embedded and `--only <invariant_name>`
And the `--only` flag value matches an enum variant exported from `overdrive-sim`

#### Scenario: The reproduction command reproduces at the same tick

Given Ana has a failing seed S and the failing invariant name I
When Ana runs the printed reproduction command
Then the harness fails on invariant I
And the failure happens at the same tick as the original run

#### Scenario: CI fails on DST red

Given the DST harness has failed at least one invariant
When the `cargo xtask dst` step completes in CI
Then the step exits with non-zero status
And the CI pipeline marks the PR as failed

#### Scenario: The harness composes real LocalStore with Sim* traits

Given the DST harness is running
When Ana inspects which implementations are wired up
Then `LocalStore` (backed by redb) is used for intent
And `SimObservationStore` is used for observation
And `SimClock`, `SimTransport`, `SimEntropy`, `SimDataplane`, `SimDriver`, `SimLlm` are used for their respective concerns

#### Scenario: The "intent never crosses into observation" invariant is `assert_always`

Given a running DST scenario
When the invariant `intent_never_crosses_into_observation` is checked on every tick
Then it holds for the entirety of the scenario

### Acceptance Criteria

- [ ] `overdrive-sim` crate exists with `Sim{Clock, Transport, Entropy, Dataplane, Driver, Llm, ObservationStore}` impls
- [ ] `cargo xtask dst` entry point exists and runs to completion green on a clean clone
- [ ] The harness composes real `LocalStore` (not a mock) with `SimObservationStore` plus every other Sim* trait
- [ ] The default invariant catalogue includes: `single_leader`, `intent_never_crosses_into_observation`, `snapshot_roundtrip_bit_identical`, `sim_observation_lww_converges`, `replay_equivalent_empty_workflow`, `entropy_determinism_under_reseed`
- [ ] Each invariant name is an enum variant in `overdrive-sim::invariants`; no inline strings in test output
- [ ] Seed is printed on every run
- [ ] `--seed <N>` reproduces bit-for-bit on the same git SHA and toolchain
- [ ] `--only <INVARIANT>` narrows the run to one invariant
- [ ] A failure prints: invariant name, seed, tick, host, cause, reproduction command
- [ ] The CI pipeline runs `cargo xtask dst` and blocks merges on non-zero exit
- [ ] Default wall-clock stays under 60s on a reference M-class laptop for the default invariant count
- [ ] A self-test (the harness's own test) runs the default suite twice on the same seed and asserts identical trajectories

### Outcome KPIs

- **Who**: Overdrive platform engineer
- **Does what**: runs the DST harness on a clean clone and uses the printed seed to reproduce failures
- **By how much**: 100% of red runs reproduce on the same git SHA and toolchain; DST wall-clock < 60s for the default catalogue
- **Measured by**: CI job duration; self-test asserting twin-run identity
- **Baseline**: no harness exists today; partial trait scaffolding in `crates/overdrive-core/src/traits/`

### Technical Notes

- Use `turmoil` as the harness foundation (whitepaper §21 + testing.md Tier 1).
- The harness is `LocalStore` + `SimObservationStore` — the `RaftStore` + real `CorrosionStore` combinations are out of scope for Phase 1.
- The `single_leader` invariant in Phase 1 may operate against a stubbed leader topology (since `RaftStore` is deferred) — document that assumption explicitly and retire it in Phase 2. The test exists to prove the invariant-machinery works, not to prove Raft correctness.
- Print output is designed for CLI readability per `nw-ux-tui-patterns`: progress counter, summary line last, errors answer "what / why / how to fix" and print the reproduction command inline.
- **Depends on**: US-01, US-02, US-03, US-04, US-05.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial six user stories for phase-1-foundation DISCUSS wave. |
