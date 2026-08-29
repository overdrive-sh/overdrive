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

Use E07's native, non-virtualized x86_64 KVM preflight and global 120-second
command-lifetime lease. Nested KVM is forbidden and Lima is compile-only. The
lease spans both control-plane processes, unclean termination, restart,
evidence collection, stop, and final cleanup.

The eventual command is:

```text
verification/harness/run-expectation.sh E09
  -> cargo xtask metal run -- <E09 native runner>
  -> built overdrive serve --data-dir <durable-dir>
  -> overdrive deploy <target-vm-job> and <sibling-vm-job>
  -> uncleanly terminate serve; restart serve with the same data dir
  -> overdrive workload describe <target-id>
  -> overdrive job stop <target-id> twice
```

The failure subcase repeats the unclean restart with a deterministic real
kernel environment that rejects the production guard reinstall. It does not
use `overdrive workload restart`, natural Job crash, or restart budget; each is
the wrong allocation-identity route.

Required evidence, all verbatim and commit-pinned:

1. **Command/state:** serve/deploy/describe/Job-stop argv and outputs; durable
   intent and pre-restart allocation id; platform-reclamation ending; the same
   allocation id on re-drive; successful guard-before-EXEC or terminal failed
   reinstall with no EXEC. State convergence is bounded to 120 seconds per
   restart and the total scenario to 300 seconds.
2. **Wire:** on success, the first post-reclamation directional tuple satisfies
   the complete D7 rule/capture equality and has TLS/no-cleartext evidence. On
   failed reinstall, no guest-originated or peer-path cleartext frame appears.
3. **Kernel:** complete guarded snapshots before reclamation, after reinstall,
   before stop, and after stop. Target deletion is exact. Every sibling retains
   tag, handle, normalized full program, packet/byte counter, and order; reset,
   replacement, loss, partial dump, or generation ambiguity fails.
4. **Stop inverse:** `overdrive job stop <id>` reports Stopped then
   AlreadyStopped for a no-rule terminal attempt, without creating a rule or
   changing siblings.
5. **Cleanup:** within 30 seconds no target VMM, cgroup, clone/index, run
   directory, netns, tap, veth, route, nft rule, capture process/socket/fd, or
   temporary durable-data copy remains. The sibling remains live until its own
   separately recorded stop.

## Evidence

None captured. This pending stub becomes satisfiable only after the
roadmap-owned native runner exists and an independent reviewer audits the
evidence.
