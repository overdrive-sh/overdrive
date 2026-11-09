# Admission rerun — transient admitted-owner deactivation was not sampled

This directory preserves 142 raw files copied byte-for-byte from
`/tmp/metal-native-remaining-20261109T024233Z-admission`, including
`foreground-invocation.json`, `foreground.log`, owner snapshots, query errors,
and controller streams. The invocation record reports driver exit 1 and source
HEAD `c5df6e910629acfdb55ba7b42289ff711e117100`; its dirty source status is
retained there. This failure is separate from the 020736Z buffered-output
capture: this driver used owner snapshots and passed the selected guest
body-start guard.

## Captured owners and source

The observed ordinary owner was run `1ebaf3ba0f13c041955e2fe8`; the exclusive
maintenance owner was run `a3d26d990cc75b05a2fd435d`. Both owner records name
commit `c5df6e910629acfdb55ba7b42289ff711e117100`, source digest
`09a28a30607e36ac3b7585052552a8f0901da00f0af445141c9f1c7ca5616140`, and
checkout key
`e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410`.

`ordinary-running.json` records the selected guest body marker while the
ordinary unit is active and running. `maintenance-waited.json` records the
exclusive owner in `waiting_for_lease` while the ordinary owner remains
active. The maintenance owner was therefore registered and waiting behind the
admitted ordinary owner.

## Exact missed state

The driver's required stopping guard is at
`infra/metal/tests/admission_journey.sh:76-95`. The first two saved stopping
snapshots show the ordinary unit active in `running`, with no terminal or
completion record; its process complement still contains `cargo-nextest`, the
guest-suite process, and the live Cloud Hypervisor VMM:

| Snapshot | Local capture time | Ordinary owner state |
|---|---|---|
| `stopping-0.json` | `02:45:13.347Z` | Active, running; no terminal or completion. |
| `stopping-1.json` | `02:45:13.893Z` | Active, running; no terminal or completion. |
| `stopping-2.json` | `02:45:15.390Z` | Inactive; terminal command exit 100 and complete receipt at `02:45:15Z`; no processes. |

Across 36 numbered stopping attempts (25 yielded valid JSON snapshots), none
records the ordinary unit as `deactivating`. The guard fails on
`stopping-0.json` and `stopping-1.json` at the `ActiveState == deactivating`
assertion. The next valid snapshot is already inactive and complete. Terminal
and completion timestamps have only one-second precision and both fall in the
same second, so this capture does not establish the exact subsecond duration
of systemd deactivation. The gap between the last active sample and first
inactive sample is 1.497 seconds.

The exclusive owner is active in `waiting_for_lease` in the first two stop
samples and has not yet acquired the barrier writer descriptor. In
`stopping-2.json` through `stopping-11.json`, its main process holds FD 8 on
`/run/lock/overdrive-metal-shared.lock` with `FLOCK ADVISORY WRITE`, but the
ordinary owner is already complete. The exclusive phase changes to `preparing`
in `stopping-12.json`. The `deactivating` sample at `stopping-33.json` belongs
to the exclusive owner itself; it has terminal exit 0, then appears inactive
and complete in `stopping-34.json`.

The driver never wrote `maintenance-stopping-predecessor.json`, so it did not
start the ordinary waiter at line 98. No snapshot proves an admitted
predecessor was deactivating while an ordinary waiter remained unadmitted
behind the exclusive lock. The placement journey was not invoked. E4 and full
native acceptance therefore remain unproven by this attempt.

## Owner path and interpretation

The ordinary durable owner writes its command terminal only after the
selected command and output finalization return
(`infra/metal/lease-holder.sh:1114-1158`). The existing systemd `ExecStopPost`
finalizer is installed by `infra/metal/bootstrap.sh:811-829`; it writes the
completion receipt. The exclusive owner acquires the canonical barrier and
checks admitted ordinary unit liveness in
`infra/metal/lease-holder.sh:414-455, 891-907`.

The external driver takes one read-only owner snapshot per SSH call
(`infra/metal/tests/admission_journey.sh:76-89`); that observation reads the
systemd unit state once via `systemctl show`
(`infra/metal/tests/observe_host.py:96-100`). The capture supports a transient
observation miss: the ordinary owner was still executing while its unit was
active, then the next saved observation saw it fully stopped with terminal
and completion. It does not show a production cleanup or admission defect.
The existing read-only observation path needs to capture the brief
`deactivating` state while preserving the E4 assertions. No production
lifecycle hook, phase change, or contract relaxation is evidenced here.
