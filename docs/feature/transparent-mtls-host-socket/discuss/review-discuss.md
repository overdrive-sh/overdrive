# DISCUSS Wave Review - transparent-mtls-host-socket

**Reviewer**: Codex, applying `nw-product-owner-reviewer` / DISCUSS hard-gate criteria
**Date**: 2026-06-11
**Verdict**: **REJECTED_PENDING_REVISIONS**
**Scope**: `docs/feature/transparent-mtls-host-socket/{feature-delta.md,wave-decisions.md,discuss/journey-enforce-transparent-mtls.yaml,slices/*.md}` plus product SSOT updates in `docs/product/jobs.yaml`, `docs/product/personas/sam-platform-security-engineer.yaml`, `docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml`, and `docs/product/outcomes/registry.yaml`.

The artifacts are directionally strong, but the handoff should not proceed to DESIGN until the blocking consistency and parse issues below are fixed.

---

## Verdict

**Rejected pending revisions before `@nw-solution-architect` DESIGN handoff.**

### Blocking Issues

#### 1. `docs/product/outcomes/registry.yaml` does not parse

**Severity**: blocking
**Dimension**: shared artifact / SSOT validity

The new `OUT-MTLS-SPIKE-INBAND-KTLS` entry opens a single-quoted scalar at `docs/product/outcomes/registry.yaml:212` and does not close it before the `feature:` key at line 218. A YAML parse fails with:

```text
did not find expected key while parsing a block mapping at line 210 column 3
```

Evidence:

- `docs/product/outcomes/registry.yaml:210` starts `OUT-MTLS-SPIKE-INBAND-KTLS`.
- `docs/product/outcomes/registry.yaml:212` starts `summary: 'Tier-3 spike...`.
- `docs/product/outcomes/registry.yaml:218` begins `feature:` while the single-quoted scalar is still open.

**Recommendation**: close the quote on the summary, or convert the summary to a folded block scalar (`summary: >`). Re-run a YAML parser against all touched product YAML files before handoff.

#### 2. Restart-survival is both "not promised" and a hard acceptance criterion

**Severity**: blocking
**Dimension**: cross-artifact consistency / acceptance contract

The product-level journey says the restart-survival guarantee is "not promised as a Phase-2 acceptance criterion beyond 'new connections re-handshake'" at `docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml:204`. The feature-local artifacts make the opposite contract:

- `docs/feature/transparent-mtls-host-socket/feature-delta.md:982` makes in-flight kTLS survival across `kill -9` an AC.
- `docs/feature/transparent-mtls-host-socket/feature-delta.md:1019` makes K5 a binary guardrail for zero in-flight sessions broken by agent restart.
- `docs/feature/transparent-mtls-host-socket/slices/slice-05-restart-survival-and-wasm-variant.md:53` repeats the survival AC.

This will send DESIGN two incompatible instructions: treat restart survival as an empirical DESIGN/spike observable only, or treat it as a committed Phase 2 acceptance promise.

**Recommendation**: choose one contract and make every artifact match it. If restart survival is in scope for Phase 2, update the product-level journey. If it is not, remove it as an AC/KPI/slice promise and keep only "new connections re-handshake" plus an explicit DESIGN investigation note.

### High Issues

#### 3. "Mechanism not pinned" conflicts with mechanism-pinning acceptance criteria

**Severity**: high
**Dimension**: priority validation / solution bias

The artifacts correctly state that DISCUSS pins WHAT, not HOW:

- `docs/feature/transparent-mtls-host-socket/feature-delta.md:407` says the mechanism is not pinned.
- `docs/feature/transparent-mtls-host-socket/feature-delta.md:412` says DISCUSS does not pin the exact sockops attach mechanism or write-block decision.

But US-MTLS-04 pins a specific gate shape:

- `docs/feature/transparent-mtls-host-socket/feature-delta.md:898` requires the sockops/sk_msg gate to be inserted synchronously at callback points.
- `docs/feature/transparent-mtls-host-socket/slices/slice-04-fail-closed-and-race-window.md:52` repeats that requirement.

That converts a DESIGN decision into a DISCUSS acceptance criterion.

**Recommendation**: rewrite the AC around the observable security property: no cleartext before encryption is armed, proved by the race-window probe. Move sk_msg/sockops callback details into Technical Notes as candidate mechanisms for DESIGN to evaluate.

#### 4. Outcome registry omits accepted restart-survival and WASM outcomes

**Severity**: high
**Dimension**: outcome traceability

The wave decision says four outcomes were added to the product registry at `docs/feature/transparent-mtls-host-socket/wave-decisions.md:173`, but US-MTLS-05 and K5 introduce two additional product-relevant outcomes:

- in-flight kTLS survives agent restart: `feature-delta.md:982`, `feature-delta.md:1019`
- WASM host-socket enforcement parity: `feature-delta.md:985`

Neither appears in `docs/product/outcomes/registry.yaml:210-313`.

**Recommendation**: either add outcome entries for restart survival and WASM parity, or explicitly demote those items from product outcomes to DESIGN validation notes. Do not leave them as unregistered acceptance commitments.

### Non-Blocking Notes

- The foundation-feature elevator-pitch exception is explicit at `feature-delta.md:461-470`. I am not blocking on it because the artifacts document the exception and use executable TEST-tier observables (`tcpdump`, `ss -K`, Lima integration tests) instead of inventing an operator verb.
- The spike-first shape is a strong mitigation for the lack of a DIVERGE wave. `wave-decisions.md:112-133` records the risk cleanly and prevents DESIGN from assuming the option space was already narrowed.
- Scope boundaries are strong: host-socket only, with #222 / #229 / #40 / #36 carve-outs consistently named.

---

## Verification Performed

- Parsed YAML with Ruby `YAML.load_file`.
  - PASS: `docs/product/jobs.yaml`
  - PASS: `docs/product/personas/sam-platform-security-engineer.yaml`
  - PASS: `docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml`
  - PASS: `docs/feature/transparent-mtls-host-socket/discuss/journey-enforce-transparent-mtls.yaml`
  - FAIL: `docs/product/outcomes/registry.yaml`
- Verified referenced research files exist under `docs/research/dataplane/`.
- Reviewed cross-artifact consistency across feature narrative, structured journey, slice briefs, and product SSOT updates.

**Handoff status**: not cleared. Fix the blocking items, then rerun DISCUSS review.
