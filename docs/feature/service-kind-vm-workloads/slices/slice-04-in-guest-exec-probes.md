# Slice 04 — In-guest Exec probes

**Story:** US-SVM-4
**Priority:** P3, highest mechanism uncertainty
**Effort:** 6 hours maximum after the spike and DESIGN
**Dependencies:** Slice 01; accepted DESIGN from the H1–H4 spike

## Goal

Ana's declared Exec probe runs in the guest workload context and its actual
termination result feeds startup, readiness, or liveness without using the VMM
status.

## IN

- Direct argv execution with no implicit shell/stdin/env/cwd behavior.
- Exit zero, nonzero exit, signal, and spawn/not-found outcomes.
- All three existing probe roles consume the guest result identically.
- Bounded request/result behavior; no stdout/stderr product surface.
- Real production `serve` plus `deploy` composition.

## OUT

- Interactive guest exec, remote shell, or arbitrary operator commands.
- Exact transport/protocol/API shape beyond the accepted DESIGN.
- Custom environment, stdin, working directory, or output streaming.

## Learning hypothesis

Failure disproves that a narrow persistent in-guest execution path can coexist
with the long-running Service command under the production VM lifecycle.
Success confirms true Exec semantic parity rather than a network-probe
substitute.

## Acceptance

- `/usr/local/bin/check-ledger --shard eu-1` exits 0 and passes.
- The same command with `--shard missing` exits 7 and fails with a bounded
  nonzero category.
- A signal and a missing executable fail distinctly; workload and VMM survive.
- An explicitly named shell behaves as argv; no shell is inserted otherwise.

## Dogfood

Ana deploys `fraud-vm.toml`, changes guest-local ledger state, and observes the
real Exec readiness result change without altering its listener.

## Reference class

Overdrive-init's existing one workload command, Kubernetes/CRI ExecSync
semantics, and KubeVirt/Kata in-guest process execution precedents.

## Pre-slice spike

Prove H1–H4: simultaneous lifecycle and repeated control sessions; correlated
results while Service runs; descendant cleanup; bounded concurrency/overload.
Timebox: 4 hours. DESIGN chooses the exact mechanism from recorded evidence.
