# CI/CD Pipeline — phase-1-foundation

**Wave**: DEVOPS (platform-architect)
**Owner**: Apex
**Date**: 2026-04-22
**Status**: Draft — pending peer review

---

## Scope discipline

Phase 1 is a Rust library + CLI + xtask workspace. There is no runtime
service, no deployment target, no container orchestrator, and no
observability stack yet. The one real CI/CD deliverable is the GitHub
Actions workflow that gates PRs against `main` with the checks ADR-0006
specifies.

This document is the decision record for that workflow. Cloud-native
sections commonly expected by generic CI/CD skill templates (monitoring
stacks, deployment strategies, environment promotion) are deliberately
omitted — they do not exist to design against in Phase 1.

---

## Pipeline shape

The pipeline sits in a single repository workflow at
`.github/workflows/ci.yml`. A second workflow at
`.github/workflows/nightly.yml` handles full-workspace mutation testing
as a trend-tracking soak (not PR-blocking).

### Required status checks (block merge to `main`)

| # | Job | Command | Gate | ADR / rule reference |
|---|---|---|---|---|
| 1 | `fmt-clippy` | `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets -- -D warnings` | Blocking | brief §9, testing.md CI topology Job A/B |
| 2 | `test` | `cargo test --workspace --locked` with `PROPTEST_CASES=1024` | Blocking | testing.md proptest rules, K5/K6 |
| 3 | `dst` | `cargo xtask dst` | Blocking | ADR-0006, testing.md Tier 1, K1/K3 |
| 4 | `dst-lint` | `cargo xtask dst-lint` | Blocking | ADR-0003, ADR-0006, K2 |
| 5 | `mutants-diff` | `cargo mutants --in-diff origin/main` (gate ≥ 80% kill rate) | Blocking (PR only) | testing.md mutation testing mandate |

### Why this exact set

These are the five gates the DESIGN wave commits to. Each has a KPI or
architectural rule behind it; none is ceremonial.

- **fmt-clippy** is the fastest feedback loop. Lefthook pre-commit runs
  the same checks locally (see `lefthook.yml`) — CI is the authoritative
  gate; the local hook is the courtesy.
- **test** covers every `#[test]` in the workspace: unit, integration
  (including the compile-fail `trybuild` cases from ADR-0005), and
  proptest roundtrips mandated by testing.md (newtype roundtrip, rkyv
  roundtrip, snapshot roundtrip, hash determinism).
- **dst** is the Tier 1 gate that gives K1 and K3 teeth. ADR-0006
  mandates the xtask wrapper path (never raw `cargo test --features
  dst`) so the seed is surfaced on the first output line and the
  `dst-output.log` + `dst-summary.json` artifacts are predictable.
- **dst-lint** is the K2 gate. Every PR attempting to introduce a banned
  API in a core crate is blocked. A false positive here is a P0 bug —
  false positives train engineers to bypass the gate.
- **mutants-diff** is the unit-level rigor gate per testing.md. Diff-
  scoped so the per-PR budget stays tight; full-corpus runs live in
  `nightly.yml`.

### Explicit skips

Tier 2, Tier 3, and Tier 4 from testing.md are deferred:

- **Tier 2 (BPF unit via `BPF_PROG_TEST_RUN`)** — deferred until the
  first eBPF program lands (Phase 2+ when `aya-rs` is wired for real).
  Phase 1 ships zero `#[aya_bpf]` code.
- **Tier 3 (real-kernel integration via QEMU + kernel matrix)** —
  deferred until there is kernel-facing code to attach and drive. Phase
  1 has none.
- **Tier 4 (veristat complexity regression, xdp-bench perf regression,
  PREVAIL second-opinion)** — all gate BPF output; deferred with Tiers 2
  and 3.

These deferrals are recorded in `wave-decisions.md` alongside the reason
(no eBPF code in Phase 1 means no kernel surface to gate).

---

## Cache strategy

Three caches compose, keyed per job via `Swatinem/rust-cache@v2`'s
`shared-key`:

1. **Cargo target cache** (`Swatinem/rust-cache`) — caches
   `~/.cargo/registry`, `~/.cargo/git`, and `target/` keyed on
   `Cargo.lock`. First run is cold (~8-12 minutes for a workspace this
   size); warm runs on an unchanged `Cargo.lock` are seconds.
2. **sccache** (`mozilla-actions/sccache-action`) — compiler-level
   cache keyed on source hashes + compiler invocation. Wins on PRs that
   touch a subset of crates and leave the dep graph unchanged.
3. **Rust toolchain** — cached implicitly by `dtolnay/rust-toolchain@stable`
   honouring `rust-toolchain.toml`.

Expected runtimes on `ubuntu-latest`:

| Job | Cold | Warm |
|---|---|---|
| fmt-clippy | ~5 min | ~30 s |
| test | ~10 min | ~1 min |
| dst | ~10 min (build-heavy; turmoil + overdrive-sim) | ~1 min (plus <60 s DST execution per K1) |
| dst-lint | ~3 min (builds xtask with `syn`) | ~10 s |
| mutants-diff | highly variable — per-mutation test reruns dominate | 5-20 min on typical PR diffs |

The critical-path target — fmt-clippy + test + dst + dst-lint in
parallel, mutants-diff running concurrently — is ≤ 15 minutes warm per
testing.md §CI topology Phase 1.

Mutants has no persistent cache; it always rebuilds the workspace per
mutation. Diff-scoping is the only lever that keeps the per-PR budget
finite. The nightly full-corpus run has a 6-hour ceiling.

---

## Seed surfacing

ADR-0006 is the primary input; this section names the mechanism end to
end.

### On a green run

The xtask wrapper prints `dst: seed = <N>` on the first line of stdout.
The CI log preserves this; no additional annotation is needed.

### On a red run

Three surfaces carry the seed, in descending order of click cost:

1. **GitHub annotation on the job** — the `dst` job's failure step
   emits a `::error title=DST failed::seed=<N> invariant=<NAME>
   tick=<T> host=<H> — reproduce: <CMD>` annotation. This lands in the
   PR's "Files changed" tab and the Checks tab without opening any
   artifact.
2. **GitHub job summary** — the same failure step writes a Markdown
   block to `$GITHUB_STEP_SUMMARY` with a table of seed/invariant/tick/
   host and a fenced code block containing the exact reproduction
   command. Copy-pasteable without leaving the browser tab.
3. **Uploaded artifacts** — `target/xtask/dst-output.log` and
   `target/xtask/dst-summary.json` are uploaded via
   `actions/upload-artifact@v4` regardless of job outcome (`if:
   always()`). Retention: 30 days. For post-mortem analysis of past
   failures.

The reproduction command format per ADR-0006 is:

```
cargo xtask dst --seed <N> --only <invariant-name>
```

This string is paste-to-terminal runnable — no environment variables,
no extra flags, no "after you set up the sim feature flag" caveats. The
twin-run identity self-test (US-06 AC) runs on every PR and catches a
regression where the printed command no longer reproduces.

### What happens if the run is OOM-killed before output is flushed

ADR-0006's first-line seed print exists for this case. The xtask wrapper
prints the seed before invoking the harness — even a killed run has the
seed in the log up to the point of kill. If the log made it to disk
before the kill, the artifact upload (`if: always()`) still fires and
captures whatever was written.

If nothing flushed, the artifact is empty and the engineer falls back to
rerunning the PR with the wrapper outputting to a less buffered sink (a
future `--log-every-tick` flag may land if this becomes a real pattern;
it is not Phase 1 scope).

---

## Mutation-testing gate

### What "in diff" means

`cargo mutants --in-diff origin/main` scopes mutation generation to
code that overlaps the PR's diff against `main`. The effect:

- PRs that only touch docs, Cargo.toml, or tests: 0 mutations; the job
  passes instantly.
- PRs touching reconciler / newtype / snapshot code: mutations generated
  only for lines in the diff's hunk windows.

This keeps the per-PR budget bounded without losing coverage — testing.md
nightly full-corpus runs (in `nightly.yml`) catch mutations outside any
active PR's diff.

### Baseline storage

`mutants-baseline/main/kill_rate.txt` — a single-line file containing
the baseline kill rate (as a percentage, e.g. `84.3`). Updated manually
by the platform team when a sustained improvement warrants raising the
bar; the nightly workflow reads this file to compute drift.

The baseline directory is committed to the repo with a
`mutants-baseline/main/.gitkeep` placeholder so the first nightly run
has a location to write `kill_rate.txt` into — without the placeholder,
the directory would not exist in a fresh clone and the trend-tracking
signal would silently disappear under the workflow's
`if-no-files-found: warn` setting. Do not delete the `.gitkeep`.

### Exclusions

`.cargo/mutants.toml` (committed at repo root) carries the exclusion
list per testing.md:

- `unsafe` blocks — kernel-verifier-adjacent; mutations produce code
  the verifier may reject, masquerading as "caught."
- `aya-rs` eBPF programs under `crates/overdrive-bpf/` — future Phase
  2+ scope; Tier 2/3 covers them. The glob is pre-listed so the
  exclusion is live the day the crate lands.
- Generated code (`#[derive]`, `build.rs` output, proc-macro expansion,
  anything under `target/`).
- Async scheduling logic (`select!` arms, future polling) — DST is the
  right tool per testing.md.
- Tests and benches — excluded explicitly as belt-and-braces; cargo-
  mutants also defaults to skipping `#[cfg(test)]`.

The file is the single source of truth for what mutation testing
covers in this repo. `.claude/rules/testing.md` §Mutation testing
(cargo-mutants) remains authoritative for the *rationale*; the TOML
mechanises it. When in doubt, update testing.md first and mirror here.

### Triggers

- **Per-PR, diff-scoped** — `ci.yml` job `mutants-diff`, only on
  `pull_request` events. Gate: ≥ 80% kill rate.
- **Nightly, full-workspace** — `nightly.yml` job `mutants-workspace`,
  scheduled `cron: '13 3 * * *'`. Gate: drift > 2 pp below baseline
  soft-fails (warning annotation); absolute < 60% hard-fails.

### What happens if the PR genuinely has no testable surface

A PR that only edits docs or Cargo.toml (non-code files) produces zero
mutations, prints "no mutations in diff — nothing to gate on", and
passes. This is intentional — a per-feature strategy only measures what
changed.

---

## Branch-protection config

GitHub branch-protection rules live in Settings UI, not in the workflow
file. For `main`, configure:

### Required status checks

Enable "Require status checks to pass before merging" and select:

- `fmt + clippy`
- `cargo test (unit + proptest)`
- `cargo xtask dst`
- `cargo xtask dst-lint`
- `cargo mutants (diff)`

(The check names above match the `name:` attribute on each job in
`ci.yml`.)

### Additional protections

- Require branches to be up to date before merging.
- Require linear history (no merge commits) — aligns with GitHub Flow
  per `branching-strategy.md`.
- Require signed commits (future, once signing is universal on the team).
- Do not allow force pushes.
- Do not allow deletions.
- Include administrators in these rules — the platform team is not above
  the gate it wrote.

### Review requirements

- Require at least 1 PR approval.
- Dismiss stale approvals when new commits are pushed.
- Require review from CODEOWNERS (once a CODEOWNERS file lands; deferred
  to Phase 2 when sub-team ownership becomes meaningful).

The user configures these via `gh api` or the Settings UI after the
workflow file lands. This document does not gate on that configuration
being in place before merging; the workflow file is valuable on its own
(it runs on every PR even without branch protection), and branch
protection is a follow-up action.

---

## Branch-protection setup (`gh api` recipe)

Copy-paste recipe to apply the branch-protection rules above. The
required-check names below match the `name:` attribute of each job in
`.github/workflows/ci.yml` verbatim — GitHub matches on the check's
display name, and a single character mismatch silently turns the gate
off. Do not edit these names without updating `ci.yml` in the same PR.

```sh
# Repo slug: confirm with `gh repo view --json nameWithOwner`
OWNER=overdrive-sh
REPO=overdrive

# Five required status checks — match `name:` in ci.yml jobs:
#   1. fmt + clippy                   (job `fmt-clippy`)
#   2. cargo test (unit + proptest)   (job `test`)
#   3. cargo xtask dst                (job `dst`)
#   4. cargo xtask dst-lint           (job `dst-lint`)
#   5. cargo mutants (diff)           (job `mutants-diff`)

gh api "repos/${OWNER}/${REPO}/branches/main/protection" \
  --method PUT \
  --input - <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "contexts": [
      "fmt + clippy",
      "cargo test (unit + proptest)",
      "cargo xtask dst",
      "cargo xtask dst-lint",
      "cargo mutants (diff)"
    ]
  },
  "enforce_admins": true,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": false,
    "required_approving_review_count": 1
  },
  "restrictions": null,
  "required_linear_history": true,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "required_conversation_resolution": true,
  "lock_branch": false,
  "allow_fork_syncing": true
}
JSON
```

### Verify

After applying the recipe, confirm the exact five names landed:

```sh
gh api "repos/${OWNER}/${REPO}/branches/main/protection" \
  | jq '.required_status_checks.contexts'
```

Expected output:

```json
[
  "fmt + clippy",
  "cargo test (unit + proptest)",
  "cargo xtask dst",
  "cargo xtask dst-lint",
  "cargo mutants (diff)"
]
```

A diff here — typo, wrong casing, missing entry — is the single biggest
UX trap in CI setup: the workflow runs green but the gate never
activates. The verify step takes ten seconds; run it.

### Notes

- `strict: true` requires branches to be up-to-date before merging.
- `enforce_admins: true` is deliberate — the platform team is not above
  its own gate, per the §Additional protections rule.
- `required_linear_history: true` matches GitHub Flow per
  `branching-strategy.md` (squash-merge or rebase-merge only).
- Signed-commit enforcement and CODEOWNERS reviews are deferred until
  the policies land (Phase 2). Re-run `gh api` with the additional
  fields at that point; the rest of the block does not need to change.
- The recipe is idempotent — re-running overwrites the protection
  settings with the exact body above. Drift back to a known-good state
  is one command.

---

## KPI instrumentation (minimal)

Per the user's DEVOPS-wave direction, dashboards and observability are
deferred. KPIs are enforced in CI directly:

| KPI | Enforced by | Failure mode |
|---|---|---|
| K1 (DST <60s on clean clone) | `dst` job timeout (10 min ceiling); xtask summary captures wall-clock | Timeout cancels run; PR fails |
| K2 (100% banned-API block; 0 false positives) | `dst-lint` job; self-test inside xtask | Lint violation fails PR; false-positive investigation is out-of-band (manual) |
| K3 (seed reproduces bit-for-bit) | `dst` job runs the twin-run identity self-test (US-06 AC) | Twin-run divergence fails DST |
| K4 (LocalStore density) | Deferred — see `upstream-changes.md` | Phase 2+ commercial guardrail |
| K5 (newtype completeness) | `test` job runs newtype static-API scan | Test fails PR |
| K6 (snapshot round-trip) | `test` job runs snapshot roundtrip proptest | Proptest failure blocks PR |

`kpi-instrumentation.md` is explicitly skipped for Phase 1 — these six
KPIs are enforced by `cargo xtask dst`, `cargo xtask dst-lint`, and
`cargo test`, not by dashboards. Phase 2+ will introduce instrumentation
surfaces when real control-plane metrics exist to measure.

---

## Rejected simple alternatives

### Alternative A — "Let lefthook be the only gate"

- **What**: Skip CI entirely; trust contributor lefthook hooks.
- **Expected impact**: Meets 0% of requirements. Lefthook can be
  bypassed (`--no-verify`), runs only on the contributor's machine, and
  provides no merge-protection mechanism.
- **Why insufficient**: Local hooks are courtesy; CI is the
  authoritative gate. K2 ("100% blocked") is structurally impossible
  without a server-side check.

### Alternative B — "One big monolithic CI job"

- **What**: One job runs every step in sequence (fmt → clippy → test →
  dst → dst-lint → mutants).
- **Expected impact**: Correct but slow. Every failure restarts the
  full chain; caching buys less because the single job's cache key is
  coarse.
- **Why insufficient**: Parallel independent jobs finish in
  `max(job_durations)` rather than `sum(job_durations)`. For a 15-minute
  critical path, the difference is ~30 minutes saved per PR.

### Alternative C — "GitHub Actions matrix across Rust toolchains"

- **What**: Run the pipeline on `stable`, `beta`, and `nightly`
  toolchains in a matrix.
- **Expected impact**: Useful for detecting toolchain regressions, but
  triples CI cost.
- **Why insufficient**: `rust-toolchain.toml` pins the project to
  `stable`. Testing beta/nightly is a Phase 2+ concern when upstream
  feature dependencies matter. Phase 1 adds no value from matrix
  toolchain runs.

---

## Peer review / revision log

| Date | Revision |
|---|---|
| 2026-04-22 | Initial DEVOPS wave ci-cd-pipeline decision record. |
| 2026-04-22 | Review iter-1 fixes (CONDITIONALLY_APPROVED → addressed): (1) added `mutants-baseline/main/.gitkeep` pointer to §Mutation-testing gate §Baseline storage; (2) updated §Exclusions to reflect the committed `.cargo/mutants.toml`; (3) added §Branch-protection setup (`gh api` recipe) with verify step; (4) removed redundant job-level `PROPTEST_CASES` in `ci.yml` (workflow-level declaration remains). |
