# E14 Evidence Audit — Independent Different-Fox Review

## Metadata

| Field | Value |
|---|---|
| Feature | `remove-legacy-exec-workload-driver` |
| Roadmap step | `03-01` — Capture E14 from the checked-in VM example |
| Expectation | `E14-vm-service-post-greenfield-cut` |
| Audit role | Independent black-box evidence reviewer (different fox) |
| Audit date | 2026-09-15T02:10:24Z |
| Evidence substrate | `native-metal` |
| Captured source SHA | `3f628ed08865aa0cdb3fbacd1f2cf989707d4a7a` |
| Capture result | successful second attempt, exit `0` |
| Final verdict | **APPROVED** |

The captured source SHA resolves to the approved post-cut parent commit. The
capture commit adds only the E14 receipt, catalogue entry, and DELIVER log; it
does not alter the checked-in VM example or the historical E06/E08 receipts.

## Evidence-only boundary and independence

This audit evaluates the retained E14 expectation, its actual native-metal
transcripts, provenance receipts, the checked-in example boundary, the
catalogue entry, and the applicable black-box contract. It does not substitute
Rust tests, implementation reasoning, or a source-level correctness claim for
the captured product outcome. No Rust test, nextest run, expectation runner,
production binary, mutation command, or `overdrive-*` crate import/link was
run by this reviewer. The only files written by this audit are this review and
the two explicitly authorized E14 status updates.

The capture author left the expectation pending and did not self-approve it.
The status changes below were applied only after this independent audit.

## Contract and anchors audited

The approved step `03-01` in
`docs/feature/remove-legacy-exec-workload-driver/deliver/roadmap.json:231-251`
requires one black-box capture that:

1. drives the checked-in healthy VM Service example through the built
   default-feature binary on native metal;
2. records stakeholder-visible VM Service success and owned cleanup against
   the exact post-cut SHA;
3. retains actual commands, output, substrate, SHA, and dirty state without
   inline specs, Rust crate imports, test harnesses, or integration assertions;
4. leaves E06 and E08 unchanged historical receipts; and
5. is evaluated by a different reviewer.

The E14 README anchors the claim to `S-SVM-01`, `US-SVM-1/2`, the post-cut
black-box event in the feature delta, and step `03-01`. `S-SVM-01` requires
`Accepted` followed by `Stable`, passing guest health, a peer VM Job receiving
the exact `SVM-E08-GUEST-OK` reply through the Service frontend, and removal of
all example-owned resources.

## Evidence inventory and integrity checks

The following retained artifacts were examined:

- `verification/expectations/E14-vm-service-post-greenfield-cut/README.md`
- `evidence/verification.yaml`
- `evidence/product-run.meta`
- `evidence/dirty-status.txt`
- `evidence/attempt-1-product-run.out`
- `evidence/attempt-2-product-run.out`
- `evidence/product-run.out`
- `execution-substrate`
- `verification/expectations/INDEX.md`
- the checked-in `examples/service-kind-vm-workloads/run-example.sh`,
  `prepare.sh`, README, and healthy Service/client specs

The receipt is internally coherent:

| Check | Result | Evidence |
|---|---|---|
| Pinned source SHA resolves | **PASS** | `git rev-parse` resolves `3f628ed08865aa0cdb3fbacd1f2cf989707d4a7a`. |
| Declared substrate | **PASS** | `execution-substrate` is `native-metal`; the transcript records fail-closed native x86_64/KVM preflight. |
| YAML receipt | **PASS** | `expectation_id: E14`, `execution_status: succeeded`, `execution_substrate: native-metal`, `executed_in_lima: false`, `runner_exit_code: 0`, and `anchor_status: present`; the file parses successfully. |
| Successful transcript identity | **PASS** | `attempt-2-product-run.out` and `product-run.out` are byte-identical; both are 81 lines and exit through the successful product path without an `xtask failed` block. |
| Actual command and target | **PASS** | `product-run.meta` records the exact `cargo xtask metal run -- examples/service-kind-vm-workloads/run-example.sh run healthy` command, target `ubuntu@151.115.99.251`, guest kernel/rootfs, timestamps, and attempt exits. |
| Dirty-state receipt | **PASS** | `working_tree_dirty: true` and `evidence/dirty-status.txt` record the dirty paths captured after the successful run. |
| Historical receipt protection | **PASS** | Source/example diff from the pinned SHA contains only E14/DELIVER receipt artifacts; E06 and E08 paths are unchanged. |
| Capture author self-approval | **PASS** | E14 README and INDEX were `pending` before this audit; no prior E14 evidence verdict existed. |

Selected SHA-256 identities at audit time were:

| Artifact | SHA-256 |
|---|---|
| `attempt-1-product-run.out` | `687027ad5ef13712e54378df16db249c457cc18eaaaf5fc7d6ce31371d7ac41a` |
| `attempt-2-product-run.out` / `product-run.out` | `aa570a72877cc9e3ed5d90250a4f556446371c24c181978ae633f2d83ac5f7f1` |
| `product-run.meta` | `a9fa100b581c0dff1affcdfacbcf4e41c112de896607c5802041bd1e759c4539` |
| `verification.yaml` | `67fefce3d28fc6108a4f38167b41bf22664b992dcf3db64c55a1bbb432b5f94a` |
| `dirty-status.txt` | `e1483c681962c8fe7dd3b1a7cd9d8b0bf3932f6534c26b5f957f6f11e0b74361` |
| `execution-substrate` | `23855df3e8416fed3a0ddb2cc85c650edf13c10f12c41346efdb039825c7ef17` |

## Claim-by-claim verdicts

### 1. Native-metal built default-feature product boundary

**SATISFIED.** The retained command is the repository's
`cargo xtask metal run --` boundary. The transcript records the native
x86_64/KVM fail-closed preflight, guest artifact preparation, and the direct
product journey. The checked-in `run-example.sh` builds
`overdrive-cli`'s `overdrive` binary with `cargo build -p overdrive-cli
--bin overdrive`, checks that `target/debug/overdrive` exists, starts
`overdrive serve`, then drives the public `deploy` and `workload describe`
commands. The evidence contains no inline workload spec, Rust test invocation,
expectation harness invocation, or crate import. `verification.yaml` correctly
declares native metal rather than Lima.

### 2. Service admission and `Accepted → Stable`

**SATISFIED.** In the successful product transcript, the Service deployment
PTY records `Accepted.` before the Service reports
`Service 'service-vm-e08' is stable` (the transcript's lines 28–36). The later
`workload describe` output identifies the same workload as kind `Service`,
with one running allocation and terminal `Stable`.

### 3. Guest TCP and HTTP health observations

**SATISFIED.** The Service description records both required passing guest
observations:

- `startup probe[0] tcp 0.0.0.0:18081 last=pass`
- `readiness probe[0] http GET http://0.0.0.0:18080/ready last=pass`

The checked-in Service spec supplies those TCP and HTTP probes, while the
transcript is the actual public observation rather than a test assertion.

### 4. Peer VM Job result and exact guest reply

**SATISFIED.** The checked-in `client-healthy.toml` is the VM Job fixture
targeting the checked-in Service name. The successful transcript records the
client deploy, `Job 'service-vm-e08-client'`, `Verdict: Succeeded`, and the
runner's exact external oracle:

`E08 PASS: peer VM Job received byte-exact SVM-E08-GUEST-OK through the Service frontend`

The `E08` label is the checked-in healthy journey's historical fixture label;
it does not overwrite or substitute the E14 expectation. The E14 receipt is
the post-cut SHA-pinned event and cites E06/E08 only as protected historical
receipts.

### 5. Owned cleanup

**SATISFIED.** The successful transcript records both owned workload stops,
removal of the private `svm-e08` materialization, and the checked-in runner's
complete zero-delta oracle:

`E08 teardown deltas: vm=0 probe=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0`

Every cleanup category named by the E14 expectation is present and zero. The
receipt is the example's stakeholder-visible cleanup result, not a substitute
for an internal lifecycle test.

### 6. Commands, output, substrate, SHA, and dirty state

**SATISFIED.** `product-run.meta` records the exact invocation, target, guest
artifacts, two attempt identities, exit codes, and timestamps. The successful
transcript retains the actual wrapper/product output. `verification.yaml`
records the exact post-cut source SHA, dirty state, substrate, execution
status, harness identity, seed, anchors, and exit code. The dirty-status
receipt is retained and truthfully reports a dirty capture; the reviewer does
not reinterpret it as a clean run.

### 7. First failed attempt is preserved append-only

**SATISFIED.** `attempt-1-product-run.out` is retained as a separate 116-line
transcript and `product-run.meta` records `attempt_1_exit: 1`. The first output
contains the same observed Service/Job success and zero-delta line, but also
records a signal-9 failed allocation and the final `metal run` exit status 1.
The README explicitly excludes that attempt from the qualifying verdict. The
second attempt is a new retained transcript with exit 0; it is not a silent
overwrite or a success inferred from cleanup alone.

### 8. E06/E08 historical receipts remain unchanged

**SATISFIED.** The E06 and E08 expectation trees have no diff from the pinned
post-cut SHA through the capture commit/current audit tree. The E14 README
explicitly states that E06 and E08 are historical receipts, not post-cut
proof. No E06/E08 evidence or status was edited by this audit.

### 9. No self-approval

**SATISFIED.** Before this audit, E14 remained `pending` in both its README
and `verification/expectations/INDEX.md`, and `verification.yaml` explicitly
states that the auditor must write the verdict. This artifact is the first
independent E14 evidence review. The status transition to `satisfied` is
performed only after this review is written.

## Findings and dispositions

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| — | — | No reachable evidence defect found within the approved E14 black-box contract. | **No finding.** All required claims are directly supported by retained native-metal output and provenance receipts. |

No implementation, architecture, API, test, mutation, or historical-receipt
change is required. The first attempt's signal-9 allocation is a retained
excluded history item, not a defect in the qualifying receipt.

## Iteration history

| Iteration | Basis | Verdict | Disposition |
|---:|---|---|---|
| 1 | Fresh independent audit of the E14 README, INDEX, checked-in example boundary, pinned receipt, both attempts, output, substrate, SHA, dirty state, and E06/E08 protection | **APPROVED** | No remediation required. Apply only the authorized E14 README/INDEX status updates. |

## Verification performed

Read-only checks performed:

- parsed `verification.yaml` and checked its identity/status fields;
- resolved the pinned source SHA and compared source/example paths from that SHA
  through the capture commit/current tree;
- compared `attempt-2-product-run.out` byte-for-byte with `product-run.out`;
- checked the successful output for the native-metal preflight, Accepted/Stable
  ordering, passing TCP/HTTP probes, Succeeded Job, exact reply, stop/removal
  receipts, and zero cleanup deltas;
- checked the first attempt's nonzero exit and signal-9 observation;
- verified E06/E08 trees are unchanged;
- verified E14 remained pending before this independent verdict; and
- recorded SHA-256 hashes for the retained evidence files.

No mutation testing was run. Mutation testing is the final DELIVER-wave gate,
not part of an individual evidence review.

## Final verdict and documentation action

**APPROVED.** The E14 receipt is a complete, anchored, native-metal,
SHA-pinned black-box capture. It demonstrates the checked-in VM Service's
accepted-to-Stable journey with passing guest TCP/HTTP probes, a successful VM
client receiving the exact guest reply through the Service frontend, and zero
owned cleanup deltas. Its actual command, output, substrate, SHA, dirty state,
and failed-attempt history are retained; E06/E08 remain immutable historical
receipts; and the capture author did not self-approve.

After the evidence review, only these catalogue status changes were made:

1. E14's README now says `Status: satisfied` and links this independent audit.
2. E14's row in `verification/expectations/INDEX.md` now says `satisfied` and
   links this independent audit.

The captured outputs, metadata, substrate declaration, E06/E08 receipts,
execution log, and implementation artifacts were not changed by the status
update.
