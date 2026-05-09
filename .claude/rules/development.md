# Development Guidelines

Rust-specific implementation patterns that apply across the Overdrive codebase.
These are not architectural decisions (those live in the whitepaper); they are
conventions for how we write Rust in the reconciler, dataplane, and store paths.

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

## State-layer hygiene (repeats §18 discipline in code form)

The three state layers each map to a specific allocation / storage pattern.
Crossing them accidentally is the class of bug the type system exists to
prevent.

| Layer | Store | Reading | Writing |
|---|---|---|---|
| Intent — what should be | `IntentStore` (redb / openraft+redb) | `&ArchivedT` via rkyv | Only via typed Raft actions |
| Observation — what is | `ObservationStore` (Corrosion / CR-SQLite) | SQL subscriptions | Owner-writer only, full rows |
| Memory — what happened | per-primitive libSQL | SQL | SQL |
| Scratch — this iteration | `Bump` | arena refs | arena alloc, dies at iteration end |

Enforce this with distinct trait objects (`IntentStore`, `ObservationStore`)
and distinct types per layer. Do not expose a shared `put(key, value)`
surface that lets the wrong call go to the wrong place.

---

## Persist inputs, not derived state

**Anywhere a value will be read back later — libSQL row, redb entry,
Corrosion table, on-disk artifact, JSON config, audit row, cache —
persist the *inputs* to whatever logic consumes the value, not the
*output* of that logic.** Derived values get recomputed on every read
from the persisted inputs and the live policy / function / lookup
table. This is the rule that makes policy evolution, schema stability,
and audit replay all work; violating it ships a stale cache of your
own logic.

The State-layer hygiene table above governs *where* state lives
(IntentStore, ObservationStore, libSQL memory, scratch). This rule
governs *what shape* the value takes regardless of which layer it sits
in.

### What counts as "derived"

A value is derived if it would change when one of these changes,
without the *inputs to it* changing:

- A constant elsewhere in the codebase (timeout, threshold, weight,
  schedule entry).
- A function or lookup table consulted at compute time (a backoff
  schedule, a scoring weighting, a Regorus policy, a routing table).
- An operator-configurable knob (today often a constant; tomorrow a
  per-tenant / per-job override).
- A cross-version migration of the logic itself (the v1 algorithm
  produced X; the v2 algorithm produces Y from the same inputs).

If the answer to "would editing this constant / table / policy change
the persisted value's correctness?" is yes, the value is derived. The
field that should be persisted is the input that feeds the constant /
table / policy, not the output it produces.

### Why

A persisted derived value is a stale cache of the logic that produced
it. Three failure modes follow, in order of how often they bite in
practice:

1. **Policy / configuration evolution silently no-ops.** Today's
   constant becomes tomorrow's operator-configurable knob (Kubernetes
   `restartPolicy` / Nomad `restart` shape; per-tenant timeout
   overrides; per-region weighting; a schedule swap). When the change
   lands, every persisted derived value is still bound to the OLD
   logic until it ages out. The change appears to apply at the
   configuration boundary but is silently ignored at the read site —
   exactly the inverse of what the operator expects. Recovery requires
   a column migration (re-deriving the missing inputs from a value
   that has already lost them) or an explicit drain (waiting until
   every persisted derived value expires).
2. **Schema migrations multiply.** Every change to the producing
   function becomes a change to the persisted schema, because the
   persisted value embedded the old function's output. Persisting
   inputs decouples the schema from the function — `(attempts,
   last_failure)` is stable across every backoff schedule that
   consumes those two fields.
3. **Audit and replay degrade.** Persisted inputs let an investigation
   re-derive a past decision against any candidate policy. Persisted
   outputs collapse the input → output trajectory into the output
   alone; the original decision cannot be re-explained without
   reconstructing the lost inputs.

### Examples

**Reconciler memory (libSQL).** A reconciler View field carries a
recompute-on-read deadline by storing the inputs the deadline depends
on, not the deadline itself:

```rust
// Bad — persists the deadline (a derived value bound to today's policy)
pub struct JobLifecycleView {
    pub restart_counts:   BTreeMap<AllocationId, u32>,
    pub next_attempt_at:  BTreeMap<AllocationId, UnixInstant>,  // derived
}

// Good — persists the inputs; deadline recomputed on every read
pub struct JobLifecycleView {
    pub restart_counts:        BTreeMap<AllocationId, u32>,         // input
    pub last_failure_seen_at:  BTreeMap<AllocationId, UnixInstant>, // input
}
// reconcile() body computes `*seen_at + backoff_for_attempt(*count)`
// every tick — picks up backoff-policy changes for free.
```

**Observation rows.** An observation row carries the field the
authoritative writer observed, not a derived classification of it.
`alloc_status.state` is an observed input; a `health_grade ∈ {green,
amber, red}` column would be derived from `state` + a threshold table
and must not be persisted — compute it at read time.

**Cache rows.** When a value is genuinely expensive to recompute (the
narrow exception below), the cache row carries an *invalidation key*
derived from the inputs and the policy identity, so a policy change
automatically invalidates the cache. A cache without such a key
violates this rule and must not be persisted.

**Audit / forensic rows.** Audit rows record the *observed inputs and
the decision that was made*, not a re-derived view of why. The
decision is the input ("we restarted alloc X at time T after attempt
N"); a "would have been retried again at T+5s" prediction is a
derived value and belongs in a viewer that recomputes from the audit
row, not in the audit row itself.

**Operator-facing config files / UI state.** A schematic file
(§Image Factory, whitepaper §23) is hashed by content. The hash is the
ID of the schematic. Persisting `schematic_id_v1: "abc..."` AND the
canonicalised schematic body is fine because the ID is *defined as* a
function of the body — they are the same datum, encoded twice for
different purposes. But persisting "this schematic produces a 247 MB
image" alongside the schematic is a derived cache; the next change to
the build system makes the number a lie.

### Codebase precedent

`crates/overdrive-control-plane/src/worker/exit_observer.rs:291`:

```rust
let backoff = RETRY_BACKOFFS.get((attempts - 1) as usize).copied();
```

The persisted input is `attempts: u32`. The policy is the indexed
`RETRY_BACKOFFS` table. Swap the table — the next attempt picks up the
new schedule with no migration, no drain, no inconsistency window.

### When NOT to apply

The rule has one narrow exception: when recomputing the derived value
is genuinely intractable on the read hot path (an expensive ML
inference, a cross-cluster correlation that costs a network
round-trip, a cryptographic operation measured in seconds). That is a
*cache*, not the primary value — and it MUST carry an invalidation
key derived from the inputs and the policy identity. If the cache
cannot carry such a key (because the policy is opaque), it must not
be persisted; recompute on every read or move the work into a workflow
that produces the value as observable state.

If you find yourself reaching for this exception for an in-process
arithmetic computation (a duration addition, a comparison, a small
table lookup, a string format), you are mistaken about the cost —
the recomputation is free. Persist the inputs.

### Symptoms during review

- A persisted field whose name describes a *future event*
  (`next_attempt_at`, `expires_at`, `scheduled_for`, `valid_until`,
  `next_run_after`, `eligible_at`). These are almost always derived
  from a "last X" timestamp + a policy.
- A persisted field whose value would change if a constant elsewhere
  in the codebase were edited. The constant IS the policy; the field
  is the cached output of applying it.
- A read site that uses a persisted field directly in a comparison or
  branch without consulting a policy table or function. The
  silhouette `if now < persisted.deadline` is the canonical smell —
  the deadline IS the cache; replace with `if now < persisted.seen_at
  + policy.backoff()`.
- A persisted field whose docstring contains the word "computed"
  or "derived" or "calculated."

If the field genuinely does NOT depend on a policy (a remote system's
externally-assigned identifier, a user-supplied free-text comment, a
fingerprint of immutable content), it is an input by construction and
the rule does not apply.

---

## Port-trait dependencies — `overdrive-host` is production, `overdrive-sim` is tests

`overdrive-core` declares the port traits (`Clock`, `Transport`, `Entropy`,
`Dataplane`, `Driver`, `IntentStore`, `ObservationStore`, `Llm`); two
sibling crates implement them.

| Crate | Class | Use | Cargo.toml placement |
|---|---|---|---|
| `overdrive-host` | `adapter-host` | Production bindings (`SystemClock`, `OsEntropy`, `TcpTransport`, …) | `[dependencies]` of crates that ship a production wiring |
| `overdrive-sim` | `adapter-sim` | Simulation bindings (`SimClock`, `SimTransport`, `SimEntropy`, `SimDriver`, `SimObservationStore`, `SimLlm`) | `[dev-dependencies]` of any crate whose tests need DST controllability |

**Rules:**

- **Never put `overdrive-host` in `[dev-dependencies]`** — it is the
  production binding crate. Tests do not run production wiring; they
  inject sim adapters. If a test reaches for `SystemClock` /
  `OsEntropy` / `TcpTransport`, the test is wrong, not the dep
  placement: replace the production binding with its sim counterpart.
- **Never put `overdrive-sim` in `[dependencies]`** — it carries
  `turmoil`, `StdRng`, and other DST machinery that must not be
  reachable from production binaries. The dst-lint gate scans for
  this.
- **Required, not defaulted, at the call site.** Types that depend on
  a port trait take the implementation as an explicit constructor
  parameter (`fn new(clock: Arc<dyn Clock>, ...)` or similar). Never
  default the field to a production binding inside the constructor —
  that silently inherits wall-clock / OS-entropy / real-network
  behaviour into tests that forgot to override, which is the exact
  failure mode the trait surface exists to prevent.
- **Builder-pattern overrides (`with_clock`, `with_transport`) are an
  anti-pattern for these traits.** A builder makes the dependency
  optional — and "optional" means "tests can forget." Make the
  dependency mandatory in `new()`; tests pass `Arc::new(SimClock::new())`,
  production passes `Arc::new(SystemClock)`, the compiler enforces
  every call site is explicit.
- **Production wiring is composed at the binary boundary.** A library
  crate's tests compose sim adapters; the CLI / control-plane binary
  composes host adapters. The library crate itself does not pick
  sides — it depends only on the trait surface in `overdrive-core`.

The compile-time consequence is the load-bearing one: a test that
forgets to inject a clock fails to compile rather than silently
running on `SystemClock`. The dst-lint gate catches the residual
"clock leaked into a `core` compile path" cases; this rule is the
upstream prevention.

---

## `src/` is production code — tooling binaries live in `bin/`

`src/` contains the crate's library and production code surface.
Tooling binaries — gates, generators, debug helpers — that import
the crate to *exercise* it live under `bin/` at the crate root, not
`src/bin/`. This keeps `src/` greppable as "what ships" and avoids
polluting the library's module tree with gate/tool-only code.

**Modules consumed only by `bin/` binaries belong in `bin/`, not in
`src/`.** If a module (parsers, pure decision fns, helpers) has no
library consumer — only tooling binaries use it — it is not
production code and does not belong in the library surface. Each
`[[bin]]` is a crate root; `mod foo;` resolves to sibling files in
the same directory, so `bin/my_gate.rs` is found naturally by
`bin/my_tool.rs` declaring `mod my_gate;`. No `#[path]` and no
`mod.rs` needed.

Layout:

```
crates/<crate>/
├── src/           # production library code
├── bin/           # tooling binaries + their private modules
│   ├── <tool>.rs          # [[bin]] crate root
│   └── <gate_logic>.rs    # mod declared by the binary
├── tests/         # test entrypoints
└── Cargo.toml     # [[bin]] entries with `path = "bin/<tool>.rs"`
```

Cargo auto-discovers `src/bin/` but NOT `bin/`. Every binary under
`bin/` needs an explicit `[[bin]]` entry in `Cargo.toml`:

```toml
[[bin]]
name = "my_tool"
path = "bin/my_tool.rs"
```

Existing crates with `src/bin/` (`overdrive-sim`, `overdrive-control-plane`)
migrate on touch — when you edit the binary, move it in the same commit.

---

## xtask is build / test / dev orchestration, NOT a runtime entry point

**Build/test/dev orchestration → xtask; runtime tools → their owning
crate.** xtask exists to drive cargo / rustc / kernel build artifacts
on behalf of the workspace; it does not exist to host binaries that
import the platform's crates to exercise them.

### Why this matters

xtask's `[dependencies]` graph is a build-time compile chain. Every
crate listed there is compiled (and its `build.rs` executed) before
any xtask subcommand can run. When a runtime crate is added to that
list, its `build.rs` is dragged into xtask's compile chain — and if
that `build.rs` fails fast on a missing artifact whose *only*
producer is itself an xtask subcommand, the build pipeline deadlocks
on a clean tree:

> `xtask` depends on `overdrive-sim` (for an in-process `Harness`).
> `overdrive-sim` depends on `overdrive-dataplane` (for one pure
> function). `overdrive-dataplane`'s `build.rs` is
> `#[cfg(target_os = "linux")]` and hard-fails when
> `target/bpf/overdrive_bpf.o` is missing. Compiling `xtask` inside
> Lima therefore requires the BPF object — and `cargo xtask
> bpf-build` is what produces it. Chicken, egg, exit 1.

This section exists because the failure mode is silent until somebody
nukes their `target/` dir on a clean tree. The next contributor wires
the same shape and the chicken-and-egg returns. The rule prevents the
recurrence. See the commit history of `xtask/Cargo.toml` for the
landed fix (relocations of `dst` and `openapi` binaries out of
xtask, plus the move of pure `maglev` modules from
`overdrive-dataplane` to `overdrive-core` to break the
`overdrive-sim → overdrive-dataplane` edge).

### What stays in xtask

Build / test / dev orchestration subcommands — tasks *about* the
codebase, consuming source / producing build artifacts. The canonical
list as of this PR:

`bpf-build`, `bpf-clippy`, `bpf-unit`, `integration-test vm`,
`verifier-regress`, `xdp-perf`, `mutants`, `dst-lint`,
`yaml-free-cli`, `ci`, `lima`, `hooks`, `mcp`, `dev-setup`.

These are allowed to depend on cargo / rustc / kernel toolchain
machinery. They MUST NOT depend on `overdrive-*` crates.

### What moves out of xtask

Runtime tools that import the platform's crates to *exercise* them.
Today:

- `dst` — the DST harness binary. Lives at
  `crates/overdrive-sim/src/bin/dst.rs`; subprocess tests live at
  `crates/overdrive-sim/tests/integration/`.
- `openapi` — the OpenAPI generator/gate. Library at
  `crates/overdrive-control-plane/src/openapi.rs` (typed
  `OpenApiError`); binary at
  `crates/overdrive-control-plane/src/bin/openapi.rs`; acceptance
  test at
  `crates/overdrive-control-plane/tests/integration/openapi_gate.rs`.

Future runtime tools (scenario runners, simulation snapshots,
debug-shell variants) ship as binaries in their owning crate — never
as xtask subcommands.

### User-facing surface

Cargo aliases in `.cargo/config.toml`, never xtask wrappers. Pattern:

```
<tool> = "run -p <crate> --bin <bin> --"
```

Worked example, post-move:

```
dst         = "run -p overdrive-sim --bin dst --"
openapi-gen = "run -p overdrive-control-plane --bin openapi -- generate"
openapi-check = "run -p overdrive-control-plane --bin openapi -- check"
```

Direct invocation always works too —
`cargo run -p overdrive-sim --bin dst -- --seed 1234`. The alias is
for ergonomics; the binary is the SSOT.

### Decision test for new tools

Two questions, in order:

1. Does it consume cargo / rustc / kernel build artifacts to
   produce a build / test / lint result? → **xtask subcommand.**
2. Does it import `overdrive-{core,sim,control-plane,dataplane,host,
   worker,…}` to *exercise* the platform? → **a binary in that
   crate's `src/bin/`, fronted by a cargo alias.**

If both answers are yes, the tool has two concerns and should be
split into two binaries.

### What xtask's `[dependencies]` MUST NOT contain

Any `overdrive-*` crate. Adding such a dep is exactly the failure
mode this rule prevents. The xtask `Cargo.toml` carries an inline
comment naming this rule above its `[dependencies]` block; the
comment is the active enforcement surface, and the lint of last
resort is:

```bash
cargo tree -p xtask | rg overdrive-
```

This must return empty. If it doesn't, the dep graph has regressed
and the bootstrap chain is at risk.

The artifact-path rename that landed alongside this rule
(`target/xtask/dst-*` → `target/dst/*`,
`target/xtask/bpf-objects/overdrive_bpf.o` → `target/bpf/overdrive_bpf.o`)
reflects the same separation: artifacts produced by runtime tools
live under their own top-level path, not under `target/xtask/`.
Mutants outputs (`target/xtask/mutants*`) stay — `mutants` is a
genuine xtask subcommand and owns that path.

### Symptom signals during review

- A new `Task::*` variant in `xtask/src/main.rs` whose dispatch arm
  destructures runtime-crate types (`SimHarness`, `OpenApiSpec`,
  reconciler `View` shapes, etc.).
- A `pub mod foo` in `xtask/src/lib.rs` whose body imports an
  `overdrive-*` crate.
- An `overdrive-*` line in `xtask/Cargo.toml [dependencies]`. The
  comment block above the deps says "MUST NOT contain"; honor it.
- A `.cargo/config.toml` alias of the form
  `foo = "run --package xtask -- foo"` for a runtime-shaped tool.
  Compare against the post-move shape:
  `dst = "run -p overdrive-sim --bin dst --"`. The first form drags
  the entire xtask compile chain into the runtime tool's startup
  path; the second resolves directly to the owning crate.

This rule is a complement to ADR-0003 (crate-class taxonomy), not a
replacement. ADR-0003 governs what kind of crate something is; this
rule governs which crate a given binary lives in once that taxonomy
is fixed.

---

## Production code is not shaped by simulation

**Production code MUST NOT carry extra logic, extra arms, extra yields,
extra polling, or extra structural concessions whose only purpose is to
make a `Sim*` adapter behave correctly under DST.** The port-trait
contract is the boundary: production wires `Host*`, tests wire `Sim*`,
and production code is written to the contract, not to the test
double's quirks. If a production call site needs a workaround to make
`Sim*` work, the bug is in the `Sim*` adapter, not in the production
code.

This is the rule that the `Clock` / `Transport` / `Entropy` / `Driver`
trait surface exists to enforce. A production `tokio::select!` arm that
exists "so DST can advance time" is the canonical violation — it
means the sim adapter has imposed a shape on the production hot path
that the production hot path would not otherwise need. The cost is
real (busy-loop CPU, redundant polls, defensive yields, wasted resource
budget) and it compounds: every future call site copies the workaround
because "that's how we use the trait here."

### Symptoms

- A `tokio::select!` in production whose only resolving arm is
  `tokio::task::yield_now()` or `clock.sleep(Duration::from_millis(1))`
  with a comment explaining "so the deadline check at the top can fire
  under SimClock."
- A production loop with a `clock.unix_now() >= deadline` check at the
  top *and* a separate timer arm in the select, where the comment
  explains the deadline check is for DST and the timer arm is for
  production.
- A production function that takes a `_dst_yield_hook: Option<...>`
  parameter, or any other signature contortion whose only caller is a
  test.
- A docstring that says some piece of production code is "redundant
  but harmless" in production and exists "to play nicely with SimClock."
- A `// dst:` or `// sim:` comment on a production line of code
  explaining behavior that would not be there absent the test double.

If you find yourself writing the comment, stop. The comment is the
admission that the rule is being broken.

### What to do instead

Fix the `Sim*` adapter to model the real-world primitive faithfully,
then write production against the trait shape that production
genuinely needs. Concretely, if a `Sim*` `sleep` / `recv` / `connect`
forces a workaround in production, the adapter is doing the wrong
thing — usually conflating two concerns (e.g. "yield" and "advance
time"), or auto-progressing state that the harness should drive
explicitly.

Reference shape: `SimClock::sleep` should *park on a deadline* (register
a waker; the harness's `tick()` wakes timers whose deadline has passed),
not auto-advance time as a side effect of being polled. Every credible
DST framework (turmoil, madsim, FoundationDB flow sim) implements
`sleep` this way; only the harness drives logical time.

### Audit checklist for any sim-adapter change

Before landing a `Sim*` change that requires modifying a production
call site, answer:

1. Could the sim adapter be reshaped so the production call site stays
   unchanged? If yes — do that instead.
2. Does the production change degrade production behavior in any way
   (extra CPU, extra latency, extra polls, defensive resource use)? If
   yes — the change is rejected; the sim adapter must be reshaped.
3. Would a future engineer reading the production code, with no
   knowledge of the sim adapter, understand why the code is shaped this
   way? If no — the shape is wrong, even if a comment papers over it.

The boundary between production and simulation is load-bearing. Erode
it once and every future call site inherits the erosion.

---

## Trait definitions specify behavior, not just signature

**A trait is a contract, and the contract is the SSOT.** Every method
on a port trait (`Clock`, `Transport`, `Dataplane`, `IntentStore`,
`ObservationStore`, `Driver`, `Llm`, `Reconciler`, …) MUST carry a
rustdoc block that pins the *observable behavior* every adapter is
required to implement — not just the type signature, not just an
English description of what the method "does."

Type signatures specify shape; they do not specify semantics.
`fn update_service(&self, vip: Ipv4Addr, backends: Vec<Backend>) ->
Result<(), DataplaneError>` says "takes a VIP and a list of backends,
returns an error or unit." It does NOT say what `update_service(vip,
vec![])` means, what the post-state of `service_backends(vip)` should
be, or whether reverse-NAT entries are purged. When the contract is
implicit, every adapter ships a private interpretation — and they
drift the moment one of them is patched without the others.

This rule is the *upstream* of § "Production code is not shaped by
simulation": that rule governs implementation shape; this one governs
the contract both implementations must honor. Without a written
contract, "production and sim must observe the same behavior" is a
slogan, not an enforceable property.

### What every trait-method docstring must specify

The four load-bearing properties:

1. **Preconditions** — what the caller must guarantee about the
   inputs. "VIP must be IPv4," "backends must be non-empty," "key
   must have been previously registered." If a precondition can be
   violated, the violation MUST map to a typed error variant; do not
   defer to "implementation may panic."
2. **Postconditions** — what the caller can rely on after the call
   returns `Ok(_)`. State the *observable* effect, not the
   implementation: "after return, `service_backends(vip)` returns the
   passed `backends` slice in insertion order," not "we write to the
   `services` map."
3. **Edge cases** — every degenerate input the signature permits.
   Empty collections, zero values, `None`, idempotent re-application,
   re-registration, removal of an absent key. Each gets a sentence
   pinning the contract: "passing `backends.is_empty()` removes the
   VIP from the dataplane entirely; `service_backends(vip)` returns
   `None` after return." If the trait author cannot say what an edge
   case means, the edge case should be made unrepresentable (a
   `NonEmpty<Backend>` newtype, a `RegisterService` / `RemoveService`
   sum type, …) rather than left under-specified.
4. **Observable invariants** — the cross-method properties adapters
   must preserve. "After `update_service(vip, [b1, b2])`, every
   reverse-NAT key derived from b1 / b2 maps to `vip`. After
   `update_service(vip, [])`, no reverse-NAT key derived from any
   prior backend of `vip` is reachable." These are the properties
   the DST equivalence harness asserts on; if they are not written,
   they cannot be tested.

### Why the contract goes in the trait, not in one adapter's docstring

The trait is the only place every adapter author reads. A contract
documented on `EbpfDataplane::update_service` is invisible to whoever
writes the next sim adapter, the next mock, the next test double; a
contract documented on the `Dataplane` trait method itself is the one
artifact every implementation MUST consult. When two adapters diverge
on an edge case, the question is always "what does the trait say?"
— if the trait says nothing, the divergence is a contract gap, not
an adapter bug.

### Symptom signals during review

- A trait method whose docstring is one sentence describing what it
  "does" (`/// Update the service backends.`) with no edge-case
  language. Reject — every degenerate input the signature permits
  must be pinned.
- Two adapter implementations of the same trait method whose
  observable behavior diverges on the same call (`update_service(vip,
  vec![])` returns `Some(vec![])` from one and `None` from the other,
  per the SimDataplane vs EbpfDataplane drift in PR review of step
  03-XX). The bug is in the trait contract, not in either adapter —
  fix the trait docstring first, then bring the adapters into line.
- A precondition expressed as a runtime panic (`assert!(!backends.
  is_empty())`) instead of a typed error variant or a non-empty
  newtype. Reject — preconditions are part of the contract; the
  type system enforces them when possible, the typed error surface
  enforces them otherwise. Panics are not a contract.
- A docstring that describes the *implementation* ("writes to the
  `services` BTreeMap") rather than the *observable effect* ("after
  return, `service_backends(vip)` reflects the passed backends").
  The implementation is per-adapter; the contract is universal.

### The DST equivalence test is the structural guard

Once the contract is written, the structural defense against future
drift is a DST harness that drives every adapter implementing the
trait through the same sequence of calls and asserts observable
equivalence at every step. The contract is the *spec*; the
equivalence test is the *enforcement*. A contract without a test
that exercises every clause is documentation, not a guarantee.

In practice, every port trait whose contract differs across adapters
in any non-trivial way (`Dataplane`, `IntentStore`, `Driver`,
`ObservationStore`) ships a `tests/integration/<trait>_equivalence.rs`
that drives both the host adapter and the sim adapter through the
same sequence and asserts on observable state through the trait's
own accessors (or, where necessary, through accessors documented as
"part of the contract for testing purposes"). When the equivalence
test fails, exactly one of the contract / host adapter / sim adapter
is wrong; the test isolates which.

This is the load-bearing pattern: the trait's docstring is the
contract; the equivalence test is the enforcement; the per-adapter
implementation is the consequence. The pattern degrades the moment
the docstring is allowed to be vague, because then the equivalence
test cannot be written.

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

### Cross-references

- Whitepaper §21 — *Deterministic Simulation Testing*; the K3
  reproducibility property (seed → bit-identical trajectory).
- `.claude/rules/testing.md` § *Sources of Nondeterminism* — every
  nondeterminism source must be injectable behind a trait; `HashMap`
  iteration order is the one the trait surface cannot intercept.

---

## Reconciler I/O

**The `Reconciler` trait collapses to a single sync method.** Per
ADR-0035 (and its companion ADR-0036), the trait surface is exactly
two associated types and one function:

```rust
pub trait Reconciler: Send + Sync {
    /// Per-reconciler typed projection of intent + observation.
    /// Per ADR-0021 (amended by ADR-0036).
    type State: Send + Sync;

    /// Per-reconciler typed memory. Persisted as a CBOR blob in the
    /// runtime-owned ViewStore. Author derives the four bounds; the
    /// runtime owns persistence end-to-end.
    type View: Serialize + DeserializeOwned + Default + Clone + Send + Sync;

    fn name(&self) -> &ReconcilerName;

    /// Pure synchronous transition. No `.await`. No I/O. No DB handle.
    /// Wall-clock only via `tick.now`. All mutations are data: returned
    /// Actions cross the publication boundary; the returned NextView
    /// is persisted by the runtime.
    fn reconcile(
        &self,
        desired: &Self::State,
        actual:  &Self::State,
        view:    &Self::View,
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View);
}
```

**The runtime owns persistence end-to-end. Reconciler authors never
write SQL, never call `migrate` / `hydrate` / `persist`, never declare
a schema, never touch a database handle.** They derive
`Serialize + Deserialize + Default + Clone` on the `View` struct and
write `reconcile`. Nothing else.

`reconcile` is a pure function over `(desired, actual, view, tick) →
(actions, next_view)`. No `.await`; no network; no subprocess spawn;
no direct IntentStore / ObservationStore / ViewStore write; no
`Instant::now()` / `SystemTime::now()`. This is what makes DST
(§21) replay and ESR verification (§18, USENIX OSDI '24 *Anvil*)
possible; it is not optional.

### Runtime mechanics — bulk-load + write-through

The runtime persists views via a `ViewStore` port living in
`overdrive-control-plane`, with a `RedbViewStore` host adapter
(production) and a `SimViewStore` sim adapter (tests). The wire format
is CBOR via `ciborium`; one redb file per node at
`<data_dir>/reconcilers/memory.redb`, one redb table per reconciler
kind, value is a CBOR-encoded `View` blob.

The two phases:

**Boot / register-time** (once per reconciler at process start):

```
register(reconciler):
  1. view_store.probe()                          (Earned Trust gate)
  2. views = view_store.bulk_load::<R::View>(name)  (BTreeMap<TR, V>)
  3. registry.insert(name, (AnyReconciler, views))
```

The runtime calls `bulk_load` once per reconciler and materialises
every persisted `(reconciler_name, target) → View` blob into a
per-reconciler `BTreeMap<TargetResource, View>` held in RAM. From that
moment on the in-memory map is the steady-state read SSOT.

**Steady-state tick** (every `tick_period_ms`, default 100 ms):

```
for evaluation in broker.drain_pending():
  1. (any_reconciler, views) = registry.lookup(name)
  2. tick    = TickContext::snapshot(clock)
  3. desired = AnyReconciler::hydrate_desired(...)
  4. actual  = AnyReconciler::hydrate_actual(...)
  5. view    = views.get(target).cloned()
                .unwrap_or_else(R::View::default)
  6. (actions, next_view) = reconciler.reconcile(
       &desired, &actual, &view, &tick)
  7. view_store.write_through(name, target, &next_view)   (fsync)
  8. views.insert(target.clone(), next_view)              (after fsync OK)
  9. action_shim::dispatch(actions, ...)
```

Steady-state reconcile pays **zero disk reads**. Every tick reads
`view = views.get(target).cloned().unwrap_or_default()` from the
in-memory map, never from disk; redb is touched only on write-through
and on cold boot.

**Step ordering 7 → 8 is load-bearing** (fsync-then-memory). On a
crash between fsync and the `BTreeMap::insert`, the next boot's
`bulk_load` sees the persisted view and convergence resumes. The
inverse ordering would let an acknowledged tick disappear on crash,
breaking durability. The `WriteThroughOrdering` DST invariant pins
this.

**`BTreeMap`, NOT `HashMap`**, per § "Ordered-collection choice"
above — the map is drained / iterated on `bulk_load` and observed by
DST invariants; iteration order must be deterministic across seeds.

### Schema evolution

Additive fields use `#[serde(default)]` — ignore-unknown-fields-by-
default is serde's tolerant deserialization, the correct shape for
additive evolution. Breaking changes use a versioned envelope:

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "v")]
enum JobLifecycleViewEnvelope {
    #[serde(rename = "1")] V1(JobLifecycleViewV1),
    #[serde(rename = "2")] V2(JobLifecycleViewV2),
}
```

Phase 1 has no breaking-change history; the envelope shape lands when
the first breaking change ships.

### Worked example — retry memory + external call

When a reconciler needs to talk to an external service (a Restate
admin API, an AWS account, a webhook, a custom internal service), the
View carries the *inputs* its retry policy depends on (per § "Persist
inputs, not derived state" above) and the runtime owns the rest:

```rust
use serde::{Deserialize, Serialize};

// View — the four derive bounds are mandatory; the runtime persists
// this blob via `ViewStore::write_through` after every successful
// reconcile. Persists *inputs* (attempts, last_failure_seen_at) — the
// `next_attempt_at` deadline is recomputed on every tick from these
// inputs + the live backoff policy, never persisted.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct RetryMemory {
    pub attempts:              u32,
    pub last_failure_seen_at:  UnixInstant,
}

impl RetryMemory {
    fn bump_if_dispatched(
        mut self,
        actions: &[Action],
        now: UnixInstant,
    ) -> Self {
        if actions.iter().any(|a| matches!(a, Action::HttpCall { .. })) {
            self.attempts            = self.attempts.saturating_add(1);
            self.last_failure_seen_at = now;
        }
        self
    }
}

// Reconciler — a single sync method. No async, no DB handle, no
// migrate/hydrate/persist. `desired`, `actual`, and `view` are all
// pre-computed by the runtime before `reconcile` is called.
impl Reconciler for RegisterReconciler {
    type State = RegisterState;
    type View  = RetryMemory;

    fn name(&self) -> &ReconcilerName { &self.name }

    fn reconcile(
        &self,
        desired: &Self::State,
        actual:  &Self::State,
        view:    &Self::View,
        tick:    &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let correlation = CorrelationKey::from((
            desired.id, desired.spec_hash, "register",
        ));
        let actions = match actual.external_call(&correlation).latest_status() {
            None => vec![Action::HttpCall {
                correlation:     correlation.clone(),
                target:          desired.endpoint.clone(),
                method:          Method::POST,
                body:            build_register_payload(desired),
                timeout:         Duration::from_secs(30),
                idempotency_key: Some(correlation.to_string()),
            }],
            Some(Status::Pending) | Some(Status::InFlight) => vec![],
            Some(Status::Completed { response }) => {
                converge_from_response(actual, response)
            }
            // Retry-budget gate: only re-dispatch once the backoff
            // window has elapsed. The deadline is RECOMPUTED every
            // tick from the persisted inputs (`attempts` +
            // `last_failure_seen_at`) and the live backoff policy —
            // never persisted. `tick.now_unix` is the runtime's
            // single-snapshot of wall-clock for this evaluation:
            // pure input, DST-controllable.
            Some(Status::Failed { .. } | Status::TimedOut { .. })
                if tick.now_unix
                    >= view.last_failure_seen_at
                        + backoff_for_attempt(view.attempts) =>
            {
                vec![/* re-dispatch */]
            }
            Some(Status::Failed { .. } | Status::TimedOut { .. }) => vec![],
        };
        // NextView carries the updated retry memory; the runtime
        // CBOR-encodes it and writes it to redb (one transaction,
        // one fsync) before updating the in-memory BTreeMap. The
        // reconciler never writes the ViewStore directly.
        let next_view = view.clone().bump_if_dispatched(&actions, tick.now_unix);
        (actions, next_view)
    }
}
```

### Rules

1. **Every external call carries an `idempotency_key`** when the
   remote API supports one. The runtime executes `HttpCall`
   at-least-once; idempotency on the remote side is what makes the
   effect exactly-once.
2. **Correlation, not request ID, links cause to response.** A
   `CorrelationKey` newtype derived from
   `(reconciliation_target, spec_hash, purpose)` lets the next
   reconcile find the prior response deterministically. Do not
   embed the `request_id` in reconcile logic — it changes per
   attempt; the correlation does not.
3. **Retry budgets live in the View.** The runtime does not
   auto-retry a failed `HttpCall` — that policy belongs to the
   reconciler. Track attempts in the typed `View`, return an updated
   `next_view`, and the runtime persists it via `write_through`.
4. **Multi-step external sequences become workflows, not chains of
   `HttpCall`s.** If the reconciler would need to coordinate three
   or more external calls that must complete as a unit, emit
   `Action::StartWorkflow` and read the workflow's result on
   completion. Reconcilers converge; workflows orchestrate.
5. **`HttpCall` responses are observation, not intent.** The
   `external_call_results` table lives in the ObservationStore and
   is gossiped like any other observation row. Reconcilers read it
   locally via `actual`, same as `alloc_status` or
   `service_backends`.
6. **Reading wall-clock: use `tick.now` from the `TickContext`
   parameter.** Never call `Instant::now()` / `SystemTime::now()`
   inside `reconcile`. The dst-lint gate catches violations at PR
   time. Time is input state, injected by the runtime — the same
   `Clock` trait DST already controls (`SystemClock` in production,
   `SimClock` under simulation). The `tick.now` snapshot is taken
   once per evaluation; every `reconcile` call sees one consistent
   "now," which is what makes the function pure over its inputs.
   `tick.deadline` is the per-tick budget (consult to checkpoint
   bounded work into the next View); `tick.tick` is a monotonic
   counter useful as a deterministic tie-breaker.
7. **Persist inputs, not derived state, in the View** (per §
   "Persist inputs, not derived state" above). A `next_attempt_at`
   deadline field is a smell — store the inputs that feed it
   (`attempts` + `last_failure_seen_at`) and recompute the deadline
   in `reconcile` against the live backoff policy. The View is
   typed memory, not a cache of today's policy.

The reference case: a Restate-operator-equivalent in Overdrive is a
reconciler whose `View` carries `RetryMemory { attempts,
last_failure_seen_at }`, whose `reconcile` emits `HttpCall { target:
restate_admin_url, method: POST, body: deployment_spec,
idempotency_key: Some(...) }` on registration when `tick.now_unix >=
view.last_failure_seen_at + backoff_for_attempt(view.attempts)`
(deadline recomputed every tick from persisted inputs, never
persisted itself), and which reads `external_call_results` on the
next tick to advance its state machine. No async in `reconcile`; no
direct HTTP client; no direct wall-clock read; no DB handle; fully
DST-replayable. See ADR-0035 / ADR-0036 for the full design.

---

## Workflow contract

Workflows are the §18 peer primitive to reconcilers. The rules are
different from reconcilers — and different in specific ways that matter.

**`async` is permitted in workflows. Only in workflows.** Anywhere else
in the codebase — reconcilers, policies, sidecars — `async fn` that
performs I/O is a violation. Workflow handlers are the one place where
`.await` on real work is the *correct* shape.

```rust
trait Workflow: Send + Sync {
    async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult;
}

// Good — all non-determinism flows through ctx; durable at each await
#[overdrive::workflow]
async fn cert_rotation(ctx: &WorkflowCtx, spec: CertSpec) -> Result<Cert> {
    let challenge = ctx.call("acme", acme::new_order(&spec)).await?;
    ctx.sleep(Duration::from_secs(spec.dns_propagation_seconds)).await;
    let validated = ctx.call("acme", acme::validate(&challenge)).await?;
    ctx.signal_all("workflow.cert_ready", validated.clone()).await?;
    Ok(validated)
}
```

Rules:

1. **All non-determinism goes through `ctx`.** Clock, network, RNG,
   signals, inter-workflow coordination — every source of non-determinism
   is a method on `WorkflowCtx`, which consumes the same injected
   `Clock` / `Transport` / `Entropy` traits the rest of the platform uses
   under DST. No `Instant::now()`, no `reqwest::get()`, no
   `tokio::time::sleep`, no `rand::random()` inside a workflow body.
2. **No side effects outside `ctx`.** Writing to a file, calling a
   non-`ctx` async function, spawning a task, mutating a global — these
   break journal replay. The SDK lints them; the runtime rejects
   workflows whose call graph references forbidden hosts.
3. **Journal replay is bit-identical.** A workflow run twice against the
   same journal must produce the same trajectory. The DST harness asserts
   this as `assert_replay_equivalent!` (§21); treat replay-equivalence
   failures as logic bugs, not flakes.
4. **Workflow versioning is additive.** A running workflow may have
   arbitrary in-flight instances. Changing the `run` body in a way that
   would deviate from an existing journal is a *breaking* change and the
   SDK rejects it at load time. Add new versions (`cert_rotation_v2`)
   alongside the old; migrate in-flight instances explicitly via
   `ctx.upgrade_to(...)` or let them drain.
5. **Bounded step budget.** Every workflow declares a maximum number of
   `await` points. Unbounded loops inside a workflow are a bug; if you
   genuinely need indefinite lifecycle, use a reconciler, not a workflow.
   The reconciler primitive exists exactly for "runs forever."
6. **Workflow → cluster mutations go through Actions.** Workflows write
   intent the same way reconcilers do: by emitting typed Actions that
   the runtime commits through Raft. Workflows do not bypass Raft to
   write IntentStore directly — `ctx` does not expose a `.put()` surface
   on IntentStore; it exposes `ctx.emit_action(...)`.

**When to reach for a workflow vs a reconciler:**

```
Runs forever; converges desired vs actual?           → Reconciler
Terminates with a Result; orchestrates a sequence?   → Workflow

"Keep N replicas running"                            → Reconciler
"Roll the certificate through 4 steps"               → Workflow
"Maintain BPF map == policy verdict"                 → Reconciler
"Migrate allocation X from region A to region B"     → Workflow
"Reach desired replica count from queue depth"       → Reconciler (rule-based)
                                                       OR Workflow (predictive,
                                                       multi-step rollout)
```

When in doubt: if the operation has a natural terminal `Ok(result)`,
it's a workflow. If it's "the cluster should always look like X," it's
a reconciler.

---

## Compile-checking

> **Use `cargo check`, not `cargo build`.** `cargo build` is **blocked** by
> a pre-tool hook (`.claude/hooks/block-cargo-build.ts`). For iterative
> typecheck-and-diagnose loops — the overwhelming majority of what the
> agent actually does — `cargo check` skips codegen and linking and is
> dramatically faster on this workspace.
>
> **Rewrite your command before submitting it:**
>
> | ❌ don't | ✅ do |
> |---|---|
> | `cargo build` | `cargo check` |
> | `cargo build -p CRATE` | `cargo check -p CRATE` |
> | `cargo build --workspace` | `cargo check --workspace` |
> | `cargo build --all-targets` | `cargo check --all-targets` |
> | `cargo build --features X` | `cargo check --features X` |
> | `cargo build --release` | `cargo check --release` |
>
> `cargo check` catches every `rustc` diagnostic `cargo build` would —
> trait resolution, borrow checker, type inference, macro expansion,
> lints via `cargo clippy`. It does NOT produce a binary or run
> `build.rs` link steps.
>
> **Legitimate `cargo build` shapes** (the hook allows these):
> - Producing a binary for a real execution target — xtask, CLI, a
>   real-kernel integration test that must boot an artifact, anything
>   about to be `exec`'d.
> - `cargo xtask ...` subcommands that internally invoke `cargo build`
>   as part of their own compilation pipeline.
> - Tier 3 / Tier 4 harness flows where the build artifact is the
>   point (`cargo xtask integration-test vm`, `cargo xtask xdp-perf`).
>
> If you find yourself needing `cargo build` for "just to see the
> errors," you want `cargo check`. If you need the binary, reach for
> the xtask wrapper that already knows how to produce it.

> **On macOS, every `cargo check` must go through Lima.**
>
> `cargo check` is **blocked** on macOS by a pre-tool hook
> (`.claude/hooks/block-bare-cargo-check.ts`) unless it is already
> wrapped in `cargo xtask lima run --`. On Linux the hook is a no-op
> — the host IS the canonical compile environment.
>
> **Rewrite your command before submitting it (macOS):**
>
> | ❌ don't | ✅ do |
> |---|---|
> | `cargo check` | `cargo xtask lima run -- cargo check` |
> | `cargo check -p CRATE` | `cargo xtask lima run -- cargo check -p CRATE` |
> | `cargo check --workspace` | `cargo xtask lima run -- cargo check --workspace` |
> | `cargo check --all-targets` | `cargo xtask lima run -- cargo check --all-targets` |
> | `cargo check --features X` | `cargo xtask lima run -- cargo check --features X` |
>
> **Why:** typecheck signal must match the canonical compile
> environment. macOS host rustc resolves `#[cfg(target_os = "linux")]`
> items differently, may miss conditional dependencies, and skips
> `build.rs` steps gated on Linux. A green `cargo check` on macOS
> without Lima is not the same signal as a green check inside Lima —
> and the next Lima-side compile diverges silently. The same
> rationale governs `cargo nextest run` and `cargo clippy` on macOS;
> see § "Running tests — Lima VM" in `.claude/rules/testing.md`.

## Committing a focused subset

The lefthook pre-commit pipeline auto-stages modified files into the
in-flight commit (it runs `cargo fmt`, clippy, and nextest-affected
across the working tree, and re-stages whatever they touch). When the
working tree carries unrelated modifications and you only want to
commit a focused subset, `git add <one-file>` is **not** sufficient —
the hook will silently bundle every other modified file into your
commit on its way through. The previous commit you just landed will
contain changes you never staged.

**Pattern: stash the unrelated paths first, then commit.**

```bash
git stash push -m "unrelated-{summary}" -- \
  <path-1> <path-2> ... <path-N>     && \
git add <path-you-actually-want>     && \
git commit -m "..."                  ; \
git stash pop
```

Three things to note:

- **`;` before `git stash pop`, not `&&`.** The stash must be restored
  even if the commit fails (e.g. a pre-commit gate rejects it),
  otherwise the unrelated work is left only in the stash and the
  working tree appears empty. The `git stash` pre-tool hook in this
  repo enforces the matched-pair shape — see the message it prints
  when blocking.
- **Path-scoped stash, not `git stash -u`.** Stashing the whole tree
  would also stash the file you want to commit; pass paths to `git
  stash push -- <paths>` so only the unrelated changes move.
- **Verify with `git show --stat HEAD` after the commit.** "1 file
  changed" should match what you intended to land. If the commit
  bundles more, the stash scope was wrong — soft-reset and retry.

This pattern is the only safe shape for landing a focused fix when
the working tree is dirty with parallel work. Do not reach for
`--no-verify` to bypass the lefthook auto-staging — the hook is also
running clippy and tests, and skipping it lands unverified code.

**Always-include path: `.nwave/des-config.json`.** When this file shows
as modified, untracked, or otherwise affected in `git status`, it MUST
be staged into the current commit — never stashed under the
focused-subset pattern, never deferred to a follow-up commit. The file
captures the active nWave rigor profile (lean / standard / thorough /
exhaustive / custom / inherit) and is the SSOT for how subsequent wave
runs in this repo behave; leaving it out of a commit that touched it
silently desyncs the committed profile from the one the next agent
will read. If the commit you are landing is otherwise unrelated, add
`.nwave/des-config.json` to it anyway — it is the one file exempt from
the "focused subset" discipline above.

## Deletion discipline

When production code becomes unused — typically after a refactor that
collapses or replaces a subsystem — **delete the production code AND
its tests in the same commit**. Do not gate, annotate, salvage, or
relocate.

Specifically, when you see a `dead_code` warning (or `unused_imports`,
`unused_variables`) after a deletion pass, the warning is the signal
that **more code needs deleting**, not that the existing code needs a
gate or an allow. The wrong moves:

- `#[cfg(test)]` on a helper that's now only called from tests.
- Moving a helper into `mod tests { … }` to keep the same effect.
- `#[allow(dead_code)]` to silence the warning.
- "Rewriting" the existing tests to test something else so they keep
  earning their keep.

The right move is a single commit that removes the production code
*and* every test that was defending it. A test exists to defend
production code; if the production code is gone, the test is gone too.
You cannot defend something that doesn't exist, and preserving the
test by repurposing it just hides the deletion in the git log — a
future reviewer reading the test name expects it to be telling them
something about a function whose name no longer resolves.

A genuinely new requirement that needs a genuinely new test (e.g. a
convention to enforce after a sweeping deletion) is a separate matter
— write it from scratch, with a name and assertions that describe
the new requirement. Don't pretend the salvaged-and-rewritten old
test was "already" testing the new thing.

The deleted code does not get a stub, a deprecation comment, a
`// removed in PR #N` marker, or a re-export shim. None of these
forms exist in this codebase; per CLAUDE.md and
`feedback_single_cut_greenfield_migrations.md`, removed is removed.

The corollary: **a test file shrinking is the correct shape of a
deletion PR**. If your "deletion" PR adds tests on net, double-check
you actually deleted what you set out to delete.

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

## aya-rs XDP / TC kernel-side patterns

These idioms govern every kernel-side eBPF program in `overdrive-bpf`. They
are sourced from the upstream aya book (https://aya-rs.dev/book/programs/xdp.html)
and adapted to project conventions. The verifier rejects programs that
deviate from these shapes; treat them as load-bearing.

### `ptr_at` — bounds-checked pointer access (canonical helper)

Every kernel-side program MUST go through this helper to dereference packet
data. Direct casting is rejected by the verifier and unsafe regardless.

```rust
#[inline(always)]
unsafe fn ptr_at<T>(ctx: &XdpContext, offset: usize) -> Result<*const T, ()> {
    let start = ctx.data();
    let end = ctx.data_end();
    let len = mem::size_of::<T>();

    if start + offset + len > end {
        return Err(());
    }

    let ptr = (start + offset) as *const T;
    Ok(&*ptr)
}
```

**When to use:** Every typed read out of an `XdpContext` / `TcContext` packet
buffer — Ethernet header, IPv4 header, TCP/UDP header, payload byte
ranges. Anywhere you'd otherwise write `*(ctx.data() as *const T)` or
similar.

**What it protects against:**

- Verifier rejection on unbounded pointer arithmetic — the `start + offset
  + len > end` check is the structural shape the verifier requires.
- Buffer-overflow reads on truncated packets (the most common
  malformed-input class).
- Silent UB from dereferencing past `data_end`.

**House-style adaptations:**

- The helper lives once per program crate at
  `crates/overdrive-bpf/src/shared/access.rs` (or similar) — do not
  copy-paste it per program. Each `programs/<name>.rs` file imports it.
- `#[inline(always)]` is required, not stylistic — the verifier needs the
  bounds check at the call site, not behind a function call.
- Use `mut_ptr_at<T>` (mirror, returning `*mut T`) for write paths
  (header-rewrite hot path during NAT). Same shape, same bounds check.
- Errors propagate via `?` to a top-level `match` that converts `Err(_)
  → XDP_PASS` (or `TC_ACT_OK`) — see "Error Handling Pattern" below.

### Packet header parsing — sequential offsets

```rust
let ethhdr: *const EthHdr = unsafe { ptr_at(&ctx, 0)? };
match unsafe { (*ethhdr).ether_type() } {
    Ok(EtherType::Ipv4) => {}
    _ => return Ok(xdp_action::XDP_PASS),
}

let ipv4hdr: *const Ipv4Hdr = unsafe { ptr_at(&ctx, EthHdr::LEN)? };
let source = u32::from_be_bytes(unsafe { (*ipv4hdr).src_addr });
```

Rules:

- **Sequential offsets via `<HdrType>::LEN` constants**, never
  hand-coded byte counts. The header crate (`network-types` or our own
  `crates/overdrive-bpf/src/shared/headers.rs`) owns the canonical
  layout.
- **Early-return on non-matching protocols.** IPv6 / ARP / non-IPv4
  EtherTypes return `XDP_PASS` so the host's other workloads keep
  working — per § 7 of the whitepaper, the LB program is not a firewall.
- **Wire-byte-order on read; convert at the boundary.** `u32::from_be_bytes`
  on the read path; `to_be_bytes` on the write path. This is the
  endianness lockstep boundary documented in
  `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 11.
  Userspace map storage is host-order; kernel-side conversion happens at
  this boundary, not in the userspace handle.

### XDP return codes

| Return | Meaning | When |
|---|---|---|
| `XDP_PASS` | Hand to kernel networking stack | Non-LB traffic, miss, edge protocols (IPv6, ICMP, ARP) |
| `XDP_DROP` | Silent drop | Malformed input, sanity-prologue violations, blocked traffic |
| `XDP_ABORTED` | Drop + tracepoint | Fallback for unhandled `Err(_)` in the top-level match |
| `XDP_TX` | Bounce back same NIC | LB hit with rewritten dest; reverse-NAT egress |
| `XDP_REDIRECT` | Forward to another NIC / `AF_XDP` | Cross-NIC LB (not used Phase 2.2) |

**Project rule:** `XDP_DROP` requires a `DROP_COUNTER` increment with an
explicit `DropClass` reason (per `crates/overdrive-core/src/dataplane/
drop_class.rs`). Silent drops are forbidden — every drop is an
observable event.

### Error-handling pattern (top-level wrapper)

```rust
#[xdp]
pub fn xdp_service_map_lookup(ctx: XdpContext) -> u32 {
    match try_xdp_service_map_lookup(ctx) {
        Ok(ret) => ret,
        Err(_) => xdp_action::XDP_ABORTED,
    }
}

fn try_xdp_service_map_lookup(ctx: XdpContext) -> Result<u32, ()> {
    // bounds checks via `?`, header parsing, map lookup, header rewrite
}
```

**Why two functions:** The `#[xdp]` entry point returns `u32` (kernel ABI).
The inner `try_*` returns `Result<u32, ()>` so `?` works for bounds-check
propagation. The wrapper converts unhandled `Err(_)` to `XDP_ABORTED` —
verifier-clean and observable via the kernel's xdp tracepoint.

**Project deviation:** `XDP_ABORTED` is acceptable as the catch-all but
prefer `XDP_PASS` when the failure mode is "we can't classify; let the
kernel handle it" (truncated frames per S-2.2-08; non-IPv4 per
S-2.2-21). Reserve `XDP_ABORTED` for genuine "this should never happen"
paths — e.g., a verifier-impossible bounds violation.

### Map access from XDP context

```rust
#[map]
static SERVICE_MAP: HashMap<ServiceKey, u32> = HashMap::with_max_entries(4096, 0);

fn lookup_backend(key: &ServiceKey) -> Option<u32> {
    unsafe { SERVICE_MAP.get(key).copied() }
}
```

Rules:

- `#[map]` declarations live in `crates/overdrive-bpf/src/maps/<name>.rs`,
  one file per map, paired with a userspace handle in
  `crates/overdrive-dataplane/src/maps/<name>_handle.rs` (per ADR-0040
  three-map split).
- `unsafe { MAP.get(...) }` is required — the verifier sees the unsafe
  block and validates the bounded operation. Do NOT wrap further in
  custom helpers; the call site IS the verifier-readable shape.
- Return `Option<T>` — null-checking via `is_some()` / `match` is
  verifier-friendly. Bare null derefs are rejected.
- The `with_max_entries` argument is the BPF map size; it must match
  the userspace handle's expected capacity.
- **HASH_OF_MAPS chained lookup.** Outer-map `bpf_map_lookup_elem`
  returns a `NonNull<c_void>` tagged `inner_map` by the verifier; chain
  to a second `bpf_map_lookup_elem` against the inner FD only after a
  NULL check. Single-level nesting only — the kernel rejects HoM-of-HoM
  at outer-map create time. See
  `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
  § D.6 for the canonical chained-lookup shape.

### `no_std` / `no_main` constraints

```rust
#![no_std]
#![no_main]

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
```

- Every kernel-side crate is `#![no_std]`. No `std::*`, no `alloc::*`
  (no `Vec`, no `String`, no `Box`).
- `no_main` because the `#[xdp]` / `#[classifier]` / `#[lsm]` macros
  emit the entry point.
- Panic handler is `loop {}` — the verifier rejects unwinding. The
  `cfg(not(test))` gate keeps host-side mod-tests compilable.
- The `panic_handler` lives once per crate, in the crate's `lib.rs` (not
  per program file).

### Attach mode — native vs generic (`SKB_MODE`)

- **Default = native (`XdpFlags::DRV_MODE`).** Requires NIC driver
  support; near-line-rate throughput. Lima virtio-net, ubuntu-latest
  virtio-net, mlx5, ena, i40e all support native.
- **`XdpFlags::SKB_MODE` (generic) is the documented fallback.** Slower
  (full kernel networking stack traversal) but driver-agnostic; useful
  for `dummy` interfaces and non-virtio kernels.
- **Project rule (architecture.md § 2 constraint 9):** the loader tries
  native first; on `EOPNOTSUPP` / `ENOTSUP` it emits a single structured
  `tracing::warn!(name: "xdp.attach.fallback_generic", iface = %iface,
  ...)` event and retries with `SKB_MODE`. Do NOT fall back on
  `EINVAL` — that masks real loader bugs (the userspace classifier
  `should_fallback_to_generic` in `crates/overdrive-dataplane/src/lib.rs`
  enforces this).

### Userspace map insertion (companion to kernel-side reads)

```rust
let mut blocklist: HashMap<_, u32, u32> =
    HashMap::try_from(bpf.map_mut("BLOCKLIST").unwrap())?;

let block_addr: u32 = Ipv4Addr::new(1, 1, 1, 1).into();
blocklist.insert(block_addr, 0, 0)?;
```

**Endianness lockstep** (architecture.md § 11):

- **Userspace stores host-order bytes** in the map. The host writes
  `u32::from(Ipv4Addr)` — that produces host-order on every supported
  arch.
- **Kernel-side converts to network-order** at the read boundary via
  `u32::from_be_bytes(packet_addr_bytes)` against the map key. No
  endian flip in userspace.
- Userspace proptests round-trip host-order writes against host-order
  reads (per `.claude/rules/testing.md` § "Property-based testing
  (proptest)" — newtype roundtrip is mandatory).

This is the load-bearing rule the typed userspace handles
(`ServiceMapHandle`, `BackendMapHandle`, etc.) enforce — they accept
host-order inputs, write host-order to the BPF map, and the kernel-side
program does the conversion. Any drift in either direction is caught by
S-2.2-17 (Tier 2 lockstep test).

### `HASH_OF_MAPS` — hand-rolled until aya 0.14+

aya 0.13.x ships no typed userspace `HashOfMaps<K, V>` wrapper and
aya-ebpf 0.1.x ships no `#[map]` macro support for
`BPF_MAP_TYPE_HASH_OF_MAPS`. The project provides a typed handle
(`HashOfMapsHandle<K, V>`) on the userspace side and a
`#[repr(transparent)]` `HashOfMaps<K, V, M>` struct on the kernel side
— both built directly over the raw `bpf()` syscall + `bpf_helper_*`
binding surface.

Outer map and inner-map prototype both created via direct `bpf()`
syscalls in `crates/overdrive-dataplane/src/sys/bpf.rs`; the typed
`HashOfMapsHandle<K, V>` is the only entry point. **Atomic backend
swap is `HashOfMapsHandle::set(&service_id, new_inner_fd)`** — kernel
ref-counting handles in-flight readers, so the swap is observers-see-
either-old-or-new with no torn states.

Migration path when aya 1.0 / PR #1446 lands: `HashOfMapsHandle` has a
deliberately PR-1446-compatible signature; replace
`HashOfMapsHandle::new_with_hash_inner(...)` with
`aya::maps::HashOfMaps::try_from(...)`, replace
`HashOfMapsHandle::set(&key, inner_fd)` with the upstream typed
equivalent, and remove the `sys/bpf.rs` HoM helpers. The
`BackendId`/`ServiceKey`/lookup-chain logic stays as-is.

See `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
§ D.1–D.3 for the full hand-rolled shape (userspace construction +
typed handle + kernel-side struct) and § F.1 for the migration
plan.

### Sharing the outer HoM between userspace and the kernel-side ELF — `pinning = ByName`

aya 0.13.x's stock ELF loader cannot create a HASH_OF_MAPS map from
the ELF alone — `MapData::create` doesn't know the inner-map
prototype, so `BPF_MAP_CREATE` rejects (the `inner_map_fd` field is
unset). The crafter for step 03-02's first attempts mistook this for
a structural blocker; **it isn't**. aya 0.13.x at `bpf.rs:495–503`
already supports the pin-by-name workaround — the same pattern
libbpf uses, and the same pattern Cilium / Katran use to share HoMs
between userspace and kernel-side BPF programs.

The wiring:

1. **Kernel-side declaration** — the project's hand-rolled
   `HashOfMaps<K, V, M>` struct (over `bpf_map_def`) bakes
   `pinning: PinningType::ByName as u32` into its const initializer.
   Required field; not optional.
2. **Userspace `EbpfDataplane::new`** runs in this order:
   1. **Create the inner-map prototype** — `bpf(BPF_MAP_CREATE)` for
      a single `Array<BackendId, 256>` (or whatever the inner shape
      is). This is the template the kernel uses to type-check
      subsequent inner FDs handed in via
      `HashOfMapsHandle::set`.
   2. **Create the outer HoM** —
      `sys::bpf::bpf_create_map(BPF_MAP_TYPE_HASH_OF_MAPS, …,
      inner_map_fd: prototype_fd)`. Returns `outer_fd`.
   3. **Pin the outer map to bpffs** —
      `sys::bpf::bpf_obj_pin(outer_fd,
      "/sys/fs/bpf/overdrive/<MAP_NAME>")`.
   4. **Load the ELF with the pin path set** —
      `EbpfLoader::new().map_pin_path("/sys/fs/bpf/overdrive").load_file(…)`.
      aya's loader sees the kernel-side `bpf_map_def.pinning ==
      ByName`, finds the existing pinned FD by name, and **reuses
      it** — no second `BPF_MAP_CREATE` is attempted. The kernel-side
      program and the userspace `HashOfMapsHandle` now reference the
      same FD.
3. **Cleanup** — bpffs pins survive across process exits; remove on
   shutdown via `unlink("/sys/fs/bpf/overdrive/<MAP_NAME>")` or
   accept the persistence (it's how Cilium operates in production).

**Pin-path discipline**: every Overdrive HoM pins under
`/sys/fs/bpf/overdrive/`. Tests use a per-test tempdir to avoid
cross-test collisions; production uses the literal path. Set this
once in the loader's `EbpfDataplane::new` and never override per-map.

**Why this is not "scope creep"**: the existing Slice 02 S-2.2-06
GREEN test's userspace surface (`ServiceMapHandle::insert`) does
NOT change shape — only the underlying handle implementation does.
The kernel-side ELF declaration and the loader call site change;
the test body stays the same.

See `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
§ D.3 (b) for the bare-create rejection mechanism, and the aya
0.13.1 source at `aya/src/bpf.rs:495–503` for the pin-by-name reuse
path. Resolves K-1 in the research doc.

### Verifier-friendly idioms — what to avoid

- **No loops with non-bounded counters.** The verifier needs to know the
  loop terminates. Use `for i in 0..N` with `N` a const, never a runtime
  variable.
- **No recursion.** Period.
- **No floating-point math.** The kernel BPF JIT does not emit FP
  instructions on most arches.
- **No `panic!` / `todo!` / `unimplemented!` in production code paths.**
  These are RED scaffolds; gate via `#[expect(clippy::todo)]` per
  `.claude/rules/testing.md` § "Production-side scaffolds". The verifier
  rejects programs that reach `panic_handler`.
- **No `Vec`, no `Box`, no `Rc` / `Arc`.** `no_std` constraint; even if
  it compiled, the verifier wouldn't accept dynamic allocation.
- **No deep call chains.** Each `#[inline(always)]` helper expands at the
  call site; deep call graphs explode the verifier's instruction budget.

### Testing tier mapping (per `.claude/rules/testing.md`)

Each kernel-side program lands across all four tiers:

- **Tier 2 (`BPF_PROG_TEST_RUN`)**: `crates/overdrive-bpf/tests/integration/<name>.rs`
  — PKTGEN/SETUP/CHECK triptych against curated input. Map state
  cleared between sub-tests by default. aya 0.13.x does NOT expose
  `BPF_PROG_TEST_RUN` as a typed method; PKTGEN/SETUP/CHECK uses the
  project's `prog_test_run()` helper at
  `crates/overdrive-dataplane/src/sys/prog_test_run.rs` (or inline raw
  `libc::syscall(SYS_bpf, BPF_PROG_TEST_RUN, ...)` until the helper
  lands). See `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
  § C.1 / F.2 — no upstream typed-wrapper effort visible, helper
  expected to remain load-bearing across multiple aya releases.
- **Tier 3 (real veth)**: `crates/overdrive-dataplane/tests/integration/<name>.rs`
  — real packet plumbing through veth pairs in Lima / ubuntu-latest.
- **Tier 4 verifier-budget**: `perf-baseline/main/verifier-budget/veristat-<name>.txt`
  — instruction count baseline, ≤ 50 % of 1M-privileged ceiling.
  Enforced by `cargo verifier-regress` (binary lives in
  `crates/overdrive-dataplane/bin/verifier_regress.rs`; reads
  `bpf_prog_info.verified_insns` via aya, NOT veristat — see
  `.claude/rules/testing.md` § "Verifier complexity").
- **Tier 1 (DST)**: not directly applicable to kernel-side code (DST
  uses `SimDataplane`); the userspace handle that wraps the map IS
  Tier 1-tested.

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
