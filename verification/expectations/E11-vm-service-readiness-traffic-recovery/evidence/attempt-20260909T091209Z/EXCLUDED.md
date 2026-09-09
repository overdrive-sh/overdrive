# E11 earlier native attempt — excluded from executable evidence

## Attempt identity

- Attempt time: `2026-09-09T09:12:09Z`
- Expectation: `E11-vm-service-readiness-traffic-recovery`
- Substrate: native metal (as recorded by the DELIVER execution log)
- Source: `docs/feature/service-kind-vm-workloads/deliver/execution-log.json:279-290`

## Retention disposition

The raw receipt, complete product output, extracted ledger, runner log, dirty
status, and dirty patch/provenance for this attempt are not present in the
checkout or in the E11 evidence directory. The only recoverable record is the
append-only DES narrative. No artifact has been reconstructed from that
narrative, and this attempt is excluded from executable evidence and from any
passing claim.

The DES record reports a readiness withdrawal failure: the readiness row was
observed at `1788944302125` while the Service remained `Running` with
`Restarts 0`, and the during-window VM Job received the exact guest reply
(`Attempt Failed 43; Verdict Failed backoff exhausted`). It also records that
the zero-delta E11 cleanup completed and that COMMIT was withheld. Those facts
are provenance of an attempted outcome only; they are not a substitute for
the missing receipt, transcript, ledger, runner log, or dirty-state capture.

This file records the unrecoverable-attempt disposition without editing the
DES history or the earlier receipt/transcript.
