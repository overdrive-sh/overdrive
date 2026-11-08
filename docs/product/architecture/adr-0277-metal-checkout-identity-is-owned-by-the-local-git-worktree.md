# ADR-0277 — Metal checkout identity is owned by the local Git worktree

## Status

**Accepted — 2026-11-08.** The user explicitly stated “Approve P1–P5”. Independent technical review is APPROVED; the exact accepted contract is in the feature delta.

## Context

The branch-sanitised remote path cannot distinguish every local Git worktree or spellings that sanitise alike. Source, artifacts and pulled evidence need one consistent checkout owner.

## Decision

Use the canonical worktree-specific Git metadata directory for a linked worktree, with canonical-checkout-qualified branch fallback for an ordinary clone. Remote directory overrides change the directory, never the identity. Leave legacy branch caches intact and start new identities with an explicit sync.

## Alternatives

Branch-only or basename-only naming retains collisions. Automatic legacy-cache adoption requires a source/identity migration protocol that this bounded change does not need.

## Consequences

Cold caches and detached ordinary-clone refusal are deliberate compatibility changes requiring user approval. Full identity comparison prevents a digest collision from becoming source mixing.

## Links

- [Authoritative proposed interfaces, lifecycle, errors and open gates](../../feature/metal-run-isolation/feature-delta.md#wave-design--ref-status-and-authority)
- [C4 context and containers](c4-diagrams.md#metal-run-isolation-proposed)
- [Bounded native probe findings](../../feature/metal-run-isolation/spike/findings.md)
