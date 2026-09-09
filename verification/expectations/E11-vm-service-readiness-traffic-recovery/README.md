# E11 — VM readiness withdraws and restores real traffic within its bound

Status: `pending` (DELIVER step 03-01; native capture requires independent audit)
Surface: E — built-product end to end
Execution substrate: `native-metal`
Walking skeleton: no; bounded lifecycle journey

## Expectation

The checked-in readiness workload returns 204, then 503, then 204 in fixed
ten-second phases. After initial Stable, describe must show readiness
`Pass -> Fail -> Pass`; checked-in plaintext VM client Jobs call the Service
name through its frontend and receive `SVM-E08-GUEST-OK`, then stop reaching
the known-failed backend within the declared one-second interval plus
one-second timeout, then receive the reply again within the same bound after
recovery. Running and Stable remain unchanged throughout.

- Anchors: S-SVM-27A, S-SVM-27B, and S-SVM-27C in
  `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md`, mapped
  respectively to the before, unavailable, and recovered VM client Jobs.
- Anchor: US-SVM-3 and K3 in `docs/feature/service-kind-vm-workloads/feature-delta.md`.
- Anchor: accepted lifecycle gate ownership in the architecture summary.

## Verification

The activated `readiness-recovery` mode samples describe and deploys the three
checked-in `[job] + [vm]` clients at the corresponding phases. Each runs the
same static client installed by the bundle's one E07-style private-rootfs
preparation and uses only an ordinary plaintext socket to the Service name;
the negative-control client attempts at least every 100 ms for its bounded
1.5-second window. The runner stores the native-metal transcript in
`evidence/product-run.out` and extracts `evidence/readiness-recovery.tsv`.
The timestamped ledger must show both readiness transitions observed within
two seconds, public client success before withdrawal and after recovery,
public negative-control success only when no exact guest reply was received
during the failed window, no restart, and a zero cleanup delta. Passing is
exactly 2/2 bounded transitions and zero failed-window backend hits; an
independent audit must review the captured evidence before changing this
status to `satisfied`.
