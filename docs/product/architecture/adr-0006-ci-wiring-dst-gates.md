# ADR-0006 — CI gates: `cargo xtask dst` and `cargo xtask dst-lint` with seed surfacing

## Status

Accepted. 2026-04-21.

## Context

User stories US-05 and US-06 require two CI checks to block merges:

- `cargo xtask dst-lint` — scans core crates for banned APIs (US-05).
- `cargo xtask dst` — runs the turmoil DST harness (US-06).

Both already have stub subcommands in `xtask/src/main.rs`; Phase 1 fills
in their bodies. Beyond wiring the commands, the CI integration must
handle three sub-concerns:

1. **Seed surfacing.** On a green run, the seed is printed in the summary
   line. On a red run, the seed, tick, host, invariant name, and a
   reproduction command must appear in the CI output so an engineer can
   paste it locally. Additionally the full DST output file (captured
   stdout + stderr) must be uploaded as a CI job artifact so an engineer
   can download it without re-running.
2. **Determinism of the seed itself.** `xtask dst` accepts `--seed <N>`.
   When not passed, the xtask generates a fresh random seed *from the OS
   entropy source* (not from any determinism-controlled path — the seed
   input is the *one* place real entropy enters the DST stack). The seed
   is printed on the very first line of output so it is preserved even
   if the run aborts mid-stream.
3. **Failure artifacts.** On failure, the CI pipeline uploads:
   - `dst-output.log` — full stdout/stderr of the `xtask dst` run.
   - `dst-summary.json` — structured summary (seed, tick, host, invariant,
     cause) that CI dashboards can parse.

The xtask `dst` command today is a one-line `cargo test --workspace
--features dst` wrapper. Phase 1 makes it the orchestrator: it emits the
seed, invokes the harness, captures output, formats the summary, and
exits with the harness's status.

## Decision

**xtask command surface** (filling in the stubs):

```
cargo xtask dst [--seed <u64>] [--only <INVARIANT_NAME>] [--nodes <N>]
cargo xtask dst-lint
```

### `cargo xtask dst`

Responsibilities:

1. Resolve the seed:
   - If `--seed <N>` provided, use it.
   - Otherwise read `OVERDRIVE_DST_SEED` environment variable.
   - Otherwise generate a fresh seed from OS entropy
     (`getrandom`-backed — xtask is a binary boundary crate, so
     `OsRng::u64()` is allowed).
2. Print the seed on the **first line** of output:
   `dst: seed = 17283645` (so a killed run still has the seed).
3. Invoke the DST harness via `cargo test --package overdrive-sim
   --features dst --test dst -- --seed <N> [--only <NAME>]`.
4. Pipe stdout + stderr through to the console verbatim.
5. On failure, append a final block to stderr:

   ```
   dst: FAILED
     seed       = 17283645
     invariant  = single_leader
     tick       = 8743
     host       = node-0
     cause      = two leaders observed
     reproduce  = cargo xtask dst --seed 17283645 --only single-leader
   ```

6. Exit with the harness's exit code.

### `cargo xtask dst-lint`

Responsibilities:

1. Read `cargo metadata` for the workspace.
2. Parse `package.metadata.overdrive.crate_class` on every workspace
   member. Fail if any member is missing the key (ADR-0003).
3. Assert the set of `crate_class = "core"` members is non-empty.
4. For each core crate, walk `src/**/*.rs` with `syn`, visit every
   `Expr::Call`, `Expr::Path`, `UseTree`, and assert no expression path
   resolves to a banned symbol.
5. Banned-symbol list is a single `const BANNED_APIS: &[BannedApi]`
   inside `xtask/src/dst_lint.rs`. Each entry carries:
   - The symbol path (e.g., `std::time::Instant::now`).
   - The replacement trait name.
   - A short one-line rationale.
6. Violations print:

   ```
   dst-lint: FAILED — 1 violation
     crates/overdrive-core/src/some_file.rs:42:12
       banned: std::time::Instant::now
       use:    Clock (crates/overdrive-core/src/traits/clock.rs)
       rule:   .claude/rules/development.md#nondeterminism-must-be-injectable
   ```

7. Exit non-zero on any violation; exit zero with
   `dst-lint: 0 violations across <N> core crates` on success.

### CI wiring (actionable for platform-architect)

Two required checks, added to the existing `cargo test` and `cargo
clippy` checks:

```yaml
# Sketch — platform-architect adapts to the actual CI DSL
- name: dst-lint
  run: cargo xtask dst-lint

- name: dst
  run: cargo xtask dst
  # Upload both artifacts regardless of success, to avoid the "what seed
  # reproduced this?" trap.
- name: upload dst logs
  if: always()
  uses: actions/upload-artifact@v4
  with:
    name: dst-output
    path: |
      target/xtask/dst-output.log
      target/xtask/dst-summary.json
    retention-days: 30
```

The `dst-output.log` and `dst-summary.json` files are written by
`xtask dst` to `$CARGO_TARGET_DIR/xtask/` on every run.

### Failure-artifact schema

```json
{
  "seed": 17283645,
  "status": "failed",
  "invariant": "single_leader",
  "tick": 8743,
  "host": "node-0",
  "cause": "two leaders observed",
  "reproduce": "cargo xtask dst --seed 17283645 --only single-leader",
  "git_sha": "abc123",
  "toolchain": "stable-1.85"
}
```

## Alternatives considered

### Option A — Run DST directly from CI without xtask wrapper

Skip xtask; CI calls `cargo test --package overdrive-sim --features dst`
directly. **Rejected.** The xtask wrapper owns seed generation,
determinism-friendly seed printing on the first line, summary formatting,
and artifact file writing. Doing any of these from CI YAML is fragile and
duplicates logic between local developer runs and CI.

### Option B — Make the seed come from a deterministic source (git SHA)

Derive the default seed from `git rev-parse HEAD`. **Rejected.** Every
run on the same SHA would use the same seed, so a flaky bug hiding on
some other seed never surfaces. The whole point of a random seed on
each run is to sample the space. The fix for a flaky discovery is
`--seed <N>` for reproduction, not deterministic seed input.

### Option C — Two xtask subcommands, no artifact upload

Skip the JSON artifact; CI scrapes the text output. **Rejected.**
Dashboards parsing text output is a known brittleness source. The JSON
artifact is a few hundred bytes, added once in xtask, and adds nothing
to the developer's local experience.

### Option D — xtask dst + dst-lint with artifacts (chosen)

See Decision above.

## Consequences

### Positive

- Single canonical invocation path: `cargo xtask dst` for developers and
  CI alike. No "well in CI we do it differently."
- Seed is discoverable on the first output line — even a truncated,
  killed, or OOM-killed run preserves it.
- JSON artifact is parseable by any CI dashboard without bespoke regex.
- Failure message is the paste-to-terminal reproduction, directly.

### Negative

- Two more CI checks on the critical path. Each is fast (dst-lint < few
  seconds, dst < 60s per K1 guardrail).
- `xtask dst` must parse the harness's exit and structured output to
  build the summary; a mismatch between the harness's output format and
  the xtask parser is a named integration risk. Mitigation: the harness
  emits a machine-readable trailer line (`DST_TRAILER: ...json...`) that
  xtask consumes.

### Neutral

- Artifact retention defaults to 30 days. Longer retention or a separate
  archive location is a platform-architect call.

## References

- `docs/feature/phase-1-foundation/discuss/user-stories.md` US-05, US-06
- `docs/feature/phase-1-foundation/discuss/outcome-kpis.md` K1, K2, K3
- `docs/feature/phase-1-foundation/discuss/shared-artifacts-registry.md`
  (`dst_seed`, `invariant_name`, `banned_api_list`)
- `.claude/rules/testing.md` (Tier 1 CI topology)
