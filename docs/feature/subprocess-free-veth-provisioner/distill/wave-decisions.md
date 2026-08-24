# DISTILL Wave Decisions — subprocess-free-veth-provisioner (GH #233)

**Wave:** DISTILL (acceptance test design). **Author:** Quinn
(acceptance-designer). **Date:** 2026-08-25. **Record:** feature-delta.md
§ "Wave: DISTILL".

## Context

Behaviour-preserving mechanism swap (ADR-0085): `ip`/`nft`/`ethtool`/
`sysctl` subprocess → netlink + `/proc/sys`. Pure derivation/diff cores
byte-identical (D10); every port-to-port observable unchanged. Rust
project — NO `.feature` files; ATs are `#[test]`/`#[tokio::test]` gated
`--features integration-tests`, Lima-run (`.claude/rules/testing.md`).

## Reconciliation HARD GATE — PASS (0 contradictions)

- DISCUSS + DEVOPS wave-decisions absent (feature started at SPIKE by
  user choice — feature-delta §2). Treated as WARN, not blocker.
- SPIKE `wave-decisions.md` ↔ DESIGN `wave-decisions.md` reconciled:
  provision→async, errno `NetlinkError`, setns dedicated `std::thread`,
  hand-rolled ethtool `FEATURES_SET`=0x0c, drop-`rustables`/hand-roll-nft,
  ported `ip rule` dump-then-add guard, deps `adapter-host`-only,
  lint-as-final-DELIVER-phase — **all agree.** 0 contradictions.

## Core decision — map-first; author only the genuine gap

Rigorous slice→guard mapping (feature-delta § coverage table) showed the
existing Tier-3 e2e already locks the entire swap surface. The three
behaviour-locks the DESIGN/task named as gaps are **all already LIVE**:

1. **D6 fwmark idempotency** ("exactly one FIB rule after two provisions")
   → `overdrive-worker/tests/integration/mtls_intercept_install.rs:500-504`
   (`ip_rule_fwmark_count == 1` across two `install_inbound_tproxy`).
2. **ethtool tx-off byte-correctness** →
   `veth_provision_idempotent.rs::provision_disables_tx_offload_…` +
   `::provision_repairs_tx_offload_drifted_back_on`.
3. **per-netns sysctl isolation** → `workload_netns_provision.rs`
   (per-host-veth `rp_filter==0` + host resolv.conf byte-identical + the
   in-netns addr/route/up observations that exercise the D4 setns helper).

Authoring duplicates would violate `.claude/rules/testing.md` (no dup e2e)
and the swap's "cores unchanged" premise. Decision: **map them, do not
duplicate; author only the ONE genuinely-new observable.**

## Authored

- `xtask/tests/dst_lint_infra_subprocess_self_test.rs` — 5 RED scaffolds
  (S-LINT-01..05) for the slice-5 ban-infra-subprocess lint (ADR-0085 D8).
  `#[should_panic(expected = "RED scaffold")]` per project convention;
  modelled on `dst_lint_self_test.rs` + `dst_lint_live_literal.rs`. Scanner
  entry-point name NOT invented (crafter defines per the dst-lint mirror).

## Not authored — decisions

- **Structural `NFTA_RULE_HANDLE` recovery** — no new observable
  (`nft -a list` renders `# handle N` for any rule; by-handle-delete
  behaviour already locked). No scaffold (Mandate-1: don't test the parse).
- **`overdrive-netlink` public surface** — internal adapter mechanism
  (impl-only, no port trait); DELIVER unit-test territory. Register-Outcomes
  SKIPPED (no new user-facing typed contract; no Rust outcomes registry).
- **D3 error-model decomposition** — observable covered by idempotency
  scenarios; typed variant shape is a DELIVER unit concern.

## Findings surfaced (non-blocking)

- **F1 (MEDIUM):** ADR-0085's "reverse_nat_e2e catches a wrong ethtool
  bitset" is imprecise — that test uses the `overdrive-testing` FIXTURE
  tx-off, not the production encoder. The `ethtool -k` feature-read (the
  existing tx-off tests) is the correct encoder oracle; acceptable, not a
  blocker. Reviewer to confirm; a real-packet encoder oracle would be a
  separate larger test-infra slice.
- **F2 (LOW):** DDD-5 async `provision` → slice 1/2 EDIT the existing
  behaviour-lock tests (`.await` + `#[tokio::test]`; assertions unchanged).
- **F3 (LOW):** DDD-13 deletes the text-parser UNIT tests with their
  parsers (not the integration behaviour-locks, which observe the kernel).

## Blockers

None. `CLARIFICATION_NEEDED`: no. Ready for the mandatory
`@nw-acceptance-designer-reviewer` (Sentinel, opus) pass.
