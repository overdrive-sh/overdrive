# Admission rerun — waiter observation occurred after maintenance completed

This directory preserves 216 raw files copied byte-for-byte from
`/tmp/metal-native-remaining-20261109T020736Z-admission`. The root foreground
driver reported exit 1. This attempt reached and passed the ordinary guest
body-start guard and the exclusive owner’s `waiting_for_lease` observation,
then failed to capture an ordinary waiter while maintenance was active.

## Captured invocations

The target was `ubuntu@151.115.99.251`. All three runs used the main source at
commit `349e7cc2fd9fc2af4bbde08f2397ab96b801cf13`, digest
`eb2dd3f66bb1f75fa0383d1a592a0c3f77cbbf63d716438b8a3682f03bee0ea8`, checkout
key `e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410`:

| Role | Run ID | Class | Command result |
|---|---|---|---|
| Timeout owner | `b162f48b28d9859edf1403a8` | ordinary | The selected Nextest case timed out at 10.002 seconds; outer status 100, as designed. |
| Maintenance owner | `5496e8a54617b3e3c113da61` | exclusive | Command completed with status 0. |
| Ordinary request | `af4b745513beabed81144db5` | ordinary | `cargo check -p xtask` completed with status 0; compile output reports 13.22 seconds. |

`ordinary-running.json` records the first owner active in `running` with its
selected body marker in its owned guest console. `maintenance-waited.json`
records the exclusive owner in `waiting_for_lease` while the timeout owner
remained active.

## Exact failure and boundary

The maintenance body checks that the predecessor unit is inactive and the
three owned roots are absent, prints `METAL-MAINTENANCE-START`, then sleeps for
three seconds. The driver waits for that marker to appear in
`maintenance.stdout` before launching the ordinary waiter. The bootstrap
collects command output after the durable command finishes, so the marker is
not available to that polling loop while the maintenance body is still
sleeping.

The first valid waiter observation, `waiter-34.json`, records the maintenance
unit inactive with a complete receipt and the ordinary request only in
`preparing`. Later valid observations do not show the required simultaneous
active/running exclusive owner and ordinary `waiting_for_lease` request. The
driver never wrote `unadmitted-waiter.json` and failed at that required-file
guard. The raw capture therefore does not prove the ordinary waiter remained
unadmitted during exclusive maintenance, and it does not prove the
admission/placement journeys passed.

The relevant owner path is `infra/metal/tests/admission_journey.sh:76-99`.
The selected command output is retained and collected by
`infra/metal/bootstrap.sh:1157-1168`; the durable runner captures command
output to its log before final collection in `infra/metal/lease-holder.sh`.
This capture identifies a driver observation-timing limitation; it does not
demonstrate an admission-lock implementation failure.
