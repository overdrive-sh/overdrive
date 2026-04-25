# fix-cli-cannot-reach-control-plane — Feature Evolution

**Feature ID**: fix-cli-cannot-reach-control-plane
**Type**: Bug fix (via `/nw-bugfix` → `/nw-deliver` pipeline)
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Date**: 2026-04-25
**Commits**:
- `e0a7b3b6fbf05c2e9c24e3cbc655f981327d1351` — `test(cli): pin serve+submit round-trip with production defaults (RED — 01-02 will flip GREEN)`
- `0d61cc1fd88b101a481ead2a058db08a2d0f5075` — `fix(cli,control-plane): decouple operator-config dir from data dir; write trust triple to ~/.overdrive/config`
- `cc2c61cb423b4d1782687290891a372b6d56127e` — `fix(cli): distinguish TLS handshake failure from TCP connect-refused in reqwest error classifier`

**Status**: Delivered (RED → GREEN → diagnostic-quality fix; 3-step roadmap, 3 commits)

---

## What shipped

`overdrive serve` now writes its freshly-minted trust triple to the
canonical operator config path (`~/.overdrive/config`) instead of
under the data dir, so `overdrive job submit` running from the same
machine reads the trust material the running server actually
authenticates with. The end-to-end Phase 1 walking-skeleton flow —
`serve` in one terminal, `job submit` in another, no explicit path
flags — works on a clean machine.

Two coupled fixes plus one diagnostic-quality fix:

- **Primary (commit `0d61cc1`):** `ServerConfig` now carries a
  separate `operator_config_dir: PathBuf` field alongside `data_dir`.
  The CLI's `Command::Serve` handler computes both via
  `default_data_dir()` and a new `default_operator_config_dir()`
  helper, threads them through `ServeArgs`, and `run_server_with_obs`
  writes `write_trust_triple` against the operator config dir, not
  the data dir. The data dir keeps its ADR-0013 §5 role as the redb +
  libSQL storage root.
- **Drift-surface closure (same commit):** `default_config_path()` in
  `main.rs` now delegates to `cluster::default_operator_config_path()`,
  which itself delegates to a new `default_operator_config_dir()` —
  one canonical resolution chain, used everywhere.
- **Diagnostic quality (commit `cc2c61c`):** `stringify_reqwest_error`
  now distinguishes TLS handshake failures (rendered as "TLS
  handshake failed (certificate not trusted) — re-run `overdrive
  cluster init` if the CA was re-minted") from TCP connect-refused
  (rendered as "could not connect to server"). reqwest's
  `is_connect()` returns true for both; the chained `source()` is
  inspected for a rustls handshake error to split the cases.

## Business context

Maps directly to the bug-report severity: P0 for Phase 1 acceptance.
The walking-skeleton path advertised by every `*_submit*` integration
test passed in CI but no operator following the README could
reproduce step 2 — the test suite manufactured a coincidence
(`TempDir` serving as both `data_dir` and operator-config root) that
the production binary never exhibits. The fix also makes the
ADR-0010 §R4 ephemeral-CA model behave the way operators intuitively
expect: each `serve` mints a fresh CA and *overwrites* the
`~/.overdrive/config` `cluster init` left behind, removing the
"whose CA is the CLI loading?" ambiguity.

## Root cause (briefly; full chain in `deliver/rca.md`)

`ServerConfig::data_dir` was overloaded — it carried both
"storage root for redb + libSQL" (ADR-0013 §5) and "operator-config
root for the trust triple" (whitepaper §8 / ADR-0019). On a default
invocation those resolve to *different* directories
(`$HOME/.local/share/overdrive` vs `$HOME/.overdrive`), so `serve`
deposited the trust triple at
`$HOME/.local/share/overdrive/.overdrive/config` while `job submit`
read from `$HOME/.overdrive/config`. The CA pinned by the CLI did
not sign the leaf the running server presented; rustls rejected the
handshake; reqwest's `is_connect()` was true; the CLI rendered
"could not connect to server" — a transport classifier collision
that disguised a trust-material mismatch as a network problem.

Every existing integration test passed one `TempDir` as both roles,
so the divergence was invisible in CI and only the production-default
invocation exposed it.

## Key decisions

- **Decouple at the field level, not via newtypes.** `ServerConfig`
  gains an `operator_config_dir: PathBuf` sibling to `data_dir`
  rather than `DataDir(PathBuf)` / `OperatorConfigDir(PathBuf)`
  newtypes. The newtype refactor is a Phase 1 follow-up captured in
  the RCA's *Prevention strategy* §1 — keeping the immediate fix
  minimal was the priority. Single-cut greenfield migration applied:
  every caller of `ServerConfig` updated in the same PR per
  `feedback_single_cut_greenfield_migrations`.
- **Bundle the diagnostic-quality fix.** The error-classifier split
  was originally scoped as a separate PR per the RCA. After review
  it landed in the same roadmap because the fix is small (~30 lines
  in `http_client.rs`) and the diagnostic improvement is the only
  way an operator hitting the *next* CA-mismatch class (e.g. server
  restart inside the same session) gets directed at the trust
  material instead of network debugging. Three commits, one
  contiguous bug-and-prevention story.
- **Drift-surface closure is in scope.** The RCA's WHY 4A flagged
  two helper functions in `main.rs` that resolved "where the
  operator config lives" independently. The fix collapses them
  through `default_operator_config_path → default_operator_config_dir`
  delegation so the type system can't encode the invariant but the
  call graph does. Same shape as the prior
  `fix-overdrive-config-path-doubled` resolution.
- **`serve` overwriting `~/.overdrive/config` is intentional, not
  a regression.** Consistent with ADR-0010 §R4 (re-mint
  unconditionally on every `serve`). User explicitly approved at
  `/nw-bugfix` Phase 2. Persisted-CA story belongs in Phase 2.
- **No new ADR.** ADR-0010 (ephemeral CA) and ADR-0013 §5 (data dir
  is XDG `data_dir()/overdrive`) already specify the boundary; the
  bug was that the code didn't enforce it. ADR clarification noting
  "operator config dir and node data dir are separate concerns"
  remains open as RCA *Prevention strategy* §4 — separate dispatch
  to the architect agent per `feedback_delegate_to_architect`.

## Regression invariants locked in

`crates/overdrive-cli/tests/integration/cluster_init_serve.rs`,
`#[serial_test::serial(env)]`:

1. **`serve_and_submit_with_production_defaults_succeeds`** —
   the test the RCA promised. Scopes `$HOME` /
   `$OVERDRIVE_CONFIG_DIR` / `$XDG_DATA_HOME` to a tempdir, spins up
   `serve` via the same path `main.rs` uses, calls `job submit` with
   no explicit `--config-dir`, and asserts the round-trip succeeds.
   This test fails on `e0a7b3b` (RED), passes on `0d61cc1` (GREEN).
   It is the test that would have caught this bug class, and is the
   gate against any future re-overload of the two roles.
2. **TLS-handshake classifier test (in
   `crates/overdrive-cli/tests/integration/http_client.rs`)** —
   pins that a self-signed-leaf served by an unrelated CA renders
   the "TLS handshake failed" diagnostic rather than the generic
   "could not connect to server". Independent of the primary bug;
   prevents the diagnostic-quality fix from regressing.
3. **13 pre-existing integration tests decoupled** — every test
   that previously passed one `TempDir` as both `data_dir` and
   read its trust triple from `tmp.path().join(".overdrive/config")`
   now passes `data: tmp.path().join("data")` and
   `config: tmp.path().join("conf")` as separate subdirectories.
   The decoupling is the structural anti-bug — a future refactor
   that re-overloads the two roles fails these tests immediately.

## Steps completed

3 phases, 3 steps, 3 commits. RED → GREEN → diagnostic.

| Step ID | Phase | Status | Notes |
|---|---|---|---|
| 01-01 | RED_ACCEPTANCE | PASS | Production-defaults round-trip test fails against pre-fix code with `CliError::Transport` |
| 01-02 | GREEN | PASS | `ServerConfig::operator_config_dir` field added; `write_trust_triple` rerouted; 13 integration tests decoupled; primary regression test passes |
| 02-01 | DIAGNOSTIC | PASS | `stringify_reqwest_error` splits TLS handshake from connect-refused; classifier test added |

Quality gates:
- 148/148 nextest tests across 7 binaries with `integration-tests` feature
- Doctests clean
- `cargo clippy --all-targets --features integration-tests -- -D warnings` clean
- L1–L4 refactor pass: no actionable opportunities (clean diff by construction)
- Adversarial review (`@nw-software-crafter-reviewer`): APPROVED, zero blockers, three low-severity warnings (not actionable in this PR)

### Mutation testing — caveat

`cargo xtask mutants --diff origin/main` was attempted on the
control-plane diff scope. The wrapper aborted before completion at
64/68 mutants generated; viable-subset kill rate over the executed
mutations was **26 / (26 + 2) = 92.9%**, above the 80% gate. The
2 missed mutations were on **pre-existing code**, not on this
bugfix's diff:

- `crates/overdrive-control-plane/src/reconciler_runtime.rs:114`
- `crates/overdrive-control-plane/src/tls_bootstrap.rs:347`

Neither miss is a regression introduced by this fix; both pre-date
the bugfix branch and would have shown the same gap on the prior
SHA. The cli-package mutation run was not attempted — user
explicitly accepted "finalize the feature" rather than rerun. The
two pre-existing misses warrant a separate follow-up; they are not
gating.

The wrapper-abort itself surfaced an `xtask mutants` bug — see
*Follow-ups* below.

## Files touched

20 files, +635 / −142.

| Crate | Files | Notes |
|---|---|---|
| `overdrive-cli` (src) | 4 | `commands/cluster.rs` (new `default_operator_config_dir`), `commands/serve.rs` (`ServeArgs::config_dir`), `http_client.rs` (handshake classifier split), `main.rs` (thread `config_dir`, delegate `default_config_path`) |
| `overdrive-cli` (tests) | 6 | `cluster_init_serve.rs` (+ regression test), `http_client.rs` (+ classifier test), `cluster_and_node_commands.rs`, `endpoint_from_config.rs`, `job_submit.rs`, `walking_skeleton.rs` (decoupling) |
| `overdrive-control-plane` (src) | 1 | `lib.rs` — `ServerConfig::operator_config_dir` field, `run_server_with_obs` rewires `write_trust_triple` |
| `overdrive-control-plane` (tests) | 6 | `concurrent_submit_toctou.rs`, `describe_round_trip.rs`, `idempotent_resubmit.rs`, `observation_empty_rows.rs`, `server_lifecycle.rs`, `submit_round_trip.rs` (decoupling) |
| `overdrive-cli` (Cargo.toml) | 1 | (unchanged in DELIVER; `serial_test` already a workspace dep from prior fix) |
| `Cargo.lock` | 1 | regenerated |
| `.claude/rules/testing.md` | 1 | `serial_test` discipline section (committed earlier in `9ed6a80`, stayed adjacent) |

## Lessons learned

- **The overload was the bug.** Naming a `PathBuf` field `data_dir`
  and using it as both the storage root *and* the operator-config
  root is the same shape as the prior path-doubling regression's
  "`PathBuf` carrying implicit `is-this-already-suffixed?`
  semantics" — overloaded paths don't survive divergent default
  resolution. The structural fix is one role per field. The newtype
  fix (RCA *Prevention strategy* §1) is the type-level closure of
  the same lesson; deferred but tracked.
- **Test convention: production-default tests must scope env, not
  pass `TempDir` as both roles.** The 13-test decoupling done in
  this PR is mechanical, but the *next* CLI test added must default
  to scoping `$HOME` / `$OVERDRIVE_CONFIG_DIR` / `$XDG_DATA_HOME`
  via `EnvGuard` and call handlers with no explicit paths. Convention
  belongs in `crates/overdrive-cli/CLAUDE.md` under the existing
  "Integration tests — no subprocess" section (RCA *Prevention
  strategy* §3, separate dispatch).
- **The error classifier was a force-multiplier on the bug.**
  `is_connect() == true` on a TLS handshake failure is a reqwest
  modelling choice that's technically correct internally but
  actively misdirects operators. Three diagnostic categories
  (TCP refused, TLS handshake failed, certificate not trusted)
  collapsing to one message is a category error; splitting them
  in commit `cc2c61c` removes the misdirection.
- **The drift-surface lesson from the prior config-path-doubled bug
  reapplies.** Two sibling helper functions computing the canonical
  path independently is exactly the shape that re-broke twice in
  two weeks. The collapse into `default_operator_config_path` →
  `default_operator_config_dir` delegation is the same fix shape,
  applied to a different pair of helpers.

## Follow-ups (separate tickets warranted)

1. **`cargo xtask mutants` wrapper bug — multi-package feature
   propagation.** When `--package <CRATE>` selects a single crate
   that declares the `integration-tests` feature, the wrapper
   correctly auto-adds `--features <CRATE>/integration-tests`. But
   when the wrapper enables the feature for one package and the
   build pulls in dependent crates that *don't* declare that
   feature, cargo errors out, and the wrapper aborts mid-mutation
   with a stale `mutants-summary.json`. Surfaced during the
   control-plane scope run on this bugfix. Repro:
   `cargo xtask mutants --diff origin/main --package overdrive-control-plane`.
   File a tracking issue against `xtask/`. Workaround for now:
   per-crate runs only on packages whose entire dep graph declares
   the feature, OR `--no-integration-tests` opt-out.
2. **Two pre-existing mutation misses** in
   `reconciler_runtime.rs:114` and `tls_bootstrap.rs:347`. Pre-date
   this bugfix; not regressions; should be closed in a separate
   testing PR with mutation coverage added.
3. **Newtype refactor** — `DataDir(PathBuf)` and
   `OperatorConfigDir(PathBuf)` per RCA *Prevention strategy* §1.
   Phase 1 follow-up; type-level closure of the lesson above.
4. **ADR clarification** — note explicitly in ADR-0010 or ADR-0013
   §5 that operator config dir and node data dir are separate
   concerns. Dispatch to the architect agent per
   `feedback_delegate_to_architect`.

## Notes on discarded workspace artifacts

Nothing lasting lived outside this evolution doc and the RCA:

- **`deliver/rca.md`** — authoritative RCA from `/nw-bugfix`
  Phase 1, user-approved at Phase 2. Its key findings (problem,
  WHY chain, two coupled fixes, regression-test design,
  prevention strategy) are folded into this evolution doc. The
  RCA file remains at
  `docs/feature/fix-cli-cannot-reach-control-plane/deliver/rca.md`
  for historical reference; it was the source specification for
  the commits, not a lasting design document. Same precedent as
  the prior `fix-overdrive-config-path-doubled` archive.
- **`discuss/bug-report.md`** — defect summary feeding the RCA;
  preserved in workspace as the operator-facing entry point that
  motivated the bugfix.
- `deliver/execution-log.json`, `deliver/roadmap.json`,
  `deliver/.develop-progress.json` — process scaffolding. Audit
  trail captured in the *Steps completed* table above; step plan
  superseded by the three landed commits.

No `design/` or `distill/` artefacts produced. No new ADRs (the
bug was a conformance violation against existing ADR-0010 / 0013;
the *clarifying* ADR edit is a follow-up). Nothing migrated to
permanent directories.

## Related

- **ADR-0010 §R1 / §R4** (ephemeral CA, re-mint unconditionally on every `serve`)
- **ADR-0013 §5** (data directory = XDG `data_dir()/overdrive`)
- **ADR-0019** (operator CLI config path specification)
- **Whitepaper §8** (Identity and mTLS — Operator Identity and CLI Authentication)
- **Memory**: `project_cli_auth.md` (Talos-shape mTLS, operator SPIFFE IDs, 8h TTL, Corrosion-gossiped revocation)
- **Prior evolution**: `2026-04-24-fix-overdrive-config-path-doubled.md` — the prior path-doubling bug on the operator config dir; this fix is the sibling on the data-dir-vs-config-dir split, same drift-surface lesson at a different layer
