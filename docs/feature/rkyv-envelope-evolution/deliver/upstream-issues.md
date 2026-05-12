# Upstream issues surfaced during DELIVER

## UI-01 — ADR-0048 § 2 Layer 1 cannot literally constrain inner payloads to `pub(crate)`

**Surfaced**: 2026-05-12 during step 01-01 (RED scaffolds, commit `0dc53e05`).

**Affected artifacts**:
- `docs/product/architecture/adr-0048-rkyv-versioned-envelope.md` § 2 Layer 1
- `docs/feature/rkyv-envelope-evolution/design/wave-decisions.md` § 4 C, § 9 (C)
- `docs/feature/rkyv-envelope-evolution/distill/red-scaffolds.md` (Group 2 / Group 3 `pub(crate)` annotation)
- `.claude/rules/development.md` § "rkyv schema evolution" Rules bullet 1

**Issue**: ADR-0048 § 2 Layer 1 mandates inner payload types (`AllocStatusRowV1`,
`NodeHealthRowV1`, etc.) be declared `pub(crate)` so that cross-crate writers
in `overdrive-store-local` cannot name the payload type and therefore cannot
construct a value to put inside `Envelope::V1(...)`.

The literal `pub(crate)` declaration fails to compile with **rustc E0446**
(`crate-private type ... in public interface`). The chain is:

1. `VersionedEnvelope` is `pub` (the trait is the codec primitive every crate
   consumes via `Envelope::latest(...)`).
2. `type Latest = AllocStatusRowV1;` inside an `impl VersionedEnvelope for
   AllocStatusRowEnvelope` makes `AllocStatusRowV1` part of the trait's
   public surface.
3. rustc rejects `pub(crate)` on a type referenced from a `pub` trait's
   associated-type assignment.

**Crafter resolution in commit `0dc53e05`**: declared the inner payload types
as plain `pub`, kept them un-re-exported from `overdrive-core::lib.rs`. Cross-crate
writers can still reach them via the verbose path
`overdrive_core::traits::observation_store::AllocStatusRowV1` — discouraged
by code review, not blocked by the compiler.

**Consequence**: Layer 1's enforcement is weaker than the ADR claims. The
**structural defense** for the write-time invariant collapses to Layer 2
(the `xtask::dst_lint` variant-construction scanner that lands in step
03-01). The compile-fail trybuild fixture in S-EV-02a (step 03-01) will
need adjustment — it cannot assert `AllocStatusRowV1` is private (E0603);
it can only assert non-importability via `use overdrive_core::AllocStatusRowV1`
(E0432, "unresolved import") because the type isn't re-exported.

**Resolution options** (awaiting user decision):

1. **Accept and amend the SSOT.** Treat Layer 1 = "inner payloads un-re-exported
   + Layer 2 in-crate variant-construction lint" and update:
   - ADR-0048 § 2 Layer 1 (and § 9 Consequences) to acknowledge rustc E0446 and
     describe the actual mechanism (non-re-export plus Layer 2).
   - `.claude/rules/development.md` § "rkyv schema evolution" Rules bullet 1
     mirror language.
   - DISTILL red-scaffolds.md note on `pub(crate)`.
   - S-EV-02a fixture (step 03-01) to assert E0432 on the import rather than
     E0603 on a `pub(crate)` access.

2. **Restructure to preserve `pub(crate)` literally.** Move `VersionedEnvelope`
   to `pub(crate)` and use a `pub trait` re-export shim. This complicates the
   cross-crate API (every consumer of `Envelope::latest(...)` would route
   through the shim) for an enforcement gain that Layer 2 already provides.
   Not recommended.

3. **Make the inner payload types `#[doc(hidden)] pub` and rely on Layer 2.**
   Mechanically the same as option 1, but adds a `doc(hidden)` annotation
   to make the intent visible at the source. Reasonable cosmetic improvement.

**Recommendation**: option 1. Layer 2 (dst-lint scanner) is the load-bearing
artifact — the ADR already acknowledges this in § 5 / § 9 ("a single
complementary trybuild fixture is still recommended, see S-EV-02"). The
amendment makes the SSOT honest about which mechanism does what work.

**Action required from user before continuing past step 01-01**: confirm
which resolution to apply. If option 1 or 3, the architect agent should
amend the SSOT files; the trybuild fixture in step 03-01 will be adjusted
accordingly when that step runs.

**Resolution (2026-05-12)**: User confirmed option 1. Amendment landed in
commit `62bf6ed6` (`docs(rkyv-envelope): amend ADR-0048 § 2 Layer 1 —
rustc E0446 reconciliation`).

---

## UI-02 — Public alias shape: payload, not envelope

**Surfaced**: 2026-05-12 during step 01-03 (originally landed at commit
`a90755a2`; the orchestrator pushed back on the crafter's shape, which
triggered an attempted re-migration; user halted and reversed direction).

**Affected artifacts**:
- `docs/product/architecture/adr-0048-rkyv-versioned-envelope.md` (any
  reference to "alias preserves call-site name" coupled with an envelope
  target)
- `docs/feature/rkyv-envelope-evolution/design/wave-decisions.md` § 4 C, § 5,
  § 9 (C), § H ("Job aggregate"), § 10 S-EV-02a precondition phrasing
- `docs/feature/rkyv-envelope-evolution/distill/red-scaffolds.md`
  Group 2 / Group 3 alias examples
- `docs/feature/rkyv-envelope-evolution/distill/test-scenarios.md`
  S-EV-02a fixture body shape
- `docs/feature/rkyv-envelope-evolution/distill/wave-decisions.md` (the
  DISTILL one, distinct from DESIGN)
- `.claude/rules/development.md` § "rkyv schema evolution" example envelope shape
- `docs/feature/rkyv-envelope-evolution/deliver/roadmap.json` step 01-03
  and 01-04 criteria

**Issue**: The roadmap criterion for step 01-03 specified
`pub type AllocStatusRow = AllocStatusRowEnvelope` (alias to the envelope
enum). Step 01-04 mirrored this for `Job` (`pub type Job = JobEnvelope`).
This shape introduces THREE public names per row type (`<Type>`,
`<Type>Envelope`, `<Type>Latest`), forces ~50-70 call-site rewrites per
type (struct-literal `<Type> { fields }` → `<Type>::latest(<Type>Latest {
fields })`), and requires every internal helper that field-accesses a row
to re-type its parameter to `<Type>Latest`.

The cost is real (high call-site churn, three-name API) and the buys are
minimal:

1. **Schema-evolution clarity** does NOT improve materially over the
   alternative shape (alias-to-payload). With alias-to-payload, V1→V2
   evolution re-aliases `<Type> = <Type>V2`; callers that reference a
   removed/renamed field break at compile time exactly where the schema
   change touches them — the correct signal at the correct moment.

2. **Layer-2 dst-lint enforcement** is identical under both shapes —
   the scanner targets `<Envelope>::V<N>(...)` constructions, which
   only the persistence-boundary code performs regardless of which name
   the public alias points at.

3. **Consistency across row types** is achievable under either shape;
   the choice is uniform per-row-type once we pick.

**Decision (2026-05-12)**: revert to the alias-to-payload shape (what
the first crafter shipped at commit `a90755a2`):

```rust
pub type AllocStatusRow = AllocStatusRowV1;           // public payload alias — callers unchanged
pub type AllocStatusRowLatest = AllocStatusRowV1;    // retained as the canonical "latest payload" name for future-proofing
pub enum AllocStatusRowEnvelope {                     // codec-internal — appears only at the redb wire boundary
    V1(AllocStatusRowV1),
}
```

Same rule for `Job` in step 01-04: `pub type Job = JobV1`. The envelope
type lives in `aggregate/mod.rs` and is consumed only by `LocalStore`'s
read/write paths.

**Consequence**: commit `a90755a2` is correct under shape 1 and remains
the GREEN landing for step 01-03. The orchestrator's earlier pushback
(which led to the partial migration in the stashed working tree) was a
design mistake.

**Action required**: the architect amends the SSOT to reflect shape 1,
including the roadmap criteria for steps 01-03 and 01-04. Then DELIVER
resumes with step 01-04 against the corrected criteria.

