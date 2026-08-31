# Mutation Report — `guest-stack-transparent-mtls-intercept`

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Date | 2026-08-31 UTC |
| HEAD | `019d0c1a449de09b82793bc124282a4803c75dd7` |
| Base | `origin/main` at `c8e2e4186bae3c1c133116d95d65e3b82cacc789` |
| Strategy | Per-feature, diff-scoped |
| Runner | Qualified native x86_64/KVM metal host through `cargo xtask metal run --` |
| Tool | `cargo-mutants` 27.1.0 via `cargo xtask mutants`; `cargo-nextest` 0.9.143 |
| Threshold | PASS at kill rate >= 80%; WARN at 70% to <80%; FAIL below 70% or when the run produces no quality signal |
| Exact roadmap command | `cargo xtask metal run -- cargo xtask mutants --diff origin/main --features integration-tests,kvm-tests --test-whole-workspace` |
| Command result | Exit 1 |
| Verdict | **FAIL — unmutated baseline failed; no mutant was evaluated** |

The command was launched with the approved native guest inputs
`OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel` and
`OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4`. Those
environment selections do not alter the roadmap command's arguments.

## Preflight

The first selector-free qualification probe refused with `selected guest
kernel is required`, as the fail-closed preflight requires. The qualified
rerun selected the canonical kernel and rootfs and passed the repository's
native-metal checks before command execution. It confirmed:

- remote HEAD `019d0c1a449de09b82793bc124282a4803c75dd7` and base
  `c8e2e4186bae3c1c133116d95d65e3b82cacc789`;
- `cargo-mutants` 27.1.0 and `cargo-nextest` 0.9.143 available;
- `target/bpf/overdrive_bpf.o` readable;
- the pre-run tracked worktree delta consisted only of the user-owned
  `AGENTS.md` modification.

Because the metal checkout normally omits `.git`, the repository bootstrap
workflow first materialised the current worktree's Git metadata using its
documented `--with-git` mode. The subsequent authoritative command used the
ordinary `cargo xtask metal run --` path and its retained native preflight and
metal lease.

## Selected production scope

The xtask wrapper materialised a 3,843,331-byte diff against `origin/main` and
cargo-mutants reported **769 selected mutation candidates** across **30
production files** in **10 crates**. The list comes from the run's
post-filter `target/xtask/mutants.out/mutants.json`, so it reflects the
authoritative diff selection and `.cargo/mutants.toml` exclusions rather than
an independently invented scope. Every selected path existed on the metal
checkout; the missing-path check returned no entries.

The exclusion source was the unchanged `.cargo/mutants.toml` at SHA-256
`097dcf12fbc2af593a95fadac4bfedd862f65b8ab68838194a9d76d31fc7aeb3`,
whose governing policy is `.claude/rules/testing.md` section “Mutation testing
(cargo-mutants).” No exclusion, manifest, test, or source file was edited for
this gate.

### Selected candidates by crate

| Crate | Candidates |
|---|---:|
| `overdrive-cli` | 5 |
| `overdrive-control-plane` | 167 |
| `overdrive-core` | 14 |
| `overdrive-dataplane` | 2 |
| `overdrive-host` | 21 |
| `overdrive-init` | 66 |
| `overdrive-netlink` | 343 |
| `overdrive-reconcilers` | 18 |
| `overdrive-store-local` | 21 |
| `overdrive-worker` | 112 |
| **Total** | **769** |

### Selected candidates by file

| File | Candidates |
|---|---:|
| `crates/overdrive-cli/src/commands/serve.rs` | 2 |
| `crates/overdrive-cli/src/render.rs` | 3 |
| `crates/overdrive-control-plane/src/action_shim/mod.rs` | 57 |
| `crates/overdrive-control-plane/src/action_shim/reclamation.rs` | 4 |
| `crates/overdrive-control-plane/src/action_shim/write_service_backend_row.rs` | 1 |
| `crates/overdrive-control-plane/src/handlers.rs` | 2 |
| `crates/overdrive-control-plane/src/lib.rs` | 4 |
| `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs` | 7 |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs` | 4 |
| `crates/overdrive-control-plane/src/veth_provisioner.rs` | 78 |
| `crates/overdrive-control-plane/src/vm_reclamation_boot.rs` | 3 |
| `crates/overdrive-control-plane/src/worker/exit_observer.rs` | 6 |
| `crates/overdrive-control-plane/src/workflow_runtime/mod.rs` | 1 |
| `crates/overdrive-core/src/traits/driver.rs` | 1 |
| `crates/overdrive-core/src/traits/observation_store.rs` | 7 |
| `crates/overdrive-core/src/traits/vmm.rs` | 3 |
| `crates/overdrive-core/src/vm/config.rs` | 3 |
| `crates/overdrive-dataplane/src/mtls/tls_config.rs` | 2 |
| `crates/overdrive-host/src/vmm.rs` | 21 |
| `crates/overdrive-init/src/main.rs` | 66 |
| `crates/overdrive-netlink/src/client.rs` | 43 |
| `crates/overdrive-netlink/src/nft.rs` | 300 |
| `crates/overdrive-reconcilers/src/vm_reclamation.rs` | 6 |
| `crates/overdrive-reconcilers/src/workload_lifecycle.rs` | 12 |
| `crates/overdrive-store-local/src/observation_backend.rs` | 21 |
| `crates/overdrive-worker/src/mtls_intercept.rs` | 9 |
| `crates/overdrive-worker/src/mtls_intercept_worker.rs` | 44 |
| `crates/overdrive-worker/src/node_health.rs` | 1 |
| `crates/overdrive-worker/src/probe_runner/mod.rs` | 1 |
| `crates/overdrive-worker/src/vm_driver.rs` | 57 |
| **Total** | **769** |

This is a candidate breakdown, not a caught/missed breakdown: the baseline
failure occurred before the first candidate ran.

## Authoritative run result

Cargo-mutants found the 769 candidates, then ran its unmutated whole-workspace
baseline with `integration-tests,kvm-tests` enabled. The build phase passed in
133.414 seconds. The test phase failed after 52.655 seconds:

- 840 of 2,504 tests started;
- 839 passed, including 4 reported as leaky;
- 1 failed;
- 27 were skipped by nextest;
- 1,664 were not run after fail-fast cancellation.

The failing baseline test was:

`overdrive-control-plane::integration::integration::openapi_gate::openapi_check_subprocess_exits_0_against_checked_in_yaml`

Its assertion reported checked-in OpenAPI drift at
`/v1/workloads/{id}/stop`, near line 609 of `api/openapi.yaml`: the live schema
had `workload_addr:` where the checked-in schema had `workload_id:`. The test
instructed regeneration with `cargo openapi-gen`. This report records that
diagnostic only; the mutation gate did not modify the schema, source, tests, or
configuration.

### Outcome counters

| Outcome | Tool-reported count | Interpretation |
|---|---:|---|
| Selected/generated candidates | 769 | Post-diff, post-exclusion candidate list |
| Evaluated mutants (`total_mutants`) | 0 | Baseline failed before mutation execution |
| Caught/killed | 0 | No mutant ran |
| Missed/survived | 0 | No mutant ran; this is not evidence of zero survivors |
| Timeout | 0 | No mutant ran |
| Unviable | 0 | No mutant ran |
| Baseline success | 0 | Unmutated test baseline failed |
| Excluded mutation count | Not reported | Cargo-mutants emitted only the 769 post-filter candidates |
| Skipped mutation count | Not reported | The 27 skipped items above are baseline tests, not skipped mutants |

The normal kill-rate formula is `caught / (caught + missed)`. Here that is
`0 / (0 + 0)`, so the empirical kill rate is **undefined**. The wrapper's JSON
contains a mechanically vacuous `kill_rate_pct: 100.0`, but also records
`baseline_success: 0`, `status: "fail"`, and the explicit no-quality-signal
reason. The 100.0 field must not be interpreted as a passing mutation score.

## Survivors and test gaps

No surviving mutant can be listed because no mutant was evaluated. The 769
selected candidates remain **unclassified**, not caught and not survived.
Consequently there is no evidence-backed function-level survivor/test-gap
mapping from this run.

The only demonstrated test gap is the pre-existing unmutated baseline defect:
the generated OpenAPI schema and checked-in `api/openapi.yaml` disagree at the
workload-stop route. That gap blocks every mutation result and therefore blocks
the >=80% final DELIVER gate.

## Verdict

**FAIL — no quality signal.** The exact authoritative command exited 1 because
the unmutated baseline failed with zero mutants tested. No PASS/WARN threshold
classification is possible from an undefined empirical kill rate, and the
769 selected candidates cannot be treated as survivors or kills. The final
DELIVER mutation gate remains blocked until the baseline passes and the same
authoritative command completes.

## Raw artifacts

Large generated and build artifacts were not committed. The evidence remains
at these paths:

- outer command log on the local workspace:
  `/Users/marcus/conductor/workspaces/helios/krakow-v3/target/xtask/guest-stack-transparent-mtls-intercept-metal-mutation.log`;
- structured wrapper summary on metal:
  `/home/ubuntu/overdrive/target/xtask/mutants-summary.json`;
- structured cargo-mutants outcomes on metal:
  `/home/ubuntu/overdrive/target/xtask/mutants.out/outcomes.json`;
- selected candidate corpus on metal:
  `/home/ubuntu/overdrive/target/xtask/mutants.out/mutants.json`;
- baseline diagnostic log on metal:
  `/home/ubuntu/overdrive/target/xtask/mutants.out/log/baseline.log`.

The wrapper summary records `status: "fail"` and reason: `unmutated baseline
failed: cargo-mutants exited non-zero with zero mutants tested`.

## Post-run integrity

Cargo-mutants used its isolated copy under `/tmp/cargo-mutants-*`; no routine
checkout or broad restore command was issued.

- All 30 selected production files had the same aggregate SHA-256 digest on
  the local workspace and metal checkout after the failed run:
  `f5ca2f36ae56d60d52d7679c05df75795123f969803d0c0643baed091b8d5237`.
- Local tracked status after the run still contained only the pre-existing
  `AGENTS.md` modification before this report was created. Metal tracked status
  matched; its additional `.overdrive-metal-source` marker was untracked
  runner metadata.
- `AGENTS.md` remained byte-for-byte unchanged at SHA-256
  `2ef762175ab8ed4e0c9f6efec0c3cfa25ddf6d53fa9ac39ff2615b8af1628daf`.
- `.cargo/mutants.toml` remained unchanged at SHA-256
  `097dcf12fbc2af593a95fadac4bfedd862f65b8ab68838194a9d76d31fc7aeb3`.
- No tracked production or test mutation remained in either checkout.
