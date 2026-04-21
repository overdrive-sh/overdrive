# Journey Scenarios — Trust the Sim

Gherkin scenarios collected from `journey-trust-the-sim.yaml` for easier reading. These live as fenced markdown blocks per the project rule in `.claude/rules/testing.md`: **no `.feature` files anywhere**. The crafter translates these into Rust `#[test]` / `#[tokio::test]` functions in `crates/{crate}/tests/`.

> Per-story scenarios with full Example Mapping coverage live in `user-stories.md`. The scenarios here are the *journey-level* happy-paths that prove Step N → Step N+1 integration.

---

## Step 1 — Fresh clone, first DST run

```gherkin
Scenario: Clean-clone DST run is fast and green
  Given Ana has cloned the overdrive repository to a clean workspace
  And no environment variables override DST defaults
  When Ana runs "cargo xtask dst"
  Then the harness boots a 3-node simulated cluster using LocalStore
    (real redb) and SimObservationStore plus SimClock, SimTransport,
    SimEntropy, SimDataplane, SimDriver, and SimLlm
  And every invariant from the default suite runs to completion
  And the summary line reports 0 failures
  And wall-clock time stays under 60 seconds on Ana's M-class laptop
  And the seed for this run is printed in the summary

Scenario: The same seed produces the same trajectory
  Given Ana has just seen a green run with seed S
  When Ana runs "cargo xtask dst --seed S" again
  Then the harness produces the same ordered invariant results
  And the same summary line
  And no tick number changes between the two runs
```

---

## Step 2 — Write code that uses the intent store

```gherkin
Scenario: Newtype round-trip is lossless across Display, FromStr and serde
  Given JobId, NodeId, AllocationId, SpiffeId, Region, SchematicId,
    CorrelationKey, InvestigationId, PolicyId, ContentHash,
    and CertSerial each have a canonical string form
  When any valid instance is formatted via Display, parsed via
    FromStr, serialized via serde_json, and deserialized back
  Then the final value equals the original
  And the serde output matches the Display output byte-for-byte

Scenario: Invalid identifier input is rejected by the constructor
  Given a malformed input such as "spiffe://overdrive.local//job//alloc"
    (double slashes, empty segments) for SpiffeId
  When Ana calls SpiffeId::from_str on that input
  Then an Err is returned with a domain-appropriate ParseError variant
  And no SpiffeId instance is constructed

Scenario: LocalStore snapshot round-trip is bit-identical
  Given a LocalStore populated with a set of JobSpec entries
  When Ana calls export_snapshot, creates a second LocalStore, calls
    bootstrap_from with the snapshot, and calls export_snapshot again
  Then the second snapshot byte slice equals the first byte-for-byte

Scenario: Watch fires when a prefix-matching key is written
  Given a LocalStore with no entries
  And Ana has subscribed to watch for prefix "jobs/"
  When another task writes a key "jobs/payments" to the store
  Then the watch stream yields a single event for that key within one
    tick of simulated time
```

---

## Step 3 — CI lint gate catches a banned API

```gherkin
Scenario: Lint gate blocks a core crate that uses Instant::now()
  Given Ana has inserted "let now = std::time::Instant::now();" into
    a source file inside a crate labelled "core" (overdrive-core)
  When Ana runs "cargo xtask dst-lint"
  Then the command exits with a non-zero status
  And the output contains the exact file path, line, and column of the
    banned call
  And the output names the `Clock` trait as the replacement
  And the output references ".claude/rules/development.md"

Scenario: Lint gate is silent when core crates are clean
  Given no core crate contains any banned API
  When Ana runs "cargo xtask dst-lint"
  Then the command exits with zero status
  And the output reports that zero violations were found

Scenario: Non-core crates may still use real implementations
  Given the wiring crate that constructs production Clock and
    Transport instances legitimately calls std::time::Instant::now()
  And that crate is not labelled "core"
  When Ana runs "cargo xtask dst-lint"
  Then no violation is reported for that crate
```

---

## Step 4 — Real invariant failure reproduces bit-for-bit

```gherkin
Scenario: A failing invariant prints the seed and an exact reproduction command
  Given a bug has been introduced that allows two leaders to be elected
    after a partition heal
  When Ana runs "cargo xtask dst"
  Then the harness reports the failing invariant by name
  And the failure output contains the seed used for that run
  And the failure output contains a reproduction command that embeds
    the same seed and narrows to the failing invariant

Scenario: The reproduction command reproduces the failure bit-for-bit
  Given Ana has captured a failing seed from a previous red run
  And the git SHA and Rust toolchain are unchanged
  When Ana runs the reproduction command
  Then the harness fails on the same invariant
  And the failure happens at the same simulated tick
  And the failure happens on the same turmoil host

Scenario: A DST failure fails the CI job
  Given the DST harness has failed at least one invariant
  When the "cargo xtask dst" step completes in CI
  Then the step exits with a non-zero status
  And the CI pipeline is marked failed
```

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial journey-level scenarios for phase-1-foundation. Per-story scenarios in `user-stories.md`. |
