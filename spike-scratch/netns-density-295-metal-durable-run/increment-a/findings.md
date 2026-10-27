# V-MDR-1 Findings — Transient Service Survives Controller Loss

Date: 2026-10-27  
Verdict: **WORKS**  
Promotion gate: **PROMOTE the transient-systemd supervisor decision to final DESIGN review**

## Question

After a transient service reads `START` and detaches stdin, stdout, and stderr
from `systemd-run --pipe`, does it remain active and retain its flock when the
fixture controller and output reader are destroyed?

## Execution boundary

- Route: existing `cargo xtask metal run --` path and canonical Metal lease.
- Host kernel: `7.0.0-38-generic`.
- Service manager: `systemd 259 (259.5-0ubuntu3.4)`.
- Fixture: one unique `/tmp/overdrive-metal-v-mdr1.*` directory, one temporary
  flock, and one uniquely named transient service.
- No management-network, host-kernel, product-test, model, resource, or
  production-source change.

## Predicted versus actual

| Observation | Predicted | Actual |
|---|---|---|
| Service state after controller/output-reader `SIGKILL` | `active` | `active` |
| Temporary flock after controller loss | held | held |
| Service terminal after five seconds | `complete` | `complete` |
| Temporary flock after service completion | acquirable | acquirable |
| Fixture directory after trap cleanup | absent | absent |
| Fixture unit after `--collect` | `LoadState=not-found` | `LoadState=not-found` |

The command returned 0 and printed `V_MDR1_VERDICT=WORKS`. Full output is in
`evidence/01-metal-run.log`; the exact process status is in
`evidence/01-metal-run.exit`.

## Edge case found

The first residue assertion incorrectly treated `systemctl show UNIT` process
success as proof that a unit exists. `systemctl show` returns zero for a missing
literal unit while reporting `LoadState=not-found`. That invalid check is
retained in `evidence/02-invalid-cleanup-check.*`; the corrected assertion and
clean result are retained in `evidence/03-cleanup-check.*`.

## Design implication

The contract-shaping premise holds on the actual Metal systemd boundary: after
descriptor detachment, the manager-owned transient service is independent of
the `systemd-run --pipe` controller and retains an inherited flock until its own
completion. The design must keep `--expand-environment=no`, explicit descriptor
detachment before `phase=running`, and systemd unit `LoadState` rather than a
bare `systemctl show` exit status as the absent-unit test.
