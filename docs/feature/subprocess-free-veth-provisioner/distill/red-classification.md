# DISTILL RED classification — subprocess-free-veth-provisioner (GH #233)

Per `nw-distill` § "Pre-DELIVER fail-for-the-right-reason gate". DELIVER
reads this at PREPARE to confirm RED is genuine. Every NEW acceptance test
this DISTILL authored is classified `MISSING_FUNCTIONALITY` (RED scaffold)
or `BEHAVIOUR_LOCK` (green-now safety net).

This is a **mechanism swap**: the behaviour surface is already locked by
the existing LIVE Tier-3 e2e (feature-delta § coverage table), so the NEW
test set is small. No new `BEHAVIOUR_LOCK` files were authored — the three
presumed behaviour-locks are all already-live existing tests (mapped, not
duplicated).

## New tests authored this wave

| Test | File | Classification | Expected state now | Goes GREEN when |
|---|---|---|---|---|
| S-LINT-01 named infra-CLI literal flagged | `xtask/tests/dst_lint_infra_subprocess_self_test.rs` | `MISSING_FUNCTIONALITY` | RED scaffold (`#[should_panic]` PASS at the bar) | slice 5 lands the scanner clause; crafter replaces panic with `scan_source_*` + asserts |
| S-LINT-02 `// subprocess-ok:` marker suppresses | " | `MISSING_FUNCTIONALITY` | RED scaffold | slice 5 |
| S-LINT-03 `#[cfg(test)]` + `bin/` exempt | " | `MISSING_FUNCTIONALITY` | RED scaffold | slice 5 |
| S-LINT-04 `overdrive-testing` excluded by scope | " | `MISSING_FUNCTIONALITY` | RED scaffold | slice 5 |
| S-LINT-05 zero violations on migrated tree | " | `MISSING_FUNCTIONALITY` | RED scaffold | slice 5 (after slices 1–4 swap both files) |

**RED-reason class:** all five are `MISSING_FUNCTIONALITY` — the slice-5
scanner clause does not exist. Per `.claude/rules/testing.md`, the bodies
`panic!("Not yet implemented -- RED scaffold …")` under
`#[should_panic(expected = "RED scaffold")]` so the runner reports PASS at
the bar (hook-compatible, no `--no-verify` needed) while the scaffold
remains discoverable via `grep -rn 'should_panic.*RED scaffold' xtask/`.
Bodies deliberately do NOT import the not-yet-existent scanner fn (that
would be `IMPORT_ERROR`/BROKEN, not RED). GREEN transition = drop
`#[should_panic]`, replace the panic with the `scan_source_*` call over
the synthetic source quoted in each test's doc comment.

## Behaviour-locks (already-live existing tests — NOT re-authored)

These are the GREEN-now safety nets for the swap. They are `BEHAVIOUR_LOCK`
by role but were NOT written this wave (they pre-exist). Listed so DELIVER
knows which existing tests MUST stay GREEN (and which get mechanically
EDITED for async — Finding F2):

| Lock | File | Stays GREEN / edited |
|---|---|---|
| D6 fwmark exactly-one | `overdrive-worker/tests/integration/mtls_intercept_install.rs:500-508` | GREEN (must not weaken) |
| ethtool tx-off + idempotent + drift | `overdrive-control-plane/tests/integration/veth_provision_idempotent.rs` | GREEN; slice-1 EDIT `.await`+`#[tokio::test]` (F2) |
| veth create/half-heal/recreate | " (same file) | GREEN; slice-1 EDIT `.await` |
| two-distinct-XDP + EBUSY | `…/serve_boot_provisions_veth.rs` | GREEN; slice-1 EDIT `.await` |
| per-alloc netns lifecycle + sysctl/resolv isolation | `…/workload_netns_provision.rs` | GREEN; slice-2 EDIT if `provision_workload_netns` async |
| alloc netns lifecycle | `…/alloc_netns_lifecycle.rs` | GREEN |
| adopt-on-restart + §5 nft sweep | `…/adopt_on_restart.rs` | GREEN |
| nft install/coexist/by-handle-delete/divert/orig-dst | `overdrive-worker/tests/integration/{mtls_intercept_install,egress_tproxy_capture,bidirectional_walking_skeleton,inbound_tproxy_harness,start_alloc_installs_both_tproxy}.rs`, `overdrive-control-plane/tests/integration/canonical_address_inbound_walking_skeleton.rs` | GREEN |

## Pre-DELIVER gate note

The five RED scaffolds are the ONLY new tests; they fail for the right
reason (`MISSING_FUNCTIONALITY` — scanner absent), not setup/import error.
The behaviour-lock existing tests are already GREEN against HEAD and their
EDIT (async) is a per-slice mechanical migration whose assertions are
unchanged. No `WRONG_ASSERTION` / `OBSERVABLE_NOT_AT_PORT` cases.
