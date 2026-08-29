# Rust Discipline

Language- and idiom-level Rust conventions for the Overdrive codebase —
type-driven design, allocation strategy, lifetimes, ordered collections,
TOCTOU-atomicity, error handling, newtypes, async discipline, and the
store/key patterns below. These override defaults for agents working in this
repo, and they are **timeless discipline** — the principle plus the symptoms
that betray its violation, not the status of any one fix.

---

## A key prefix and its record classifier are ONE SSOT — never hand-enumerate a sub-key skip-list

When a keyed store holds a **family** of keys under a shared prefix
(`workloads/`, `workflows/`, `nodes/`, `allocations/`) in which the
canonical *body* record coexists with **sibling sub-key sentinels** —
`workloads/<id>` alongside `workloads/<id>/stop`, `/kind`, `/generation`,
… — then two things are load-bearing SSOT and MUST live in exactly one
place (the key type, e.g. `IntentKey`) and be consumed by every scan/skip
site:

1. **The prefix literal.** Expose `IntentKey::workload_prefix()` (mirroring
   the existing `workflow_instance_prefix()`); never re-spell `b"workloads/"`
   at the scan site. The `intent_key_canonical` acceptance test already
   enforces that each prefix literal appears in exactly one production file —
   a re-spelled literal is a test failure, not a style nit.
2. **The "is this the canonical body?" classifier.** Expose a *structural*
   predicate — `IntentKey::is_canonical_workload_record(key)` — that returns
   true iff the suffix after the prefix is non-empty and contains no `/`.
   Every scan site skips non-canonical keys through THIS predicate.

**Never hand-enumerate the sub-key skip list** — `key.ends_with("/stop") ||
key.ends_with("/kind")`. A hand-enumerated list is a **closed set that
silently goes stale the moment a new sibling sub-key is added**: the new
sub-key is not skipped, and the consumer mis-decodes its sentinel / scalar
value as the canonical body. A structural predicate (the suffix contains no
`/`) excludes every *current and future* sibling with no edit — the drift is
made **unrepresentable** rather than merely discouraged (§ "Type-driven
design", `development.md`).

**Corollary — one scan+decode helper, not N copies.** The loop that scans
the prefix, classifies canonical records, and decodes each body belongs in
one shared helper (`overdrive_core::aggregate::scan_workload_intents`) so
the prefix, the classifier, and the decode cannot drift apart across
consumers. A site that *cannot* call the shared async helper because it runs
before the store object exists — the store's own boot-validation walks in
`redb_backend.rs` — still shares the *predicate*. The classification is the
part that drifts, so it is the part that must be centralized even where the
loop itself cannot be.

**The soundness of a structural classifier rests on the id grammar.**
`is_canonical_workload_record` is only safe to key on "the suffix contains
no `/`" because `WorkloadId`'s grammar (`validate_label`) forbids `/` in an
id — so no canonical `<id>` is ever wrongly excluded. Before writing a
`/`-based (or any separator-based) classifier for a new key family, confirm
the id newtype cannot contain the separator; if it can, the predicate is
wrong and must key on something the grammar guarantees.

### Why

A hand-enumerated skip list is a **cached derivation** of "which keys under
this prefix are sub-keys" — exactly the anti-pattern § "Persist inputs, not
derived state" (`development.md`) forbids one layer down. The input is the
key's *structure* (does the suffix carry a separator?); the enumerated
`{/stop, /kind}` list is a stale snapshot of today's sibling set. When the
sibling set grows, the snapshot is silently wrong — and because the consumer
is usually a decode-or-refuse boundary, the failure surfaces two layers
downstream as an unexplained refusal, never as "you forgot to add
`/generation` to a list."

### Symptoms during review

- A prefix byte/string literal (`b"workloads/"`, `"workloads/"`) spelled at
  a scan site instead of `IntentKey::<family>_prefix()`.
- A skip guard that `ends_with("/stop") || ends_with("/kind")` (or any
  enumerated suffix list) instead of a structural predicate — the closed set
  that goes stale.
- The same "scan prefix → keep canonical → decode body" loop copied across
  two or more files, with the classification spelled slightly differently in
  each (one uses `suffix.contains('/')`, another enumerates suffixes) — the
  divergence IS the latent drift.
- A new `IntentKey::for_<family>_<subkey>()` constructor added without a
  matching thought for every place that scans the family prefix. If the
  classifier is structural and centralized, no such thought is needed; if a
  skip list is enumerated anywhere, the new sub-key just broke it.

### Precedent

`workloads/<id>/generation` (ADR-0073) is written by `overdrive workload
restart` as a persistent, monotonic `IncrementU64` and never deleted. Three
reconciler-side scanners classified canonical records structurally
(`suffix.contains('/')`) and were immune; the two `LocalIntentStore`
boot-validation walks (`open` / `bootstrap_from`) hand-enumerated
`{/stop, /kind}` and were never updated when `/generation` landed. Every
control-plane boot *after any workload restart* then hit the 8-byte
generation value, decoded it as a `WorkloadIntent` envelope, failed
(`InvalidSubtreePointer`), and refused with `health.startup.refused`.

The fix centralized the prefix (`IntentKey::workload_prefix`), the
classifier (`IntentKey::is_canonical_workload_record`, structural
`/`-exclusion), and the scan loop
(`overdrive_core::aggregate::scan_workload_intents`) as one SSOT consumed by
all five sites; the regression lock is
`crates/overdrive-store-local/tests/acceptance/reopen_after_generation_bump.rs`.
The reproduction was confirmed by reverting the walk to the enumerated skip
list (RED: `Envelope { … InvalidSubtreePointer }`) and restoring the
predicate (GREEN).

---

## Type-driven design

Guiding principle: **make invalid states unrepresentable.** Lean on the type
system to enforce correctness at compile time rather than runtime
validation.

### Sum types over sentinels

Use `enum` to model mutually exclusive states explicitly. Do not use
sentinels (`None`, `-1`, empty `Vec`) to carry semantic meaning.

```rust
// Bad — ambiguous: does None mean "not yet computed" or "no match"?
struct Alloc {
    placement: Option<NodeId>,
}

// Good — every state is explicit
enum Placement {
    Pending,
    Scheduled(NodeId),
    Rejected { reason: String },
    Failed { error: DriverError },
}
```

This compounds with the newtype rules below: once a concept has a
dedicated type, its states should be a dedicated sum type.

### Label enums own their string representation

When an enum's variants have a canonical lowercase string form (for
display, serialization keys, metric labels, CLI output), the
`as_str(&self) -> &'static str` method lives on the enum itself — not
as a free function at the call site. The enum is the SSOT for its own
vocabulary; scattering `match` arms across consumer crates duplicates
the mapping and drifts the moment a variant is added.

```rust
// Bad — free function in a consumer crate
const fn probe_role_label(role: ProbeRole) -> &'static str {
    match role {
        ProbeRole::Startup => "startup",
        ProbeRole::Readiness => "readiness",
        ProbeRole::Liveness => "liveness",
    }
}

// Good — method on the enum
impl ProbeRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Readiness => "readiness",
            Self::Liveness => "liveness",
        }
    }
}
```

This applies to every enum whose variants map 1:1 to static string
labels — `ProbeRole`, `WorkloadKind`, `DropClass`, status enums, etc.
`Display` may delegate to `as_str`; `as_str` is the primitive.

---

## Allocation strategy

Two complementary tools. Pick the right one for the lifetime shape of the data.

### Arena allocation (bumpalo) — reconciler scratch

Use for short-lived intermediate state whose lifetime is bounded by a single
reconcile iteration, a single request, or a single workflow step.

```rust
fn reconcile(intent: &IntentNode) -> Result<Vec<Action>> {
    let bump = Bump::new();
    let parsed  = parse_spec_into(&bump, &intent.spec)?;
    let actual  = fetch_actual_into(&bump, &intent.id)?;
    let diff    = compute_diff(&bump, &parsed, &actual);
    emit_actions(&diff)     // only the returned actions escape
    // bump drops here — all intermediates freed in one pointer reset
}
```

**When to reach for it:**
- Reconciler hot path (diff buffers, derived intermediate state, per-iteration
  scratch).
- Per-request work in the gateway or sidecar handler chain.
- Per-investigation work in the SRE agent (tool-call buffers, prompt
  assembly).

**When NOT to use it:**
- Anything that needs to outlive the iteration. Cache entries, reconciler
  memory rows, workflow journal entries — these go on the global heap or in
  the per-primitive libSQL store.
- Types with non-trivial `Drop` (file handles, sockets, other RAII
  resources). Bump skips destructors by default; you'll leak.
- Async scopes that span multiple reconcile iterations or `.await` points
  outside the arena's scope. Keep the arena within a single synchronous span
  or single async task.
- I/O-bound work. If the bottleneck is the syscall, not the allocator,
  bumpalo buys nothing.

### Zero-copy deserialization (rkyv) — persistent inputs

Use for durable data that reconcilers *read* — IntentStore rows, Raft log
entries, Corrosion row payloads, incident-memory blobs. rkyv encodes the
in-memory layout directly; readers access `&ArchivedT` against mmap'd bytes
without a deserialization pass.

**When to reach for it:**
- Any hot-path read out of redb, the Raft log, or Corrosion row values.
- Archived telemetry events in-flight (pre-Parquet).
- Incident-memory blobs retrieved for LLM context assembly.

**When NOT to use it:**
- External wire formats (gRPC, REST, OTel export) — stay with serde +
  protobuf/JSON for interop and schema evolution.
- Data with rapidly evolving schemas under active design. rkyv's evolution
  story is stricter than serde; additive-only discipline works, breaking
  changes need a migration step.
- Small, cold reads where deserialization cost is in the noise.

### Composition

The two stack naturally:

```
rkyv    →  read ArchivedJob directly from redb bytes         (no alloc)
bumpalo →  build diff, candidate placements, action buffers   (arena alloc)
heap    →  return Vec<Action> through Raft                    (global alloc)
```

The borrow checker enforces the boundaries: arena references can't escape the
`Bump`, archived references can't escape the backing byte slice, and the
`Action` values returned to Raft must be owned.

### Pre-size collections when the length is known — `with_capacity` over `new`

**Prefer `Vec::with_capacity(n)` (and `String::with_capacity`,
`HashMap::with_capacity`, …) over `::new()` whenever the final length is known
or cheaply computable before you start pushing.** `Vec::new()` + a run of
`push`/`extend_from_slice` reallocates on the geometric-growth schedule
(1 → 4 → 8 → 16 → …), copying the buffer each time; `with_capacity(n)` does the
single allocation up front and every subsequent `push` is a bounds-free write.
When the size is locked, that is a strict, free win — and it documents the
buffer's expected size at the allocation site.

Where it applies (the size is derived from the inputs, before the loop):

- **Wire / byte-buffer assembly.** A TLV whose payload length you already
  know, an octet concatenation, a netlink/genl attribute buffer — the header
  is fixed and the payload length is in hand, so
  `Vec::with_capacity(HDR + payload.len())` is exact. Precedent: the nft
  NETLINK_NETFILTER encoder in `overdrive-netlink/src/nft.rs` (feature
  `subprocess-free-veth-provisioner`) pre-sizes its message buffers this way;
  the golden-bytes fixture stayed byte-identical, because **capacity affects
  allocation only, never the emitted contents** — an encoding/roundtrip test
  is unchanged by the swap.
- **1:1 transforms.** Mapping one collection into another of the same length —
  prefer `src.iter().map(f).collect()` (which pre-sizes from `size_hint`) or,
  when you must build imperatively, `Vec::with_capacity(src.len())`.
- **Seed with the first element** (`vec![first]`) instead of
  `with_capacity(1)` + `push(first)` where an accumulator always starts with a
  known head — it reads cleaner and still avoids the empty-then-grow realloc.

When NOT to reach for it:

- **The final size is genuinely unknown** — a decode/parse accumulator whose
  element count depends on runtime input (a netlink dump walk, a variable-arity
  reply), or an early-return-heavy builder. Leave `Vec::new()`; do **not**
  invent a magic capacity number to look tidy. If you cannot *name* the size,
  do not pre-size.
- **Cold, tiny, one-shot buffers** where the realloc cost is in the noise and a
  capacity hint would only add clutter. The rule earns its keep on hot paths and
  on buffers built in a loop, not on a two-element `Vec` constructed once at
  boot.

The smell to catch in review: a `Vec::new()` (or `String::new()`) immediately
followed by a bounded `for`/`extend` whose iteration count is a known function
of the inputs — that is a pre-sizeable buffer written the slow way.

---

## Lifetime discipline in internal APIs

Orthogonal to the allocator choice — applies even when bumpalo and rkyv are
not in play.

- **Prefer `&str` over `String`** in function signatures when the callee does
  not need ownership. Borrow from the deserialized input for the duration of
  the call.
- **Use `Cow<'_, T>`** for data that is usually borrowed but occasionally
  needs modification. Common in label / annotation / header handling — the
  fast path stays zero-copy.
- **Use `#[serde(borrow)]`** on serde-deserialized structs where the parsed
  struct can hold `&str` into the original input bytes. Kills allocation for
  every string field in JSON/YAML-heavy reconcilers.
- **Reserve `Arc<T>`** for genuinely long-lived state shared across tasks
  (engines, caches, connection pools). Do not reach for `Arc` to dodge
  lifetime annotations on per-request data.

---

## Ordered-collection choice

`core` and control-plane hot paths default to `BTreeMap` for keyed maps
whose iteration order is observed (drain, snapshot, JSON output,
invariant evaluation). `HashMap` is a first-class nondeterminism source
on the same footing as `Clock` / `Transport` / `Entropy` and must be
treated with the same discipline.

| Iteration shape | Choice | Notes |
|---|---|---|
| Drained / iterated / snapshotted | `BTreeMap<K, V>` | Default. Order is `Ord` on `K` — deterministic across processes, runs, seeds. |
| Serialised (JSON, rkyv archived field, audit log) | `BTreeMap<K, V>` | Output bytes must be canonical for content hashing and trace-equivalence DST assertions. |
| Walked by an invariant or property test | `BTreeMap<K, V>` | Reproduction requires bit-identical traversal under the seed. |
| Point-accessed only (`get` / `insert` / `remove`, never iterated) | `HashMap<K, V>` with `// dst-lint: hashmap-ok <reason>` | Allowed in `core` only with the justification comment. |

The escape hatch — `HashMap` in a `core`-class crate — requires an
explicit `// dst-lint: hashmap-ok <one-line reason>` comment on (or
immediately above) the use site. Without the comment the dst-lint gate
rejects the file at PR time. The comment is the load-bearing artifact:
it documents *why* iteration nondeterminism cannot surface here, and a
reviewer who disagrees with the reason has a single line to push back
on.

### Why

`std::collections::HashMap`'s default `RandomState` is per-process
random-seeded — two seeded DST runs produce divergent dispatch
orderings the moment ≥2 distinct keys are held. That violates the K3
*seed → bit-identical trajectory* property documented in whitepaper §21
and `.claude/rules/testing.md` § "Sources of Nondeterminism": every
source of nondeterminism in core logic must be injectable, and `RandomState`
is the one the type system silently smuggles past every other gate.

The defect was discovered in
`crates/overdrive-control-plane/src/eval_broker.rs`, where
`EvaluationBroker::drain_pending` returned evaluations in
non-deterministic order. The fix landed as a `BTreeMap` swap in commit
`8cf9119` (`fix(eval-broker): switch pending map to BTreeMap for
deterministic drain order`). The structural rule prevents the class
from recurring; the dst-lint clause enforces it.

### When NOT to apply

Bounded-cardinality maps that are NEVER iterated — point-accessed only
via `get` / `contains_key` / `insert` / `remove`, with no observable
drop order or iteration call site — MAY use `HashMap` with the
justification comment, since the iteration nondeterminism never
surfaces. Examples: per-allocation handle caches keyed by
`AllocationId` where the cache is consulted only by point lookup and
never enumerated, or per-request memo tables whose lifetime ends before
any reduction over their entries.

If you find yourself reaching for the escape hatch and the cardinality
is small (say, <16), prefer `BTreeMap` anyway — the constant-factor
cost is in the noise and the `// dst-lint: hashmap-ok` comment is
upkeep that future contributors must justify.

### Marker comment syntax

The dst-lint scanner accepts exactly one escape form. Other shapes —
`#[allow(dst_lint::hashmap)]` attributes, `// SAFETY:`-style prose,
crate-level `#![allow(...)]` — are NOT recognised; the scanner will
still reject the file.

**Form**:

```
// dst-lint: hashmap-ok <one-line reason>
```

- The literal prefix is `// dst-lint: hashmap-ok` (single space after
  the colon, single space before `hashmap-ok`). Casing matters — the
  scanner is case-sensitive on the marker tokens.
- A one-line reason is **required** in human-readable code review
  contexts even though the scanner does not enforce reason text. A
  marker without a reason will be rejected at code review time, not at
  lint time. Put the *why* on the line; the *what* is obvious from the
  next line of source.
- **Placement**: on the line **immediately above** the use site, OR as
  a trailing comment **on the same line** as the use site. Both are
  recognised:

  ```rust
  // dst-lint: hashmap-ok per-allocation handle cache, point access only
  let cache: HashMap<AllocationId, Handle> = HashMap::new();
  ```

  ```rust
  let cache: HashMap<AllocationId, Handle> = HashMap::new(); // dst-lint: hashmap-ok per-allocation handle cache, point access only
  ```

- The marker suppresses violations on the marked line only. Multiple
  use sites in the same function each need their own marker. Do not
  put one marker at the top of a function and expect it to cover the
  whole body.
- The marker covers `HashMap` and `HashSet` together — a single
  `// dst-lint: hashmap-ok` suppresses both type families on the
  marked line. There is no separate `hashset-ok` form.

**What the marker does not cover**:

- `std::collections::hash_map::HashMap` — the scanner walks
  `TypePath` and `ExprPath` and catches the type by last segment
  regardless of qualifying path; the marker still applies.
- `BuildHasherDefault<...>` / `RandomState` / custom hashers — these
  are different concerns and require their own justification at the
  use site (typically a `// SAFETY:`-style prose comment, since they
  do not flow through dst-lint).
- Type aliases (`type Cache = HashMap<...>;`) — the alias declaration
  IS the use site; the marker goes on the alias line. Subsequent
  references to the alias type do not need their own markers (the
  alias's marker is the load-bearing artifact).

**Why the marker has to be precise**: ad-hoc patterns (`// hashmap-ok`,
`// dst: hashmap-ok`, `// allow hashmap`) would silently slip past the
scanner without a clear failure mode. The strict syntax is the
trade-off for catching the rule's enforcement gap mechanically.

### Sim-internal exception

`adapter-sim` and `adapter-host` crates are NOT scanned by the dst-lint
clause (only `core` is), but the principle applies as guidance —
multi-step DST harnesses that observe iteration order should still
prefer `BTreeMap`. The precedent at
`crates/overdrive-sim/src/adapters/observation_store.rs:215-218`
documents this choice for `BTreeMap<AllocationId, AllocStatusRow>` with
the same rationale: when the harness asserts on the row stream, the
stream must be deterministic across seeds.

---

## Check-and-act must be atomic (no TOCTOU)

**A presence/identity check and the mutation it gates MUST be a single
atomic operation. The mutating call's own return value IS the check at
the moment of mutation — use it; never re-check separately, and never
discard it.** Splitting "is this slot taken?" from "take it" — whether
across two calls (`contains()` then `insert()`) or by dropping the
return that already encodes the outcome (`let _ = set.insert(k)`) —
opens a time-of-check-to-time-of-use (TOCTOU) window where a second
actor slips between the check and the act. This is a recurring defect
class in this codebase, not a one-off.

### The shape

Every instance is the same silhouette: a primitive whose mutation
returns the race outcome, and code that throws the outcome away and
proceeds as if a stale earlier check still held.

| Primitive | The return that IS the check | Discarding it re-opens |
|---|---|---|
| `BTreeSet`/`HashSet::insert` | `bool` — `false` iff already present | a second claimant proceeds |
| `OnceLock`/`OnceCell::set` | `Result` — `Err` iff a racer won | the lost-race value is silently absorbed |
| `*::remove` | `Option`/`bool` — was it there? | a double-free / double-fire |
| `AtomicT::compare_exchange` | `Result<prev, prev>` | the CAS loop's whole point |
| `fs::OpenOptions::create_new` | `Err(AlreadyExists)` | a TOCTOU file race (`exists()` then `create()`) |
| SQL `INSERT … ON CONFLICT` / upsert | conflict outcome | a SELECT-then-INSERT race |

The recheck-at-the-moment-of-mutation is free and correct; a *separate*
earlier check is a stale snapshot by the time you act on it.

### The fix, strongest first

1. **Type the racy surface away (preferred).** When a "claim a slot"
   pattern recurs, wrap the shared cell so the *only* reachable
   operation is the atomic claim-and-act, and the split is
   unrepresentable — the bug cannot be written. This is the
   `make-invalid-states-unrepresentable` lever (§ "Type-driven
   design"). Two reusable primitives live in `overdrive-core` for this:
   - **`ClaimSet<K>`** (`crates/overdrive-core/src/claim_set.rs`)
     exposes only `try_claim(k) -> Option<ClaimGuard>` (RAII release)
     and `snapshot()` — no `contains`, no bare `insert`, no `remove`.
     Its `try_claim` return is `#[must_use]`, so the *next* misuse
     (dropping the guard, which instantly releases the claim) is itself
     a compile-time lint.
   - **`RaceOnceCell<T>`** (`crates/overdrive-core/src/race_once_cell.rs`)
     wraps a `OnceLock` so the lost-race verdict is consumed, not
     discarded: `set_or_read_winner` (any winner is read back — the one
     sanctioned discard, behind a named contract) and `set_or_verify`
     (a divergent winner returns `SetOutcome::Conflict`). The built-in-ca
     lost-race fix's `set_or_verify_winner` helper was promoted into it;
     `RcgenCa` now holds its root/intermediate anchors in `RaceOnceCell`s.
2. **Use the atomic return in place.** Where a typed primitive is
   overkill, branch on the mutation's own return: `if !set.insert(k) {
   return … }`, `match cell.set(v) { Err(_) => verify_winner(…) }`,
   `create_new()?`, `ON CONFLICT`. One locked op, no second check.
3. **Never `let _ =` / `.ok()` / statement-drop a return that encodes
   a race outcome.** This is the § "Errors" discipline ("never silently
   absorb a fallible boundary read into a default") applied to
   concurrency: a discarded `insert`/`set`/`compare_exchange` return is
   a discarded race verdict.

### Symptoms during review

- `if x.contains(k) { … } … x.insert(k)` — the canonical split. The
  two calls are a TOCTOU even under one lock if the lock is released
  between them, and always so across two lock acquisitions.
- `let _ = set.insert(k);` / `cell.set(v); Ok(())` / `set.insert(k);`
  as a bare statement on a *claim-shaped* set or cell — the return was
  the verdict. (A `BTreeMap::insert` discarding its `Option<V>` on a
  value-write is fine — this rule is about membership/identity claims,
  not value overwrites.)
- Two `.lock()` acquisitions of the same field within one function
  where the first reads and the second mutates — the gap between them
  is the window.
- `path.exists()` then `File::create(path)` — the filesystem TOCTOU;
  use `create_new`. (`reconcilers.md`'s "adopt-and-skip" symptom — `if
  resource.exists() { return Ok(()) }` — is the kernel/OS sibling of
  this rule; that doc owns the converge-on-boot remedy.)

### Precedent

- **`6b9bafde`** — `WorkflowEngine::start` discarded `live_instances.
  insert(correlation)`'s `bool` and spawned unconditionally; a second
  `StartWorkflow` for a live instance drove the same journal twice,
  interleaving appends the positional cursor cannot replay. Fixed
  with an atomic claim, then hardened by retiring the raw
  `Mutex<BTreeSet>` for `ClaimSet`.
- **`ade22762`** (built-in-ca) — `adopt_persisted_root` discarded
  `OnceLock::set`'s `Err` on a lost race; the lock ended up holding an
  ephemeral anchor and every subsequent issuance signed under the wrong
  key, orphaning pinned certs. Fixed with `set_or_verify_winner`, since
  promoted into the `RaceOnceCell<T>` primitive above.
- **`31265d4b` / ADR-0061** (veth) — `if exists() { adopt-and-skip }`,
  the filesystem/kernel TOCTOU; remedy lives in `reconcilers.md`
  (converge-on-boot).

### When NOT to apply

A value *overwrite* where the prior value is irrelevant
(`map.insert(k, v)` as a write-through, the reconciler `ViewStore`
update) is not a claim — discarding its `Option<V>` return is correct.
The rule fires only when the return encodes a *membership / identity /
race* verdict that a downstream branch depends on.

---

## Rust patterns

### Errors

- **Use `thiserror` for typed errors** in all library / core crates. Typed
  errors provide structured data for audit trails, reconciler retry logic,
  and investigation-agent tool outputs.
- **Use `eyre` only at CLI / binary boundaries** for user-facing messages.
  `eyre` is a fork of `anyhow` with pluggable report handlers — pair it
  with `color-eyre` in binaries to get backtraces, `tracing-error`
  spantraces, and `Help` suggestions in one formatted report. Prefer
  `eyre::Result<T>` over `anyhow::Result<T>` in new code; do not mix the
  two in one crate.
- **Library code never returns `eyre::Report` (or `anyhow::Error`).** The
  caller loses the ability to branch on variant, and re-exporting a
  `Report` as part of a public API ties your SemVer to eyre's — an
  `eyre` major bump in a downstream app becomes a breaking change you
  cannot control. Return a `thiserror` enum; let the binary convert at
  the boundary via `?` (`eyre::Report: From<E>` for any `E: Error`).
- **`wrap_err` / `wrap_err_with` for context**, not `Display` string
  concatenation. The returned `Report` preserves the full error chain;
  `color-eyre`'s formatter renders it as `Caused by:` sections. Do not
  use `.map_err(|e| format!("...: {e}"))` — it collapses the chain to a
  string and breaks downcasting.
- **`eyre!` and `bail!` for one-off errors** at the boundary only.
  Inside a library, construct the typed variant.
- **Consistent constructors.** Every error enum variant should have an
  associated constructor method (`Error::validation(...)`,
  `Error::internal(...)`, `Error::not_found(...)`). Call sites read as
  English; variant shape can evolve without a breaking grep.
- **Pass-through embedding, not duplication.** When a higher-level error
  wraps a lower-level error, embed via `#[from]` rather than redefining
  variants. Preserves the full nested structure (and its queryable
  fields) through audit logs and investigation outputs.

  ```rust
  // Bad — duplicates lower variants; manual From impls; loses fields
  pub enum ReconcilerError {
      IntentPutFailed { source: DbError },
      DriverStartFailed { message: String, alloc_id: Option<String> },
  }

  // Good — pass-through via #[from]; nested structure preserved
  pub enum ReconcilerError {
      Validation { message: String, field: Option<String> },
      Intent { #[from] source: IntentStoreError },
      Driver { #[from] source: DriverError },
  }
  ```

  Use service-specific variants for local concerns (validation, business
  logic); use pass-through for errors from lower layers that need no
  transformation.

- **Distinct failure modes get distinct error variants. Never silently
  absorb a `Result<_, io::Error>` (or any other fallible boundary read)
  into a default value.** Using `.unwrap_or_default()`, `.ok()`, or
  `.unwrap_or(_)` on a boundary I/O / parse / env read collapses every
  distinguishable failure (PermissionDenied, EIO, broken procfs, missing
  mount, malformed input, ...) into the same neutral value. The next
  downstream check then misdiagnoses the cause and prescribes the wrong
  remediation — and the cost is paid by the operator, who follows
  guidance that does not fix the actual problem. "Boot a newer kernel"
  does not repair a permissions error on `/proc/filesystems`; "the JSON
  field is missing" is not the same diagnosis as "the JSON file is
  unparseable." **Default to propagation**: `.map_err(...)?` into a
  discrete typed variant whose `Display` form names the actual cause
  and the actual fix. Absorbing a specific `ErrorKind` into a default
  is allowed only when the application semantics legitimately treat
  that kind the same as the default — `NotFound` on `/proc/filesystems`
  IS the cgroup-v1-host signal, but `PermissionDenied` is not, even
  when the downstream check happens to fire in both cases.

  ```rust
  // Bad — every io::Error becomes the empty string, which then
  // triggers NoCgroupV2 with a "boot a newer kernel" remediation
  // regardless of the actual cause (permission denied, EIO, broken
  // procfs, /proc unmounted).
  let proc_fs = std::fs::read_to_string(proc_filesystems).unwrap_or_default();
  if !proc_fs.lines().any(|l| l.contains("cgroup2")) {
      return Err(NoCgroupV2 { kernel: uname_release() });
  }

  // Good — NotFound flows to the v1-host signal because that IS the
  // application semantics; every other ErrorKind surfaces as its own
  // discrete variant with its own Display message and its own
  // remediation.
  let proc_fs = match std::fs::read_to_string(proc_filesystems) {
      Ok(s) => s,
      Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
      Err(err) => return Err(ProcFilesystemsUnreadable { source: err }),
  };
  ```

  Symptom to watch for during review: an error variant whose docstring
  describes one failure mode but whose *triggering code path* fires for
  several unrelated reasons. That is the smell — a variant has become a
  catch-all for everything not explicitly handled, and operators
  downstream receive the wrong remediation. The structural fix is
  always the same: split the catch-all into discrete variants, propagate
  the originating error via `.map_err(...)?`, and let `Display` carry
  the cause-specific guidance.

- **Never flatten a typed error to `Internal(String)` at a composition
  boundary.** When a function returns a typed error enum (e.g.
  `CgroupBootstrapError`, `WorkloadsBootstrapError`), the call site
  that converts it to the top-level error (`ControlPlaneError`) MUST
  use a dedicated `#[from]` variant — never
  `.map_err(|e| ControlPlaneError::internal("...", e))`. The
  `internal(context, source)` constructor calls `format!("{context}:
  {source}")`, collapsing the typed variant into a plain `String` and
  destroying the caller's ability to `matches!` on the variant for
  structured diagnostics. The pattern to follow is `ViewStoreBoot`,
  `Tls`, `Cgroup` — each has a dedicated `#[from]` variant on
  `ControlPlaneError` so the CLI can branch on the type without
  `Display`-grepping. The anti-pattern is any `.map_err(|e|
  ControlPlaneError::internal(...))` on a typed bootstrap/infra
  error — that is always a bug, not a convenience.

### Safe byte-slice access

**Never index a byte slice directly (`bytes[0]`) when the source is
external or fallible (store reads, network payloads, user input).**
Use `.first().copied()` (or `.get(n).copied()` for arbitrary offsets)
and handle the `None` case explicitly. Direct indexing panics on
empty or short slices; the safe accessors return `Option<u8>`.

```rust
// Bad — panics on empty Bytes
let kind = WorkloadKind::from_discriminator_byte(stored[0]);

// Good — gracefully handles empty/short slices
if let Some(b) = stored.first().copied() {
    kind = WorkloadKind::from_discriminator_byte(b);
}
```

This applies to any `Bytes`, `Vec<u8>`, or `&[u8]` whose length is
not statically guaranteed by the type system. When the write path
always produces a fixed-length value, the read path must still
defend against corruption, truncation, or future schema evolution
that changes the length.

### Logically unreachable `None` / `Err` — use `unreachable!()`, not `?` or `.expect()`

When a prior guard guarantees that an `Option` is `Some` (or a
`Result` is `Ok`), do not propagate with `?` — that suggests early
return is a valid path and hides the invariant from future readers.
Use `.unwrap_or_else(|| unreachable!("..."))` to make the invariant
explicit and signal "this is a logic error" rather than "a runtime
failure I hope won't happen."

```rust
// Bad — suggests None is a valid early-return path
let running = rows.iter().filter(|r| r.is_running()).max_by_key(|r| r.ts)?;

// Bad — expect panics the process; never use in production library code
let running = rows.iter().filter(|r| r.is_running()).max_by_key(|r| r.ts)
    .expect("at least one running");

// Good — communicates the invariant: a prior guard made this unreachable
let running = rows.iter().filter(|r| r.is_running()).max_by_key(|r| r.ts)
    .unwrap_or_else(|| unreachable!("running_count >= 1 guarantees at least one Running row"));
```

**`.expect()` has no place in production library code** — it panics
and kills the process. The only legitimate use is at CLI / binary
boundaries (per § "Concurrency & async" → `.expect()` in CLI
binaries) where a panic IS the intended exit strategy. In library
and core crates, either propagate the error via `?` into a typed
variant, or use `unreachable!()` when a logical invariant makes the
path structurally impossible.


### Concurrency & async

- **Tokio is the standard runtime.** Do not reach for `async-std`,
  `smol`, or a hand-rolled executor. Consistency matters more than
  marginal performance differences.
- **`Send + Sync` on core data structures.** Shared long-lived state
  (engines, caches, pools) must be safely sendable across threads. If a
  type is not `Send + Sync`, justify it in a comment.
- **Cancellation safety.** Async tasks must tolerate being cancelled at
  any `.await` point. A task holding a partially-applied mutation
  across an `.await` is a bug.
- **Async effects require async APIs — never hide I/O behind runtime lookup
  and a detached spawn to preserve a stale synchronous signature.** If an
  operation must acquire an async lock, write a socket, wait for a process, or
  otherwise `.await`, make the trait/function method `async` and have its caller
  await completion. Do not implement a synchronous method by calling
  `tokio::runtime::Handle::try_current()` and `spawn(async move { ... })`.
  Scheduling a detached task is not completion: the caller cannot observe the
  effect's result, uphold a happens-before postcondition, or distinguish write
  failure from runtime shutdown/cancellation. This is especially forbidden for
  release, commit, install, or fail-closed operations whose names promise that
  the effect happened before return. Existing `#[async_trait]` traits should
  evolve the method signature and bounded implementer/call-site fallout rather
  than retain a synchronous API around newly async work.

  **Symptom during review:** a synchronous trait method discovers the current
  Tokio runtime, clones state, and spawns a future solely because its new body
  needs `.await`; or a caller proceeds past `release_*` / `commit_*` while the
  named operation is merely queued. Replace the workaround with an async method,
  await it at the orchestration boundary, and keep error/cancellation handling
  in that structured call path.
- **`JoinHandle::abort()` is never the shutdown mechanism — a cooperative
  signal is.** `abort()` stops a task at its next yield point. A task
  doing blocking work — anything on `spawn_blocking`, a blocking
  `accept()` / `poll()` / `recv()` — has no yield point until the syscall
  returns, so `abort()` does **not** interrupt it. The task keeps running,
  and keeps holding whatever it owns: a bound socket, a netns handle, a
  cgroup scope. Shutdown must therefore be driven by something the task
  itself observes — a `CancellationToken`, or an `AtomicBool` checked
  between bounded slices — and `abort()` is at most a backstop that stops
  the task being re-polled once it has already yielded.

  Two shapes, both live in tree:

  - **Fully async task tree → tokens alone; no `abort()`, no `JoinSet`.**
    `probe_runner` owns a root `CancellationToken` plus one intermediate
    token per `ProbeRole` (`crates/overdrive-worker/src/probe_runner/supervisor.rs`),
    and each task's body is a `tokio::select!` with `biased;` on its
    token. Cancelling the root drains every task; cancelling one role
    token retires exactly that role — which is what lets a non-terminal
    `Stable` stop startup supervision while readiness and liveness keep
    ticking (ADR-0080 § D4). Handles are detached; nothing is collected.
  - **Blocking task → cooperative flag first, `abort()` as backstop
    only.** `MtlsInterceptWorker::stop_alloc`
    (`crates/overdrive-worker/src/mtls_intercept_worker.rs`) stores
    `stop = true`, which the accept loops observe between 200 ms poll
    slices, *then* aborts. The comment there states the reason plainly:
    `abort` alone cannot interrupt a blocking `accept()`. The DNS
    responder shutdown (`crates/overdrive-control-plane/src/lib.rs`) has
    the same shape — `responder.stop()` is the mechanism, `abort()` is
    "belt-and-braces".

  **Symptom during review:** a `.abort()` with no accompanying flag or
  token, on a task that does blocking I/O. It reads as shutdown and is
  not — the socket stays bound and runtime teardown hangs. Also: an
  `abort()` whose comment claims it *stops* the task rather than
  preventing a re-poll.
- **Never hold a lock across `.await`.** Grab the lock, mutate or clone,
  drop the guard, then `await`. Holding `parking_lot` across `.await` is
  a deadlock waiting for an unfair scheduler tick; holding `tokio::sync`
  across `.await` is a latency spike waiting to happen.
- **Use `parking_lot::RwLock` / `Mutex`** over `std::sync::RwLock` /
  `Mutex` for synchronous critical sections. Avoids lock poisoning,
  faster uncontended path, smaller. Use `tokio::sync::RwLock` / `Mutex`
  only when the critical section *must* cross `.await` — and per the
  rule above, try hard not to need that.
- **`.expect()` in CLI binaries.** In `main()` and CLI entry points, use
  `.expect("description")` instead of verbose `match` / `unwrap_or_else`
  + `process::exit()` patterns. `expect` already prints and panics;
  wrapping `process::exit` around fallible constructors adds noise with
  no benefit.
- **No blocking `std::fs::*` inside `async fn`.** Filesystem I/O inside
  an `async fn` body in an `adapter-host`-class crate goes through
  `tokio::fs::*` (preferred — same syscall surface, async API) or
  `tokio::task::spawn_blocking` (escape hatch — the sync closure runs
  on the blocking pool). Sync `std::fs::*` blocks the tokio worker
  thread and stalls every other future scheduled on it until the
  syscall returns. The dst-lint gate enforces this at PR time:
  `xtask/src/dst_lint.rs::scan_source_async_fs` walks every `async fn`
  body (plus `async {}` blocks and `async fn` inside `#[async_trait]`
  impls) in `adapter-host` crate `src/` and flags any path under
  `std::fs::*`. Two exemptions:
  - **Sync helper fns** are allowed to use `std::fs::*` directly. The
    lint only fires when the *enclosing* fn / closure / async block
    is async — sync helpers called from an `async fn` are still a
    smell, but if you genuinely cannot make the helper async, wrap
    its call site in `tokio::task::spawn_blocking`.
  - **`#[cfg(test)]` items.** Tests may use sync `std::fs` for fixture
    setup without penalty. The lint detects `#[cfg(test)]` on modules
    and on individual fns and skips both.
  Note: `tokio::fs::*` itself dispatches each call onto the blocking
  pool internally — the *kernel* still does blocking I/O. The
  difference is that the `async fn` body is never the one blocked.

### Hashing requires deterministic serialization

When a hash is used as an identity, address, or integrity check (content
hashes in Garage, schematic IDs, Raft log digests, investigation-trace
reproducibility), the serialization that feeds the hash MUST be
deterministic.

- **Internal data → rkyv.** rkyv's archived bytes are canonical by
  construction. Hash the archived slice directly.
- **External / JSON data → RFC 8785 (JCS).** If a hash must be computed
  over JSON (interop requirement, external-facing audit log), use a JCS
  implementation — never `serde_json::to_string()`. `{"a":1,"b":2}` and
  `{"b":2,"a":1}` must produce the same hash; serde does not guarantee
  that.
- **TOML / YAML schematics → canonicalize, then hash.** Round-trip
  through a canonical form before SHA-256. The schematic ID is a content
  hash; non-deterministic input means non-deterministic ID.

```rust
// Bad — key ordering is not guaranteed; hash varies run to run
let digest = sha256(&serde_json::to_string(&record)?);

// Good — archived bytes are canonical
let archived = rkyv::to_bytes::<_, 256>(&record)?;
let digest = sha256(&archived);
```

### Dependencies

- **Workspace dependencies always.** Use `foo.workspace = true` in
  per-crate `Cargo.toml`; never hardcode versions in a leaf crate. Version
  drift across crates is a merge-conflict generator and an audit
  nightmare.
- **Use standard crates.** Don't roll custom base64 / hex / crypto / UUID
  / time formatting. Use `base64`, `hex`, `ring` / `aws-lc-rs`, `uuid`,
  `time` / `chrono` — whichever is already in the workspace graph.

### Cargo.toml conventions

- **Every workspace member declares `integration-tests = []`** in its
  `[features]` block, even crates with no integration tests of their
  own. The declaration is a no-op for the latter and the actual gate
  for the former. This makes `cargo {check,test,mutants} --features
  integration-tests` resolve uniformly under per-package scoping —
  cargo refuses the bare feature on packages that don't declare it,
  which historically broke mutation testing's per-mutant invocations.
  See `.claude/rules/testing.md` § "Integration vs unit gating" /
  "Workspace convention" for the full story; an xtask `#[test]`
  enforces the rule mechanically (`xtask::mutants::tests::every_
  workspace_member_declares_integration_tests_feature`).
- **`xtask/Cargo.toml [dependencies]` MUST NOT contain any
  `overdrive-*` crate.** xtask is build / test / dev orchestration;
  runtime tools live in their owning crate's `src/bin/` and are
  fronted by cargo aliases. See § "xtask is build / test / dev
  orchestration, NOT a runtime entry point" above for the bootstrap
  RCA and the decision test for new tools.

### Newtypes — STRICT by default

Raw primitives (`String`, `&str`, `u64`, `i64`, `[u8; 32]`) for domain
concepts are blocking violations. All identifiers and domain-bearing
values MUST use newtypes from `overdrive-core`:

| Concept | Newtype |
|---|---|
| Workload identity | `SpiffeId` |
| Job | `JobId` |
| Allocation | `AllocationId` |
| Node | `NodeId` |
| Policy | `PolicyId` |
| Region | `Region` |
| Investigation | `InvestigationId` |
| Correlation | `CorrelationKey` |
| Image schematic | `SchematicId` |
| WASM module / chunk | `ContentHash` (SHA-256) |
| Certificate serial | `CertSerial` |

**Only exception** — an explicitly approved, issue-tracked deferral with
scope and exit criteria. Outside a tracked deferral, do not accept
"follow-up" language in review — the types exist, use them now.

**Symptom signals.** A new `normalize_spiffe_id()`, `normalize_node_id()`,
or similar helper is almost always a symptom of a missing newtype
constructor. If you find yourself writing one, the fix is to move the
normalization into the newtype's constructor, not to ship the helper.

### Newtype completeness

Every newtype must implement:

- `FromStr` — with validation; returns `Result<Self, ParseError>`.
- `Display` — the canonical string form.
- `Serialize` / `Deserialize` — matching `Display` / `FromStr` exactly.
- Constructors that **validate and return `Result`**. No infallible
  `new()` that silently accepts garbage.

**Case-insensitive parsing.** `FromStr` for identifiers that humans type
or paste — SPIFFE IDs, region codes, schematic IDs — must be
case-insensitive. The canonical form emitted by `Display` is lowercase.
SHA-256-style content hashes stay case-sensitive (they are not
human-typed).

### One shared length ceiling for label-shaped ids

**Every label-shaped identifier shares ONE length ceiling
(`overdrive_core::id::LABEL_MAX`, 253 — the DNS-name maximum). A
*derived* id MUST size its own ceiling off that shared const, never a
bespoke smaller magic number.** When id B is produced by mapping id A
into B's grammar, B's ceiling must be large enough to hold the *entire*
mapped A. If B's ceiling is smaller than A's, the mapping truncates —
and truncation of a content-addressed id is an **identity collision**:
two distinct A values that share a truncated prefix collapse onto one B.

The lossy-truncation collision is the same defect class as § "Check-and-act
must be atomic" (two distinct things silently treated as one), reached
through a different door: there, a discarded race verdict; here, a
discarded suffix. The fix shape is also the same — make the collision
*unrepresentable* (size the ceiling so truncation cannot happen) rather
than detect-and-recover after the fact.

#### Why a content-addressed suffix makes this acute

The canonical correlation form is `target:purpose/<hex>` — the
discriminating, content-addressed `<hex>` sits at the **end** of the
string. End-of-string is exactly the region truncation drops. So the
*one part of the key that guarantees distinctness* is the first casualty.
Any id whose entropy is concentrated in a suffix (hashes, ULIDs, content
addresses) is maximally vulnerable to prefix-preserving truncation.

#### The rule

- **Derive the ceiling, don't invent one.**
  `const B_MAX: usize = overdrive_core::id::LABEL_MAX + PREFIX.len();` —
  account for any fixed prefix the mapping prepends (a `wf-`-style
  leading-char guarantee consumes from the budget). Do NOT write
  `const B_MAX: usize = 127;` and hope the inputs stay short.
- **A truncation guard in a mapping loop is a smell.** If
  `if out.len() >= MAX { break; }` can fire for a valid input, the
  ceiling is too small. Size it so the guard is *structurally
  unreachable* for every in-range input, and say so in a comment — keep
  the guard only as a defensive invariant against future grammar drift,
  never as a routine path.
- **"Inputs are short in practice" is not a sizing argument.** It is the
  exact hand-wave that ships the latent collision. Size for the input
  type's declared maximum, not its typical value.

#### Symptoms during review

- A second, smaller length const for an id that is *derived from* another
  id (`WORKFLOW_ID_MAX = 127` while `CorrelationKey` is `LABEL_MAX = 253`).
- A doc comment calling truncation "defensive" or "correlation keys are
  already short" — that is the latent-collision tell, not a safety margin.
- A mapping/sanitising loop with a `break` on a length ceiling, where the
  source type's max exceeds the destination ceiling.

#### Precedent

`WorkflowId::for_correlation` (`crates/overdrive-control-plane/src/journal/mod.rs`)
truncated the mapped `CorrelationKey` to a bespoke 127-char ceiling while
correlation keys may be 253. Two keys sharing their first 124 mapped chars
but differing only in the dropped suffix derived the **same**
`WorkflowId`; the second instance's `start()` opened the first's journal
(silent no-op on a `Terminal` row, or wrong-sequence replay). Fixed by
sizing `WORKFLOW_ID_MAX = overdrive_core::id::LABEL_MAX + WF_PREFIX.len()`
so the full key always maps without truncation. (Note: the char-fold step
`:`/`/` → `-` is *independently* lossy, but only collides hand-built
`CorrelationKey::new()` keys carrying those chars; every `derive()`-built
key keeps its full hash suffix once truncation is gone, so all in-tree
usage is collision-free.)

### Documentation

- **Rustdoc `///` on every public item.** If the public API is not worth
  documenting, it probably should not be public.
- **Doctests for usage examples.** Examples in rustdoc fenced blocks run
  as tests — code that rots in a `README` is unverifiable; code in a
  doctest fails the build when the API drifts.
- **No aspirational docs.** Never document behaviour that is not
  implemented. An empty doc comment is strictly better than a lie.

### Import style

Import types directly. Do not use fully-qualified paths in function
signatures or struct fields.

```rust
// Bad
fn reconcile(id: &overdrive_core::SpiffeId) -> Result<(), Error> {
    let seen: HashSet<overdrive_core::SpiffeId> = HashSet::new();
    // ...
}

// Good
use overdrive_core::SpiffeId;

fn reconcile(id: &SpiffeId) -> Result<(), Error> {
    let seen: HashSet<SpiffeId> = HashSet::new();
    // ...
}
```

Exception: full paths only to disambiguate two types with the same name:

```rust
use overdrive_core::JobId;
use legacy::JobId as LegacyJobId;
```

---

## Why this matters

The reconcile loop runs constantly, per object, across thousands of objects.
Its allocator behaviour is one of the largest determinants of tail latency
and steady-state memory use. The goal is not a micro-optimization; it is a
predictable hot path:

- Arena allocation removes malloc/free overhead on the transient middle and
  eliminates heap fragmentation across iterations.
- Zero-copy deserialization removes the deserialization pass entirely for
  the durable inputs.
- Lifetime-bound references make "this data cannot escape this iteration" a
  compile-time guarantee, which is the only way the pattern survives across
  a team.

In a GC language this pattern is approximated with object pools and
discipline. In C++ it is arenas plus hope. Rust makes the invariants
mechanical.
