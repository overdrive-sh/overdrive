# E08 — VM guest boot failure is truthful and clean; exit 78 remains ordinary

**Surface:** E (end-to-end) · **KPI:** Q7 · **Status:** `pending`

## Expectation

Through the built operator binary, a fresh VM Job whose real production
outbound TPROXY append is rejected by a deterministic kernel-chain fixture is
terminal, never executes, and leaves no allocation residue. A second VM Job
whose deploy-selected custom rootfs makes the production resolver write fail is
reported as a pre-READY boot failure with the same execution/cleanup guarantees.
The complement is equally load-bearing: after successful READY/EXEC, an
operator command that exits 78 is reported as an ordinary Job result rather
than a setup-failure sentinel.

- Anchor: S-GTI-05, S-GTI-08a, and S-GTI-08b in
  `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
- Anchor: DESIGN Q7 in
  `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`
- Anchor: ADR-0088 and ADR-0089

## Verification contract

Use the same native, non-virtualized x86_64 KVM preflight and canonical
host-global lease specified by E07. The canonical metal Run, Sync, and every
supported direct bootstrap writer must acquire
`/run/lock/overdrive-metal-shared.lock` before any shared-tree mutation; raw
unleased writers are prohibited. E08 Run holds the same descriptor across
sync, preflight, every subcase, evidence, assertion-safe fixture restoration,
and final probes. Nested KVM is rejected; Lima is compile-only. Unknown or
contradictory virtualization signals block. Runtime evidence is invalid until
the universal Run/Sync/bootstrap writer boundary is implemented.

The eventual command is:

```text
verification/harness/run-expectation.sh E08
  -> cargo xtask metal run -- <E08 native runner>
  -> snapshot the exact host nft/FIB baseline
  -> preflight the appliance kernel's wrong-hook-base-chain TPROXY errno
  -> create the E08 production-named INPUT-hook base-chain fixture
  -> built overdrive serve
  -> overdrive deploy <fresh-guard-install-failure-spec>
  -> overdrive workload describe <guard-failed-id>
  -> stop serve; restore the exact recorded kernel delta; prove restoration
  -> built overdrive serve
  -> overdrive deploy <independent-sibling-vm-job>
  -> overdrive deploy <custom-rootfs-resolver-failure-spec>
  -> overdrive workload describe <failed-id>
  -> overdrive deploy <ready-then-exit-78-spec>
  -> overdrive workload describe <exit-78-id>
  -> overdrive job stop <id> for the sibling and any remaining intent
```

The fresh-install failure uses no injection flag or test-owned production
result. It begins only when strict snapshots prove the `ip overdrive-mtls`
table and its feature-owned fwmark rule/local route are absent; otherwise E08
blocks without mutation. Before fixture creation, the runner installs
EXIT/INT/TERM restoration traps, records normalized nft plus typed FIB state,
and preflights the pinned appliance kernel in a disposable table: a base chain
at `NF_INET_LOCAL_IN` receives the same encoded IPv4 TPROXY expression used by
production and must reject the `NFT_MSG_NEWRULE` append with
`-EOPNOTSUPP`. The probe is removed and exact baseline equality is rechecked.

The real fixture then creates `ip overdrive-mtls` and a **base** chain named
`prerouting` with `type filter hook input priority mangle; policy accept;`, plus
one structurally tagged foreign sentinel rule. Production
`ensure_base_chain` receives typed `EEXIST` for that name and continues; its
real shared exemption/output-chain/FIB work may run. The unchanged production
allocation installer then sends its real outbound TPROXY expression to the
INPUT-hook base chain. Linux `nft_tproxy_validate` permits TPROXY only at
`NF_INET_PRE_ROUTING`; upstream Linux
[`nft_tproxy_validate`](https://github.com/torvalds/linux/blob/master/net/netfilter/nft_tproxy.c#L294-L303)
and
[`nft_chain_validate_hooks`](https://github.com/torvalds/linux/blob/master/net/netfilter/nf_tables_api.c#L10898-L10913)
therefore pin `-EOPNOTSUPP` for this INPUT-hook base chain. Evidence must
contain the production
`MtlsInterceptInstallError::OutboundTproxyInstall` stage, outer operation
`append-egress`, inner netlink operation `append-rule`, and typed errno
`-EOPNOTSUPP`. The subcase fails if the production append is absent, succeeds,
returns another operation/errno, or if a test error seam is invoked.

Fixture delta includes the test-owned table/base chain/sentinel and any
production-created shared exemption, `output` chain, fwmark rule, or local
route. Product cleanup is observed first. After serve and capture shutdown, the
pre-installed trap removes only objects absent from the saved clean baseline
and requires normalized nft plus typed FIB equality to that baseline, even when
an assertion or signal terminates the subcase. On those exits the trap first
stops serve and capture processes, performs bounded best-effort product cleanup,
then restores the recorded fixture delta and runs the final equality probe.
Sentinel program identity must remain unchanged until fixture teardown. Fixture
restoration is never counted as allocation cleanup.

Required evidence, all verbatim and commit-pinned:

1. **Command/state:** baseline/probe/fixture commands, appliance kernel build,
   production install error, deploy and describe outputs, and 90-second bounded
   poll trails. The fresh guard case reaches terminal Failed with the real
   kernel install detail. Its durable Running row may be observed before the
   failed install supersedes it; that is not success. EXEC is never released
   and the operator command never runs. The resolver case shows one terminal Failed
   attempt, resolver-stage detail, exact available VMM exit code, and unchanged
   durable restart count; it never shows READY, Running, or operator EXEC. The
   complement proves READY precedes EXEC and EXIT and describe reports ordinary
   exit code 78, never boot failure or an unreported guest exit. Total deadline
   is 300 seconds for all three cases.
2. **Wire:** neither failure allocation emits a guest-originated frame, guest
   `EXIT` frame, operator marker, or peer-path cleartext; capture
   drop/truncation/ambiguity is failure. The exit-78 case may emit only traffic
   attributable after its READY and guard-live boundaries.
3. **Kernel:** strict complete snapshots prove the test-owned wrong-hook base
   chain and sentinel are structurally unchanged, the preflight and production
   append both report `-EOPNOTSUPP`, no allocation-scoped target rule survives
   the rejected fresh install, and final fixture teardown equals the recorded
   nft/FIB baseline.
   The resolver subcase additionally proves the ordered sequence of an
   independent running allocation's `(userdata, handle, normalized full
   program, packets, bytes)` snapshot is unchanged. Kernel reads use the strict
   D7 framing; a partial dump is no evidence.
4. **Cleanup:** within 30 seconds, no failed-allocation VMM, cgroup,
   rootfs clone, clone index, run directory, netns, tap, veth, route, nft rule,
   capture process/socket/fd, or custom-rootfs working copy remains for either
   failed allocation. Console
   absent/empty/unreadable/open/read/mid-read errors may change only bounded
   detail selection; none may mask the original rejection or cleanup result.
   Fixture restoration is separately delta-scoped and must not be confused
   with product cleanup.

## Evidence

None captured. This is a pending stub, not a narrated pass. Satisfaction needs
the native command and independent evidence-only review.
