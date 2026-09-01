# Mutation Report — `guest-stack-transparent-mtls-intercept`

## Current executive verdict

**FAIL — the final DELIVER mutation gate remains blocked with no mutation
quality signal.** Attempt 3 ran the exact roadmap command at
`2316c423ffeba74a61d8fa5589bc17c786293f00` after `ovd-tp-0000` had been
independently verified absent with no owner, lease, or process. The unmutated
whole-workspace baseline nevertheless reproduced the same native VM failure
journey: Cloud Hypervisor reported that `ovd-tp-0000` already existed, the
sibling workload finalized `Failed`, and the test timed out. Cargo-mutants
therefore evaluated zero of the 769 selected candidates. The empirical kill
rate is undefined; the wrapper's vacuous `100.0` field is not a pass.

## Attempt 1 — OpenAPI baseline blocker

The following Attempt 1 record is preserved as provenance for the original
baseline failure and its selected mutation scope.

### Metadata

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

### Preflight

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

### Selected production scope

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

#### Selected candidates by crate

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

#### Selected candidates by file

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

### Authoritative run result

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

#### Outcome counters

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

### Survivors and test gaps

No surviving mutant can be listed because no mutant was evaluated. The 769
selected candidates remain **unclassified**, not caught and not survived.
Consequently there is no evidence-backed function-level survivor/test-gap
mapping from this run.

The only demonstrated test gap is the pre-existing unmutated baseline defect:
the generated OpenAPI schema and checked-in `api/openapi.yaml` disagree at the
workload-stop route. That gap blocks every mutation result and therefore blocks
the >=80% final DELIVER gate.

### Verdict

**FAIL — no quality signal.** The exact authoritative command exited 1 because
the unmutated baseline failed with zero mutants tested. No PASS/WARN threshold
classification is possible from an undefined empirical kill rate, and the
769 selected candidates cannot be treated as survivors or kills. The final
DELIVER mutation gate remains blocked until the baseline passes and the same
authoritative command completes.

### Raw artifacts

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

### Post-run integrity

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

## Attempt 2 — post-OpenAPI-remediation rerun

### Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Date | 2026-08-31 UTC |
| Local source HEAD | `a2e69409dbb144107eef48df53fae2ac6130b6c3` |
| Approved OpenAPI remediation | `42aed6f997847357b70df222bb2e00ee0f86a455` |
| Approved remediation review at HEAD | `a2e69409dbb144107eef48df53fae2ac6130b6c3` |
| Base | `origin/main` at `c8e2e4186bae3c1c133116d95d65e3b82cacc789` |
| Strategy | Per-feature, diff-scoped |
| Runner | Qualified native x86_64/KVM metal host through `cargo xtask metal run --` |
| Tool | `cargo-mutants` 27.1.0 via `cargo xtask mutants`; `cargo-nextest` 0.9.143 |
| Threshold | PASS at kill rate >= 80%; WARN at 70% to <80%; FAIL below 70% or when the run produces no quality signal |
| Exact roadmap command | `cargo xtask metal run -- cargo xtask mutants --diff origin/main --features integration-tests,kvm-tests --test-whole-workspace` |
| Command result | Exit 1 |
| Verdict | **FAIL — unmutated baseline timed out; no mutant was evaluated** |

The command used the canonical native guest selections
`OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel` and
`OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4`. These
environment selections satisfy the repository's fail-closed native preflight
without altering the roadmap command's arguments.

### Preflight and source identity

The exact command acquired the canonical metal lease, synchronised the local
working tree to `/home/ubuntu/overdrive`, and passed the fail-closed native
x86_64/KVM preflight before running cargo-mutants. The wrapper found both
required tools and materialised a 3,844,182-byte diff against `origin/main`.

The synchronised source content came from local HEAD
`a2e69409dbb144107eef48df53fae2ac6130b6c3`. The metal runner deliberately
does not rsync `.git`; its retained remote Git metadata still named
`019d0c1a449de09b82793bc124282a4803c75dd7`, while the synchronised working
tree contained the later approved OpenAPI remediation and review. This is why
remote `git status` reported `api/openapi.yaml` as modified relative to the
retained metadata. The actual schema bytes matched the approved remediation at
SHA-256
`b6a55e7e5fd4718735048d350a15ece9c7a9d0b6de518228298960d22b4b3183`,
and the former failing OpenAPI baseline test ran without appearing among the
baseline failures.

Before the run, the local tracked worktree delta consisted only of the
user-owned `AGENTS.md` modification. No production, test, manifest, mutation
configuration, roadmap, review, or execution-log file was edited for this
gate.

### Selected production scope

Cargo-mutants reported **769 selected mutation candidates** across **30
production files** in **10 crates**. This is the authoritative post-diff,
post-exclusion selection from
`target/xtask/mutants.out/mutants.json`; it is not an inferred allowlist. The
scope is identical to Attempt 1 because the approved remediation changed only
the generated OpenAPI artifact, which does not add a mutable production site.

The exclusion source remained `.cargo/mutants.toml` at SHA-256
`097dcf12fbc2af593a95fadac4bfedd862f65b8ab68838194a9d76d31fc7aeb3`.
No exclusion, source, test, or manifest was altered to change the result.

#### Selected candidates by crate

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

#### Selected candidates by file

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

Because the baseline failed before candidate execution, these are candidate
counts rather than caught/missed outcome counts.

### Authoritative run result

Cargo-mutants generated the 769 candidates, then executed the unmutated
whole-workspace baseline with `integration-tests,kvm-tests`. Compilation
passed in 128.137 seconds. The test phase failed after 143.071 seconds:

- 2,315 of 2,504 tests ran;
- 2,314 passed, including 5 reported as leaky;
- 1 timed out;
- 27 were skipped by nextest;
- 189 were not run after fail-fast cancellation.

The baseline timeout was:

`overdrive-cli::integration::integration::guest_stack_mtls_egress::a_microvm_that_cannot_address_its_network_is_refused_as_a_boot_failure`

The test exceeded nextest's 120-second limit while waiting for sibling
workload `gti-resolver-sibling` to reach `Running`. Its last observed row was
instead `Failed` with `VmGuestExitUnreported { vmm_exit_code: None,
vmm_signal: Some(9) }`. The captured Cloud Hypervisor diagnostic was:

`Tap ovd-tp-0000 already exists. IP configuration will not be overwritten.`

The run therefore demonstrated a pre-existing native-host tap collision in
the unmutated baseline. Per the gate instructions, no alternate mutation
command, scope change, exclusion change, or source/test remediation was
invented after this failure.

#### Outcome counters and empirical rate

| Outcome | Tool-reported count | Interpretation |
|---|---:|---|
| Selected/generated candidates | 769 | Post-diff, post-exclusion candidate list |
| Evaluated mutants (`total_mutants`) | 0 | Baseline timed out before mutation execution |
| Caught/killed | 0 | No mutant ran |
| Missed/survived | 0 | No mutant ran; this is not evidence of zero survivors |
| Timeout | 0 | No mutant ran; the timeout was the unmutated baseline test |
| Unviable | 0 | No mutant ran |
| Baseline success | 0 | Unmutated baseline failed |

The empirical kill-rate formula is `caught / (caught + missed)`. Attempt 2
therefore produced `0 / (0 + 0)`, an **undefined** empirical kill rate. The
wrapper also emitted `kill_rate_pct: 100.0`, but paired it with
`baseline_success: 0`, `status: "fail"`, and the explicit no-quality-signal
reason. That mechanically vacuous value is not a PASS and is not compared to
the 80% threshold.

### Survivors and test gaps

There is no survivor list: no mutant was evaluated. All 769 selected
candidates remain **unclassified**, not caught and not survived. Consequently
there is no honest per-function survivor/test-gap mapping from Attempt 2.

The demonstrated blocker is the unmutated native baseline's tap collision and
resulting VM-journey timeout. The approved OpenAPI remediation did close
Attempt 1's blocker: its formerly failing test started at baseline test 824 and
was not the baseline failure. This does not substitute for a completed
mutation run.

### Verdict

**FAIL — no mutation quality signal.** The exact authoritative command exited
1 because the unmutated baseline timed out before any mutant ran. With an
undefined empirical kill rate, neither PASS nor WARN classification is
available, and the final DELIVER mutation gate remains blocked.

### Raw artifacts

Large generated and build artifacts were not committed. Attempt 2 evidence
remains at:

- outer command log on the local workspace:
  `/Users/marcus/conductor/workspaces/helios/krakow-v3/target/xtask/guest-stack-transparent-mtls-intercept-metal-mutation-attempt-2.log`;
- structured wrapper summary on metal:
  `/home/ubuntu/overdrive/target/xtask/mutants-summary.json` (SHA-256
  `c894c713525f6c16cd8fbe32a69e0c81d2a18e90c6c36c67cf3e7f86db291f0e`);
- structured cargo-mutants outcomes on metal:
  `/home/ubuntu/overdrive/target/xtask/mutants.out/outcomes.json` (SHA-256
  `ccb99478b0895a5bd18e5e8f65c68936eee8c9b05bb930f988a859ea174f816a`);
- selected candidate corpus on metal:
  `/home/ubuntu/overdrive/target/xtask/mutants.out/mutants.json` (SHA-256
  `6cbe623b608092a841cd1b1e84c3fdee4cecd10e385b5aeb31ae4ceb95a382c3`);
- unmutated baseline log on metal:
  `/home/ubuntu/overdrive/target/xtask/mutants.out/log/baseline.log`
  (SHA-256
  `05d83f600e1cdc37106d7a1ce4df2b8cf0ff179daf900e8cb89f83c372a43f4b`).

The structured summary records `status: "fail"` and reason: `unmutated
baseline failed: cargo-mutants exited non-zero with zero mutants tested`.

### Post-run integrity

Cargo-mutants used its isolated copy under
`/tmp/cargo-mutants-overdrive-VV8hTV.tmp`; no broad checkout, reset, clean, or
routine source restore was issued.

- The 30 selected production files had the same aggregate SHA-256 digest
  before and after the run, and the post-run local workspace and metal
  checkout matched:
  `f5ca2f36ae56d60d52d7679c05df75795123f969803d0c0643baed091b8d5237`.
- Local tracked status after the run still contained only the pre-existing
  `AGENTS.md` modification before this report was edited.
- `AGENTS.md` remained byte-for-byte unchanged at SHA-256
  `2ef762175ab8ed4e0c9f6efec0c3cfa25ddf6d53fa9ac39ff2615b8af1628daf`.
- `.cargo/mutants.toml` remained byte-for-byte unchanged at SHA-256
  `097dcf12fbc2af593a95fadac4bfedd862f65b8ab68838194a9d76d31fc7aeb3`.
- The approved `api/openapi.yaml` remained byte-for-byte unchanged at SHA-256
  `b6a55e7e5fd4718735048d350a15ece9c7a9d0b6de518228298960d22b4b3183`.
- The roadmap remained unchanged at SHA-256
  `cb29232a916af4c5ef99c57586cba66a535212643285c59be9ec5b971963c305`.
- The baseline-remediation review remained unchanged at SHA-256
  `24c19c05bf14bfc0fbe995aa9e55e25c8aa0d2b51b933f91d07667ebe2ae43c2`.
- No tracked production or test mutation remained in either checkout.

## Attempt 3 — rerun after independent tap-absence verification

### Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Date | 2026-08-31 UTC |
| Local source HEAD | `2316c423ffeba74a61d8fa5589bc17c786293f00` |
| Base | `origin/main` at `c8e2e4186bae3c1c133116d95d65e3b82cacc789` |
| Strategy | Per-feature, diff-scoped |
| Runner | Qualified native x86_64/KVM metal host through `cargo xtask metal run --` |
| Tool | `cargo-mutants` 27.1.0 via `cargo xtask mutants`; `cargo-nextest` 0.9.143 |
| Threshold | PASS at kill rate >= 80%; WARN at 70% to <80%; FAIL below 70% or when the run produces no quality signal |
| Exact roadmap command | `cargo xtask metal run -- cargo xtask mutants --diff origin/main --features integration-tests,kvm-tests --test-whole-workspace` |
| Command result | Exit 1 |
| Verdict | **FAIL — unmutated baseline timed out; no mutant was evaluated** |

The command used the canonical native guest selections
`OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel` and
`OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4`. Their
SHA-256 digests were respectively
`b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`
and
`43e50ea8743245c4103e87bd8cb0dcf9ce4ba9f98b030478ba272f4ad6961bf8`.
These environment selections satisfy the fail-closed native preflight without
altering the roadmap command's arguments.

### Preflight and source identity

Attempt 3 was dispatched only after an independent read-only audit had
verified that the previously reported `ovd-tp-0000` was absent and had no
owner, lease, or process. The exact command then acquired the canonical metal
lease (`token=426ac03dde29de7834e7dd27`, bootstrap PID `2808767`), synchronised
the local working tree to `/home/ubuntu/overdrive`, and passed the fail-closed
native x86_64/KVM preflight before running cargo-mutants. The wrapper found
both required tools and materialised a 3,844,182-byte diff against
`origin/main`.

The synchronised source content came from local HEAD
`2316c423ffeba74a61d8fa5589bc17c786293f00`. As in Attempt 2, the metal runner
does not rsync `.git`; its retained remote Git metadata still named
`019d0c1a449de09b82793bc124282a4803c75dd7`, while the synchronised working
tree held the current local bytes. Before the run, the local tracked worktree
delta consisted only of the user-owned `AGENTS.md` modification. No
production, test, manifest, mutation configuration, roadmap, review, or
execution-log file was edited for the gate.

### Selected production scope

Cargo-mutants reported **769 selected mutation candidates** across **30
production files** in **10 crates**. This is the authoritative post-diff,
post-exclusion selection from
`target/xtask/mutants.out/mutants.json`; its SHA-256 digest
`6cbe623b608092a841cd1b1e84c3fdee4cecd10e385b5aeb31ae4ceb95a382c3`
is identical to Attempts 1 and 2. The unchanged exclusion source was
`.cargo/mutants.toml` at SHA-256
`097dcf12fbc2af593a95fadac4bfedd862f65b8ab68838194a9d76d31fc7aeb3`.
No exclusion, source, test, manifest, timeout, or test command was altered.

#### Selected candidates by crate

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

#### Selected candidates by file

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

Because the baseline failed before candidate execution, these are candidate
counts rather than caught/missed outcome counts.

### Authoritative run result

Cargo-mutants generated the 769 candidates, then executed the unmutated
whole-workspace baseline with `integration-tests,kvm-tests`. Compilation
passed in approximately 127 seconds. The test phase failed after 142.273
seconds:

- 2,315 of 2,504 tests ran;
- 2,314 passed, including 5 reported as leaky;
- 1 timed out after 120.014 seconds;
- 27 were skipped by nextest;
- 189 were not run after fail-fast cancellation.

The baseline timeout was again:

`overdrive-cli::integration::integration::guest_stack_mtls_egress::a_microvm_that_cannot_address_its_network_is_refused_as_a_boot_failure`

The test waited 60 seconds for sibling workload `gti-resolver-sibling` to
reach `Running`. Its last observed row instead finalized `Failed` with
`VmGuestExitUnreported { vmm_exit_code: None, vmm_signal: Some(9) }`. The
captured Cloud Hypervisor diagnostic was again:

`Tap ovd-tp-0000 already exists. IP configuration will not be overwritten.`

This is a recurrence during the unmutated Attempt 3 baseline despite the
independent pre-run absence/ownership verification. A direct read-only probe
after the command completed reported `Device "ovd-tp-0000" does not exist`;
no Cloud Hypervisor or `overdrive serve` process and no `/dev/net/tun` owner
was then present. No host resource was deleted. The run establishes the
recurrence but does not by itself establish which concurrent or lifecycle
path created and removed the transient tap.

#### Outcome counters and empirical rate

| Outcome | Tool-reported count | Interpretation |
|---|---:|---|
| Selected/generated candidates | 769 | Post-diff, post-exclusion candidate list |
| Evaluated mutants (`total_mutants`) | 0 | Baseline timed out before mutation execution |
| Caught/killed | 0 | No mutant ran |
| Missed/survived | 0 | No mutant ran; this is not evidence of zero survivors |
| Timeout | 0 | No mutant ran; the timeout was the unmutated baseline test |
| Unviable | 0 | No mutant ran |
| Baseline success | 0 | Unmutated baseline failed |

The empirical kill-rate formula is `caught / (caught + missed)`. Attempt 3
therefore produced `0 / (0 + 0)`, an **undefined** empirical kill rate. The
wrapper emitted `kill_rate_pct: 100.0`, but paired it with
`baseline_success: 0`, `status: "fail"`, and the explicit no-quality-signal
reason. That mechanically vacuous value is not a PASS and is not compared to
the 80% threshold.

### Survivors and blocker

There is no survivor list: no mutant was evaluated. All 769 selected
candidates remain **unclassified**, not caught and not survived. Consequently
there is no honest per-function survivor/test-gap mapping from Attempt 3.

The demonstrated blocker is the recurrent unmutated native baseline tap-name
collision and resulting VM-journey timeout. Per the gate instructions, no
alternate mutation scope, test command, timeout, exclusion, source, test, or
host-cleanup remediation was substituted after this failure.

### Verdict

**FAIL — no mutation quality signal.** The exact authoritative command exited
1 because the unmutated baseline timed out before any mutant ran. With an
undefined empirical kill rate, neither PASS nor WARN classification is
available, and the final DELIVER mutation gate remains blocked.

### Raw artifacts

Attempt 3's generated artifacts were copied without modification into the
local ignored target tree so a later run does not overwrite them:

- structured wrapper summary:
  `/Users/marcus/conductor/workspaces/helios/krakow-v3/target/xtask/guest-stack-transparent-mtls-intercept-mutation-attempt-3/mutants-summary.json`
  (SHA-256
  `c894c713525f6c16cd8fbe32a69e0c81d2a18e90c6c36c67cf3e7f86db291f0e`);
- structured cargo-mutants outcomes:
  `/Users/marcus/conductor/workspaces/helios/krakow-v3/target/xtask/guest-stack-transparent-mtls-intercept-mutation-attempt-3/outcomes.json`
  (SHA-256
  `a291f027fe9a4496e46e1b3f6e2dc11145af8b63ce196b8b2ad0fdcce00d773a`);
- selected candidate corpus:
  `/Users/marcus/conductor/workspaces/helios/krakow-v3/target/xtask/guest-stack-transparent-mtls-intercept-mutation-attempt-3/mutants.json`
  (SHA-256
  `6cbe623b608092a841cd1b1e84c3fdee4cecd10e385b5aeb31ae4ceb95a382c3`);
- unmutated baseline log:
  `/Users/marcus/conductor/workspaces/helios/krakow-v3/target/xtask/guest-stack-transparent-mtls-intercept-mutation-attempt-3/baseline.log`
  (SHA-256
  `3caa18465716c8860729868f9cb96e6fd8230b3996e4533b6083a62f1c804baa`).

The corresponding metal paths at collection time were
`/home/ubuntu/overdrive/target/xtask/mutants-summary.json`,
`/home/ubuntu/overdrive/target/xtask/mutants.out/outcomes.json`,
`/home/ubuntu/overdrive/target/xtask/mutants.out/mutants.json`, and
`/home/ubuntu/overdrive/target/xtask/mutants.out/log/baseline.log`.

### Post-run integrity

Cargo-mutants used its isolated copy under
`/tmp/cargo-mutants-overdrive-NtLYyr.tmp`; no broad checkout, reset, clean,
routine source restore, or host-resource deletion was issued.

- The 30 selected production files had the same aggregate SHA-256 digest on
  the local workspace and metal checkout after the run:
  `f5ca2f36ae56d60d52d7679c05df75795123f969803d0c0643baed091b8d5237`.
  This is also the digest recorded after Attempts 1 and 2.
- Local tracked status after the run still contained only the pre-existing
  `AGENTS.md` modification before this report was edited.
- `AGENTS.md` remained byte-for-byte unchanged at SHA-256
  `2ef762175ab8ed4e0c9f6efec0c3cfa25ddf6d53fa9ac39ff2615b8af1628daf`.
- `.cargo/mutants.toml` remained byte-for-byte unchanged at SHA-256
  `097dcf12fbc2af593a95fadac4bfedd862f65b8ab68838194a9d76d31fc7aeb3`.
- The approved `api/openapi.yaml` remained byte-for-byte unchanged at SHA-256
  `b6a55e7e5fd4718735048d350a15ece9c7a9d0b6de518228298960d22b4b3183`.
- The roadmap remained unchanged at SHA-256
  `cb29232a916af4c5ef99c57586cba66a535212643285c59be9ec5b971963c305`.
- The baseline-remediation review remained unchanged at SHA-256
  `24c19c05bf14bfc0fbe995aa9e55e25c8aa0d2b51b933f91d07667ebe2ae43c2`.
- No tracked production or test mutation remained in either checkout.
