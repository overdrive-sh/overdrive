# ADR-0003 — Core-crate labelling via `package.metadata.overdrive.crate_class`

## Status

Accepted. 2026-04-21. Amended 2026-04-23 — see **Amendment
2026-04-23** below.

## Context

The banned-API lint gate (`cargo xtask dst-lint`, US-05) must scan
**exactly** the crates whose code is on the DST determinism path —
"core" crates in the architectural sense (ports + pure logic). It must
*not* scan:

- Adapter crates legitimately implementing real I/O (the whole point of
  an adapter is that it uses `Instant::now`, `TcpStream::connect`, etc.).
- Binary crates (CLI, xtask) that run the adapters.
- Future sim/adapter crates that use `turmoil`, `StdRng`, etc.

A crate's class must therefore be *declared*, not inferred. Several
mechanisms are available:

1. **`package.metadata.overdrive.crate_class = "core"`** in each crate's
   `Cargo.toml`. Cargo ignores unknown `[package.metadata.*]` keys; any
   tool can read them via `cargo metadata --format-version=1`.
2. **`workspace.metadata.overdrive.core_crates = [...]`** in the workspace
   root `Cargo.toml`. One central list.
3. **Build-script markers** — a crate opts in by having a `build.rs` that
   writes a sentinel file.
4. **Filesystem convention** — a crate is core if its path matches a
   regex (e.g. `crates/*-core`). Labelling is implicit.
5. **A dedicated manifest file** at workspace root (e.g.
   `.overdrive.crate-classes.toml`).

Requirements:

- Labels must live next to the crate they label, or be trivially
  discoverable, so that adding a new core crate is a local change, not a
  cross-file hunt.
- A missing label for a *new* core crate must be detectable and blockable
  (otherwise the lint gate silently skips the new crate — the precise
  failure mode `shared-artifacts-registry` rates HIGH risk).
- The mechanism must be consumable by `xtask` without a new dependency
  (we already parse workspace metadata).
- Documentation in `.claude/rules/development.md` must reference the
  mechanism by name without restating the list.

## Decision

**Per-crate metadata: `package.metadata.overdrive.crate_class = "<class>"`**
in each crate's `Cargo.toml`.

The class values are a fixed enum:

- `"core"` — ports + pure logic. Banned-API lint scans.
- `"adapter-host"` — host adapter (implements a port against the host
  OS / kernel / network with real I/O). Renamed from `"adapter-real"`
  on 2026-04-23; see Amendment below.
- `"adapter-sim"` — sim adapter (implements a port for DST; uses turmoil,
  StdRng, etc.).
- `"binary"` — binary boundary (CLI, xtask).

Additional requirements:

- `xtask dst-lint` reads workspace metadata via `cargo_metadata` (or a
  manual `cargo metadata --format-version=1` invocation; either is fine,
  both are zero-cost dependencies relative to what xtask already pulls).
- An **early assertion** at the start of `dst-lint`: the set of
  `crate_class = "core"` crates is non-empty; otherwise fail with
  "no core-class crates found — did someone rename the metadata key?"
- A **second assertion**: *every* workspace crate MUST declare a
  `crate_class`. An unlabelled crate fails the lint with a structured
  error pointing at the Cargo.toml to add the label to. This closes the
  "new core crate added but not labelled" blind spot.

Phase 1 crate labelling:

```toml
# crates/overdrive-core/Cargo.toml
[package.metadata.overdrive]
crate_class = "core"

# crates/overdrive-store-local/Cargo.toml
[package.metadata.overdrive]
crate_class = "adapter-host"

# crates/overdrive-sim/Cargo.toml
[package.metadata.overdrive]
crate_class = "adapter-sim"

# crates/overdrive-cli/Cargo.toml
# xtask/Cargo.toml
[package.metadata.overdrive]
crate_class = "binary"
```

## Alternatives considered

### Option A — `workspace.metadata.overdrive.core_crates = [...]`

One central list in workspace `Cargo.toml`. **Rejected.** Adding a new
core crate requires editing a file in an unrelated directory. The
central list is a perennial source of drift (someone adds the crate,
forgets the list, lint silently passes). Discoverability is poor — an
engineer looking at `overdrive-core/Cargo.toml` cannot tell whether it is
a core crate without grepping the workspace root.

### Option B — Filesystem convention

`crate_class = "core"` iff `crates/overdrive-core`-named. **Rejected.**
Implicit labelling has no place to add new classes (`adapter-host`
vs `adapter-sim`) without either renaming crates or adding further
regex rules. Naming is an organisational concern, not a contractual one.

### Option C — Build-script marker file

A crate opts in by having a `build.rs` that emits a sentinel. **Rejected.**
Adds a build.rs to every core crate, slowing builds for no gain. The
sentinel file is a second source of truth; the Cargo.toml would still
need to reference it. Strictly worse than A, B, or D.

### Option D — Dedicated manifest file at workspace root

`.overdrive.crate-classes.toml` at workspace root. **Rejected.** Same
drift problem as Option A plus a new file to document.

### Option E — Per-crate `package.metadata` (chosen)

See Decision above.

## Consequences

### Positive

- Adding a core crate is a local change: new `Cargo.toml` gets a metadata
  block, period. Engineers who add crates naturally edit their own
  `Cargo.toml`.
- `cargo metadata` is a standard toolchain feature; no new dependency.
- The "every crate must declare" assertion closes the blind spot.
- The metadata block is visible to humans scanning `Cargo.toml`.

### Negative

- Slight duplication across crates (four classes listed in four places in
  Phase 1). Acceptable — the list grows linearly with crate count, not
  with code size.
- A mistyped class value (`"adapter_host"` instead of `"adapter-host"`)
  is not caught without a validating parser. Mitigated by a `FromStr`
  parser on the class enum inside xtask with an exhaustive error message.

### Neutral

- The mechanism scales naturally to future classes (`test-only`,
  `wasm-plugin`, etc.) without structural change.

## Amendment 2026-04-23 — `adapter-real` renamed to `adapter-host`

The `"adapter-real"` class value is renamed to `"adapter-host"`. The
taxonomy shape (four classes, per-crate `package.metadata.overdrive.
crate_class`, xtask-enforced declarations) is unchanged. Only the
string value moves.

Rationale:

- **Symmetric vocabulary.** `adapter-host` vs `adapter-sim` is the
  structural axis in the codebase — host-side bindings against the
  OS / kernel / network vs simulated-host bindings against turmoil.
  "Real vs fake" was DST-discourse framing, not an observable crate
  property; "host vs sim" matches what the code actually is.
- **Consistency with whitepaper.** The whitepaper already uses
  host/guest vocabulary (§6 guest agent, §17 `vhost-user-fs`); the
  class name now aligns.
- **Co-landed crate extraction.** The same session extracted the
  `overdrive-host` crate (holds `SystemClock`, `OsEntropy`,
  `TcpTransport`) out of the `overdrive-sim::real::*` module that had
  been parked behind a `real-adapters` Cargo feature. The new crate's
  class name and the class value now share a word, which is the point.
  The extraction itself is recorded in ADR-0016.

Scope of the rename:

- `xtask` taxonomy enum, `dst-lint` allow-list, allow-list failure
  messages, and acceptance-test strings updated.
- `crates/overdrive-host/Cargo.toml` and
  `crates/overdrive-store-local/Cargo.toml` declare `crate_class =
  "adapter-host"`.
- Every doc referencing the class value (`CLAUDE.md`, `brief.md`,
  ADR-0008, ADR-0012, `wave-decisions.md`) follows.
- The `adapter-sim`, `core`, and `binary` values are unchanged.

This is an in-place amendment, not a supersession: the labelling
mechanism (per-crate metadata, every-crate-must-declare, etc.) stands
unmodified. Only the string value of one enum variant changed.

## References

- `docs/feature/phase-1-foundation/discuss/shared-artifacts-registry.md`
  (`core_crate_boundary` artifact, HIGH integration risk)
- `docs/feature/phase-1-foundation/discuss/user-stories.md` US-05
- Cargo book: [Custom
  metadata](https://doc.rust-lang.org/cargo/reference/manifest.html#the-metadata-table)
- ADR-0016 — `overdrive-host` crate extraction and `adapter-real` →
  `adapter-host` rename.
