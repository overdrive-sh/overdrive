# DESIGN Wave Review — workload-kind-discriminator

**Reviewer**: `nw-solution-architect-reviewer` (independent hard gate before DEVOPS / DISTILL)
**Date**: 2026-05-10
**Verdict**: **APPROVED**

**Scope**: ADR-0047 (NEW), amendments to ADR-0011/0031/0032/0033/0037 (all 2026-05-10), brief.md §54-§62, design/wave-decisions.md, design/c4-diagrams.md, design/upstream-changes.md, whitepaper amendments (24 sites).

The DESIGN wave is **internally consistent, complete, and ready for handoff to DEVOPS and DISTILL**. Zero blocking issues across all 10 review categories.

---

## Verdict

**APPROVED for parallel handoff to:**
- `@nw-platform-architect` (DEVOPS wave) — receives KPIs + brief.md §62 CI handoff annotations.
- `@nw-acceptance-designer` (DISTILL wave) — receives full DESIGN package.

No review iterations needed.

### Blocking-issue count by category

| Category | Issues |
|---|---|
| Reuse Analysis (RCA F-1 fix) | 0 |
| C4 diagrams (L1+L2+L3) | 0 |
| ADR completeness & consistency | 0 |
| Design-time decisions (3/3) | 0 |
| Universal flattening principle | 0 |
| Forward pointers & deferral discipline | 0 |
| Whitepaper amendments | 0 |
| Cross-artifact consistency | 0 |
| Hard rules (CLAUDE.md) | 0 |
| Test gating (Lima-aware) | 0 |

---

## Top 3 things Morgan got conspicuously right (`praise:`)

### praise: ADR-0047 decision quality is exceptional

The ADR carries all five required sections — context (RCA root causes B+C+D + 13/15 vendor industry validation), decision (four sections covering aggregate shape, wire shape, streaming protocol, denormalisation, render branches), alternatives (five considered A-E, all rejected with reasoning), consequences (4 positive + 4 negative + 6 quality attributes), and implementation notes (slice ordering locked).

Every architectural choice — enum over independent types, per-kind enums over flat enum, section-as-discriminator over internally-tagged field — has a load-bearing rationale that prevents future misunderstanding.

### praise: Whitepaper "Changed Assumptions" section is honest and surgical

The whitepaper amendment is NOT a rewrite. It names the driver ADR (ADR-0047) and date (2026-05-10), documents the 24 site edits, AND documents what was deliberately NOT edited (SPIFFE IDs, code identifiers, generic English uses of "job", DST test code) with sound rationale: `job/<name>` is the canonical workload-identity scheme across all three kinds; code identifiers are governed by ADR amendments, not whitepaper prose.

This is mature documentation discipline. The architect resisted the temptation to rename everything and instead kept anchors in place.

### praise: C4 diagrams are complete, unambiguous, and show the load-bearing parts

L1 unchanged (correctly documented as such). L2 annotated with the four affected containers (`overdrive-core`, `overdrive-cli`, `overdrive-control-plane`, `xtask`, all EXTENDED). L3 NEW — spec-parser pipeline showing the three branch points (parser → streaming dispatcher → render dispatcher) with closed `WorkloadKind` enum used as the discriminator at all three.

Every arrow has a verb. No abstraction-level mixing. The diagram makes the "three sibling enums" decision visible at the architectural level.

---

## Top 3 most surprising findings

### Universal flattening principle is canonically stated AND consistently enforced

ADR-0047 §2 explicitly names the principle across all workload-level concerns — `[microvm]`, `[[sidecars]]`, `[[policies]]`, `[security]` are sibling top-level tables, not nested under the kind discriminator. This supersedes any prior whitepaper TOML examples showing nested form. The architect got this right post-correction: it's not just a parser convenience, it's an architectural constraint that prevents the "kind container carries too many nested concerns" anti-pattern.

### Reuse Analysis: 10 EXTEND, 1 CREATE NEW — counted correctly

Wave-decisions.md §"Reuse Analysis" is comprehensive. 10 components extend (Job→JobSpec rename, DriverInput preservation, Proto reuse, SubmitEvent split, AllocStatusRow additive columns, render functions, xtask, coinflip.toml). 1 CREATE NEW (`Listener`) with explicit justification: bounded-context separation from dataplane `Backend` to avoid the collision the user explicitly named during the GH #164 fold-in. Default for overlap was EXTEND; only the bounded-context-distinct Listener type justified CREATE NEW.

### Three design-time decisions resolved are non-obvious, well-defended, will survive Phase 2

- **Slice 06 kept whole** — alloc-status render extension is ~2h mechanical against embedded `Vec<ListenerRow>`; splitting would create halves where 06b cannot demonstrate value alone. Decision is right and doesn't foreclose adding a separate listener-index table later per ADR-0047 §4a.
- **K3 cadence = pre-release manual gate** — usability cannot be measured by continuous CI; one-shot at feature release is right-sized; the automated parsing-from-fixtures regression test is the complementary check.
- **Listener embedding on AllocStatusRow** — render path is Phase 1's only consumer; full-row write matches whitepaper §4 guardrail; future VIP allocator (#167) can own a separate table if needed without breaking the embedded shape.

---

## Detailed findings by dimension

### 1. ADR completeness and consistency

All five amendments (to ADR-0011, 0031, 0032, 0033, 0037) carry: 2026-05-10 date, decision-maker named (Morgan, DESIGN wave), section reference (e.g., "ADR-0047 §1 / §3 / §4"), self-contained language (future reader can read just the amendment block), cross-references to sibling amendments.

Cross-ADR consistency verified: `JobSubmitEvent` has no `ConvergedRunning` (ADR-0032) AND `Job` emits `Completed{exit_code}/Failed{exit_code}` (ADR-0037) AND `AllocStatusRow.kind` denormalised (ADR-0033) AND `ServiceSpecInput` has `[[listener]]` array (ADR-0031). All four amendments compose without contradiction.

**Special case — ADR-0019**: wave-decisions.md correctly notes "NO CHANGE" — ADR-0019 governs operator config (YAML→TOML), not workload specs (ADR-0031's surface). The DISCUSS handoff list mistakenly named ADR-0019; the architect caught the error and documented the non-edit.

### 2. Forward pointer discipline

All deferrals cite verified GitHub issues:
- **#166** (Schedule execution semantics) — referenced in ADR-0047 §6, wave-decisions.md, brief.md §57-§59
- **#167** (VIP allocator primitive) — referenced in ADR-0047 §4a, wave-decisions.md, brief.md §56, §62
- **#163** (Listener dataplane wiring) — wave-decisions.md (pre-existing, out of scope)
- **#170** (Service health-check primitive) — wave-decisions.md (replaces closed #169)

#169 (settle-window) correctly closed 2026-05-10 and superseded by #170. No invented numbers, no `<N>` placeholders.

### 3. Hard rules (CLAUDE.md) compliance

- **No GH issue creation by architect**: orchestrator created #170; architect did not create issues.
- **All-priorities scope**: feature resolves three open questions (Slice 06 split, K3 cadence, listener denormalisation shape). All covered.
- **No effort budget cuts**: slice 06 kept whole; no deferred sub-work.
- **Single-cut migration**: Job → JobSpec rename in one PR train. No compat layer documented. Phase 1 greenfield.
- **ADR-0046 collision resolved**: original numbering error caught and corrected post-return; renumbered to 0047; 46 references migrated across 9 files; downstream `docs/evolution/` and `docs/research/` references to the existing collision-free-backend-id-allocator ADR correctly preserved.

### 4. Test gating (Lima-aware)

Brief.md §62 handoff annotations:
- `cargo xtask lima run -- cargo nextest run -p overdrive-cli -E 'test(coinflip_honesty)'` — correctly Lima-wrapped
- `cargo xtask lima run -- cargo nextest run -p overdrive-cli -E 'test(service_listener_roundtrip)'` — correctly Lima-wrapped
- `cargo openapi-check` — not Lima-wrapped (correct; compile-side check)
- `xtask dst-lint` — not Lima-wrapped (correct; linting pass)

Both integration tests correctly routed through `cargo xtask lima run --` per `.claude/rules/testing.md` § "Running tests — Lima VM".

### 5. Architectural bias check

| Bias pattern | Assessment |
|---|---|
| Technology preference over requirements? | No — tagged enum chosen because natural Rust shape for closed discriminator |
| Resume-driven development | No — no new dependencies, no new patterns |
| Unproven tech | No — Rust 2024, serde, thiserror, existing workspace deps |
| Complexity exceeds team capability | No — type-shape + parser + streaming dispatcher + render branches |

### 6. Quality attribute coverage

| Quality attribute | Coverage |
|---|---|
| Functional correctness | Excellent — invalid states unrepresentable; `ConvergedRunning` structurally absent for Job |
| Security | Preserved — no new attack surface |
| Reliability | Improved — honesty rate (K1) 0% → ≥99% by construction |
| Maintainability | Improved — exhaustive match on kind at three branch points |
| Testability | Good — per-kind streaming enums testable in isolation |
| Observability | Preserved |
| Performance | Neutral — three enum variants vs one; negligible impact |

---

## Cross-artifact consistency check

**DISCUSS vocabulary survives intact**: Service / Job / Schedule three kinds, Ana persona, KPIs K1-K6, listener / alloc_status / spec / streaming bounded contexts.

**DISCUSS slices unchanged**: 6 slices land in same order; no deferral language moved; #166/#167/#163/#170 verified real.

**DISCUSS assumptions hold**: section-as-discriminator (D3) confirmed in ADR-0047 §2; per-kind streaming protocol (D2+D7) confirmed in ADR-0047 §3 + ADR-0032 Amendment; kind denormalised on observation rows (D4) confirmed in ADR-0047 §4 + ADR-0033 Amendment.

No DISCUSS user story is invalidated. No DISCUSS journey is contradicted.

---

## Final assessment

The DESIGN wave is approved for immediate handoff:

- **DEVOPS (`@nw-platform-architect`)**: required CI checks (xtask dst-lint, KPI K1 integration test, KPI K6 integration test, cargo openapi-check) properly specified in brief.md §62. Lima-gating correctly applied where required.
- **DISTILL (`@nw-acceptance-designer`)**: source AC from `docs/feature/workload-kind-discriminator/discuss/user-stories.md` + design decisions. Trait surfaces (IntentStore, ObservationStore, Clock) stable. Every AC observable through DST, LocalStore, or lint-gate output.

Morgan delivered a complete, coherent, well-defended architecture that closes the bug, respects constraints, and sets up Phase 2 extensions.

## Reviewer attestation

- Reviewed all 11 DESIGN-wave artifacts (1 new ADR + 5 amendments + brief.md extension + 3 feature/design files + whitepaper).
- Verified GH issue references against orchestrator's prior verification (#163, #166, #167, #170; #169 closed and correctly superseded).
- Did not modify any artifact (review-only output).
- Did not create any GitHub issue.

**Handoff cleared. Ready for `/nw-devops` and `/nw-distill` dispatch in parallel.**
