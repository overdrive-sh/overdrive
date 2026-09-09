# E09 v2 — VM Service TCP truthfulness through one persistent control plane

Status: `pending` (native capture succeeded; independent evidence audit pending)

Surface: E — built-product end to end  
Execution substrate: `native-metal`  
Scope: 20 healthy/failure VM Service pairs cycled through one unchanged
`overdrive serve` process at concurrency 10

## Expectation

The v2 journey reuses the original E09 checked-in guest programs and source
specifications, but builds the default-feature product once, prepares the
immutable guest artifacts once, starts one control-plane process, and cycles
20 uniquely identified healthy/failure Service pairs through that same live
process in two ten-pair cohorts. It must not restart the control plane per
case, retry a pair, replace a pair, or discard a pair.

Each healthy Service must render Stable with the real guest TCP startup probe
as its witness, report passing guest TCP and HTTP observations through public
`workload describe`, and serve the exact `SVM-E08-GUEST-OK` reply to its
checked-in VM peer Job through the Service frontend. Each failure Service must
return a nonzero startup result, publish `StartupProbeFailed` for guest TCP
port `18999`, never render Stable, and deny the negative-control VM peer Job's
backend connection. The public describe may retain the existing
replacement-start trajectory (`Running` with a positive restart count and a
same allocation's prior `Failed` snapshot); that trajectory is accepted only
alongside the failed startup-probe observation and the nonzero deploy result.
Each worker completes healthy peer and Service stop plus runtime cleanup before
announcing `healthy-cleanup-complete`; the cohort owner releases failure
submissions only after all ten workers announce that marker. Existing public
deploy, describe, and `job stop` operations are the only product boundaries
driven by the expectation.

The ledger has one deterministic row per input trial, retains `failed` and
`not-run-cancelled` outcomes, and rejects retries or discarded failures. It
records the control-plane PID/start identity, public terminal observations,
probe targets, peer Job outcomes, allocation-scoped runtime cleanup, timing,
cohort concurrency, and durable-store growth. Runtime snapshots cover Cloud
Hypervisor processes, workload cgroup scopes, VM run directories, network
names, attached BPF programs, nftables rule identities, rootfs clone
staging/index entries, loop devices, mounts, and serve-store totals.
Snapshots are compared only after the cohort's owned workers have stopped;
durable Service/Job records intentionally retained by the existing API are
reported separately from transient allocation/VM resources. Final cleanup is
measured before and after the single control-plane shutdown.

The remote example owner has a 1200-second (20-minute) setup-and-trials budget, followed
by at most 60 seconds of bounded cleanup grace. A timeout is nonzero and the
runner retains partial ledger, timing, identity, and transcript output before
ordinary materialization cleanup. Transport/bootstrap time is outside the
remote owner budget and the parent wait is bounded separately to allow both
remote windows to complete. This 20-pair sample is bounded functional
acceptance, not reliability, native capacity, or throughput proof; independent evidence audit remains pending. A longer soak, if later desired, is separate
optional activity and is not a gate.

- Anchor: S-SVM-25 in `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`
- Anchor: US-SVM-1 and K1 in `docs/feature/service-kind-vm-workloads/feature-delta.md`
- Anchor: ADR-0090 registration-time VM target projection
- Anchor: ADR-0083 allocation-scoped VM artifact ownership
- Anchor: GH #257 service-kind-vm-workloads

## Verification

`runner.sh` first validates the unchanged source bundle, then invokes the
operator-runnable v2 example through `cargo xtask metal run --`. Its remote
shell command applies `timeout --signal=TERM --kill-after=60s 1200s` around the
example process group for descendant termination and forwards
`SVM_E09_V2_CONCURRENCY=10`. The runner
captures output and extracts the v2 ledger even when the remote owner times
out; only a complete 20-row, all-pass capture can return success. The
expectation does not run Cargo tests, a Rust test binary, or any
`overdrive-*` crate and does not claim status `satisfied` automatically.

The companion `examples/service-kind-vm-workloads-v2/test-scheduler.sh` and
`verification/harness/test-e09-v2-runner.sh` are host-safe synthetic checks.
They prove scheduler barriers, bounded overlap, single-process identity,
out-of-order completion, deterministic partial ledgers, and remote timeout
ownership only; they are not substitutes for native product capture.
