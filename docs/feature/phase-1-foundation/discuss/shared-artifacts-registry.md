# Shared Artifact Registry — phase-1-foundation

Tracking data that flows across the journey. Every `${variable}` in mockups must have a single source of truth.

## Registry

```yaml
shared_artifacts:

  dst_seed:
    source_of_truth: "crates/xtask/src/dst.rs — RNG seed sampled on harness init and printed on every run"
    consumers:
      - "DST harness summary output (green runs)"
      - "DST failure output (red runs)"
      - "Reproduction command printed inline with failure"
      - "CI artifact uploaded on failure"
      - "Bug reports linked from the failure (future)"
    owner: "overdrive-sim + xtask"
    integration_risk: "HIGH — if the seed format diverges between green and red output, engineers lose the ability to paste seeds across runs."
    validation: "A DST test asserts that given seed S, two sequential invocations produce identical summary bytes."

  sim_cluster_size:
    source_of_truth: "crates/xtask/src/dst.rs default (3) overrideable via --nodes flag"
    consumers:
      - "DST harness header line"
      - "Quorum-dependent tests (majority/minority)"
    owner: "xtask"
    integration_risk: "LOW — the value is only displayed, not exchanged across module boundaries."
    validation: "xtask unit test asserts default equals 3."

  invariant_name:
    source_of_truth: "crates/overdrive-sim/src/invariants.rs — a single enum whose variants are the canonical names"
    consumers:
      - "DST progress lines (green)"
      - "DST failure lines (red)"
      - "Reproduction --only <NAME> flag"
      - "Documentation references"
    owner: "overdrive-sim"
    integration_risk: "HIGH — if the name printed in tests differs from what --only accepts, reproductions silently target nothing."
    validation: "xtask test asserts every printed name round-trips through FromStr back into the same enum variant."

  newtype_canonical_string:
    source_of_truth: "Display impl on each newtype in crates/overdrive-core/src/id.rs (and related modules)"
    consumers:
      - "FromStr parsing (must accept the Display output)"
      - "serde Serialize/Deserialize (must match the Display output byte-for-byte)"
      - "rkyv archived form (must be derivable from the same canonical bytes)"
      - "CLI output (future)"
      - "Log messages"
    owner: "overdrive-core"
    integration_risk: "HIGH — any drift breaks content-hashing (development.md: 'Hashing requires deterministic serialization') and breaks DST replay determinism."
    validation: "proptest round-trip for every newtype: `x == FromStr(Display(x))` AND `Display(x) == serde_json::to_string(&x).trim_matches('\"')` (or equivalent byte comparison)."

  snapshot_bytes:
    source_of_truth: "IntentStore::export_snapshot() — rkyv-archived bytes with a defined framing header"
    consumers:
      - "LocalStore tests (round-trip)"
      - "RaftStore bootstrap_from (future, HA migration)"
      - "Disaster-recovery backups (future)"
      - "Snapshot catalogue on disk or in object storage (future)"
    owner: "overdrive-core IntentStore trait + LocalStore impl"
    integration_risk: "HIGH — the non-destructive single→HA migration story in commercial.md depends on this being bit-identical across implementations."
    validation: "LocalStore::bootstrap_from(export_snapshot(S1)) produces a store S2 whose own export_snapshot() equals S1's export_snapshot() byte-for-byte."

  banned_api_list:
    source_of_truth: "crates/xtask/src/dst_lint.rs — BANNED_APIS constant (array of &'static str symbol paths)"
    consumers:
      - "xtask dst-lint scanner"
      - "CI gate"
      - ".claude/rules/development.md (documentation references the constant name, does not re-list items)"
    owner: "xtask"
    integration_risk: "MEDIUM — if docs drift from the constant, engineers learn rules that the gate does not enforce. Lower than HIGH because the gate's behaviour is what matters operationally; docs are advisory."
    validation: "xtask unit test covers every banned symbol with a synthetic source file and asserts the scanner flags it."

  core_crate_boundary:
    source_of_truth: "workspace Cargo.toml — a metadata field (e.g. `package.metadata.overdrive.crate_class = \"core\"`) on each core crate"
    consumers:
      - "dst-lint scanner (determines which crates to scan)"
      - ".claude/rules/development.md (documents the rule and where the labels live)"
    owner: "workspace + xtask"
    integration_risk: "HIGH — a new core crate that is not labelled becomes a lint-gate blind spot."
    validation: "xtask dst-lint has a built-in assertion that the set of core crates is non-empty and that every scanned crate carries the label."
```

## Consistency check questions (answered)

- **Does every ${variable} in TUI mockups have a documented source?** Yes — see table.
- **If `dst_seed` format changes, would all consumers automatically update?** Yes — the seed is formatted in one place (`xtask::dst::print_summary`) and all printing of seeds routes through it.
- **Are there hardcoded values that should reference a shared artifact?** The default `sim_cluster_size` (3) is hard-coded in xtask; acceptable because it's the documented default and is displayed, not exchanged.
- **Do any two steps display the same data from different sources?** No — invariant names come from the `overdrive-sim` enum in every step, seed comes from the xtask RNG in every step.

## Quality gates

- [x] Journey completeness — all 4 steps have goal, command, mockup, emotional annotation, artifacts, integration checkpoint, failure modes, gherkin.
- [x] Emotional coherence — Skeptical → Confident → Reassured → Trusting follows the Confidence Building pattern; no jarring transitions.
- [x] Horizontal integration — `dst_seed` and `invariant_name` appear across Steps 1 and 4 and are sourced from the same modules; `newtype_canonical_string` is single-sourced.
- [x] CLI UX compliance — output follows clig.dev patterns (progress counter, summary line, error messages answer "what/why/fix").

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial registry for phase-1-foundation. |
