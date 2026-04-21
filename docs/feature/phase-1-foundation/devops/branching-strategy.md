# Branching Strategy — phase-1-foundation

**Wave**: DEVOPS (platform-architect)
**Owner**: Apex
**Date**: 2026-04-22
**Model**: GitHub Flow
**Status**: Accepted

---

## Decision

Phase 1 uses **GitHub Flow**: `main` is the single long-lived branch;
work lands through short-lived feature branches via PR.

No `develop`, no `release/*`, no `hotfix/*`. No semantic version
release cadence yet — Phase 1 ships no shipped artifact.

---

## Branching

| Branch | Purpose | Lifetime | Protection |
|---|---|---|---|
| `main` | Authoritative integration branch. Every CI gate must pass to merge. | Permanent | Branch-protected (see `ci-cd-pipeline.md` §Branch-protection config) |
| `{initials}/{feature-id}` | Feature or task branch off `main`. | Short-lived (< 1 week ideal) | None |

**Naming**: `{initials}/{feature-id}` — e.g. `mk/phase-1-foundation`,
`ap/dst-lint-bug`. The current branch `marcus-sa/phase-1-foundation` is
a pre-convention variant; new branches follow `{initials}/{id}`.

**Rebase, don't merge**: PRs land via squash-merge or rebase-merge to
preserve the linear history required by branch protection. No merge
commits on `main`.

---

## PR flow

```
feature branch ── push ──► GitHub
                             │
                             ├── CI workflow triggers (ci.yml)
                             │   └── fmt-clippy, test, dst, dst-lint, mutants-diff
                             │       must all pass
                             │
                             ├── at least 1 approving review required
                             │
                             └── squash-merge / rebase-merge to main
                                    │
                                    └── main CI run (push event) re-runs gates
```

### Required status checks before merge

Per `ci-cd-pipeline.md` §Branch-protection config:

- `fmt + clippy`
- `cargo test (unit + proptest)`
- `cargo xtask dst`
- `cargo xtask dst-lint`
- `cargo mutants (diff)`

All five must show green; blocked otherwise.

---

## Trunk-based aspiration

Once CI is consistently green across several weeks — meaning flakes are
zero and the team trusts the gate — consider relaxing PR review for
trivial changes (docs, dependency bumps caught by `cargo deny`,
lefthook-only edits). This is a judgment call, not a policy; Phase 1
ships with review required.

Moving to full trunk-based development — direct commits to `main`
without PR — is a long-horizon goal that depends on CI being fast enough
that the round trip feels free (testing.md notes <15 min critical path,
which is borderline). Not a Phase 1 commitment.

---

## Release tagging

Phase 1 is pre-release. No release tags, no `CHANGELOG.md` discipline,
no crates.io publication.

A release-tagging strategy lands when Phase 2 produces the first
shippable artifact (most likely the `overdrive` CLI binary, then the
`overdrive-node` binary). At that point:

- Semantic versioning (`v0.1.0`, `v0.2.0`, ...).
- Tags signed (gpg or SSH sigstore).
- A release workflow generates binaries for macOS (Apple Silicon) and
  Linux x86_64, uploads to GitHub Releases.
- `cargo publish` for library crates once `overdrive-core` API
  stabilises.

None of this is wired in Phase 1. It will be designed in Phase 2's
DEVOPS wave.

---

## Hotfix protocol

Not formalised. Phase 1 has nothing in production to hotfix. If a
regression blocks `main` CI, revert the offending commit via a PR
(`git revert` → push → normal CI gate). A direct push to `main` to
"unblock CI" is never acceptable — the gate exists to catch exactly
this.

---

## Cross-references

- `ci-cd-pipeline.md` — the CI workflow that enforces the gate
- `wave-decisions.md` — DEVOPS wave decisions summary
- `.claude/rules/testing.md` §CI topology — per-tier gate layout
- `docs/product/architecture/adr-0006-ci-wiring-dst-gates.md` — the
  `xtask dst` + `dst-lint` CI contract
