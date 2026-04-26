# CI/CD Pipeline — phase-1-control-plane-core delta

**Status**: DEVOPS wave artifact. Describes the **delta** from
phase-1-foundation's `.github/workflows/ci.yml` + `nightly.yml` — not
a full pipeline redesign.

**Principle**: phase-1-foundation landed the authoritative CI
foundation (five required checks, one nightly mutation trend job).
This feature inherits all of it, adds one new required check, and
relies on existing jobs to absorb the rest of the new material.

---

## Existing required status checks (unchanged)

Phase-1-foundation configured branch protection on `main` to require
five status checks — see `docs/feature/phase-1-foundation/devops/
ci-cd-pipeline.md` §"Branch-protection setup recipe" for the `gh api`
commands. This feature does NOT modify any of them:

| Job name in ci.yml | What it runs | Continues unchanged |
|---|---|---|
| `fmt-clippy` | `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets -- -D warnings` | ✅ Picks up `overdrive-control-plane` automatically |
| `test` | `cargo nextest run --workspace --locked --profile ci` + `cargo test --doc --workspace --locked` | ✅ New crate's tests discovered automatically |
| `integration` | `cargo nextest run --workspace --features integration-tests -E 'binary(integration)'` | ✅ New crate's `tests/integration/*.rs` discovered automatically |
| `dst` | `cargo xtask dst` (iterates `ALL_VARIANTS`) | ✅ New invariants auto-scheduled once scaffolds are replaced |
| `dst-lint` | `cargo xtask dst-lint` (scans `crate_class = "core"` crates) | ✅ `overdrive-control-plane` is `adapter-host`, not scanned; `overdrive-core::reconciler` additions are scanned |
| `mutants-diff` | `cargo xtask mutants --diff origin/main` | ✅ Existing `.cargo/mutants.toml` exclusion list unchanged; new crate is in-scope |

No changes to required-status-check configuration on `main` beyond
adding `openapi-check` (see §New required check below).

---

## Existing nightly job (unchanged)

`nightly.yml` job `mutants-workspace`: `cargo xtask mutants
--workspace` at 03:13 UTC on schedule. Soft-fails on >2pp drift below
baseline; hard-fails below 60% absolute. Unchanged — the new crate's
mutants contribute to the workspace-wide kill rate automatically.

---

## New required status check

### `openapi-check` — schema-drift gate per ADR-0009

**Job name in ci.yml**: `openapi-check`
**Trigger**: `pull_request: [main]` and `push: [main]` (same as
existing jobs)
**Dependencies**: none (runs in parallel with `fmt-clippy`, `test`, etc.)
**Timeout**: 10 minutes (matches existing job ceiling)

**What it runs**:

```
cargo xtask openapi-check
```

This xtask subcommand (per ADR-0009) regenerates the OpenAPI schema
from the Rust types in `overdrive-control-plane::api`, writes it to
a temp file, and runs a byte-exact diff against the checked-in
`api/openapi.yaml`. Non-empty diff → exit non-zero → CI fails.

**Why a separate job and not a step in `test`**: isolation. When the
CI fails, the failure mode ("schema drift, run `cargo xtask
openapi-gen`") is different from a unit-test failure. Separate jobs
give separate GitHub Checks entries, making the remediation
instruction unambiguous in the PR UI.

**Job shape (to be implemented by DELIVER)**:

```yaml
openapi-check:
  name: cargo xtask openapi-check
  runs-on: ubuntu-latest
  timeout-minutes: 10
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
      with:
        shared-key: "phase-1-openapi-check"
    # sccache soft-fail pattern per existing jobs
    - id: sccache-setup
      name: Setup sccache (soft-fail on cache backend outage)
      continue-on-error: true
      uses: mozilla-actions/sccache-action@v0.0.10
    - name: Enable sccache when setup succeeded
      if: steps.sccache-setup.outcome == 'success'
      run: |
        echo "RUSTC_WRAPPER=sccache" >> "$GITHUB_ENV"
        echo "SCCACHE_GHA_ENABLED=true" >> "$GITHUB_ENV"
    - name: Warn when sccache unavailable
      if: steps.sccache-setup.outcome != 'success'
      run: echo "::warning::sccache setup failed; building without compiler cache."
    - name: cargo xtask openapi-check
      run: cargo xtask openapi-check
    - name: Annotate failure with regeneration instruction
      if: failure()
      run: |
        {
          echo "## openapi-check failure"
          echo ""
          echo "\`api/openapi.yaml\` is out of sync with the Rust types in"
          echo "\`overdrive-control-plane::api\`. Regenerate locally:"
          echo ""
          echo '```'
          echo "cargo xtask openapi-gen"
          echo "git add api/openapi.yaml"
          echo '```'
          echo ""
          echo "Then commit and push. See ADR-0009."
        } >> "$GITHUB_STEP_SUMMARY"
        echo "::error title=openapi-check failed::schema drift — run 'cargo xtask openapi-gen' to regenerate"
```

**Branch-protection delta**: the required-status-checks list grows
from 5 to 6. The `gh api` recipe that phase-1-foundation documented
gains one entry:

```
contexts: [
  "fmt-clippy",
  "test",
  "integration",     # already present from phase-1-foundation
  "dst",
  "dst-lint",
  "mutants-diff",
  "openapi-check"    # NEW
]
```

The user configures this once via GitHub Settings → Branches → Branch
protection rules, or via the `gh api` recipe — same operator action,
one new entry.

---

## Coverage already handled without new jobs

Three concerns in the task prompt do not require new CI jobs; the
existing workspace-wide invocations absorb them.

### `cargo check --workspace` on new deps

The five new workspace dependencies (`axum`, `axum-server`,
`utoipa`, `utoipa-axum`, `libsql`) resolve and compile as part of
the existing `test` and `fmt-clippy` jobs. `cargo nextest run
--workspace --locked` performs `cargo check` implicitly before
running tests; any resolution failure (yanked version, feature
conflict, MSRV mismatch) surfaces there.

**No new job required.** If a dep resolution regression ever needs
an isolated signal, the `fmt-clippy` job can be extended with a
`cargo check --workspace --locked --all-targets` step — but
phase-1-foundation's authors chose not to, and this feature agrees.

### `cargo nextest` affected-test routing on new crate

Adding `overdrive-control-plane` to the root `Cargo.toml`'s
`members` list causes:

- `cargo nextest run --workspace` (CI `test` job) — discovers the
  new crate's tests automatically.
- Lefthook pre-commit `nextest-affected` hook — uses an
  `rdeps(...)` filter that expands transitively. Editing
  `overdrive-core` (now with new `reconciler` module) triggers
  downstream acceptance tests in `overdrive-control-plane` AND
  `overdrive-cli`. The hook body in `lefthook.yml` requires zero
  edits.
- Integration-gated tests under `crates/overdrive-control-plane/
  tests/integration/*.rs` — discovered by the `integration` CI job
  via `-E 'binary(integration)'` filter; `--features
  integration-tests` applies per-crate that declares the feature.

**Crafter action required**: declare `integration-tests` feature in
`crates/overdrive-control-plane/Cargo.toml` per testing.md
§"Integration vs unit gating" **only** if any test in the new crate
needs real infra (real libSQL file I/O, real axum server binding).
Unit-shaped tests stay in the default lane.

### DST invariants absorbed by existing `dst` job

DISTILL DWD-06 scaffold inventory pre-lands the three new invariant
enum variants (`AtLeastOneReconcilerRegistered`,
`DuplicateEvaluationsCollapse`, `ReconcilerIsPure`) into
`xtask/src/dst.rs`. The existing `cargo xtask dst` subcommand
iterates `ALL_VARIANTS`; the new variants are scheduled on the next
DST run automatically once the scaffolds compile.

**CI will be RED during DELIVER** — scaffold `panic!` bodies fail
their DST invariants immediately. This is **the expected red-green
cycle** per ADR-0001 scaffolding discipline; the crafter replaces
scaffolds one at a time, and CI transitions green as each body is
filled in. A reviewer encountering red CI mid-DELIVER should look
at the failing invariant name against `docs/feature/phase-1-control-
plane-core/slices/slice-4-*.md` progress — not at the workflow.

---

## `--no-verify` discipline during DELIVER

Phase-1-foundation's lefthook hooks auto-fail on scaffold `panic!`
bodies (specifically, the `clippy`, `doctest`, and
`nextest-affected` pre-commit commands). The expected DELIVER
workflow:

1. Crafter replaces a scaffold body in one file.
2. `git commit` — pre-commit hook runs; if the scaffold is
   genuinely incomplete (other files still panic), hooks fail.
3. Crafter chooses:
   - **Preferred**: squash-replace enough scaffolds in one commit
     that hooks pass. Tight feedback loop.
   - **Acceptable for WIP**: `git commit --no-verify` to land an
     intermediate commit. MUST be rebased away before PR opens.
4. PR open → CI runs the authoritative gate. CI has NO `--no-verify`
   escape; a PR with scaffold `panic!` bodies fails the `test` or
   `dst` job and blocks merge.

This is the same discipline phase-1-foundation operated under. No
changes to the hook configuration, no new escape hatches, no
"disable hooks for this feature" mode.

---

## Rollback strategy: N/A

Wizard decision: "Recreate. Walking-skeleton only — there's no
'previous version' running to blue-green-swap against at this phase."

This section exists to satisfy the skill requirement that every
DEVOPS artifact addresses rollback. **There is nothing to roll back
because there is nothing deployed.** If a PR introduces a regression:

- CI fails → PR does not merge → `main` stays green. No rollback
  needed at the code level.
- A contributor who `git pull`'d a bad commit locally reverts via
  `git reset` or `git revert`. No infrastructure rollback — their
  `tempfile::TempDir` goes away with the next test run anyway.

The first real rollback-design wave is Phase 2's "first-workload"
feature, when a contributor has a persistent `~/.overdrive/` config
and running allocations. This DEVOPS wave explicitly defers to that.

---

## References

- `.github/workflows/ci.yml` — the authoritative CI definition being
  extended (not replaced)
- `.github/workflows/nightly.yml` — unchanged by this feature
- `lefthook.yml` — unchanged by this feature; `nextest-affected`
  rdeps expansion covers the new crate automatically
- `.cargo/mutants.toml` — unchanged by this feature
- `docs/product/architecture/adr-0009-openapi-schema-derivation.md`
  — source of the `openapi-check` requirement
- `docs/feature/phase-1-foundation/devops/ci-cd-pipeline.md` — the
  full pipeline this file delta's against
- `docs/feature/phase-1-control-plane-core/devops/wave-decisions.md`
  §"New decisions for this feature" — the top-level rationale

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial CI/CD delta for phase-1-control-plane-core. One new required check (`openapi-check`); six existing jobs unchanged; coverage explanation for the three "already-handled" concerns. |
