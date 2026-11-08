# Metal run isolation — DESIGN construction evidence

2026-11-08. This records completed measurements and failed attempts separately. It does not claim the runner feature is implemented, that the full test suite passed, or that proposed contracts are accepted. All native work used the existing permitted `cargo xtask metal run --` path; queued attempts waited/retried without interrupting another workspace. No production implementation or production test body changed. The final successful run was independently collected again with status 0; its retained raw transcript is [here](native-increment-o-recollect.log).

## Proven primitives and current composition

| Construction | Actual evidence | Result / limit |
|---|---|---|
| Shared ordinary / exclusive maintenance flock | Increment A, run `459aeff4c46c7a3c302e60d3`; [raw log](native-increment-a.log) | Two shared holders coexist; exclusive refuses until both release. This is the primitive, not the implemented v2 runner. |
| Actual Git identity cases | Increment C; [source](../../../../spike-scratch/metal-run-isolation/increment-c/probe.py), [output](local-identity-increment-c.log) | Same-branch linked worktrees differ; branch switch/detached linked identity stays stable; ordinary clone uses branch fallback; detached ordinary refuses; symlink alias has one identity. |
| Kernel local-mode namespace keying | Increment F, run `8d05ef07c5e1f3218a36aeec`; [raw output](f-local-vsock-namespace-probe.stdout) | Two simultaneously held local namespaces use identical CID `16785415` and fixed port 1234; a duplicate inside either namespace is refused; first owner ending leaves the second listener intact; initial namespace modes unchanged. |
| Real production VMM / paired disposable guests | Increment G in that same qualified run; [paired result](g-paired-result.json), [each actual output](g-pair-0.stdout), [second](g-pair-1.stdout) | Both ready-guest intervals overlap, guest boot IDs differ, both actual statuses are 0, same numeric CID is safe in different local namespaces. Observed physical module/nftables/BPF/routes/test-cgroup complement is unchanged. |
| Actual selected kernel body via existing group | [Wrapper input and complete guest transcript](f-wrapper-existing-kernel-cell.stdout), [nextest result](f-wrapper-existing-kernel-cell.stderr) | Existing `integration::probe_gold::the_probe_passes_and_leaves_no_residue_on_a_correctly_provisioned_node` executes in the real guest through existing `GuestFile`/`GuestPeer`/production VMM machinery; exact nextest executable/argv is retained. |
| Exact nonzero body result | [255 body transcript](f-wrapper-nextest-255.stdout), [nextest aggregate](f-wrapper-nextest-255.stderr) | Guest body returns 255; wrapper returns 255; actual nextest command returns 100. These are different owners, not a fabricated aggregate status. |
| Pure Sim remains host | Increment I; [actual output](i-pure-sim.stdout), [result](i-pure-sim.stderr) | Existing Sim beacon test execs in the initial host namespace without a guest. |
| Retry freshness and status | Increment I; [actual output](i-guest-255-retry.stdout), [nextest result](i-guest-255-retry.stderr) | Two actual attempts each return body status 255, with different fresh guest boot IDs; nextest returns its actual 100. |
| Actual compiled Linux test universe | [Native metadata](f-nextest-list-workspace.stdout), [build/list stderr](f-nextest-list-workspace.stderr) | `cargo nextest list --workspace --features integration-tests,kvm-tests --message-format json` compiled/listed 94 binaries, 4,640 identities including ignored cases. [Proposed placement partition](../design/placement-inventory.md) has no unclassified identity. This is not a test-pass claim. |

The real paired guests were launched through `GuestKit::boot` → production `CloudHypervisorVmm::create`, with its actual backend, identity and confinement. No userspace vhost preclaim is proposed. The direct-ioctl namespace primitive probe is deliberately separate from this production launch proof.

## Pinned source inspection completed

The uninitialised local submodule was not treated as inspected. Only required files were recovered from the official Linux repository at kernel.org, using the **exact** gitlink revision `028ef9c96e96197026887c0f092424679298aae8`. [Official tag ref capture](linux-v7-tag.txt) peels v7.0 to that same commit. [Each source URL, captured file and SHA-256](../design/kernel-source-provenance.json) is retained.

Inspected facts at v7.0:

- `drivers/vhost/vsock.c:74-88,688-690,788-824`: device opener namespace, namespace-aware CID lookup and atomic kernel claim.
- `include/net/af_vsock.h:279-323`: child-mode locking and local/global namespace reachability.
- `net/vmw_vsock/af_vsock.c:781-832,2824-2864,2923-2932`: actual bind scope, read-only current mode, private-parent child-mode configuration and inheritance.
- `drivers/vhost/vsock.c:753-785`: CID removal, socket orphan handling and namespace reference release at last device close.
- `kernel/cgroup/cgroup.c:6758-6759,6958-6965`: ordinary child cgroup inheritance; production VMM source does not move these fixture children into another physical host scope.

The initial upstream raw-GitHub attempts timed out/returned 429; the alternate official mirror succeeded. No different version or full kernel checkout was substituted. The 224-window allocation/claim proposal is removed because the inspected and measured kernel mechanism provides the needed ownership directly.

## Direct-native fixture premise revalidated

The original native identity/data body failed at peer readiness both inside the candidate local namespace (increment I) and in the unmodified initial namespace (increment K). That contrast rejected the hypothesis that the namespace caused this failure. The first observer-copy attempt also had a spike-workspace CWD error; it is not counted as a native comparison.

The corrected diagnostic-only observer copy in increment L captured the actual VMM ending:

```text
CreateApiServerSocket ... Permission denied (os error 13)
allocation child: uid=4200 gid=4200 mode=0750
its run parent: uid=0 gid=0 mode=0750
work parent: uid=0 gid=4200 mode=0710
```

The actual supplied run parent was not traversable by the production uid/gid. Assigning the allocation child cannot repair its parent. This is the existing native fixture construction path (`vsock_vmm_device.rs::launch_config_on`), not a speculative cancellation/failure model and not a reason to weaken confinement or substitute nested evidence.

Increment N changes only that test-support construction input in its copied fixture: run parent group 4200, mode 0710 before the existing production `Vmm::create`. It retains **128 MiB guest RAM**, existing exact UID/GID/device-group/no-new-privs/seccomp/Landlock assertions, backend and data round trip. It passes in both the initial and local host namespaces on the proposed `/srv/vm/metal/<run-id>` root. The actual API pathname is **103 bytes**, below the Linux 108-byte bound. [Complete raw native contrast](native-increment-n.log), retained source under `spike-scratch/metal-run-isolation/increment-n/`.

Thus bounded native fixture parent preparation is required fallout. It uses the same construction contract `GuestKit::boot` already implements. No production VMM interface, product semantics, native assertion, memory declaration or kernel guarantee was relaxed. Do not present the unchanged old fixture as passing, and do not fix it by running it nested.

## Completion-owner construction

The early increment-B attempt was refused by the old global lease (75 before START). Its queued retry, run `1c4340733c50faa1d91faa71`, completed and observed prior descendants gone before `ExecStopPost`; [actual output](native-increment-b-retry.log). It established ordering, not the necessity of a new protocol.

The later H/L construction uses the actual supported owner path: existing nextest timeout → selected wrapper → real GuestKit/VMM, under an existing-style `Type=exec`, `KillMode=mixed` transient unit. It first demonstrates that cancellation bypasses Rust Drop and leaves that wrapper's owned image/runtime files; then the same current systemd stop lifecycle plus a scoped finalizer removes only those roots. Prior VMM descendants are reaped in both cases. No forced test-only task abort, product Sim seam or fabricated control-plane state is involved.

One early H harness omitted `--pipe`, so its stdout capture could not prove the ready guest; that attempt was rejected and corrected. The successful L summary is retained in [the durable L output](native-increment-l.log). Its standalone remote raw files were removed by a subsequent one-way source sync before retrieval; the final O construction repeats it with **complete raw stdout/stderr also printed into the durable run log**, so no conclusion depends only on that lost raw capture.

## Final environment / durable raw capture gate

Increment N's additional environment case correctly failed because the spike selected a previously built host wrapper executable after its source changed; nextest rebuilt the libtest target but not that normal host executable. That is a preparation error in the spike, not evidence that the guest environment mechanism fails. The actual dispatcher must be built by its existing lane preparation owner; an attempt must never build it opportunistically or fall back to host execution.

Increment O explicitly prepares the current dispatcher, then verifies the selected guest receives the exact requested property seed/budget, backtrace setting and nextest identity/attempt through existing `env` + `GuestPeer::exec`. It independently repeats the actual timeout/cleanup contrast and retains full raw data in the durable output. **HELD.** Run `208ce3d2102a3f5d8f9f46b6` completed with program status 0; the dispatcher preparation, actual guest environment case and actual timeout/cleanup contrast each returned their expected 0 construction result. [Complete durable output with unfiltered raw case stdout/stderr](native-increment-o.log). Both timeout commands themselves returned actual nextest status 100; the construction succeeds because the expected failure/cleanup behaviours were observed, not because those test bodies passed.

## Mechanical validation and scope

`nwave-ai outcomes check-delta docs/feature/metal-run-isolation/feature-delta.md` exited 0 with [exact output](outcome-collision-check.log): `0 outcomes checked, 0 collisions found across 0 outcomes`. This is successful tool invocation with an empty checked set, not semantic no-duplication proof.

The installed density resolver returned `lean`, `ask-intelligent`, `explicit_override`. Required reference sections, explicit contracts/error mappings, C4 context/containers, focused proposed ADRs and the source/placement evidence exist. `git diff --check` and probe syntax checks passed. No implementation suite, mutation test, roadmap, DES events, project commit, production implementation or production-test-body edit was made. Pre-existing dirty work and the stopped service DELIVER remain untouched.

Contract-shaping construction work is complete. Acceptance remains gated on explicit P1–P5 user approval and independent design review. No independent unfinished inventory/source/probe task is relabelled `SPECIFICATION_AMBIGUITY`, and no unresolved specification ambiguity is handed downstream. Actual two-worktree execution through the implemented v2 runner is delivery verification; these measurements establish its contract-shaping construction premises.

## Independent-review premise and required formal-gate remediation

Review F1 identified six current Rust Gateway acceptance identities whose Sim ports still retain deterministic real frontend sockets. A single bounded existing-entry proof in native run `eb2bc9a1a2f401aa7935d2a3` reproduced status 101 / `AddrInUse` in one of two concurrent unchanged selected bodies while its twin passed. Exactly those six proposed placements now reuse `cgroup-default`; no production/test-body change or new context is introduced. The separate claimed `dst --only` route was disproved: both variants are deliberately absent from the actual parser/default catalogue; the real CLI returns 1 / unknown invariant before a seed. [Complete disposition and raw captures](../design/f1-disposition.md).

The required P2/P5 protocol model is permanent under `specs/quint/metal-run-isolation/`, not spike code. Its final recorded run passed 18/18 expected checks: exhaustive safety and two progress properties, two expected hazard violations and thirteen named scenarios. Current source hashes pass repository evidence verification. Exact shell/kernel/systemd projections, explicit model-only ghosts and native assumptions are pinned without a product API or Sim seam. Missing-rsync preparation status/retention and each executable component's Contract Shape are also pinned. [Bounded remediation and completed validation](../design/remediation.md). The proposal is ready for technical re-review; P1–P5 approval remains pending.
