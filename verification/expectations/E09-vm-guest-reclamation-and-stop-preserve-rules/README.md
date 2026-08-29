# E09 — VM reclamation and Job stop preserve exact sibling rules

**Surface:** E (end-to-end) · **KPI:** D6 lifecycle · **Status:** `pending`

## Expectation

An unclean control-plane restart reclaims an unsupervised VM while durable
intent still stands and re-drives the **same allocation id**. Successful
reinstall protects its first mesh flow; failed reinstall never releases EXEC.
`overdrive job stop <id>` then removes exactly the target guard, and repeated
stop without a guard is idempotent, while every sibling rule remains exactly
unchanged.

- Anchor: S-GTI-06a, S-GTI-06b, S-GTI-12a, and S-GTI-12b in
  `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
- Anchor: DESIGN D6 and D7 in
  `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`
- Anchor: ADR-0089

## Verification contract

Use E07's native, non-virtualized x86_64 KVM preflight and canonical 120-second
host-global lease. Every metal Run, Sync, and supported direct bootstrap writer
must acquire `/run/lock/overdrive-metal-shared.lock` before shared-tree
mutation; raw unleased writers are prohibited. E09 Run holds the same
descriptor across sync, both control-plane processes, unclean termination,
restart, evidence collection, stop, assertion-safe fixture restoration,
cleanup, and final probes. Runtime evidence is invalid until this universal
Run/Sync/bootstrap writer boundary is implemented.

The eventual command is:

```text
verification/harness/run-expectation.sh E09
  -> cargo xtask metal run -- <E09 native runner>
  -> successful/sibling-preserving journey:
  -> built overdrive serve --data-dir <durable-dir>
  -> overdrive deploy <target-vm-job> and <sibling-vm-job>
  -> uncleanly terminate serve; restart serve with the same data dir
  -> overdrive workload describe <target-id>
  -> overdrive job stop <target-id> twice
  -> failed-reinstall journey, in a fresh sibling-free durable dir:
  -> built overdrive serve --data-dir <failure-durable-dir>
  -> overdrive deploy <quiescent-target-vm-job>; await Running and snapshot
  -> uncleanly terminate serve; install the wrong-hook base-chain fixture
  -> restart serve with the same failure durable dir; do not deploy again
  -> overdrive workload describe <same-target-id>
  -> prove product cleanup; restore exact target-filtered nft/FIB baseline
```

The failure journey is isolated from the sibling-preservation journey because
its fixture temporarily replaces shared nft state. It has exactly one target
and no sibling. The target command remains quiescent and capture remains armed
during the unclean-stop/fixture window. Before any mutation the runner records
the target's Running row, standing durable intent, allocation id, boot epoch,
complete normalized nft/FIB state, target rule handle, and the exact expected
post-cleanup state `filter_target(baseline)`, which removes the target nft rule
and target-scoped FIB routes while retaining every shared and unrelated object.
It installs EXIT/INT/TERM restoration traps before touching nft state.

After uncleanly terminating serve, the runner does not invoke deploy,
`overdrive workload restart`, natural Job crash, or restart budget. It deletes
the captured production table only in this sibling-free subcase and recreates
`ip overdrive-mtls` with the production chain name `prerouting` as a base chain
at `type filter hook input priority mangle; policy accept;`, plus a structurally
tagged foreign sentinel. The pinned appliance-kernel preflight must first prove
that the same encoded TPROXY append to an INPUT-hook base chain returns
`-EOPNOTSUPP`. Restarting the built server against the unchanged durable data
must then produce the boot-epoch `StoppedBy::PlatformReclaimed` ending and a
production `Action::RestartAllocation` dispatch for the same allocation id.
The dispatch trace, unchanged intent/data dir, absence of a second deploy, and
same id jointly prove the restart install gate rather than fresh install.

The unchanged restart-arm installer must reach
`MtlsInterceptInstallError::OutboundTproxyInstall`, outer operation
`append-egress`, inner netlink operation `append-rule`, errno `-EOPNOTSUPP`.
The test fails if this call is absent, succeeds, uses another operation/errno,
or routes through an injection seam. The replacement guest's durable Running
row may be visible before install failure supersedes it; final Failed with the
typed reinstall detail is mandatory, while EXEC/operator marker and all guest
frames/cleartext are forbidden.

After product cleanup is independently proved, the restoration trap deletes
only the wrong-hook fixture and recreates the exact captured correct shared
state minus all target-scoped nft/FIB objects; it never resurrects a dead
target. Normalized nft plus typed FIB state must equal the precomputed
`filter_target(baseline)`. On assertion failure or signal the trap first stops
serve and captures, performs bounded best-effort product cleanup, then restores
and runs the same final equality probe. The separately executed
successful/stop journey owns sibling preservation; the destructive failure
journey must not claim it.

Required evidence, all verbatim and commit-pinned:

1. **Command/state:** serve/deploy/describe/Job-stop and fixture argv/outputs;
   durable intent and pre-restart allocation id; boot epoch and
   platform-reclamation ending; exact `RestartAllocation` dispatch; the same
   allocation id on re-drive; successful guard-before-EXEC or terminal failed
   reinstall with no EXEC. A transient Running row before failed install is
   permitted and recorded. State convergence is bounded to 120 seconds per
   restart and each journey has a 300-second total deadline.
2. **Wire:** on success, the first post-reclamation directional tuple satisfies
   the complete D7 rule/capture equality and has TLS/no-cleartext evidence. On
   failed reinstall, the quiescent original and replacement emit no
   guest-originated frame, operator marker, or peer-path cleartext frame.
3. **Kernel:** complete guarded snapshots before reclamation, after reinstall,
   before stop, and after stop. Target deletion is exact. If `B` is the ordered
   before-stop sequence of `(userdata, handle, normalized full program,
   packets, bytes)` and `A` is the after-stop sequence, require
   `A == filter(B, handle != target_handle)` and require the target handle
   absent. This preserves surviving sibling values and relative order without
   claiming an impossible unchanged absolute ordinal. Reset, replacement,
   loss, partial dump, or generation ambiguity fails.
4. **Stop inverse:** `overdrive job stop <id>` reports Stopped then
   AlreadyStopped for a no-rule terminal attempt, without creating a rule or
   changing siblings.
5. **Failed-reinstall kernel/restoration:** complete snapshots record the
   before baseline, target-filtered expected baseline, wrong-hook fixture,
   production `append-egress`/`append-rule -EOPNOTSUPP`, absence of a target
   rule, product cleanup, and exact assertion-safe restoration. Any sibling in
   this destructive subcase, skipped append, partial dump, unexpected errno,
   or final nft/FIB inequality fails.
6. **Cleanup:** within 30 seconds no target VMM, cgroup, clone/index, run
   directory, netns, tap, veth, route, nft rule, capture process/socket/fd, or
   temporary durable-data copy remains. The sibling remains live until its own
   separately recorded stop.

## Evidence

None captured. This pending stub becomes satisfiable only after the canonical
writer lease and roadmap-owned native runner exist and an independent reviewer
audits the evidence.
