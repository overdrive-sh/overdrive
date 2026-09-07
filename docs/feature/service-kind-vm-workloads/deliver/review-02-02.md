# DELIVER review — step 02-02: E08 walking skeleton

## Metadata

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` |
| Step | `02-02` |
| Commits reviewed | `37f1f6d84a5912ccd1048f8604ba90026b7e7d1e`; remediation `75d049ded61aa814bf4023134f5f1d035988518f` |
| Parent | `894c13f79ddc6782e97fcd488c1da83969953bff` |
| Reviewer | Fresh isolated implementation reviewer |
| Iterations | 1–2 |
| Final verdict | **APPROVED** |

## Accepted contract

Roadmap step 02-02 activates the sole E08 walking skeleton. Its required
outcome is a built default-feature product journey which preserves `Running`
for the startup-failure observation, makes a startup pass change only Service
`Stable`, renders guest TCP and HTTP outcomes, has a peer VM Job establish the
byte-exact `SVM-E08-GUEST-OK` result, and reports zero example-owned teardown
deltas.

The pre-existing E08 expectation makes the public boundary exact: one Service
`overdrive deploy service.toml` command follows `serve`, and that command's
stdout must order one `Accepted` before one `Stable`
(`verification/expectations/E08-vm-service-guest-health/README.md:28-47`).
The runner must drive the built binary only; it may not replace product
behavior with a test harness or crate import.

## Iteration 1 review

### Proven production-path evidence

- The newly transitioned reconciler acceptance tests have the required
  `CONTRACT_SHAPE` and outcome-anchor declarations, and they exercise the
  existing startup-failure and startup-pass action shapes
  (`crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs:68-142`).
- On the real path a `Stable` terminal condition preserves the prior allocation
  state and workload address, writes the observation, and retires only startup
  supervision (`crates/overdrive-control-plane/src/action_shim/mod.rs:1623-1736`).
  This conforms to the accepted ownership boundary: `Running` is owned by the
  action shim/driver and `Stable` by ServiceLifecycle.
- The captured native-metal run is genuine built-product evidence: it records
  the projected TCP and HTTP guest probes as `last=pass`, a `Running`
  allocation, a successful peer VM Job, and all eight requested teardown
  deltas as zero
  (`verification/expectations/E08-vm-service-guest-health/evidence/product-run.out:38-97`).
  The runner and example use CLI commands plus an ordinary guest Job; neither
  imports an `overdrive-*` crate or invokes a Rust test binary.

### Findings

#### R02-02-1 — issue (blocking): E08 fabricates the required one-command Accepted → Stable ordering by concatenating two deploy invocations

The accepted E08 boundary requires one Service deployment whose stdout contains
the ordered `Accepted` and `Stable` events. The implementation instead:

1. submits `service.toml` through `deploy --detach` and takes `Accepted` from
   that detached acknowledgement
   (`examples/service-kind-vm-workloads/run-example.sh:296-311`);
2. resubmits the already-created Service through a separate PTY command solely
   to obtain `Stable` (`run-example.sh:313-327`); and
3. appends the two transcripts before testing their apparent line order
   (`run-example.sh:327-346`).

This is not a theoretical distinction. The captured evidence shows `Accepted`
at line 22, then a distinct `script` command beginning at line 29 and emitting
`stable` at line 30
(`verification/expectations/E08-vm-service-guest-health/evidence/product-run.out:22-32`).
The production CLI deliberately selects detached JSON acknowledgement for
`--detach` or non-TTY stdout, and selects the streaming lane only for an
unset-`--detach` TTY (`crates/overdrive-cli/src/main.rs:83-129`). Therefore no
single invocation in this capture produced the asserted order.

The current shell assertion can pass even if the streaming deploy fails to
produce `Accepted`, or if the first detached acknowledgement and the later
idempotent stream refer to different submission behavior. It does not prove the
operator contract named by E08, so the evidence cannot be marked satisfied.

**Remediation:** keep one fresh Service deployment, execute that one un-detached
command under the existing PTY recorder, and assert `Accepted` then `Stable`
inside its single transcript. Remove the detached first submission and the
idempotent resubmission/concatenation. Re-capture E08 through
`verification/harness/run-expectation.sh E08`; do not amend the product output
by hand. This is a runner/evidence correction only and introduces no public API,
lifecycle policy, or new product mechanism.

### Evidence hygiene and scope

- **praise:** the captured output preserves the remote command, native-metal
  lease, product stdout, and teardown ledger rather than narrating their
  result. The dirty SHA is accompanied by both `dirty-status.txt` and the full
  dirty patch, so the captured source state is auditable.
- `git diff --check 37f1f6d8^ 37f1f6d8` reports trailing whitespace only in
  verbatim evidence: terminal-aligned table rows in `product-run.out` and
  `run.log`, plus the raw `dirty-diff.patch`. No `.gitattributes` exception is
  present, but this is the existing catalogue convention: E07 and multiple
  earlier expectation captures retain the same whitespace in raw output and
  dirty patches. Trimming it would alter the required verbatim evidence and
  corrupt the patch. This is justified evidence preservation, not an in-scope
  defect.
- `bash -n` passes for the changed E08 runner and example. `shellcheck` emits
  only non-blocking pre-existing-style warnings (`SC2155` command substitutions
  in `readonly` assignments and unused `KEK_DESCRIPTION`); none changes the
  observed product contract.
- No public API, driver, probe protocol, lifecycle owner, or test-only
  production wiring was added by this commit. The scope is otherwise limited to
  the step-owned acceptance scaffolds, E08 example/runner, evidence, README,
  and DES event record.

## Verification

| Command or evidence | Result |
| --- | --- |
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --test acceptance -E 'test(vm_startup_failure_leaves_running_owned_by_beacon_and_fails_only_startup) or test(vm_startup_pass_changes_only_service_stable)'` | PASS — 2/2 |
| `bash -n examples/service-kind-vm-workloads/run-example.sh verification/expectations/E08-vm-service-guest-health/runner.sh verification/harness/run-expectation.sh` | PASS |
| Captured `verification/harness/run-expectation.sh E08` | Executed successfully on declared `native-metal`; **refuted as a satisfied E08 oracle** by R02-02-1 because its claimed ordering is assembled from two deploy commands. |
| `git diff --check 37f1f6d8^ 37f1f6d8` | Expected evidence-only whitespace findings; preserved verbatim per the disposition above. |

## Remediation disposition

| Finding | Disposition |
| --- | --- |
| R02-02-1 | **Open — return to the original step 02-02 crafter.** Make the one-command PTY capture correction and refresh the E08 harness evidence. |

## Verdict

**REJECTED.** The lifecycle tests, real guest probe output, peer-VM success,
and zero-delta teardown evidence are valuable and remain in scope. The sole
walking skeleton nevertheless fails its exact operator boundary: `Accepted`
and `Stable` are spliced from distinct deploy commands. Approval requires a
fresh single-command capture and re-review.

## Iteration 2 review — Accepted-before-Stable remediation

### Design and implementation conformance

ADR-0093 and its independent DESIGN review are approved and authorize exactly
one private successful-stream assembly change: compose the existing Accepted
renderer once before the existing Stable renderer once in the existing
`DeployStreamingOutput.summary`. The remediation does exactly that in the
`ServiceSubmitEvent::Stable` arm: it renders the retained existing Accepted
fields with `workload_submit_accepted`, renders the existing Stable detail, and
concatenates those two values (`crates/overdrive-cli/src/commands/deploy.rs:738-767`).

The diff changes no public method, type, enum variant, field, command,
argument, route, wire event, dependency, lifecycle owner, probe behavior, or
detached/non-TTY lane. `Failed`, `Stopped`, decode, cancellation, and exit
paths remain outside the changed Stable-only arm (`deploy.rs:769-805`). This
is the exact success-only boundary of ADR-0093 rather than a new streaming
mechanism or an E08-specific rendering path.

The direct `S-SVM-25` regression feeds existing ordered `Accepted` then
`Stable` Service events through the current streaming consumer. It asserts the
complete expected composed summary, one occurrence of each detail, and their
strict order; it also carries the required Contract Shape and outcome-anchor
declarations (`crates/overdrive-cli/tests/acceptance/service_kind_vm_workloads.rs:301-350`).

### One-command E08 evidence

The example now makes one fresh un-detached Service deployment through the PTY
recorder, then finds and orders `Accepted` and `Stable` inside that same
`service-stream.log`. The former detached Service submission, idempotent
resubmission, output concatenation, and post-acceptance splice are absent
(`examples/service-kind-vm-workloads/run-example.sh:296-318`). The later
detached command is solely the separately specified peer Job deployment
(`run-example.sh:332-340`).

The fresh native-metal capture records exactly one PTY command for
`service.toml` (`product-run.out:23`), followed in that command's transcript by
one `Accepted.` block (`:24-30`) and one Stable detail naming startup probe
index 0 (`:31`), then a zero exit (`:33`). The same capture shows the guest TCP
and HTTP observations passing (`:53-54`), peer Job success and the required
byte-exact payload (`:64-69`), and the normal cleanup sequence (`:70-74`).
Its harness receipt declares `native-metal`, `runner_exit_code: "0"`, and a
successful E08 invocation (`verification.yaml:1-15`). The receipt is correctly
marked dirty at the time of capture and names the preceding committed SHA;
the retained raw dirty patch contains the exact Stable-summary and one-PTY
source hunks subsequently committed in `75d049de`. That makes the captured
dirty source state auditable rather than falsely attributing it to an earlier
clean commit.

### Evidence hygiene and scope

- `git diff --check 75d049de^ 75d049de` reports only whitespace in verbatim
  raw terminal capture and its retained dirty patch. The CRLF/terminal-aligned
  content is required to preserve the actual PTY output; as in iteration 1 and
  the existing expectation catalogue, it is evidence preservation rather than
  a source-format defect.
- The expectation remains black-box: its runner drives the built
  default-feature binary and uses no Rust test binary or `overdrive-*` crate.
  The direct CLI regression is independent in-process evidence.
- No DES event was changed by this review. All unrelated dirty DESIGN,
  architecture, and review artifacts remain preserved.

## Iteration 2 verification

| Command or evidence | Result |
| --- | --- |
| `cargo xtask lima run -- cargo nextest run -p overdrive-cli --test acceptance -E 'test(streaming_service_success_summary_orders_one_accepted_before_one_stable)'` | PASS — 1/1 |
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --test acceptance -E 'test(vm_startup_failure_leaves_running_owned_by_beacon_and_fails_only_startup) or test(vm_startup_pass_changes_only_service_stable)'` | PASS — 2/2 |
| `bash -n examples/service-kind-vm-workloads/run-example.sh verification/expectations/E08-vm-service-guest-health/runner.sh verification/harness/run-expectation.sh` | PASS |
| `cargo fmt --check` | PASS |
| Captured `verification/harness/run-expectation.sh E08` | PASS — declared native-metal, runner exit 0; one fresh un-detached PTY Service deploy transcript contains the required ordered pair, guest-probe observations, peer payload, and cleanup. |
| `git diff --check 75d049de^ 75d049de` | Expected raw-evidence-only whitespace findings, retained verbatim as documented above. |

## Remediation disposition

| Finding | Disposition |
| --- | --- |
| R02-02-1 | **Resolved.** ADR-0093 authorized the minimal existing-renderer composition; `75d049de` implements it, S-SVM-25 guards strict order and multiplicity, and fresh E08 evidence proves the one-command operator transcript. |

## Iteration 2 final verdict

**APPROVED.** The remediation removes the fabricated two-command oracle and
truthfully renders one existing Accepted acknowledgement before one existing
Stable detail from the same successful streaming Service command. It conforms
to ADR-0093's narrow public-shape and failure-boundary limits, while the direct
regression and native-metal black-box evidence independently verify the
required outcome.
