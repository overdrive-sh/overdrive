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
