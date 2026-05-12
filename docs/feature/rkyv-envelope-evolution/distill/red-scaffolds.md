# RED scaffolds — rkyv-envelope-evolution

**Feature**: `rkyv-envelope-evolution`
**Wave**: DISTILL
**Date**: 2026-05-12

Per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
failing commits": every production type / trait / scanner the
acceptance tests reference must exist as a compilable RED scaffold
before DELIVER starts. The scaffolds are an executable specification
of work-not-yet-done. Each carries a `// SCAFFOLD: true` marker and a
`todo!("RED scaffold: <one-line spec>")` body gated by
`#[expect(clippy::todo, reason = "RED scaffold; lands GREEN in DELIVER")]`.

DISTILL produces the SPECIFICATION of these scaffolds (this document).
The DELIVER software-crafter creates the actual scaffold commit
before writing the first acceptance test body. This split keeps the
SSOT discipline: DISTILL says *what must exist*, DELIVER says *what
each one does*.

> Crafter note: per `.claude/rules/development.md` § "rkyv schema
> evolution" (as amended 2026-05-12 UI-01 reconciliation; further
> reconciled 2026-05-12 UI-02 — **alias-to-payload public API**):
>
> - **Public alias points at the payload struct**, not the envelope:
>   `pub type AllocStatusRow = AllocStatusRowV1` (NOT
>   `= AllocStatusRowEnvelope`). Public callers continue to construct
>   `AllocStatusRow { ... }` (= V1 payload) with struct-literal syntax
>   exactly as before — no `.latest(...)` wrapping at call sites.
> - **The envelope enum (`AllocStatusRowEnvelope`) is codec-internal**.
>   It is declared `pub` but **MUST NOT be re-exported** from the
>   crate root (`overdrive-core::lib.rs`). Cross-crate code reaching
>   for the envelope finds only the verbose
>   `overdrive_core::traits::observation_store::AllocStatusRowEnvelope`
>   path, signposted as "reaching into the persistence boundary."
> - **Inner payload types** (`AllocStatusRowV1`, …) are declared
>   `pub` (NOT `pub(crate)` — rustc E0446 forbids that under a `pub`
>   trait's `type Latest = <Payload>;`) and likewise **NOT re-exported
>   from the crate root** as defense in depth.
> - **The persistence-boundary code** (`LocalObservationStore::write_alloc_status`,
>   `LocalStore::open` / `write_entry`) is the sole call site that
>   names the envelope and uses `Envelope::latest(payload)` / `envelope.into_latest()?`.
> - The non-re-export discipline is the load-bearing convention surface
>   for Layer 1; the dst-lint clause (Layer 2, Group 5) is the structural
>   enforcement gate.

---

## Group 1 — Codec module (new)

**Path**: `crates/overdrive-core/src/codec/envelope.rs` (the `codec`
module does not yet exist — also create `crates/overdrive-core/src/codec/mod.rs`
re-exporting `envelope`)

### 1.1 — `VersionedEnvelope` trait

```rust
// SCAFFOLD: true
pub trait VersionedEnvelope {
    type Latest;

    /// RED scaffold spec: construct the envelope wrapping the latest payload.
    /// GREEN: every implementer wraps payload into the highest variant.
    fn latest(payload: Self::Latest) -> Self;

    /// RED scaffold spec: up-convert through historical From impls to Latest.
    /// GREEN: each implementer matches on its variants and converts via From.
    fn into_latest(self) -> Result<Self::Latest, EnvelopeError>;
}
```

### 1.2 — `EnvelopeError` enum

```rust
// SCAFFOLD: true
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    /// Bytes decoded to a variant tag this binary does not know.
    #[error("envelope carries unknown version tag {observed} (this binary supports up to V{supported_max})")]
    UnknownVersion { observed: u8, supported_max: u8 },

    /// Bytes did not decode as the envelope at all.
    #[error("envelope bytes are malformed: {source}")]
    Malformed {
        #[source]
        source: rkyv::rancor::Error,
    },
}
```

---

## Group 2 — Per-envelope enums + payload types

**Path**: `crates/overdrive-core/src/traits/observation_store.rs`
(replacing/wrapping the existing four row types at lines 283, 392,
463, 494)

> **Inner-payload visibility (amended 2026-05-12 UI-01 reconciliation;
> further reconciled 2026-05-12 UI-02 — alias-to-payload).**
> Inner payload types are declared `pub`, NOT `pub(crate)`. The
> literal `pub(crate)` declaration fails to compile with **rustc
> E0446** because the `pub trait VersionedEnvelope`'s
> `type Latest = <PayloadV1>;` associated-type assignment exposes
> the payload as part of the trait's public surface, and rustc
> rejects a `pub(crate)` type referenced from a `pub` trait.
>
> Under UI-02 (alias-to-payload) the **publicly re-exported name**
> from `overdrive-core` is `AllocStatusRow` (= `AllocStatusRowV1`,
> the payload struct). The codec-internal envelope enum
> (`AllocStatusRowEnvelope`) is `pub` but NOT re-exported from the
> crate root; inner payload variant types (`AllocStatusRowV1`,
> future `AllocStatusRowV2`) are likewise `pub` but un-re-exported
> as defense in depth. Cross-crate writers see only the payload
> alias `AllocStatusRow` at the short path; reaching the envelope
> name requires the verbose
> `overdrive_core::traits::observation_store::AllocStatusRowEnvelope`
> path, signposted as "reaching into the persistence boundary."
>
> The Layer 1 enforcement mechanism is therefore **non-re-export of
> the envelope enum from `overdrive-core::lib.rs`** (a code-review
> convention; the envelope is the codec-internal name a cross-crate
> writer would need in order to call `<Envelope>::V<N>(payload)` or
> `<Envelope>::latest(payload)`), with payload non-re-export as
> defense in depth. The structural defense against drift is Layer 2
> (the dst-lint clause; see Group 5).

### 2.1 — `AllocStatusRowEnvelope` (codec-internal) + `AllocStatusRow` (public payload alias)

```rust
// SCAFFOLD: true — codec-internal envelope; NOT re-exported from crate root
#[derive(Debug, Clone, PartialEq, Eq,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum AllocStatusRowEnvelope {
    V1(AllocStatusRowV1),
    // Future: V2(AllocStatusRowV2)
}

// Public payload alias — callers continue to use `AllocStatusRow { ... }`
// with struct-literal syntax (UI-02 amendment, 2026-05-12).
pub type AllocStatusRow = AllocStatusRowV1;
pub type AllocStatusRowLatest = AllocStatusRowV1;

// SCAFFOLD: true — pub per ADR-0048 § 2 Layer 1 (rustc E0446 forbids
// literal pub(crate); enforcement is non-re-export from lib.rs as
// defense in depth — the load-bearing non-re-export target is the
// envelope enum above).
#[derive(Debug, Clone, PartialEq, Eq,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AllocStatusRowV1 {
    pub alloc_id: AllocationId,
    pub workload_id: WorkloadId,
    pub node_id: NodeId,
    pub state: AllocState,
    pub updated_at: LogicalTimestamp,
    pub reason: Option<TransitionReason>,
    pub detail: Option<String>,
    pub terminal: Option<TerminalCondition>,
    pub stderr_tail: Option<String>,
    pub kind: WorkloadKind,
    pub listeners: Vec<ListenerRow>,
}

impl VersionedEnvelope for AllocStatusRowEnvelope {
    type Latest = AllocStatusRowV1;

    fn latest(payload: Self::Latest) -> Self {
        todo!("RED scaffold: wrap payload into Self::V1(payload)")
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        todo!("RED scaffold: match Self::V1(v1) => Ok(v1)")
    }
}
```

> **Crafter note on the wrapping site (UI-02).** The persistence-
> boundary code (`LocalObservationStore::write_alloc_status` and the
> read paths) is the ONLY site that names `AllocStatusRowEnvelope` —
> writes wrap via `AllocStatusRowEnvelope::latest(row.clone())`,
> reads project via `envelope.into_latest()?`. Non-boundary code
> (handlers, CLI commands, internal helpers) takes
> `&AllocStatusRow` (= `&AllocStatusRowV1`) and never names the
> envelope.

### 2.2 — `NodeHealthRowEnvelope` (same shape; wraps existing `NodeHealthRow` as `NodeHealthRowV1`)

### 2.3 — `ServiceHydrationResultRowEnvelope` (same shape; wraps existing `ServiceHydrationResultRow` as `V1`)

### 2.4 — `ServiceBackendRowEnvelope` (same shape; wraps existing `ServiceBackendRow` as `V1`)

---

## Group 3 — Intent aggregate envelope

**Path**: `crates/overdrive-core/src/aggregate/mod.rs` (wrapping the
existing `Job` at lines 96-117)

### 3.1 — `JobEnvelope` (codec-internal) + `Job` (public payload alias)

```rust
// SCAFFOLD: true — codec-internal envelope; NOT re-exported from crate root
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum JobEnvelope {
    V1(JobV1),
}

// Public payload alias — callers continue to use `Job { ... }`
// with struct-literal syntax (UI-02 amendment, 2026-05-12). No
// `Job::latest(JobLatest { ... })` rewrite at call sites; the
// `LocalStore` persistence boundary is the sole wrapping site.
pub type Job = JobV1;
pub type JobLatest = JobV1;

// pub per ADR-0048 § 2 Layer 1 (rustc E0446 forbids literal pub(crate);
// enforcement is non-re-export from overdrive-core::lib.rs as defense
// in depth — the load-bearing non-re-export target is JobEnvelope).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct JobV1 {
    pub id: WorkloadId,
    pub replicas: NonZeroU32,
    pub resources: Resources,
    pub driver: WorkloadDriver,
}
```

> **No call-site rename under UI-02 amendment.** Today `Job` is the
> public struct via `pub struct Job { ... }`. After the envelope lands,
> `Job` becomes a type alias `pub type Job = JobV1` (= the payload
> struct), so every existing struct-literal `Job { id, replicas,
> resources, driver }` construction across `overdrive-store-local`,
> `overdrive-control-plane`, `overdrive-cli`, and fixtures continues
> to compile unchanged. The persistence boundary — `LocalStore::open`
> (read via `JobEnvelope::into_latest()`), `LocalStore::write_entry`
> (write via `JobEnvelope::latest(job.clone())`) — is the SOLE site
> that names `JobEnvelope`. Per ADR-0048 § 4 (outer-envelope-only)
> embedded types `WorkloadDriver` and `Exec` are NOT wrapped.

---

## Group 4 — Error variants on existing error enums

### 4.1 — `IntentStoreError::Envelope`

**Path**: `crates/overdrive-core/src/traits/intent_store.rs` (existing file)

```rust
// SCAFFOLD: true
#[error("intent envelope decode failed for {redb_path}: {source}. Remediation: delete {redb_path} and restart the control-plane")]
Envelope {
    redb_path: PathBuf,
    #[from]
    #[source]
    source: EnvelopeError,
},
```

### 4.2 — `ObservationStoreError::Envelope`

**Path**: `crates/overdrive-core/src/traits/observation_store.rs`

```rust
// SCAFFOLD: true
#[error("observation envelope decode failed: {source}")]
Envelope {
    #[from]
    #[source]
    source: EnvelopeError,
},
```

---

## Group 5 — xtask scanner functions

**Path**: `xtask/src/dst_lint.rs` (existing file — extend; no
`overdrive-*` import per ADR-0048 § 2 Layer 2)

### 5.1 — `scan_for_envelope_variant_construction`

```rust
// SCAFFOLD: true
pub fn scan_for_envelope_variant_construction(
    source: &str,
    path: &Path,
) -> Vec<Violation> {
    todo!(
        "RED scaffold: AST-walk `source` for <Envelope>::V<N>( call expressions \
         outside `fn into_latest` or `impl From<...V<N>...> for ...V<N+1>...` blocks. \
         Return one Violation per offending site with the file path, line number, \
         and offending pattern."
    );
}
```

### 5.2 — `scan_for_envelope_fixture_coverage`

```rust
// SCAFFOLD: true
pub fn scan_for_envelope_fixture_coverage(
    crate_root: &Path,
) -> Vec<Violation> {
    todo!(
        "RED scaffold: walk <crate_root>/src/ for `enum *Envelope` definitions \
         with `V<N>(<Payload>)` variants. For each found envelope, verify a file \
         exists at <crate_root>/tests/schema_evolution/<envelope_snake>.rs and \
         contains `FIXTURE_V<N>: &str` for every variant. Return one Violation \
         per missing fixture file or constant."
    );
}
```

---

## Crafter compile-cleanliness checklist (before opening DELIVER PR)

After landing the RED scaffolds:

1. `cargo xtask lima run -- cargo check --workspace --all-targets` passes.
2. `cargo xtask lima run -- cargo nextest run --workspace --features integration-tests --no-run` compiles all test binaries.
3. All test files in `crates/overdrive-core/tests/schema_evolution/` exist with `#[test] #[should_panic(expected = "RED scaffold")] fn ...()` bodies that panic with the per-scenario message.
4. `cargo xtask dst-lint` runs (may produce known scaffold panics — fine for the scaffolding commit; assertion to lift in the first GREEN slice).
5. Pre-commit lefthook passes (no `--no-verify`).

Once all five hold, the crafter opens the GREEN slice 1 (Walking
Skeleton — `AllocStatusRowEnvelope` V1 roundtrip per
`walking-skeleton.md`).
