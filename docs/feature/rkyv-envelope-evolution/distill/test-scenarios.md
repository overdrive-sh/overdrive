# Acceptance test scenarios — rkyv-envelope-evolution

**Feature**: `rkyv-envelope-evolution`
**Wave**: DISTILL
**Designer**: Quinn (nw-acceptance-designer)
**Date**: 2026-05-12
**Source of truth**: DESIGN wave-decisions § 10 (7 scenarios — S-EV-02
split into S-EV-02a + S-EV-02b per review-revision M2) + ADR-0048

> **Note on format**: Per `.claude/rules/testing.md` § "Testing": this
> repository forbids `.feature` files. The Gherkin G/W/T blocks below
> are **specification only** — the crafter translates them into Rust
> `#[test]` / `#[tokio::test]` functions during DELIVER. Each scenario
> names its target Rust test file, the driving-port surface, and the
> exact types / error variants / log events to assert on.

## Glossary — driving ports for this feature

The "driving port" for this feature is the Rust API surface of each
host adapter and codec module, since this is a persistence-layer
refactor with no HTTP / CLI entry. Per the skill mandate, every WS
scenario is tagged `@driving_port`:

| Driving port | Surface (entry point production code calls) |
|---|---|
| `Envelope::latest(payload)` | `overdrive-core::codec::envelope::VersionedEnvelope::latest` |
| `Envelope::into_latest()` | `overdrive-core::codec::envelope::VersionedEnvelope::into_latest` |
| `LocalObservationStore::alloc_status_rows` | `overdrive-store-local::observation_backend` — read API |
| `LocalObservationStore::write_alloc_status` | `overdrive-store-local::observation_backend` — write API |
| `LocalStore::open(path)` | `overdrive-store-local::redb_backend` — bootstrap entrypoint |
| `xtask::dst_lint::scan_for_envelope_variant_construction` | `xtask/src/dst_lint.rs` — AST scanner |
| `xtask::dst_lint::scan_for_envelope_fixture_coverage` | `xtask/src/dst_lint.rs` — coverage gate |

---

## S-EV-01 — Per-envelope schema-evolution golden-bytes roundtrip

**Source**: DESIGN § 10 S-EV-01; testing.md § "Archive schema-evolution roundtrip"
**Driving port**: `VersionedEnvelope::into_latest`
**Tier**: Tier 1 (default-lane, pure-Rust)
**Tag**: `@happy_path @walking_skeleton (for AllocStatusRow only)`
**Test file**: `crates/overdrive-core/tests/schema_evolution/<envelope_snake>.rs` (×5)

### Preconditions (referenced from source)

- Envelope types defined in `crates/overdrive-core/src/traits/observation_store.rs` (lines 283, 392, 463, 494) and `crates/overdrive-core/src/aggregate/mod.rs:96-117`
- `VersionedEnvelope` trait at `crates/overdrive-core/src/codec/envelope.rs` (NEW — RED scaffold)
- Golden-bytes fixture rule from `.claude/rules/testing.md` § "Archive schema-evolution roundtrip"

### Sub-scenario S-EV-01.1 — `AllocStatusRowEnvelope` V1 decodes through current envelope

```gherkin
Scenario: A V1-archived AllocStatusRow decodes through the current envelope and projects to the latest payload shape
  Given a hex-encoded golden-bytes constant FIXTURE_V1 pinning the rkyv-archived
        AllocStatusRowEnvelope::V1(AllocStatusRowV1 {
            alloc_id: "alloc-test-01",
            workload_id: "svc-payments",
            node_id: "node-001",
            state: Running,
            updated_at: LogicalTimestamp { counter: 1, writer: "node-001" },
        })
  When the test hex-decodes FIXTURE_V1 to a Bytes buffer
    And the buffer is rkyv-deserialized into AllocStatusRowEnvelope
    And the envelope's into_latest() method is called
  Then the call returns Ok(AllocStatusRowLatest)
    And the returned Latest payload equals the canonical hand-pinned expected projection
        (V1 -> V2 via From<V1> for V2, defaulting V2-only fields)
```

### Sub-scenario S-EV-01.2 — `NodeHealthRowEnvelope` V1 roundtrip

```gherkin
Scenario: A V1-archived NodeHealthRow decodes through the current envelope
  Given a hex-encoded FIXTURE_V1 pinning NodeHealthRowEnvelope::V1(NodeHealthRowV1 {
            node_id: "node-001",
            region: "us-east-1",
            last_heartbeat: LogicalTimestamp { counter: 5, writer: "node-001" },
        })
  When the test hex-decodes FIXTURE_V1 and rkyv-deserializes into the envelope
    And the envelope's into_latest() method is called
  Then the call returns Ok(NodeHealthRowLatest)
    And every field on the returned payload equals the canonical expected value
```

### Sub-scenarios S-EV-01.3 / S-EV-01.4 / S-EV-01.5

Structurally identical to S-EV-01.1 / S-EV-01.2 for
`ServiceHydrationResultRowEnvelope`, `ServiceBackendRowEnvelope`, and
`JobEnvelope` respectively. The crafter generates each fixture by
constructing the canonical V1 payload, calling
`rkyv::to_bytes::<_, rkyv::rancor::Error>(&envelope)`, hex-encoding
the result, and pasting it as `const FIXTURE_V1: &str = "..."`.

### Failure mode the scenarios pin

If a future commit changes the archived layout of any payload variant
without minting a new variant (the exact failure mode that surfaced
2026-05-12 when fields were appended to `AllocStatusRow`), the
fixture's hex bytes no longer decode through the unchanged envelope
shape — the test fails with a structured `EnvelopeError::Malformed`
or an rkyv validator error. Per testing.md, existing fixtures are
**NEVER touched**; the crafter adds a new `FIXTURE_V2` and a new
assertion in the same commit.

---

## S-EV-02a — Layer-1 write enforcement: cross-crate payload unreachability

**Source**: DESIGN § 10 S-EV-02a (split per review-revision M2); ADR-0048 § 2 Layer 1
**Driving port**: rustc itself (compile-time enforcement)
**Tier**: trybuild compile-fail (default-lane)
**Tag**: `@enforcement @compile_fail`
**Test files**:
- `crates/overdrive-store-local/tests/compile_fail.rs` (entrypoint calling `trybuild::TestCases::new()`)
- `crates/overdrive-store-local/tests/compile_fail/alloc_status_row_payload_unreachable.rs` (fixture)
- `crates/overdrive-store-local/tests/compile_fail/alloc_status_row_payload_unreachable.stderr` (pinned diagnostic)

### Preconditions

- `AllocStatusRowV1` declared `pub(crate)` in `overdrive-core` (per ADR-0048 § 2 Layer 1; NEW — RED scaffold)
- `AllocStatusRowV1` NOT re-exported from `overdrive-core::lib.rs`

### Scenario

```gherkin
Scenario: A cross-crate writer cannot name the pub(crate) inner payload type to construct it
  Given the fixture source file at
        crates/overdrive-store-local/tests/compile_fail/alloc_status_row_payload_unreachable.rs
        contains the following body:
            use overdrive_core::traits::observation_store::AllocStatusRowEnvelope;
            use overdrive_core::traits::observation_store::AllocStatusRowV1;
            fn main() {
                let _: AllocStatusRowEnvelope = AllocStatusRowEnvelope::V1(
                    AllocStatusRowV1 {
                        alloc_id: AllocationId::default(),
                        workload_id: WorkloadId::default(),
                        node_id: NodeId::default(),
                        state: AllocState::Pending,
                        updated_at: LogicalTimestamp::default(),
                    });
            }
  When trybuild compiles the fixture against the production overdrive-core crate
  Then the compilation fails
    And the compiler diagnostic identifies the AllocStatusRowV1 type as private
        (rustc error code E0603 "private struct" OR E0432 "unresolved import")
    And the diagnostic explicitly names AllocStatusRowV1 as the offending identifier
    And the diagnostic does NOT mention the envelope variant V1 as the offending construct
        (Layer 1 blocks the payload type, not the variant constructor — per ADR-0048 § 2)
  And the pinned .stderr fixture file matches the captured diagnostic verbatim
```

### Why this is correct under ADR-0048 § 2

The fixture explicitly does NOT assert that the variant constructor
`AllocStatusRowEnvelope::V1(...)` is unreachable. Per ADR-0048 § 2
"What Layer 1 does NOT block": the envelope enum is `pub`, so the
variant constructor expression is syntactically reachable from any
crate. Layer 1 only blocks *constructing a payload value*
cross-crate. The fixture pins exactly that property.

One fixture is sufficient (the Layer 1 mechanism is uniform across all
five envelopes).

---

## S-EV-02b — Layer-2 write enforcement: in-crate variant-construction lint

**Source**: DESIGN § 10 S-EV-02b; ADR-0048 § 2 Layer 2
**Driving port**: `xtask::dst_lint::scan_for_envelope_variant_construction`
**Tier**: xtask unit test (default-lane; runs inside `xtask` crate)
**Tag**: `@enforcement`
**Test file**: `xtask/src/dst_lint.rs` `#[cfg(test)] mod tests` OR `xtask/tests/dst_lint_envelope.rs`

### Preconditions

- `scan_for_envelope_variant_construction` exists at `xtask::dst_lint::scan_for_envelope_variant_construction` (NEW — RED scaffold)
- Scanner is purely syntactic — operates on source strings, does NOT import any `overdrive-*` crate (per ADR-0048 § 2 Layer 2 "Why dst_lint, not a separate crate binary")

### Sub-scenario S-EV-02b.1 — In-crate violation flagged

```gherkin
Scenario: The scanner flags a forbidden in-crate variant construction
  Given a synthetic source string at simulated path
        "crates/overdrive-core/src/some_module.rs"
        containing the literal expression:
            AllocStatusRowEnvelope::V1(payload)
        outside any function named "into_latest" or any "impl From<...> for ..." block
  When xtask::dst_lint::scan_for_envelope_variant_construction is called with that source
  Then the scanner returns a non-empty Vec<Violation>
    And the violation's message names the file path "some_module.rs"
    And the violation's message names the offending pattern "AllocStatusRowEnvelope::V1"
    And the violation's line number is the line where the construction occurs
```

### Sub-scenario S-EV-02b.2 — Allowed call site inside `into_latest` is NOT flagged

```gherkin
Scenario: The scanner accepts the canonical envelope construction inside into_latest
  Given a synthetic source string containing:
            impl VersionedEnvelope for AllocStatusRowEnvelope {
                fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
                    match self {
                        Self::V1(v1) => Ok(v1.into()),
                        Self::V2(v2) => Ok(v2),
                    }
                }
            }
  When xtask::dst_lint::scan_for_envelope_variant_construction is called with that source
  Then the scanner returns an empty Vec<Violation>
    (the pattern Self::V1(v1) inside into_latest is the allowed call site)
```

### Sub-scenario S-EV-02b.3 — Allowed call site inside `impl From<V1Inner> for V2Inner` is NOT flagged

```gherkin
Scenario: The scanner accepts the From<V1> for V2 conversion
  Given a synthetic source string containing:
            impl From<AllocStatusRowV1> for AllocStatusRowV2 {
                fn from(v1: AllocStatusRowV1) -> Self {
                    AllocStatusRowV2 { /* ... */ }
                }
            }
  When xtask::dst_lint::scan_for_envelope_variant_construction is called with that source
  Then the scanner returns an empty Vec<Violation>
```

### Sub-scenario S-EV-02b.4 — Clean source produces no violations

```gherkin
Scenario: The scanner returns empty for source containing no envelope constructions
  Given a synthetic source string containing only unrelated Rust code
        (e.g. a struct definition with field accessors, no envelope expressions)
  When xtask::dst_lint::scan_for_envelope_variant_construction is called with that source
  Then the scanner returns an empty Vec<Violation>
```

---

## S-EV-03 — Intent fail-fast on unknown / malformed envelope

**Source**: DESIGN § 10 S-EV-05; ADR-0048 § 3 (intent layer) + § 6 (operator remediation)
**Driving port**: `LocalStore::open(path: &Path) -> Result<LocalStore, IntentStoreError>`
**Tier**: Tier 1 integration-tests-gated (real `redb` via `tempfile::TempDir`)
**Tag**: `@error_path @real-io @adapter-integration`
**Test file**: `crates/overdrive-store-local/tests/integration/envelope_intent_refuse.rs`

### Preconditions

- `IntentStoreError::Envelope { #[from] source: EnvelopeError }` variant exists on `IntentStoreError` (NEW — RED scaffold)
- `LocalStore::open` decodes the `entries` redb table values through `JobEnvelope::into_latest()` and propagates `EnvelopeError` as `IntentStoreError::Envelope` (NEW — RED scaffold)
- `health.startup.refused` event emitted via `tracing::error!(name: "health.startup.refused", ...)` before the function returns

### Sub-scenario S-EV-03.1 — Malformed bytes cause refuse-to-start

```gherkin
@error_path @real-io @adapter-integration
Scenario: Control-plane boot refuses to start when an intent row contains malformed bytes
  Given a tempfile-managed redb file at "{tmpdir}/intent.redb"
    And the file contains exactly one entry under the "entries" table:
          key: JobId "job-malformed-01"
          value: raw bytes b"\xff\xfe\xfd\xfc this is not a valid rkyv archive"
  When LocalStore::open("{tmpdir}/intent.redb") is invoked
  Then the call returns Err(IntentStoreError::Envelope { source })
    And the source variant matches EnvelopeError::Malformed { source: _ }
    And the IntentStoreError's Display form contains the literal substring "{tmpdir}/intent.redb"
    And the Display form contains the literal substring "delete"
        (operator remediation per ADR-0048 § 6)
    And a structured log event with name "health.startup.refused" was emitted before the function returned
    And the event's fields include redb_path = "{tmpdir}/intent.redb"
```

### Sub-scenario S-EV-03.2 — Unknown future variant tag causes refuse-to-start

```gherkin
@error_path @real-io @adapter-integration
Scenario: Control-plane boot refuses to start when an intent row carries an unknown future-version variant tag
  Given a tempfile-managed redb file at "{tmpdir}/intent.redb"
    And the file contains exactly one entry under the "entries" table whose value bytes
        carry a rkyv-archived envelope with discriminant byte 99
        (a tag value that does not correspond to any known JobEnvelope variant)
  When LocalStore::open("{tmpdir}/intent.redb") is invoked
  Then the call returns Err(IntentStoreError::Envelope { source })
    And the source variant matches EnvelopeError::UnknownVersion { observed: 99, supported_max: _ }
    And the Display form names the observed and supported_max version values
    And a structured "health.startup.refused" event was emitted with redb_path and envelope_error fields
```

### Note on synthesising the "unknown future variant" bytes

For S-EV-03.2 the crafter constructs the bytes by writing a small
test-only helper that emits an rkyv archive with a hand-crafted
discriminant byte at the canonical offset (per ADR-0048 § 1 the rkyv
enum tag is at offset 0). The helper lives in
`crates/overdrive-store-local/tests/integration/envelope_helpers.rs`
and is gated by `#[cfg(test)]`.

---

## S-EV-04 — Observation log + skip self-heal

**Source**: DESIGN § 10 S-EV-04; ADR-0048 § 3 (observation layer)
**Driving port**: `LocalObservationStore::alloc_status_rows` (and equivalents for the other three row types)
**Tier**: Tier 1 integration-tests-gated (real `redb` via `tempfile::TempDir`)
**Tag**: `@error_path @real-io @adapter-integration`
**Test file**: `crates/overdrive-store-local/tests/integration/envelope_observation_skip.rs`

### Preconditions

- `ObservationStoreError::Envelope { #[from] source: EnvelopeError }` variant exists (NEW — RED scaffold)
- `LocalObservationStore::alloc_status_rows` decodes through `AllocStatusRowEnvelope::into_latest()` and on `EnvelopeError` emits a structured `tracing::warn!(name: "observation.envelope.decode_failed", ...)` event, omits the offending row from the returned iterator, and returns `Ok(remaining_rows)` (NEW — RED scaffold)

### Sub-scenario S-EV-04.1 — Malformed AllocStatusRow row is skipped, valid rows survive

```gherkin
@error_path @real-io @adapter-integration
Scenario: A malformed alloc-status row is skipped and the convergence tick proceeds normally
  Given a tempfile-managed redb file at "{tmpdir}/observation.redb"
    And the file's "observation_alloc_status" table contains two entries:
          K1: a valid V1-archived AllocStatusRow envelope for alloc "alloc-01" in state Running
          K2: raw garbage bytes (16 bytes of 0xFF) — does not decode through the envelope
  When LocalObservationStore::alloc_status_rows is called on the open store
  Then the returned iterator yields exactly one AllocStatusRowLatest payload
    And the yielded payload's alloc_id equals "alloc-01"
    And the call returns Ok(iter) — no error propagates
    And a structured "observation.envelope.decode_failed" log event was emitted exactly once
    And the event's fields include table = "observation_alloc_status" and key referencing K2
    And no "health.startup.refused" event was emitted
        (asymmetric policy: observation degrades; only intent refuses to start)
```

### Sub-scenario S-EV-04.2 — Subsequent re-write of the same key recovers

```gherkin
@error_path @real-io @adapter-integration
Scenario: Re-writing the malformed key with a valid envelope re-enables reads
  Given the precondition state from S-EV-04.1 (one valid row K1, one malformed row K2)
    And a first call to alloc_status_rows yielded one row (K1) and logged the K2 decode failure
  When the test calls LocalObservationStore::write_alloc_status with a valid AllocStatusRow at K2
    And LocalObservationStore::alloc_status_rows is called again
  Then the returned iterator yields exactly two AllocStatusRowLatest payloads
    And no further "observation.envelope.decode_failed" event is emitted on this second read
```

### Sub-scenarios S-EV-04.3 / S-EV-04.4 / S-EV-04.5

Structurally identical to S-EV-04.1 for `NodeHealthRow`,
`ServiceHydrationResultRow`, and `ServiceBackendRow` respectively —
each table gets one malformed-row + one valid-row scenario asserting
the same skip + log + remaining-row-yield behaviour.

---

## S-EV-05 — Golden-bytes fixtures pin historical variants (subsumed by S-EV-01)

**Source**: DESIGN § 10 S-EV-01 + testing.md § "Archive schema-evolution roundtrip"

S-EV-05 is the *invariant* that S-EV-01 enforces structurally. Stated
explicitly as a check the DELIVER crafter must satisfy:

### Verification checklist (mechanical, no Gherkin)

1. For each of the 5 envelopes, exactly one fixture file exists at
   `crates/overdrive-core/tests/schema_evolution/<envelope_snake>.rs`.
2. Each file contains at least one `const FIXTURE_V<N>: &str = "..."` constant for every historical variant currently in the envelope enum.
3. Each file contains one `#[test]` function per fixture that hex-decodes the constant, rkyv-deserializes into the envelope, calls `into_latest()`, and asserts equality against a canonical hand-pinned `Latest` projection.
4. The test entrypoint `crates/overdrive-core/tests/schema_evolution.rs` exists and declares the five sub-modules.
5. **When a future commit adds variant `V<N+1>`** (per development.md § "Version-bump procedure" 6-step checklist), the same commit MUST:
   - Add a new `FIXTURE_V<N>: &str` constant pinning the *previous* version's archived bytes (note: the index is `V<N>`, not `V<N+1>` — the prior version's bytes are what's being pinned).
   - NOT modify any pre-existing `FIXTURE_V<M>` constant for any `M < N`.
6. The xtask coverage gate in S-EV-06 enforces clause (1) and (2) above mechanically.

### Why this is not a Gherkin scenario

The "test" here is the existence of the fixture files and their
constants. It is verified structurally by S-EV-06 (coverage gate)
plus the fact that the scenarios in S-EV-01 cannot exist without the
fixtures. Treating it as a separate runnable scenario would
duplicate either S-EV-01 (the fixture executes) or S-EV-06 (the
fixture file exists).

---

## S-EV-06 — dst-lint coverage gate: every envelope has a fixture

**Source**: DESIGN § 10 — implicit in "structural defense against drift"; ADR-0048 § 6 testing
**Driving port**: `xtask::dst_lint::scan_for_envelope_fixture_coverage`
**Tier**: xtask unit test (default-lane)
**Tag**: `@enforcement`
**Test file**: `xtask/src/dst_lint.rs` `#[cfg(test)] mod tests` OR `xtask/tests/dst_lint_fixture_coverage.rs`

### Preconditions

- `scan_for_envelope_fixture_coverage(crate_root: &Path) -> Vec<Violation>` exists at `xtask::dst_lint::scan_for_envelope_fixture_coverage` (NEW — RED scaffold)
- Scanner walks `<crate_root>/src/` for `enum *Envelope` definitions whose variants follow the `V<N>(<PayloadType>)` pattern
- Scanner walks `<crate_root>/tests/schema_evolution/` for `.rs` files containing `FIXTURE_V<N>: &str` constants
- Scanner is purely syntactic — does NOT import any `overdrive-*` crate

### Sub-scenario S-EV-06.1 — Envelope without fixture file is flagged

```gherkin
Scenario: The coverage scanner flags an envelope that has no matching fixture file
  Given a synthetic crate-root directory at "{tmpdir}/fake_crate/"
    And "{tmpdir}/fake_crate/src/lib.rs" contains:
            pub enum FooEnvelope { V1(FooV1), V2(FooV2) }
            pub(crate) struct FooV1 { /* fields */ }
            pub(crate) struct FooV2 { /* fields */ }
    And "{tmpdir}/fake_crate/tests/schema_evolution/" does NOT contain a "foo.rs" file
  When xtask::dst_lint::scan_for_envelope_fixture_coverage("{tmpdir}/fake_crate") is called
  Then the scanner returns a non-empty Vec<Violation>
    And the violation's message names "FooEnvelope" as the envelope without a fixture file
    And the violation's message includes the expected fixture-file path
        "tests/schema_evolution/foo.rs"
```

### Sub-scenario S-EV-06.2 — Envelope with fixture file but missing variant fixture is flagged

```gherkin
Scenario: The coverage scanner flags an envelope whose fixture file is missing a variant's FIXTURE_V<N>
  Given the synthetic crate-root from S-EV-06.1
    And "{tmpdir}/fake_crate/tests/schema_evolution/foo.rs" exists and contains:
            const FIXTURE_V1: &str = "deadbeef";
            #[test] fn v1_decodes() { /* ... */ }
        (V1 is pinned but V2 is not)
  When xtask::dst_lint::scan_for_envelope_fixture_coverage("{tmpdir}/fake_crate") is called
  Then the scanner returns a non-empty Vec<Violation>
    And the violation's message names "FooEnvelope::V2" as the variant without a fixture
    And the violation's message includes "FIXTURE_V2" as the expected constant name
```

### Sub-scenario S-EV-06.3 — Complete coverage produces no violations

```gherkin
Scenario: The coverage scanner accepts an envelope with all variants pinned
  Given the synthetic crate-root from S-EV-06.1
    And "{tmpdir}/fake_crate/tests/schema_evolution/foo.rs" contains:
            const FIXTURE_V1: &str = "deadbeef";
            const FIXTURE_V2: &str = "cafef00d";
            #[test] fn v1_decodes() { /* ... */ }
            #[test] fn v2_decodes() { /* ... */ }
  When xtask::dst_lint::scan_for_envelope_fixture_coverage("{tmpdir}/fake_crate") is called
  Then the scanner returns an empty Vec<Violation>
```

### Sub-scenario S-EV-06.4 — Real production tree passes

```gherkin
Scenario: The real overdrive-core crate-root passes the coverage gate
  Given the production crate root at
        "/Users/marcus/conductor/workspaces/helios/new-york-v1/crates/overdrive-core"
    And all five expected envelope types defined in src/
        (AllocStatusRowEnvelope, NodeHealthRowEnvelope,
         ServiceHydrationResultRowEnvelope, ServiceBackendRowEnvelope,
         JobEnvelope)
    And all five expected fixture files exist under tests/schema_evolution/
        with at least FIXTURE_V1 each
  When xtask::dst_lint::scan_for_envelope_fixture_coverage on the real crate root
  Then the scanner returns an empty Vec<Violation>
```

### Closing-the-loop guarantee

S-EV-06 is the *only* scenario that prevents a future PR from adding
a new envelope and forgetting the fixture. Without it, the structural
defense degrades to "did the author remember to add a fixture file?"
— a non-mechanical check that fails open under reviewer fatigue.

---

## Cross-scenario summary

| Scenario | Count of sub-scenarios | Happy / Error / Enforcement |
|---|---|---|
| S-EV-01 | 5 (one per envelope) | Happy path (golden-bytes roundtrip) |
| S-EV-02a | 1 | Enforcement (compile-fail) |
| S-EV-02b | 4 (violation + 3 allow cases) | Enforcement |
| S-EV-03 | 2 (malformed + unknown-future) | Error path (intent refuse-to-start) |
| S-EV-04 | 5 (one per row type + recovery) | Error path (observation skip + self-heal) |
| S-EV-05 | (verification checklist, no runnable test) | Subsumed by S-EV-01 + S-EV-06 |
| S-EV-06 | 4 (3 synthetic + 1 real-tree) | Enforcement (coverage gate) |

**Total runnable sub-scenarios**: 21 (5 + 1 + 4 + 2 + 5 + 4).
**Happy-path count**: 5 (S-EV-01).
**Error-path count**: 7 (S-EV-03 + S-EV-04).
**Enforcement count**: 9 (S-EV-02a + S-EV-02b + S-EV-06).
**Error + enforcement / total**: 16/21 ≈ 76% — well above the 40% minimum.

**Walking-skeleton scenarios**: 1 (S-EV-01.1 alloc_status_row +
S-EV-04.1 observation skip + S-EV-03.1 intent refuse, all tagged
`@walking_skeleton @driving_port`); see `walking-skeleton.md`.

**Adapter integration scenarios** (`@real-io @adapter-integration`):
S-EV-03.1, S-EV-03.2, S-EV-04.1, S-EV-04.2 (and the four parallels
for the remaining observation row types). All driven adapters
(`LocalStore`, `LocalObservationStore`) covered.

---

## Cross-scenario traceability

| ADR-0048 § | DESIGN § 10 ID | DISTILL scenario | Mandate |
|---|---|---|---|
| § 1 (per-type rkyv enum) | S-EV-01 | S-EV-01.1 .. .5 | Coverage; observable behaviour |
| § 2 Layer 1 (pub(crate)) | S-EV-02a | S-EV-02a | Compile-time enforcement |
| § 2 Layer 2 (dst-lint) | S-EV-02b | S-EV-02b.1 .. .4 | In-crate enforcement |
| § 3 Intent fail-fast | S-EV-05 | S-EV-03.1 + .2 | Asymmetric read policy |
| § 3 Observation degrade | S-EV-04 | S-EV-04.1 .. .5 | Asymmetric read policy |
| § 6 Operator remediation | S-EV-05 | S-EV-03.1 (Display assertion) | Operator-facing observable |
| testing.md § "Archive schema-evolution roundtrip" | (S-EV-01) | S-EV-05 verification checklist + S-EV-06 | Coverage gate |
| development.md § "Version-bump procedure" | implicit | S-EV-05 clause 5 | Procedural |

Every DESIGN § 10 handoff scenario is mapped to at least one runnable
DISTILL sub-scenario.
