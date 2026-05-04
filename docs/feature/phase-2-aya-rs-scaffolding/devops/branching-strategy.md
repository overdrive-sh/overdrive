# Branching Strategy — Phase 2.1 aya-rs eBPF scaffolding

**Feature ID:** `phase-2-aya-rs-scaffolding`
**Driving issue:** GH #23
**Wave:** DEVOPS
**Architect:** Apex
**Date:** 2026-05-04

---

## 1. Strategy

**GitHub Flow** — already established for this repo. Single long-lived
`main` branch; feature branches like the current
`marcus-sa/phase2-ebpf-start`; PRs merge to `main` after passing
required-status-checks and reviewer approval.

#23 introduces no branching-model change. This document exists to
record the **branch protection delta** that #23 requires: three new
required-status-checks must be added to the `main` branch protection
rules to gate merges on the new CI jobs.

---

## 2. Branch protection delta (operator action required)

After the PR landing #23 merges, the repo administrator MUST update
the `main` branch protection rules at GitHub Settings > Branches >
Branch protection rules to require these three additional checks:

- `bpf-build`
- `bpf-unit`
- `integration-test-vm-latest`

The existing required checks (`fmt-clippy`, `test`, `dst`, `dst-lint`,
`yaml-free-cli`, `mutants-diff`) remain required. The `integration`
job stays at its current required status (no change to its
configuration is needed).

**Sequencing.** The required-status-check addition can only happen
after the CI workflow file has been updated and at least one PR has
exercised the new jobs successfully — GitHub does not allow marking a
status check as required until it has been observed at least once on
a PR or push event. The recommended sequencing is:

1. Land #23 (which adds the three new jobs to `ci.yml`).
2. Verify the three jobs pass on the merged commit's push-to-main
   event.
3. Add the three checks to branch protection.

This is the same sequencing the existing six checks went through.
There is a one-PR window between step 1 and step 3 during which a
subsequent PR could merge without the new checks being mandatory,
but that PR's CI run will still execute the new jobs (they trigger
on every PR per `ci-cd-pipeline.md` §2) — failures will be visible,
just not blocking. Operators schedule this transition during a low-
traffic window.

---

## 3. dst-lint impact (no new check required)

ADR-0038 §8 explicitly states "dst-lint scope unchanged" — both new
crates declare non-`core` `crate_class` (`overdrive-bpf` is `binary`,
`overdrive-dataplane` is `adapter-host`). The existing `dst-lint`
job's `cargo xtask dst-lint` invocation continues to scan only
`crate_class = "core"` crates and the new crates are skipped
automatically.

The existing `dst-lint` self-test (asserting the `core`-class set is
non-empty — see `xtask/src/dst_lint.rs`) continues to pass:
`overdrive-core` and `overdrive-scheduler` remain the two `core`
crates. No new dst-lint check or asserted invariant is required.

---

## 4. Workspace-convention check (already enforced, no new work)

Per `.claude/rules/testing.md` § "Workspace convention", every
workspace member must declare `integration-tests = []`. The existing
xtask self-test
`xtask::mutants::tests::every_workspace_member_declares_integration_tests_feature`
walks the workspace `members` list and fails the PR if any member is
missing the declaration.

ADR-0038 §1 requires both new crates to declare the feature
(`overdrive-bpf` as a deliberate no-op, `overdrive-dataplane` as the
gate for any future host-side integration tests). The existing self-
test catches a missing declaration at PR time. No new self-test or
gate is required.

---

## 5. References

- `docs/feature/phase-2-aya-rs-scaffolding/devops/ci-cd-pipeline.md`
  §4 — required-status-checks list and rationale.
- `.github/workflows/ci.yml` lines 8–17 — comment block listing the
  current required checks (must be updated by the #23 PR to add the
  three new names).
- `docs/product/architecture/adr-0038-ebpf-crate-layout-and-build-pipeline.md`
  §8 — dst-lint scope unchanged claim.
